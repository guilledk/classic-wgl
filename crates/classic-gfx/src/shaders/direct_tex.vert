#version 300 es

in vec4 vertexPos;
in vec2 texCoord;

uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;
uniform float useIsoDepth;
uniform vec4 isoDepthCorners;

out mediump vec2 vTexCoord;

void main(void ) {
    gl_Position = projectionMatrix * cameraMatrix * modelMatrix * vertexPos;
    if (useIsoDepth > 0.5) {
        float bottomDepth = mix(isoDepthCorners.x, isoDepthCorners.y, vertexPos.x);
        float topDepth = mix(isoDepthCorners.z, isoDepthCorners.w, vertexPos.x);
        float cornerDepth = mix(topDepth, bottomDepth, vertexPos.y);
        gl_Position.z = clamp(cornerDepth, 0.0, 1.0);
    }
    vTexCoord = texCoord;
}
