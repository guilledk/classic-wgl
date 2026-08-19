#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D tex_sampler;

uniform vec2 tile_set_size;
uniform float tile_id_flat;
uniform float ghost_alpha;
uniform float use_uv_rect;
uniform vec4 uv_rect;
uniform highp vec2 trim_offset;
uniform highp vec2 source_size;
uniform highp vec2 content_size;

out vec4 fragColor;

vec4 getTilePixel(float tile_id_flat, vec2 tex_coord) {
    if (use_uv_rect > 0.5) {
        // The quad is drawn at `source_size`; the trimmed content sits at
        // `trim_offset` within it (both in source pixels), so map tex_coord
        // to the content sub-rect and discard the surrounding padding.
        highp vec2 content_min = trim_offset / source_size;
        highp vec2 content_ext = content_size / source_size;
        highp vec2 content_uv = (tex_coord - content_min) / content_ext;
        if (content_uv.x < 0.0 || content_uv.x > 1.0
            || content_uv.y < 0.0 || content_uv.y > 1.0) {
            discard;
        }
        highp vec2 uv = mix(uv_rect.xy, uv_rect.zw, content_uv);
        return texture(tex_sampler, uv);
    }

    vec2 tile_id = vec2(floor(mod(tile_id_flat, tile_set_size.x)), floor(tile_id_flat / tile_set_size.x));

    vec2 setNormalSize = vec2(1, 1) / tile_set_size;

    vec2 tileCornerNorm = tile_id * setNormalSize;
    vec2 localTileCoord = tex_coord * setNormalSize;

    return texture(tex_sampler, tileCornerNorm + localTileCoord);
}

void main(void ) {
    vec4 color = getTilePixel(tile_id_flat, vec2(vTexCoord.x, vTexCoord.y));
    if (color.a < 0.01) discard;
    if (ghost_alpha > 0.0) {
        color.a = ghost_alpha;
    }
    fragColor = color;
}
