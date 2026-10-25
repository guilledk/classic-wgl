#version 300 es

precision mediump float;

in highp vec2 vTexCoord;
in highp vec3 vLightPos;

uniform sampler2D tex_sampler;
uniform sampler2D depth_sampler;
uniform sampler2D normal_sampler;

uniform float ghost_alpha;
uniform float use_depth_map;
// Whether this draw participates in scene lighting at all.  The 2D/baked
// `draw_sprite` path (cursor, HUD, UI) sets 0: it never writes a light-space
// position, so its `vLightPos` is meaningless.  This used to be implied by
// `use_normal_map == 0`, which also silently unlit every *world* sprite that
// happens to ship without a normal map.
uniform float use_lighting;
uniform float use_normal_map;
// Sprite normal maps are baked in Blender world space (`render/materials.py`
// emits `Geometry.Normal` with no view transform) — the same metric world space
// the terrain normals live in — so they are used as-is, with no normal matrix.
uniform vec3 ambient_color;
uniform vec3 light_direction;
uniform vec3 light_color;
uniform vec3 selection_color;

#ifdef INSTANCED
// Instanced iso sprites (`sprite_batch.rs`): the per-sprite values arrive as
// flat varyings (see `direct_tex.vert`), under the uniform path's names.
flat in highp vec4 vUvRect;
flat in highp vec4 vContent;
flat in vec4 vTint;
flat in highp vec4 vMisc;
flat in highp vec4 vOutline;
#define depth_base vMisc.x
#define depth_scale vTint.w
#define selected vMisc.z
#define tint vTint.rgb
#define outline_delta vOutline.xy
#define vSource vOutline.zw
#define vGrid vMisc.w
#define tile_set_size vUvRect.yz
#ifndef ID_PASS
// The ghost pass's group test: the id of the sprite group that last passed
// the normal pass's depth test at this pixel (`Gfx::begin_ghost_ids`), the
// stencil `NOTEQUAL ghost_group` test of the per-draw path.
uniform highp sampler2D ghost_ids;
uniform float use_ghost_ids;
#endif
#else
uniform vec2 tile_set_size;
uniform float tile_id_flat;
uniform float use_uv_rect;
uniform highp vec4 uv_rect;
uniform highp vec2 trim_offset;
uniform highp vec2 source_size;
uniform highp vec2 content_size;
/// The sprite's ground-anchor window depth `[0, 1]` (see the `gl_FragDepth`
/// re-anchor below).
uniform float depth_base;
// Window units per `gray − 0.5` of the depth map: 1 for a legacy global sheet,
// `2R / (DEPTH_NEAR − DEPTH_FAR)` for an anchor-relative one.
uniform float depth_scale;
// Per-sprite tint (multiplied onto the albedo before the Lambertian term).
// Defaults to (1,1,1) — a no-op for every existing asset.  Tintable sprites
// ship a grayscale albedo and set this at runtime (see IsoSprite.color).
uniform vec3 tint;
// RTS selection silhouette: when `selected` is set, transparent edge texels
// adjacent to an opaque texel (within `outline_delta` sheet-UV) are drawn in
// `selection_color` as a per-sprite outline.
uniform float selected;
uniform vec2 outline_delta;
#endif

uniform highp sampler2DShadow shadow_map;
uniform mat4 light_view_proj;
uniform float shadow_bias;
uniform float shadow_strength;
uniform vec2 shadow_texel;
uniform float use_shadow;
uniform float shadow_debug;
uniform float shadow_normal_offset;

#define MAX_LIGHTS 256

struct Light {
    vec4 pos_radius;
    vec4 color_intensity;
    vec4 dir_cone;
};

layout(std140) uniform LightBlock {
    vec4 count;
    Light lights[MAX_LIGHTS];
} u_lights;

out vec4 fragColor;

// --- BEGIN SHARED LIGHTING (byte-identical in sheet.frag / iso_tilemap.frag /
// --- mesh.frag / particle.vert; pinned by `lit_shaders_share_the_lighting_block`) ---
//
// `p` and `l.pos_radius.xyz` are both **metric world space** (+Z up, metres),
// so `length` is a true distance and `dot(n, L)` a true cosine.  They previously
// lived in the isometric screen space, which compresses y by 2x; every point
// light was therefore an ellipsoid evaluated as if it were a sphere.
vec3 evaluateLight(Light l, vec3 n, vec3 p) {
    vec3 toLight = l.pos_radius.xyz - p;
    float dist = length(toLight);
    float radius = l.pos_radius.w;
    // Smooth windowed falloff: `w(d)^2 / (1 + d^2)`, `w = saturate(1 - d^2)`,
    // `d = dist / radius`.  Softer than the previous `w = saturate(1 - d^4)` /
    // `1 + 8 d^2` form, whose quartic window + 8x inverse-square term made the
    // light read as a hot core with a sharp cutoff at roughly a tenth of the
    // radius (nearly invisible for a low light like the rocket's flame, whose
    // Lambertian grazing angle already shrinks the ground pool).  The quadratic
    // window + unit inverse-square term keeps a bounded, C0 edge while letting
    // the light actually span the authored `radius`.
    float attenuation = 1.0;
    if (radius > 0.0) {
        float d = dist / radius;
        float d2 = d * d;
        // Radius early-out: at `d2 >= 1` the window is exactly 0, so the
        // light contributes exactly 0 and skipping the cone/Lambert work
        // changes no pixel.  The test reuses the falloff's own `d2` (not a
        // separate `dot(toLight, toLight) >= radius * radius`, which rounds
        // differently at the edge), so it is bit-exact.
        if (d2 >= 1.0) {
            return vec3(0.0);
        }
        float window = clamp(1.0 - d2, 0.0, 1.0);
        attenuation = window * window / (1.0 + d2);
    }
    vec3 L = toLight / max(dist, 0.0001);
    float cone = 1.0;
    if (l.dir_cone.w > 0.0) {
        // `dir_cone.xyz` is the beam direction (light -> target) and `w` the
        // half-angle of the full-intensity cone; the edge softens out to 1.25x.
        float cosTheta = dot(-L, normalize(l.dir_cone.xyz));
        cone = smoothstep(cos(l.dir_cone.w * 1.25), cos(l.dir_cone.w), cosTheta);
    }
    float diff = max(dot(n, L), 0.0);
    return attenuation * cone * diff * l.color_intensity.rgb * l.color_intensity.a;
}

// Every active light, in UBO order.  The screen-tile culled variant is
// `evaluateLightsTiled` (the LIGHT GRID block of the lit fragment shaders);
// it falls back to this loop wherever it has no list.
vec3 evaluateLights(vec3 n, vec3 p) {
    vec3 acc = vec3(0.0);
    int cnt = int(u_lights.count.x + 0.5);
    for (int i = 0; i < MAX_LIGHTS; i++) {
        if (i >= cnt) {
            break;
        }
        acc += evaluateLight(u_lights.lights[i], n, p);
    }
    return acc;
}
// --- END SHARED LIGHTING ---

// --- BEGIN LIGHT GRID (byte-identical in sheet.frag / iso_tilemap.frag /
// --- mesh.frag; pinned by `lit_fragment_shaders_share_the_light_grid`) ---
//
// Screen-tile light culling.  The layout is declared once, in
// `classic_gfx::light_grid` (`LightGrid`); the constants below mirror it and are
// pinned by `shader_constants_match_the_layout`.  `light_grid` is one R32UI
// texture read as a flat word array, `LIGHT_GRID_ROW` words per row: word `t`
// (tile `t = ty * tiles_x + tx`) is the tile header `offset << 9 | count`, and
// its light indices (bytes, ascending, low byte first) start at word `offset`.
// `count == LIGHT_GRID_OVERFLOW` marks a tile with no list: it runs the full
// `evaluateLights` loop, as does every fragment while `light_grid_on` is 0.
// The lists are conservative and ascending, so the sum is bit-identical to
// the full loop (the lights a list leaves out contribute exactly 0).
#define LIGHT_GRID_ROW 1024
#define LIGHT_GRID_ROW_SHIFT 10
#define LIGHT_GRID_COUNT_BITS 9u
#define LIGHT_GRID_OVERFLOW 511u

uniform highp usampler2D light_grid;
// xy: `gl_FragCoord` -> tile scale of the bound target; zw: tiles_x, tiles_y.
uniform highp vec4 light_grid_map;
uniform float light_grid_on;

highp uint lightGridWord(highp int k) {
    return texelFetch(light_grid, ivec2(k & (LIGHT_GRID_ROW - 1), k >> LIGHT_GRID_ROW_SHIFT), 0).r;
}

vec3 evaluateLightsTiled(vec3 n, vec3 p) {
    if (light_grid_on < 0.5) {
        return evaluateLights(n, p);
    }
    highp ivec2 dims = ivec2(light_grid_map.zw);
    highp ivec2 tile = clamp(ivec2(gl_FragCoord.xy * light_grid_map.xy), ivec2(0), dims - 1);
    highp uint header = lightGridWord(tile.y * dims.x + tile.x);
    highp uint count = header & LIGHT_GRID_OVERFLOW;
    if (count == LIGHT_GRID_OVERFLOW) {
        return evaluateLights(n, p);
    }
    highp int base = int(header >> LIGHT_GRID_COUNT_BITS);
    vec3 acc = vec3(0.0);
    for (highp uint j = 0u; j < count; j += 4u) {
        highp uint word = lightGridWord(base + int(j >> 2u));
        highp uint m = min(count - j, 4u);
        for (highp uint b = 0u; b < m; b++) {
            highp int index = int((word >> (b * 8u)) & 255u);
            acc += evaluateLight(u_lights.lights[index], n, p);
        }
    }
    return acc;
}
// --- END LIGHT GRID ---

// --- BEGIN SUN SHADOW (byte-identical in mesh.frag / iso_tilemap.frag /
// --- sheet.frag; pinned by `lit_shaders_share_the_sun_shadow_block`) ---
//
// Directional-shadow lookup.  `lightPos` (plus the normal offset) is
// projected into light clip space, remapped to `[0,1]` UVs and compared by
// the hardware against the stored (nearest-occluder) depth: `shadow_map` is
// a `sampler2DShadow` (`COMPARE_REF_TO_TEXTURE`, `LEQUAL`, `LINEAR`), so one
// `texture()` returns the bilinear blend of a 2x2 depth compare.  The
// reference is the fragment depth minus `shadow_bias`, i.e. lit when
// `fragDepth <= stored + shadow_bias`.  Four taps at the corners of the
// fragment's texel make a tent filter over the 3x3 texels around it (the
// footprint of the old 9-tap box PCF).  Outside the light box there is no
// shadow; `shadow_strength` floors the result so a fully shadowed pixel keeps
// part of its sun diffuse.  `n` is the receiver's world normal: nudging the
// sample point along it by ~a texel keeps a surface from sampling the very
// texel it wrote (shadow acne) without detaching the shadow from its caster
// the way a large depth bias would.
float shadowFactor(vec3 lightPos, vec3 n) {
    vec4 lp = light_view_proj * vec4(lightPos + n * shadow_normal_offset, 1.0);
    vec3 ndc = lp.xyz / lp.w;
    vec2 suv = ndc.xy * 0.5 + 0.5;
    if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
        return 1.0;
    }
    float ref = ndc.z * 0.5 + 0.5 - shadow_bias;
    vec2 h = 0.5 * shadow_texel;
    float shadow = 0.25 * (texture(shadow_map, vec3(suv + vec2(-h.x, -h.y), ref))
        + texture(shadow_map, vec3(suv + vec2(h.x, -h.y), ref))
        + texture(shadow_map, vec3(suv + vec2(-h.x, h.y), ref))
        + texture(shadow_map, vec3(suv + vec2(h.x, h.y), ref)));
    return mix(shadow_strength, 1.0, shadow);
}
// --- END SUN SHADOW ---

vec2 tileUv(float tile_id_flat, vec2 tex_coord) {
    vec2 tile_id = vec2(floor(mod(tile_id_flat, tile_set_size.x)), floor(tile_id_flat / tile_set_size.x));

    vec2 setNormalSize = vec2(1, 1) / tile_set_size;

    vec2 tileCornerNorm = tile_id * setNormalSize;
    vec2 localTileCoord = tex_coord * setNormalSize;

    return tileCornerNorm + localTileCoord;
}

// UV into the sprite's sheet (and, for depth-mapped sprites, the parallel
// depth atlas) at the given quad tex coord.  The packed-atlas path maps the
// trimmed content sub-rect into `uv_rect`; the uniform-grid path uses
// `tileUv`.  Discards the trimmed padding in the packed path.
vec2 sheetUv(vec2 tex_coord) {
#ifdef INSTANCED
    if (vGrid > 0.5) {
        // A uniform-grid frame: `vUvRect` = (frame, tile_set_size).
        return tileUv(vUvRect.x, tex_coord);
    }
    // The packed form: `vContent` = trim offset + content size, `vSource` = source size (the
    // per-draw path's exact expression).
    highp vec2 content_min = vContent.xy / vSource;
    highp vec2 content_ext = vContent.zw / vSource;
    highp vec2 content_uv = (tex_coord - content_min) / content_ext;
    if (content_uv.x < 0.0 || content_uv.x > 1.0
        || content_uv.y < 0.0 || content_uv.y > 1.0) {
        discard;
    }
    return mix(vUvRect.xy, vUvRect.zw, content_uv);
#else
    if (use_uv_rect > 0.5) {
        highp vec2 content_min = trim_offset / source_size;
        highp vec2 content_ext = content_size / source_size;
        highp vec2 content_uv = (tex_coord - content_min) / content_ext;
        if (content_uv.x < 0.0 || content_uv.x > 1.0
            || content_uv.y < 0.0 || content_uv.y > 1.0) {
            discard;
        }
        return mix(uv_rect.xy, uv_rect.zw, content_uv);
    }
    return tileUv(tile_id_flat, tex_coord);
#endif
}

vec4 getTilePixel(vec2 tex_coord) {
    return texture(tex_sampler, sheetUv(tex_coord));
}

void main(void ) {
#if defined(INSTANCED) && !defined(ID_PASS)
    // Ghost-group test first: it rejects without a texture fetch.
    if (use_ghost_ids > 0.5 && vMisc.y > 0.5) {
        highp float id = texelFetch(ghost_ids, ivec2(gl_FragCoord.xy), 0).r * 255.0;
        if (abs(id - vMisc.y) < 0.5) {
            discard;
        }
    }
#endif
    vec4 color = getTilePixel(vec2(vTexCoord.x, vTexCoord.y));

    if (selected > 0.5) {
        if (color.a < 0.01) {
            // Transparent texel: draw a silhouette edge where a cardinal
            // neighbour is opaque.  `sheetUv` maps the current texel to the
            // sheet; neighbours are sampled directly (no re-discard).
            vec2 suv = sheetUv(vec2(vTexCoord.x, vTexCoord.y));
            float neighbour = max(
                max(texture(tex_sampler, suv + vec2(outline_delta.x, 0.0)).a,
                    texture(tex_sampler, suv - vec2(outline_delta.x, 0.0)).a),
                max(texture(tex_sampler, suv + vec2(0.0, outline_delta.y)).a,
                    texture(tex_sampler, suv - vec2(0.0, outline_delta.y)).a)
            );
            if (neighbour < 0.01) {
                discard;
            }
#ifdef ID_PASS
            fragColor = vec4(vMisc.y / 255.0, 0.0, 0.0, 1.0);
#else
            fragColor = vec4(selection_color, 1.0);
#endif
            return;
        }
    } else if (color.a < 0.01) {
        discard;
    }

    color.rgb *= tint;
    if (use_depth_map > 0.5) {
        // The depth sheet stores view depth relative to the frame's ground
        // anchor (`gray = 0.5` at the anchor), so offset it by the sprite's
        // absolute ground-anchor depth to land on its real map depth.
        // `depth_base` is that anchor's window depth (the model translation
        // re-projected through the camera); `depth_scale` turns `gray − 0.5`
        // into window units: 1 for a legacy sheet baked over the whole window,
        // `2R / (DEPTH_NEAR − DEPTH_FAR)` for a sheet v2 anchor-relative one
        // over `±R` metres (`classic_core::vehicle_def`).
        gl_FragDepth = depth_base
            + (texture(depth_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).r - 0.5)
                * depth_scale;
    }
#ifdef ID_PASS
    // The ghost-id pass (`Gfx::draw_ghost_id_batches`): the same coverage and
    // depth as the normal pass, writing the group id instead of colour.
    fragColor = vec4(vMisc.y / 255.0, 0.0, 0.0, 1.0);
    return;
#endif

    // Normal from the sheet's normal-map companion, used in world space (the
    // Blender space it was baked in).  A (0.5,0.5,0.5) texel decodes to
    // (0,0,0) and marks an *emissive* region (e.g. the rocket flame), which
    // keeps flat albedo and skips shading entirely.
    //
    // `emissive` is that sentinel and nothing else.  It used to also swallow
    // "this sprite has no normal map at all", which left such sprites with no
    // ambient, no sun and no point lights — raw albedo floating in the scene.
    // Those now shade off a flat +Z normal like a decal on the ground.
    vec3 rawNormal = vec3(0.0);
    bool emissive = false;
    if (use_normal_map > 0.5) {
        rawNormal = texture(normal_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).rgb * 2.0 - 1.0;
        emissive = dot(rawNormal, rawNormal) <= 0.001;
    }
    // Normal-offset bias needs a direction even where the sprite is emissive or
    // has no normal map; away from the terrain (+Z) is the safe default.
    vec3 n = dot(rawNormal, rawNormal) > 0.001
        ? normalize(rawNormal)
        : vec3(0.0, 0.0, 1.0);

    // Bring-up diagnostic (CLASSIC_SHADOW_DEBUG): sun visibility only.  The
    // alpha silhouette and iso depth are kept so the sprite still occludes
    // correctly and its outline stays readable against the terrain.
    if (shadow_debug > 0.5) {
        float vis = use_shadow > 0.5 ? shadowFactor(vLightPos, n) : 1.0;
        fragColor = vec4(vec3(vis), color.a);
        return;
    }

    if (use_lighting > 0.5 && !emissive) {
        float diff = max(dot(n, light_direction), 0.0);
        if (use_shadow > 0.5) {
            diff *= shadowFactor(vLightPos, n);
        }
        // Point lights are modulated by albedo, exactly like the sun.  They
        // used to be added *after* the albedo multiply, so a point light
        // washed every surface toward its own colour regardless of texture —
        // which is what made it read as a glowing decal rather than as light.
        color.rgb *= ambient_color + diff * light_color + evaluateLightsTiled(n, vLightPos);
    }
    if (ghost_alpha > 0.0) {
        color.a = ghost_alpha;
    }
    fragColor = color;
}
