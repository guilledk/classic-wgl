precision mediump float;

varying mediump vec2 vTexCoord;

uniform sampler2D texSampler;

uniform vec2 tileSetSize;
uniform float tileIdFlat;

vec4 getTilePixel(float tileIdFlat, vec2 texCoord) {
    vec2 tileId = vec2(
        floor(mod(tileIdFlat, tileSetSize.x)),
        floor(tileIdFlat / tileSetSize.x));

    vec2 setNormalSize = vec2(1, 1) / tileSetSize;

    vec2 tileCornerNorm = tileId * setNormalSize;
    vec2 localTileCoord = texCoord * setNormalSize;

    return texture2D(texSampler, tileCornerNorm + localTileCoord);
}

void main(void) {
    gl_FragColor = getTilePixel(tileIdFlat, vec2(vTexCoord.x, vTexCoord.y));
}
