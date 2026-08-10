#version 300 es

precision mediump float;

in vec3 vertexPos;
in vec2 mapCoord;
in float tileId;
in vec3 normal;

uniform mat4 isoMatrix;
uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;
uniform mat3 normalMatrix;

uniform vec2 mapSize;
uniform vec2 tilePixelSize;

out mediump vec2 vMapCoord;
out mediump float vTileId;
out mediump vec3 vNormal;

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
    vNormal = normalize(normalMatrix * normal);
}
