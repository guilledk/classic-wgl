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

uniform vec3 ambient_color;
uniform vec3 light_direction;
uniform vec3 light_color;

uniform sampler2D shadow_map;
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
// --- mesh.frag; pinned by `lit_shaders_share_the_lighting_block`) ---
//
// `p` and `l.pos_radius.xyz` are both **metric world space** (+Z up, metres),
// so `length` is a true distance and `dot(n, L)` a true cosine.  They previously
// lived in the isometric screen space, which compresses y by 2x; every point
// light was therefore an ellipsoid evaluated as if it were a sphere.
vec3 evaluateLight(Light l, vec3 n, vec3 p) {
    vec3 toLight = l.pos_radius.xyz - p;
    float dist = length(toLight);
    vec3 L = toLight / max(dist, 0.0001);
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
        float window = clamp(1.0 - d2, 0.0, 1.0);
        attenuation = window * window / (1.0 + d2);
    }
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

float shadowSample(vec2 suv, float fragDepth) {
    float stored = texture(shadow_map, suv).r;
    return (stored + shadow_bias < fragDepth) ? 0.0 : 1.0;
}

float shadowFactor(vec3 lightPos, vec3 n) {
    vec4 lp = light_view_proj * vec4(lightPos + n * shadow_normal_offset, 1.0);
    vec3 ndc = lp.xyz / lp.w;
    vec2 suv = ndc.xy * 0.5 + 0.5;
    if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
        return 1.0;
    }
    float fragDepth = ndc.z * 0.5 + 0.5;
    float acc = 0.0;
    for (int x = -1; x <= 1; x++) {
        for (int y = -1; y <= 1; y++) {
            acc += shadowSample(suv + vec2(float(x), float(y)) * shadow_texel, fragDepth);
        }
    }
    float shadow = acc / 9.0;
    return mix(shadow_strength, 1.0, shadow);
}

// The CPU reference is `TeamTint::apply`; keep the two in step.
vec4 teamTint(vec4 color, highp vec2 uv) {
    if (all(greaterThanEqual(uv, team_rect.xy)) && all(lessThan(uv, team_rect.zw))) {
        color.rgb *= team_tint;
    }
    return color;
}

void main(void ) {
    vec4 color = use_texture > 0.5 ? texture(tex_sampler, vTexCoord) : base_color;
    if (use_team > 0.5 && use_texture > 0.5) {
        color = teamTint(color, vTexCoord);
    }
    if (color.a < 0.01) {
        discard;
    }

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
    color.rgb *= ambient_color + diff * light_color + evaluateLights(n, vLightPos);
    fragColor = color;
}
