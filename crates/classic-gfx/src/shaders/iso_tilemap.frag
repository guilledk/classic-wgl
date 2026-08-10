#version 300 es

precision mediump float;

in mediump vec2 vMapCoord;
in mediump float vTileId;
in mediump vec3 vNormal;

uniform sampler2D mapData;
uniform vec2 mapSize;

uniform sampler2D tileSet;
uniform vec2 tileSetSize;
uniform vec2 tilePixelSize;

uniform vec2 selectedTile;
uniform vec2 selectionBegin;
uniform vec4 selectionColor;
uniform int selectionMode;
uniform vec4 wallColor;

uniform float gridRadius;
uniform int showGrid;
uniform vec3 gridColor;

uniform vec3 ambientColor;
uniform vec3 lightDirection;
uniform vec3 lightColor;

out vec4 fragColor;

float getMapData(vec2 pos) {
    vec4 rawData = texture(mapData, pos);
    return floor(rawData.r * 256.0);
}

vec4 getTilePixel(float tileIdFlat, vec2 mapCoord) {
    vec2 tileId = vec2(floor(mod(tileIdFlat, tileSetSize.x)), floor(tileIdFlat / tileSetSize.x));

    vec2 mapTileNormalSize = vec2(1, 1) / mapSize;
    vec2 setNormalSize = vec2(1, 1) / tileSetSize;

    vec2 tileCornerNorm = tileId * setNormalSize;

    vec2 localTileCoord = fract(mapCoord / mapTileNormalSize) * setNormalSize;

    vec4 texColor = texture(tileSet, tileCornerNorm + localTileCoord);

    if (selectionMode != -1) {
        vec2 selectedNormalStart = floor(min(selectionBegin, selectedTile)) * mapTileNormalSize;
        vec2 selectedNormalEnd = ceil(max(selectionBegin, selectedTile)) * mapTileNormalSize;

        bvec2 selectStart = greaterThanEqual(mapCoord, selectedNormalStart);
        bvec2 selectEnd = lessThanEqual(mapCoord, selectedNormalEnd);

        if (all(selectStart) && all(selectEnd)) {
            if (selectionMode == 0)
                return vec4(1.0 - texColor.r, 1.0 - texColor.g, 1.0 - texColor.b, 1.0);

            if (selectionMode == 1) {
                float average = (texColor.r + texColor.g + texColor.b) / 3.0;
                return vec4(average, average, average, texColor.a) * selectionColor;
            }
        }
    }

    return texColor;
}

void main(void ) {
    vec4 color;

    if (vTileId > 0.5) {
        color = wallColor;
    } else {
        vec2 mapCoord = vec2(vMapCoord.x, vMapCoord.y);
        color = getTilePixel(getMapData(mapCoord), mapCoord);
    }

    if (color.a < 0.01) discard;

    float diff = max(dot(normalize(vNormal), lightDirection), 0.0);
    color.rgb *= ambientColor + diff * lightColor;

    if (showGrid > 0 && selectionMode == -1 && vTileId <= 0.5) {
        vec2 tileCoord = vMapCoord * mapSize;
        vec2 localUV = fract(tileCoord);
        float mt = floor(selectedTile.x);
        float nt = floor(selectedTile.y);
        float ct = floor(tileCoord.x);
        float rt = floor(tileCoord.y);
        float dist = max(abs(ct - mt), abs(nt - rt));
        if (dist <= gridRadius) {
            float edge = 0.04;
            float dx = min(localUV.x, 1.0 - localUV.x);
            float dy = min(localUV.y, 1.0 - localUV.y);
            float edgeDist = min(dx, dy);
            float border = 1.0 - smoothstep(0.0, edge, edgeDist);
            float fade = 1.0 - dist / max(gridRadius, 0.01);
            color.rgb = mix(color.rgb, gridColor, border * fade * 0.85);
        }
    }

    fragColor = color;
}
