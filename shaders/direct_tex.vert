attribute vec4 vertexPos;
attribute vec2 texCoord;

uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;

varying highp vec2 vTexCoord;

void main(void) {
    gl_Position = projectionMatrix * cameraMatrix * modelMatrix * vertexPos;
    vTexCoord = texCoord;
}
