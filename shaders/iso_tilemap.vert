attribute vec4 vertexPos;
attribute vec2 mapCoord;

uniform mat4 isoMatrix;
uniform mat4 modelMatrix;
uniform mat4 projectionMatrix;

uniform vec2 mapTileSize;

varying highp vec2 vMapCoord;

void main(void) {
    mat4 scaleMat = mat4(mat2(mapTileSize.x,0,0,mapTileSize.y));
    gl_Position = projectionMatrix * modelMatrix * isoMatrix * scaleMat *  vertexPos;
    vMapCoord = mapCoord;
}
