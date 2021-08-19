precision highp float;

varying highp vec2 vMapCoord;

uniform sampler2D mapData;
uniform vec2 mapSize;

uniform sampler2D tileSet;
uniform vec2 tileSetSize;
uniform vec2 tilePixelSize;


float getMapData(vec2 pos) {
    vec4 rawData = texture2D(mapData, pos);
    return rawData.r * 256.0;
}

vec4 getTilePixel(float tileIdFlat, vec2 mapCoord) {
    vec2 tileId = vec2(
        floor(mod(tileIdFlat, tileSetSize.x)),
        floor(tileIdFlat / tileSetSize.x));

    vec2 mapTileNormalSize = vec2(1, 1) / mapSize;
    vec2 setNormalSize = tilePixelSize / (tileSetSize * tilePixelSize);

    vec2 tileCornerNorm = tileId / tileSetSize;

    vec2 localTileCoord = fract(
        mapCoord / mapTileNormalSize) * setNormalSize;

    return texture2D(tileSet, tileCornerNorm + localTileCoord);

}

void main(void) {
    vec2 tileCoord = vec2(vMapCoord.x, vMapCoord.y);
    float tileId = getMapData(tileCoord);
    
    gl_FragColor = getTilePixel(
        getMapData(tileCoord), tileCoord);

    // gl_FragColor = vec4(1.0 / float(tileId), 0.0, 0.0, 1.0);

}
