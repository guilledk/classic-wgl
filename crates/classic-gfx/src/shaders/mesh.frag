#version 300 es

precision mediump float;

in highp vec2 vTexCoord;
in highp vec3 vNormal;
in highp vec3 vLightPos;

uniform sampler2D tex_sampler;
// Whether to sample `tex_sampler` (1) or use the flat `base_color` (0).
uniform float use_texture;
uniform vec4 base_color;

// Team tint (`classic_gfx::TeamTint`): when `use_team` is 1, albedo texels
// whose TEXCOORD_0 lies in `team_rect` = [u0, v0, u1, v1] (top-left origin,
// half-open on the far edges) are multiplied by `team_tint`.  The rect is
// highp: mediump cannot resolve texel edges of a 2048-wide atlas.
uniform float use_team;
uniform highp vec4 team_rect;
uniform vec3 team_tint;

// Self-lit regions (`classic_core::model::EMISSIVE_EXTRA`): the first of the
// `emissive_count` rects containing the fragment's TEXCOORD_0 (same half-open
// rule as the team rect) adds `albedo * emissive_strength[i]` to the lit
// colour, unlit.  CPU reference: `classic_gfx::emissive_shade`.
#define MAX_EMISSIVE 8
uniform float emissive_count;
uniform highp vec4 emissive_rect[MAX_EMISSIVE];
uniform float emissive_strength[MAX_EMISSIVE];

uniform vec3 ambient_color;
uniform vec3 light_direction;
uniform vec3 light_color;

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

// 1 when this model opts out of the ghost pass (`Model.ghost = false`: a deck
// or floor sprites stand on); written to the model target's ghost mask.
uniform float no_ghost;

layout(location = 0) out vec4 fragColor;
// The model target's R8 ghost mask (`ModelTarget`): alpha 1, so the global
// alpha blend replaces it and the nearest model's flag wins.
layout(location = 1) out vec4 ghostMask;

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

// The CPU reference is `TeamTint::apply`; keep the two in step.
vec4 teamTint(vec4 color, highp vec2 uv) {
    if (all(greaterThanEqual(uv, team_rect.xy)) && all(lessThan(uv, team_rect.zw))) {
        color.rgb *= team_tint;
    }
    return color;
}

float emissiveStrength(highp vec2 uv) {
    for (int i = 0; i < MAX_EMISSIVE; i++) {
        if (float(i) >= emissive_count) {
            break;
        }
        highp vec4 r = emissive_rect[i];
        if (all(greaterThanEqual(uv, r.xy)) && all(lessThan(uv, r.zw))) {
            return emissive_strength[i];
        }
    }
    return 0.0;
}

void main(void ) {
    ghostMask = vec4(no_ghost, 0.0, 0.0, 1.0);
    vec4 color = use_texture > 0.5 ? texture(tex_sampler, vTexCoord) : base_color;
    if (use_team > 0.5 && use_texture > 0.5) {
        color = teamTint(color, vTexCoord);
    }
#ifdef ALPHA_TEST
    // Only the `meshAlphaTest` variant cuts: a `discard` anywhere in a
    // program turns early depth testing off for every draw with it, so the
    // (overwhelmingly opaque) rest of the model pass skips it.
    if (color.a < 0.01) {
        discard;
    }
#endif

    vec3 n = normalize(vNormal);

    if (shadow_debug > 0.5) {
        float vis = use_shadow > 0.5 ? shadowFactor(vLightPos, n) : 1.0;
        fragColor = vec4(vec3(vis), color.a);
        return;
    }

    float diff = max(dot(n, light_direction), 0.0);
    if (use_shadow > 0.5) {
        diff *= shadowFactor(vLightPos, n);
    }
    vec3 albedo = color.rgb;
    color.rgb *= ambient_color + diff * light_color + evaluateLightsTiled(n, vLightPos);
    if (emissive_count > 0.5 && use_texture > 0.5) {
        color.rgb += albedo * emissiveStrength(vTexCoord);
    }
    fragColor = color;
}
