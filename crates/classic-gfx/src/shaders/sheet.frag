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
// `team_tint.rgb` (when `vMisc.z` has flag 4), emissive strength.
flat in vec4 vTeam;
#define depth_base vMisc.x
#define depth_scale vTint.w
#define selected mod(vMisc.z, 2.0)
#define tint vTint.rgb
#define outline_delta vOutline.xy
#define vSource vOutline.zw
#define vGrid vMisc.w
#define tile_set_size vUvRect.yz
#ifdef INTERP
flat in highp vec4 vUvB;
flat in highp vec4 vContentB;
flat in highp vec4 vInterp;
flat in highp vec4 vQuad;
flat in highp vec3 vTilt;
#endif
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

#ifdef INTERP
// ---- Direction interpolation (sheet v2 `dir_interp`) ----------------------
//
// Two-neighbour backward warp (the prototype's "bwd4"): each output pixel `q`
// (sheet pixels from the ground anchor, y down) is looked up in the two baked
// directions straddling the drawn heading.  For a direction rotated `phi`
// (world radians about +Z through the anchor) away from the drawn heading, a
// fixed-point search finds the source pixel `s` whose depth-unprojected
// point, rotated by `phi`, projects onto `q`; it is accepted within 0.75 px.
// The two results merge by angle where their depths agree within 6 cm, else
// the nearer wins, and either fills the other's holes.  Normals rotate with
// the point; depth, team mask and emissive travel with the chosen texel.
uniform sampler2D tex_sampler_b;
uniform sampler2D depth_sampler_b;
uniform sampler2D normal_sampler_b;
// Sheet pixels per metre (shared with the vertex stage).
uniform highp float ppm;
// `[DEPTH_NEAR, DEPTH_FAR]` (metres): the window span of an anchor offset.
uniform highp vec2 depth_span;

#define WARP_STEPS 4
#define WARP_ACCEPT_PX 0.75
#define WARP_MERGE_M 0.06
// Tilted retry: only first searches that ended this close to the content.
#define WARP_RETRY_PX 16.0
// Depth grays at or below this are background (half an 8-bit step).
#define WARP_BACKGROUND_GRAY 0.002

// `classic_core::math::iso_basis` (the bake camera, `render/iso.py`).
const highp vec3 CAM_R = vec3(0.70710678, -0.70710678, 0.0);
const highp vec3 CAM_U = vec3(0.35355339, 0.35355339, 0.86602540);
const highp vec3 CAM_B = vec3(-0.61237244, -0.61237244, 0.5);

struct WarpTexel {
    vec4 color;
    vec3 normal;
    float team;
    highp float depth;
    bool ok;
};

// World offset -> pixel offset (y down) and view-depth offset (+ farther).
// Its inverse at pixel `q`, depth `d` is `(q.x R - q.y U) / ppm - d B`.
highp vec3 projectPx(highp vec3 p) {
    return vec3(dot(p, CAM_R) * ppm, -dot(p, CAM_U) * ppm, -dot(p, CAM_B));
}

highp vec3 rotZ(highp vec3 p, highp float a) {
    highp float c = cos(a);
    highp float s = sin(a);
    return vec3(c * p.x - s * p.y, s * p.x + c * p.y, p.z);
}

// A neighbour's full residual rotation `R = Q · Rz(phi)`: its yaw to the
// drawn heading, then the shared world-frame tilt residual `Q` (`vTilt` =
// the unit quaternion's xyz, w >= 0; zero = identity) from its baked tilt
// level to the drawn tilt (`classic_core::vehicle_def::tilt_residual`).
// Both pivot on the ground anchor, the bake rig's origin.
highp vec3 residual(highp vec3 p, highp float phi) {
    highp vec3 v = rotZ(p, phi);
    highp vec3 u = vTilt;
    // A pure yaw blend (flat per instance, so the branch is coherent).
    if (u == vec3(0.0)) {
        return v;
    }
    highp float w = sqrt(max(1.0 - dot(u, u), 0.0));
    return v + 2.0 * cross(u, cross(u, v) + w * v);
}

// Sheet UV of a source-cell pixel position; false outside the trimmed
// content (`tc` = trim offset, content size).
bool cellUv(highp vec2 cell, highp vec4 uvr, highp vec4 tc, out highp vec2 uv) {
    highp vec2 c = (cell - tc.xy) / tc.zw;
    uv = mix(uvr.xy, uvr.zw, c);
    return c.x >= 0.0 && c.y >= 0.0 && c.x < 1.0 && c.y < 1.0;
}

// Anchor-relative depth (metres, + farther) at source-cell pixel `cell`;
// background (gray 0, or outside the content) is the anchor's depth, 0.
highp float warpDepth(sampler2D dep, highp vec4 uvr, highp vec4 tc, highp vec2 cell,
                      highp float rangeM, out bool solid) {
    highp vec2 uv;
    highp float g = 0.0;
    if (cellUv(cell, uvr, tc, uv)) {
        g = textureLod(dep, uv, 0.0).r;
    }
    solid = g > WARP_BACKGROUND_GRAY;
    return solid ? (g - 0.5) * 2.0 * rangeM : 0.0;
}

// The fixed-point search for the source pixel of output pixel `q` (`mx`,
// `my`, `md`: the linear unproject -> rotate -> project columns).  Source
// background is taken at depth `bg` (a `dir_interp` sheet keeps gray 0 for
// transparent texels).  A tilt residual makes `md` large enough that on faces
// seen near edge-on (a steep depth gradient) the plain step `s += q - f(s)`
// diverges (`|I - J| > 1`, `J = [mx my] + md ∇d`); with `newton` (tilted
// blends) a step whose residual stopped halving switches to a Newton step,
// `∇d` from two fetches 2 px out.  A pure yaw keeps the plain step (one
// fetch).
highp vec2 warpSearch(sampler2D dep, highp vec4 uvr, highp vec4 tc, highp vec2 anchor,
                      highp vec3 mx, highp vec3 my, highp vec3 md, highp float rangeM,
                      highp vec2 q, bool newton, highp float bg) {
    highp vec2 s = q;
    // Newton only once the plain step stops converging (its residual no
    // longer halves): well-conditioned faces never pay the gradient fetches.
    bool diverging = false;
    highp float prev = 1e9;
    for (int i = 0; i < WARP_STEPS; i++) {
        bool solid;
        highp float d = warpDepth(dep, uvr, tc, anchor + s, rangeM, solid);
        d = solid ? d : bg;
        highp vec2 r = q - (mx.xy * s.x + my.xy * s.y + md.xy * d);
        highp float rl = length(r);
        diverging = diverging || rl > 0.5 * prev;
        prev = rl;
        if (newton && diverging && solid) {
            bool sx;
            bool sy;
            highp float dx = warpDepth(dep, uvr, tc, anchor + s + vec2(2.0, 0.0), rangeM, sx);
            highp float dy = warpDepth(dep, uvr, tc, anchor + s + vec2(0.0, 2.0), rangeM, sy);
            highp vec2 g = vec2(sx ? (dx - d) * 0.5 : 0.0, sy ? (dy - d) * 0.5 : 0.0);
            highp mat2 j = mat2(mx.xy + md.xy * g.x, my.xy + md.xy * g.y);
            highp float det = determinant(j);
            if (abs(det) > 0.05) {
                r = inverse(j) * r;
            }
        }
        s += r;
    }
    return s;
}

// Whether source pixel `s` is an opaque texel that projects onto `q`; sets
// `uv` (its sheet UV) and `f` (its projected pixel and depth).
bool warpAccept(sampler2D col, sampler2D dep, highp vec4 uvr, highp vec4 tc, highp vec2 anchor,
                highp vec3 mx, highp vec3 my, highp vec3 md, highp float rangeM, highp vec2 q,
                highp vec2 s, out highp vec2 uv, out vec4 color, out highp vec3 f) {
    color = vec4(0.0);
    f = vec3(0.0);
    if (!cellUv(anchor + s, uvr, tc, uv)) {
        return false;
    }
    color = textureLod(col, uv, 0.0);
    if (color.a <= 0.02) {
        return false;
    }
    highp float d = (textureLod(dep, uv, 0.0).r - 0.5) * 2.0 * rangeM;
    f = mx * s.x + my * s.y + md * d;
    return length(f.xy - q) < WARP_ACCEPT_PX;
}

WarpTexel warpDirection(sampler2D col, sampler2D dep, sampler2D nrm, highp vec4 uvr,
                        highp vec4 tc, highp vec2 anchor, highp float phi,
                        highp float rangeM, highp vec2 q) {
    WarpTexel w;
    w.ok = false;
    // unproject -> rotate -> project is linear in (s, d): precompute its
    // columns once (one sin/cos pair and the tilt quaternion), so a step is
    // two fused vec3 terms.
    highp vec3 mx = projectPx(residual(CAM_R / ppm, phi));
    highp vec3 my = projectPx(residual(-CAM_U / ppm, phi));
    highp vec3 md = projectPx(residual(-CAM_B, phi));
    bool tilted = vTilt != vec3(0.0);
    highp vec2 uv;
    highp vec3 f;
    highp vec2 s = warpSearch(dep, uvr, tc, anchor, mx, my, md, rangeM, q, tilted, 0.0);
    bool ok = warpAccept(col, dep, uvr, tc, anchor, mx, my, md, rangeM, q, s, uv, w.color, f);
    // Just outside the source silhouette a tilt can pull in surface that the
    // search, starting on background at the anchor depth, converges past
    // (onto a background fixed point).  Retry those from a nearer background
    // (half the depth range toward the camera) -- only where the first search
    // ended within `WARP_RETRY_PX` of the source content, which skips the
    // quad's far background.
    highp vec2 c = anchor + s - tc.xy;
    bool nearContent = all(greaterThan(c, vec2(-WARP_RETRY_PX)))
        && all(lessThan(c, tc.zw + WARP_RETRY_PX));
    if (!ok && tilted && nearContent) {
        s = warpSearch(dep, uvr, tc, anchor, mx, my, md, rangeM, q, true, -0.5 * rangeM);
        ok = warpAccept(col, dep, uvr, tc, anchor, mx, my, md, rangeM, q, s, uv, w.color, f);
    }
    if (!ok) {
        return w;
    }
    w.normal = vec3(0.0);
    w.team = 1.0;
    if (use_normal_map > 0.5) {
        vec4 ns = textureLod(nrm, uv, 0.0);
        w.normal = residual(ns.rgb * 2.0 - 1.0, phi);
        w.team = ns.a;
    }
    w.depth = f.z;
    w.ok = true;
    return w;
}

// The merged direction blend at this fragment (discards where neither
// neighbour covers it).  `vOutline.zw` / `vInterp.xy` are the anchors,
// `vInterp.zw` the drawn heading minus each neighbour's (tile radians; a
// tile-space turn is the opposite world turn).
WarpTexel blendDirections() {
    highp vec2 q = vQuad.xy + vTexCoord * vQuad.zw;
    highp float rangeM = depth_scale * (depth_span.x - depth_span.y) * 0.5;
    WarpTexel a = warpDirection(tex_sampler, depth_sampler, normal_sampler, vUvRect, vContent,
                                vOutline.zw, -vInterp.z, rangeM, q);
    WarpTexel b = warpDirection(tex_sampler_b, depth_sampler_b, normal_sampler_b, vUvB, vContentB,
                                vInterp.xy, -vInterp.w, rangeM, q);
    if (!a.ok && !b.ok) {
        discard;
    }
    if (!b.ok) {
        return a;
    }
    if (!a.ok) {
        return b;
    }
    if (abs(a.depth - b.depth) >= WARP_MERGE_M) {
        return a.depth < b.depth ? a : b;
    }
    highp float t = clamp(vInterp.z / max(vInterp.z - vInterp.w, 1e-6), 0.0, 1.0);
    WarpTexel m;
    m.ok = true;
    m.color = mix(a.color, b.color, t);
    vec3 n = mix(a.normal, b.normal, t);
    // Keep the emissive marker (zero normal) when both sides carry it.
    m.normal = dot(n, n) > 0.001 ? normalize(n) : vec3(0.0);
    m.team = mix(a.team, b.team, t);
    m.depth = mix(a.depth, b.depth, t);
    return m;
}
#endif

void main(void ) {
#if defined(INSTANCED) && !defined(ID_PASS)
    // Ghost-group test first: it rejects without a texture fetch.
    if (ghost_alpha > 0.0 && mod(floor(vMisc.z * 0.5), 2.0) > 0.5) {
        discard;
    }
    if (use_ghost_ids > 0.5 && vMisc.y > 0.5) {
        highp float id = texelFetch(ghost_ids, ivec2(gl_FragCoord.xy), 0).r * 255.0;
        if (abs(id - vMisc.y) < 0.5) {
            discard;
        }
    }
#endif
#ifdef INTERP
    // A direction blend (never selected: the engine draws a selected sprite
    // on its nearest direction, with the silhouette).
    WarpTexel blended = blendDirections();
    vec4 color = blended.color;
#else
    vec4 color = getTilePixel(vec2(vTexCoord.x, vTexCoord.y));
#endif

#ifndef INTERP
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
#endif

    color.rgb *= tint;
#ifdef INTERP
    // The warped texel's view-depth offset (metres) from the anchor.
    gl_FragDepth = depth_base + blended.depth / (depth_span.x - depth_span.y);
#else
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
#endif
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
    // The normal sheet's alpha: the team mask on a `team_mask` sheet (sheet v2).
    float teamAlpha = 1.0;
#ifdef INTERP
    rawNormal = blended.normal;
    teamAlpha = blended.team;
    emissive = use_normal_map > 0.5 && dot(rawNormal, rawNormal) <= 0.001;
#else
    if (use_normal_map > 0.5) {
        vec4 normalSample = texture(normal_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y)));
        rawNormal = normalSample.rgb * 2.0 - 1.0;
        teamAlpha = normalSample.a;
        emissive = dot(rawNormal, rawNormal) <= 0.001;
    }
#endif
#ifdef INSTANCED
    // Team colour: weight `clamp((1 - a) * 2, 0, 1)` of the normal alpha
    // (`classic_core::vehicle_def::team_mask_weight`), separate from `tint`.
    if (vMisc.z >= 3.5) {
        float teamWeight = clamp((1.0 - teamAlpha) * 2.0, 0.0, 1.0);
        color.rgb *= mix(vec3(1.0), vTeam.rgb, teamWeight);
    }
#endif
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
#ifdef INSTANCED
    // Emissive strength (`IsoSprite::emissive`): the marker pixels stay unlit
    // and scale; above 1 they brighten into the bloom pass.
    if (emissive) {
        color.rgb *= vTeam.w;
    }
#endif
    if (ghost_alpha > 0.0) {
        color.a = ghost_alpha;
    }
    fragColor = color;
}
