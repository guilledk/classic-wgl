precision mediump float;

attribute vec4 vertexPos;
attribute vec2 mapCoord;

uniform mat4 isoMatrix;
uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;

uniform vec2 mapSize;

uniform vec2 tilePixelSize;

varying mediump vec2 vMapCoord;

void main(void) {
    gl_Position =
        projectionMatrix *
        cameraMatrix *
        modelMatrix *
        isoMatrix * 
        vertexPos;

    vMapCoord = mapCoord;
}
