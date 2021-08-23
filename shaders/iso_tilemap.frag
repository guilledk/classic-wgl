precision mediump float;

varying mediump vec2 vMapCoord;

uniform sampler2D mapData;
uniform vec2 mapSize;

uniform sampler2D tileSet;
uniform vec2 tileSetSize;
uniform vec2 tilePixelSize;

uniform vec2 selectedTile;
uniform vec2 selectionBegin;
uniform vec4 selectionColor;
uniform int selectionMode;


float getMapData(vec2 pos) {
    vec4 rawData = texture2D(mapData, pos);
    return floor(rawData.r * 256.0);
}

vec4 getTilePixel(float tileIdFlat, vec2 mapCoord) {
    vec2 tileId = vec2(
        floor(mod(tileIdFlat, tileSetSize.x)),
        floor(tileIdFlat / tileSetSize.x));

    vec2 mapTileNormalSize = vec2(1, 1) / mapSize;
    vec2 setNormalSize = vec2(1, 1) / tileSetSize;

    vec2 tileCornerNorm = tileId * setNormalSize;

    vec2 localTileCoord = fract(
        mapCoord / mapTileNormalSize) * setNormalSize;

    vec4 texColor = texture2D(tileSet, tileCornerNorm + localTileCoord);
    
    if (selectionMode != -1) {
        vec2 selectedNormalStart = floor(min(selectionBegin, selectedTile)) * mapTileNormalSize;
        vec2 selectedNormalEnd = ceil(max(selectionBegin, selectedTile)) * mapTileNormalSize;

        bvec2 selectStart = greaterThanEqual(mapCoord, selectedNormalStart);
        bvec2 selectEnd = lessThanEqual(mapCoord, selectedNormalEnd);

        if (all(selectStart) && all(selectEnd)) {
            if (selectionMode == 0)  // invert
                return vec4(
                    1.0 - texColor.r,
                    1.0 - texColor.g,
                    1.0 - texColor.b,
                    1.0);

            if (selectionMode == 1)  {  // colorized
                float average = (texColor.r + texColor.g + texColor.b) / 3.0;
                return vec4(average, average, average, texColor.a) * selectionColor;
            }
        }
    }
    
    return texColor;
}

void main(void) {
    vec2 mapCoord = vec2(vMapCoord.x, vMapCoord.y);
    
    gl_FragColor = getTilePixel(
        getMapData(mapCoord), mapCoord);

    // gl_FragColor = vec4(1.0 / float(tileId), 0.0, 0.0, 1.0);

}
