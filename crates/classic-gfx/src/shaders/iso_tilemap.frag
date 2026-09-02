#version 300 es

precision mediump float;

in mediump vec2 vMapCoord;
in mediump float vTileId;
in highp vec3 vNormal;
in highp vec3 vLightPos;

uniform sampler2D map_data;
uniform vec2 map_size;

uniform sampler2D tile_set;
uniform vec2 tile_set_size;
uniform vec2 tile_pixel_size;

uniform vec2 selected_tile;
uniform vec2 selection_begin;
uniform vec4 selection_color;
uniform int selection_mode;
uniform vec4 wall_color;

uniform float grid_radius;
uniform int show_grid;
uniform vec3 grid_color;

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

// --- BEGIN SHARED LIGHTING (must stay byte-identical to iso_tilemap.frag;
// --- pinned by `lit_shaders_share_the_lighting_block`) ---
//
// `p` and `l.pos_radius.xyz` are both **metric light space** (+Z up, `ppm` px
// per metre on every axis — see `classic_core::math::iso_world_light_matrix`), so
// `length` is a true distance and `dot(n, L)` a true cosine.  They previously
// lived in the isometric space, which compresses y by 2x; every point light
// was therefore an ellipsoid evaluated as if it were a sphere.
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
// --- END SHARED LIGHTING ---

// Manual directional-shadow compare.  `worldPos` is projected into light clip
// space, remapped to `[0,1]` UVs, and compared against the stored depth.  A
// fragment is shadowed when the stored (nearest-occluder) depth is nearer than
// the fragment's depth (with a bias).  Outside the light box there is no
// shadow.  PCF (3x3) softens the texel edges; `shadow_strength` floors the
// result so a fully-shadowed pixel keeps part of its sun diffuse.
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

float getMapData(vec2 pos) {
    vec4 rawData = texture(map_data, pos);
    return floor(rawData.r * 256.0);
}

vec4 getTilePixel(float tile_id_flat, vec2 map_coord) {
    vec2 tile_id = vec2(floor(mod(tile_id_flat, tile_set_size.x)), floor(tile_id_flat / tile_set_size.x));

    vec2 mapTileNormalSize = vec2(1, 1) / map_size;
    vec2 setNormalSize = vec2(1, 1) / tile_set_size;

    vec2 tileCornerNorm = tile_id * setNormalSize;

    vec2 localTileCoord = fract(map_coord / mapTileNormalSize) * setNormalSize;

    vec4 texColor = texture(tile_set, tileCornerNorm + localTileCoord);

    if (selection_mode != -1) {
        vec2 selectedNormalStart = floor(min(selection_begin, selected_tile)) * mapTileNormalSize;
        vec2 selectedNormalEnd = ceil(max(selection_begin, selected_tile)) * mapTileNormalSize;

        bvec2 selectStart = greaterThanEqual(map_coord, selectedNormalStart);
        bvec2 selectEnd = lessThanEqual(map_coord, selectedNormalEnd);

        if (all(selectStart) && all(selectEnd)) {
            if (selection_mode == 0)
                return vec4(1.0 - texColor.r, 1.0 - texColor.g, 1.0 - texColor.b, 1.0);

            if (selection_mode == 1) {
                float average = (texColor.r + texColor.g + texColor.b) / 3.0;
                return vec4(average, average, average, texColor.a) * selection_color;
            }
        }
    }

    return texColor;
}

void main(void ) {
    vec4 color;

    if (vTileId > 0.5) {
        color = wall_color;
    } else {
        vec2 map_coord = vec2(vMapCoord.x, vMapCoord.y);
        color = getTilePixel(getMapData(map_coord), map_coord);
    }

    if (color.a < 0.01) discard;

    vec3 n = normalize(vNormal);

    // Bring-up diagnostic: show sun visibility alone (white = lit, black =
    // occluded), with no albedo, ambient, Lambert term or point lights to hide
    // behind.  See CLASSIC_SHADOW_DEBUG.
    if (shadow_debug > 0.5) {
        float vis = use_shadow > 0.5 ? shadowFactor(vLightPos, n) : 1.0;
        fragColor = vec4(vec3(vis), 1.0);
        return;
    }

    float diff = max(dot(n, light_direction), 0.0);
    if (use_shadow > 0.5) {
        diff *= shadowFactor(vLightPos, n);
    }
    // Point lights are modulated by albedo, exactly like the sun.  They used to
    // be added *after* the albedo multiply, so a point light washed the terrain
    // toward its own colour regardless of the tile texture — which is what made
    // it read as a glowing decal rather than as light.
    color.rgb *= ambient_color + diff * light_color + evaluateLights(n, vLightPos);

    if (show_grid > 0 && selection_mode == -1 && vTileId <= 0.5) {
        vec2 tileCoord = vMapCoord * map_size;
        vec2 localUV = fract(tileCoord);
        float mt = floor(selected_tile.x);
        float nt = floor(selected_tile.y);
        float ct = floor(tileCoord.x);
        float rt = floor(tileCoord.y);
        float dist = max(abs(ct - mt), abs(nt - rt));
        if (dist <= grid_radius) {
            float edge = 0.04;
            float dx = min(localUV.x, 1.0 - localUV.x);
            float dy = min(localUV.y, 1.0 - localUV.y);
            float edgeDist = min(dx, dy);
            float border = 1.0 - smoothstep(0.0, edge, edgeDist);
            float fade = 1.0 - dist / max(grid_radius, 0.01);
            color.rgb = mix(color.rgb, grid_color, border * fade * 0.85);
        }
    }

    fragColor = color;
}
