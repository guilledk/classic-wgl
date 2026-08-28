#version 300 es

precision mediump float;

in highp vec2 vTexCoord;

uniform sampler2D tex_sampler;
uniform vec2 tile_set_size;
uniform float tile_id_flat;
uniform float use_uv_rect;
uniform highp vec4 uv_rect;
uniform highp vec2 trim_offset;
uniform highp vec2 source_size;
uniform highp vec2 content_size;

out vec4 fragColor;

vec2 tileUv(float tile_id_flat, vec2 tex_coord) {
    vec2 tile_id = vec2(floor(mod(tile_id_flat, tile_set_size.x)), floor(tile_id_flat / tile_set_size.x));

    vec2 setNormalSize = vec2(1, 1) / tile_set_size;

    vec2 tileCornerNorm = tile_id * setNormalSize;
    vec2 localTileCoord = tex_coord * setNormalSize;

    return tileCornerNorm + localTileCoord;
}

// Same sprite-sheet UV mapping as `sheet.frag`, so the shadow silhouette lines
// up with the sprite's colour frame (packed-atlas rect or uniform-grid frame).
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

// Depth-only pass: keep the sprite's alpha silhouette (discard transparent
// pixels) so it casts a shaped shadow, not a full billboard quad.  The depth
// written is the billboard's own light-space depth.
void main(void ) {
    float alpha = texture(tex_sampler, sheetUv(vTexCoord)).a;
    if (alpha < 0.5) discard;
    fragColor = vec4(1.0);
}
