#version 300 es

precision mediump float;

in mediump vec2 vMapCoord;
in mediump float vTileId;
in mediump vec3 vNormal;

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

out vec4 fragColor;

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

    float diff = max(dot(normalize(vNormal), light_direction), 0.0);
    color.rgb *= ambient_color + diff * light_color;

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
