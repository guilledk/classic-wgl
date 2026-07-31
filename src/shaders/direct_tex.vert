attribute vec4 vertexPos;
attribute vec2 texCoord;

uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;
uniform float useIsoDepth;
uniform float isoDepth;

varying mediump vec2 vTexCoord;

void main(void ) {
    gl_Position = projectionMatrix * cameraMatrix * modelMatrix * vertexPos;
    if (useIsoDepth > 0.5) {
        gl_Position.z = clamp(isoDepth, 0.0, 1.0);
    }
    vTexCoord = texCoord;
}
