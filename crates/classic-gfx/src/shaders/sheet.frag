#version 300 es

precision mediump float;

in highp vec2 vTexCoord;

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

out vec4 fragColor;

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
    if (color.a < 0.01) discard;
    color.rgb *= tint;
    if (use_depth_map > 0.5) {
        highp float gray = texture(depth_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).r;
        // `depth_base` and `depth_range` are both window-space iso depths, so
        // `gl_FragDepth` (also window-space) needs no clip→window remap.
        gl_FragDepth = depth_base + (0.5 - gray) * depth_range;
    }
    if (use_normal_map > 0.5) {
        vec3 n = texture(normal_sampler, sheetUv(vec2(vTexCoord.x, vTexCoord.y))).rgb * 2.0 - 1.0;
        // Unlit sentinel: a (0.5,0.5,0.5) texel decodes to (0,0,0) and skips the
        // Lambertian term, so emissive sprite regions (e.g. the rocket flame)
        // stay flat albedo instead of being shaded.
        if (dot(n, n) > 0.001) {
            n = normalize(n);
            float diff = max(dot(n, light_direction), 0.0);
            color.rgb *= ambient_color + diff * light_color;
        }
    }
    if (ghost_alpha > 0.0) {
        color.a = ghost_alpha;
    }
    fragColor = color;
}
