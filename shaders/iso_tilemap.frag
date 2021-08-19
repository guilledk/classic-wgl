precision highp float;
varying highp vec2 vMapCoord;

uniform sampler2D mapData;

float getMapData(vec2 pos) {
    vec4 rawData = texture2D(mapData, pos);
    return rawData.r;
}

void main(void) {
    vec2 tileCoord = vec2(vMapCoord.x, vMapCoord.y);
    gl_FragColor = vec4(getMapData(tileCoord), 0, 0, 1.0);
}
