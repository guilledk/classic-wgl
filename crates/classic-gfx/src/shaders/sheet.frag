#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D tex_sampler;

uniform vec2 tile_set_size;
uniform float tile_id_flat;
uniform float ghost_alpha;

out vec4 fragColor;

vec4 getTilePixel(float tile_id_flat, vec2 tex_coord) {
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
