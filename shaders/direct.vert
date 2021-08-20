attribute vec4 vertexPos;

uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;

void main(void) {
    gl_Position = projectionMatrix * cameraMatrix * modelMatrix * vertexPos;
}
