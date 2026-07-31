precision mediump float;

attribute vec3 vertexPos;
attribute vec2 mapCoord;
attribute float tileId;

uniform mat4 isoMatrix;
uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;

uniform vec2 mapSize;
uniform vec2 tilePixelSize;

varying mediump vec2 vMapCoord;
varying mediump float vTileId;
varying mediump float vIsoDepth;

void main(void ) {
    vec4 worldPos = modelMatrix * isoMatrix * vec4(vertexPos, 1.0);
    worldPos.y -= vertexPos.z;
    vec4 clipPos = projectionMatrix * cameraMatrix * worldPos;
    float isoDepth = clamp(
        (vertexPos.x - vertexPos.y) / 400.0 + 0.5 - vertexPos.z / 14500.0,
        0.0,
        1.0
    );
    clipPos.z = isoDepth;
    gl_Position = clipPos;
    vMapCoord = mapCoord;
    vTileId = tileId;
    vIsoDepth = isoDepth;
}
