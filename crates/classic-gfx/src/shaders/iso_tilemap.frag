#version 300 es

precision mediump float;

// highp: a 400-tile map needs ~1/64-tile resolution at coordinates ~400,
// past what an fp16 `mediump` carries (11-bit mantissa) on real mobile GPUs.
in highp vec2 vMapCoord;
in mediump float vTileId;
in highp vec3 vNormal;
in highp vec3 vLightPos;

// mediump: the ids are decoded from 8-bit channels as `r * 256`, which a
// `lowp` (default sampler precision) fetch cannot represent exactly.
uniform mediump sampler2D map_data;
uniform highp vec2 map_size;

// Material blending (`Tilemap.blend`).  `blend_enabled == 0` is the classic
// one-id-per-tile path, untouched.  Lengths are in tiles.
uniform int blend_enabled;
uniform mediump sampler2D blend_lut;   // 256x1: r = class, g = priority, b = hard
uniform float blend_width;
uniform float blend_max_lod;
uniform float blend_noise_scale;
uniform float blend_noise_strength;
uniform float blend_height_strength;

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

// `n` is the receiver's surface normal in world space.  Nudging the sample
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

// Tile id at integer tile coords, clamped to the map.  `texelFetch` on the
// integer cell is exactly what the `NEAREST` lookup used to resolve, without
// depending on the normalized coordinate landing inside the right texel.
float tileIdAt(ivec2 cell) {
    ivec2 last = ivec2(map_size) - 1;
    return floor(texelFetch(map_data, clamp(cell, ivec2(0), last), 0).r * 256.0);
}

float getMapData(highp vec2 pos) {
    return tileIdAt(ivec2(floor(pos * map_size)));
}

// Atlas UV of tile `tile_id_flat` at the fragment's position inside its tile.
// Every atlas cell tiles seamlessly, so any id can be sampled at any
// fragment's local coordinate.
highp vec2 tileUv(float tile_id_flat, highp vec2 map_coord) {
    vec2 tile_id = vec2(floor(mod(tile_id_flat, tile_set_size.x)), floor(tile_id_flat / tile_set_size.x));

    highp vec2 mapTileNormalSize = vec2(1, 1) / map_size;
    vec2 setNormalSize = vec2(1, 1) / tile_set_size;

    vec2 tileCornerNorm = tile_id * setNormalSize;

    highp vec2 localTileCoord = fract(map_coord / mapTileNormalSize) * setNormalSize;

    return tileCornerNorm + localTileCoord;
}

vec4 getTileTexel(float tile_id_flat, highp vec2 map_coord) {
    return texture(tile_set, tileUv(tile_id_flat, map_coord));
}

// The blend path samples under per-corner conditions, i.e. in non-uniform
// control flow, where implicit-LOD `texture()` derivatives are undefined, so
// it passes the level computed up front in `main` (uniform control flow).
vec4 tileTexelLod(float tile_id_flat, highp vec2 map_coord, float lod) {
    return textureLod(tile_set, tileUv(tile_id_flat, map_coord), lod);
}

// Editor selection highlight (invert / colorize) over a terrain colour.
vec4 applySelection(vec4 texColor, highp vec2 map_coord) {
    highp vec2 mapTileNormalSize = vec2(1, 1) / map_size;
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

// ---- Material blending -----------------------------------------------------
//
// The tile ids are a NEAREST grid, so sampling only the fragment's own id
// gives every material (and every variant of one material) a square edge.
// Instead: warp the lookup position with low-frequency noise, fetch the 2x2
// ids around the half-tile-shifted lattice point (tile centres), and mix
// their atlas texels.
//
//   * Bilinear weights over the 2x2 corners, summed per material class,
//     give each class a continuous membership field.  Its contours cut
//     across the tile staircase (as marching squares would), so a boundary
//     follows the material region rather than the grid.
//   * Within a class (the variants of one material) the variants resolve
//     the same way on smooth weights, which hides the tile grid on
//     single-material ground without washing the texture out.
//   * Across classes the memberships are sharpened, height-blend style:
//     a class mixes in only within `blend_width` of the strongest one, so
//     the transition is a narrow band on the contour.  With
//     `blend_height_strength > 0` the tileset alpha is per-texel height and
//     joins (with the class priority) that comparison: crater pits fill with
//     dust first, rock crowns poke through.
//   * A `hard` class collapses that band to a crisp edge (still following
//     the warped contour) with no height mix.
//   * Where all four corners share one id, a single atlas sample is taken.
//   * Minified terrain samples the tileset's capped mip chain at an explicit
//     LOD (`blend_max_lod`); magnified terrain stays NEAREST.
//
// The warp is a deterministic hash value noise (no texture, no clock), so the
// result is stable frame to frame and identical on every backend.

// Integer-lattice hash -> [0, 1).  Coordinates are offset positive first:
// GLSL ES leaves negative int -> uint conversion undefined.
highp float blendHash(ivec2 p) {
    uvec2 q = uvec2(p + 8192) * uvec2(1597334677u, 3812015801u);
    uint n = (q.x ^ q.y) * 1597334677u;
    return float(n >> 8u) * (1.0 / 16777216.0);
}

highp float blendValueNoise(highp vec2 p) {
    highp vec2 i = floor(p);
    highp vec2 f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    ivec2 c = ivec2(i);
    highp float a = blendHash(c);
    highp float b = blendHash(c + ivec2(1, 0));
    highp float d = blendHash(c + ivec2(0, 1));
    highp float e = blendHash(c + ivec2(1, 1));
    return mix(mix(a, b, f.x), mix(d, e, f.x), f.y);
}

// Three-octave warp vector in [-1, 1]^2: the base octave meanders a
// boundary across tiles, the finer ones roughen it.
highp vec2 blendWarp(highp vec2 p) {
    highp float x = blendValueNoise(p) + 0.5 * blendValueNoise(p * 2.03 + vec2(17.1, 3.7))
        + 0.25 * blendValueNoise(p * 4.07 + vec2(-31.4, 11.2));
    highp float y = blendValueNoise(p + vec2(41.3, 29.9))
        + 0.5 * blendValueNoise(p * 2.03 + vec2(-7.9, 53.2))
        + 0.25 * blendValueNoise(p * 4.07 + vec2(23.6, -19.8));
    return (vec2(x, y) / 1.75 - 0.5) * 2.0;
}

vec4 blendedTerrain(highp vec2 map_coord, float lod) {
    highp vec2 t = map_coord * map_size;
    highp vec2 p = t;
    if (blend_noise_strength > 0.0) {
        p += blend_noise_strength * blendWarp(t * blend_noise_scale);
    }
    highp vec2 q = p - 0.5;
    highp vec2 cell = floor(q);
    highp vec2 f = q - cell;
    ivec2 c0 = ivec2(cell);

    float ids[4];
    ids[0] = tileIdAt(c0);
    ids[1] = tileIdAt(c0 + ivec2(1, 0));
    ids[2] = tileIdAt(c0 + ivec2(0, 1));
    ids[3] = tileIdAt(c0 + ivec2(1, 1));

    // Single-id neighbourhood: one sample, nothing to blend.
    if (ids[0] == ids[1] && ids[0] == ids[2] && ids[0] == ids[3]) {
        return vec4(tileTexelLod(ids[0], map_coord, lod).rgb, 1.0);
    }

    // Linear bilinear corner weights.  Summed per class they form a membership
    // field whose iso-contours cut straight across the tile staircase
    // (marching-squares style) instead of following it; a smoothstep here
    // would flatten the field around tile centres and round the staircase
    // rather than remove it.  The variants of one class resolve on the
    // smoothstep weights `ws`, which have no crease at the tile centres.
    float w[4];
    w[0] = (1.0 - f.x) * (1.0 - f.y);
    w[1] = f.x * (1.0 - f.y);
    w[2] = (1.0 - f.x) * f.y;
    w[3] = f.x * f.y;
    vec2 s = smoothstep(vec2(0.0), vec2(1.0), f);
    float ws[4];
    ws[0] = (1.0 - s.x) * (1.0 - s.y);
    ws[1] = s.x * (1.0 - s.y);
    ws[2] = (1.0 - s.x) * s.y;
    ws[3] = s.x * s.y;

    float cls[4];
    float prio[4];
    vec4 col[4];
    bool anyHard = false;
    for (int i = 0; i < 4; i++) {
        vec4 lut = texelFetch(blend_lut, ivec2(int(ids[i]), 0), 0);
        cls[i] = floor(lut.r * 255.0 + 0.5);
        prio[i] = lut.g;
        anyHard = anyHard || lut.b > 0.5;
        col[i] = tileTexelLod(ids[i], map_coord, lod);
    }

    // Every corner carries its whole class: membership W (sum of the class's
    // corner weights), the variants' resolved colour C and height H, and how
    // many corners share the class (n), so a class counts once.  Variants are
    // resolved like classes, height-blend style on their smooth weights: a
    // linear cross-fade would average uncorrelated textures over most of the
    // tile and wash the material out; this keeps full texture contrast and
    // makes variant seams interlock along texel heights.
    bool useHeight = blend_height_strength > 0.0 && !anyHard;
    // How far below the best variant another variant still mixes in.  Wider
    // than a class band: variants are the same material, so their seams
    // should melt, but not so wide that they average back to a wash.
    const float VARIANT_DEPTH = 0.6;
    float depth = anyHard ? 0.0001 : max(blend_width, 0.0001);
    float u[4];
    for (int j = 0; j < 4; j++) {
        u[j] = ws[j] + (useHeight ? blend_height_strength * col[j].a : 0.0);
    }
    float W[4];
    vec3 C[4];
    float H[4];
    float n[4];
    for (int i = 0; i < 4; i++) {
        W[i] = 0.0;
        C[i] = vec3(0.0);
        H[i] = 0.0;
        n[i] = 0.0;
        float umax = -4.0;
        for (int j = 0; j < 4; j++) {
            bool same = cls[j] == cls[i];
            W[i] += same ? w[j] : 0.0;
            n[i] += same ? 1.0 : 0.0;
            umax = same ? max(umax, u[j]) : umax;
        }
        float S = 0.0;
        for (int j = 0; j < 4; j++) {
            bool same = cls[j] == cls[i];
            float b = same ? max(u[j] - umax + VARIANT_DEPTH, 0.0) : 0.0;
            S += b;
            C[i] += b * col[j].rgb;
            H[i] += b * col[j].a;
        }
        float inv = 1.0 / max(S, 1e-6);
        C[i] *= inv;
        H[i] *= inv;
    }

    // Across classes: sharpen the membership contour to `blend_width` and,
    // with heights on, let the higher texel (+ class priority) win near it.
    // A class with (almost) no membership never competes.
    float v[4];
    float best = -4.0;
    for (int i = 0; i < 4; i++) {
        float hv = useHeight ? blend_height_strength * (H[i] + prio[i]) : 0.0;
        v[i] = W[i] > 1e-3 ? W[i] + hv : -4.0;
        best = max(best, v[i]);
    }
    vec3 rgb = vec3(0.0);
    float total = 0.0;
    for (int i = 0; i < 4; i++) {
        float b = max(v[i] - best + depth, 0.0) / n[i];
        rgb += b * C[i];
        total += b;
    }
    return vec4(rgb / max(total, 1e-6), 1.0);
}

void main(void ) {
    vec4 color;

    // Tileset level for the blend path: atlas texels per screen pixel, from
    // derivatives taken here in uniform control flow.  Magnification stays at
    // level 0 (`NEAREST`, the pixel look); minification walks the tileset's
    // capped mip chain instead of aliasing.
    highp vec2 tcoord = vMapCoord * map_size * tile_pixel_size;
    highp float texelsPerPx = max(length(dFdx(tcoord)), length(dFdy(tcoord)));
    float lod = clamp(log2(max(texelsPerPx, 1e-6)), 0.0, blend_max_lod);

    if (vTileId > 0.5) {
        color = wall_color;
    } else {
        highp vec2 map_coord = vMapCoord;
        if (blend_enabled != 0) {
            color = applySelection(blendedTerrain(map_coord, lod), map_coord);
        } else {
            color = applySelection(getTileTexel(getMapData(map_coord), map_coord), map_coord);
        }
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
