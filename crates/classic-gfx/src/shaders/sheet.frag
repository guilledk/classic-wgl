#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D texSampler;

uniform vec2 tileSetSize;
uniform float tileIdFlat;
uniform float ghostAlpha;

out vec4 fragColor;

vec4 getTilePixel(float tileIdFlat, vec2 texCoord) {
    vec2 tileId = vec2(floor(mod(tileIdFlat, tileSetSize.x)), floor(tileIdFlat / tileSetSize.x));

    vec2 setNormalSize = vec2(1, 1) / tileSetSize;

    vec2 tileCornerNorm = tileId * setNormalSize;
    vec2 localTileCoord = texCoord * setNormalSize;

    return texture(texSampler, tileCornerNorm + localTileCoord);
}

void main(void ) {
    vec4 color = getTilePixel(tileIdFlat, vec2(vTexCoord.x, vTexCoord.y));
    if (color.a < 0.01) discard;
    if (ghostAlpha > 0.0) {
        color.a = ghostAlpha;
    }
    fragColor = color;
}
