#version 300 es

precision mediump float;

in highp vec2 vTexCoord;
in highp vec3 vWorldPos;
in highp vec3 vLightPos;

uniform sampler2D tex_sampler;
uniform sampler2D depth_sampler;
uniform sampler2D normal_sampler;

uniform vec2 tile_set_size;
uniform float tile_id_flat;
uniform float ghost_alpha;
uniform float use_uv_rect;
uniform highp vec4 uv_rect;
uniform highp vec2 trim_offset;
uniform highp vec2 source_size;
uniform highp vec2 content_size;
uniform float use_depth_map;
uniform highp float depth_base;
uniform highp float depth_range;
uniform float use_normal_map;
uniform vec3 ambient_color;
uniform vec3 light_direction;
uniform vec3 light_color;
// Per-sprite tint (multiplied onto the albedo before the Lambertian term).
// Defaults to (1,1,1) — a no-op for every existing asset.  Tintable sprites
// ship a grayscale albedo and set this at runtime (see IsoSprite.color).
uniform vec3 tint;
// RTS selection silhouette: when `selected` is set, transparent edge texels
// adjacent to an opaque texel (within `outline_delta` sheet-UV) are drawn in
// `selection_color` as a per-sprite outline.
uniform float selected;
uniform vec3 selection_color;
uniform vec2 outline_delta;

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

vec3 evaluateLight(Light l, vec3 n, vec3 p) {
    vec3 toLight = l.pos_radius.xyz - p;
    float dist = length(toLight);
    vec3 L = toLight / max(dist, 0.0001);
    float radius = l.pos_radius.w;
    // Soft windowed falloff: `saturate(1 - (d/r)^2)^2` gives a smooth, natural
    // gradient with no hard circular edge (vs a linear `1 - d/r` blob).
    float attenuation;
    if (radius <= 0.0) {
        attenuation = 1.0;
    } else {
        float d = dist / radius;
        attenuation = clamp(1.0 - d * d, 0.0, 1.0);
        attenuation *= attenuation;
    }
    float cone = 1.0;
    if (l.dir_cone.w > 0.0) {
        float cosAngle = cos(l.dir_cone.w);
        float cosTheta = dot(L, normalize(l.dir_cone.xyz));
        cone = smoothstep(cosAngle * 0.6, cosAngle, cosTheta);
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

// Manual directional-shadow compare (see iso_tilemap.frag for the derivation).
// PCF (3x3) softens the texel edges; `shadow_strength` floors the result.
float shadowSample(vec2 suv, float fragDepth) {
    float stored = texture(shadow_map, suv).r;
    return (stored + shadow_bias < fragDepth) ? 0.0 : 1.0;
}

// `n` is the receiver's surface normal in light space.  Nudging the sample
// point along it by ~a texel keeps a surface from sampling the very texel it
// wrote, which is what causes shadow acne, without detaching the shadow from
// its caster the way a large depth bias would.
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
}

vec4 getTilePixel(float tile_id_flat, vec2 tex_coord) {
    return texture(tex_sampler, sheetUv(tex_coord));
}

void main(void ) {
    vec4 color = getTilePixel(tile_id_flat, vec2(vTexCoord.x, vTexCoord.y));

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
            fragColor = vec4(selection_color, 1.0);
            return;
        }
    } else if (color.a < 0.01) {
        discard;
    }

    color.rgb *= tint;
    if (use_depth_map > 0.5) {
        highp float gray = texture(depth_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).r;
        // `depth_base` and `depth_range` are both window-space iso depths, so
        // `gl_FragDepth` (also window-space) needs no clip→window remap.
        gl_FragDepth = depth_base + (0.5 - gray) * depth_range;
    }

    // World-space normal from the sheet's normal-map companion.  A
    // (0.5,0.5,0.5) texel decodes to (0,0,0) and marks an *unlit* region (e.g.
    // the rocket flame), which keeps flat albedo and skips shading entirely.
    vec3 rawNormal = vec3(0.0);
    if (use_normal_map > 0.5) {
        rawNormal = texture(normal_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).rgb * 2.0 - 1.0;
    }
    bool lit = dot(rawNormal, rawNormal) > 0.001;
    // Normal-offset bias needs a direction even where the sprite is unlit or
    // has no normal map; away from the terrain (+Z) is the safe default.
    vec3 n = lit ? normalize(rawNormal) : vec3(0.0, 0.0, 1.0);

    // Bring-up diagnostic (CLASSIC_SHADOW_DEBUG): sun visibility only.  The
    // alpha silhouette and iso depth are kept so the sprite still occludes
    // correctly and its outline stays readable against the terrain.
    if (shadow_debug > 0.5) {
        float vis = use_shadow > 0.5 ? shadowFactor(vLightPos, n) : 1.0;
        fragColor = vec4(vec3(vis), color.a);
        return;
    }

    if (lit) {
        float diff = max(dot(n, light_direction), 0.0);
        if (use_shadow > 0.5) {
            diff *= shadowFactor(vLightPos, n);
        }
        color.rgb *= ambient_color + diff * light_color;
        color.rgb += evaluateLights(n, vWorldPos);
    }
    if (ghost_alpha > 0.0) {
        color.a = ghost_alpha;
    }
    fragColor = color;
}
