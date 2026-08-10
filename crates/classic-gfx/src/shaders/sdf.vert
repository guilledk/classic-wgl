#version 300 es

in vec4 vertexPos;
in vec2 texCoord;

uniform mat4 modelMatrix;
uniform mat4 cameraMatrix;
uniform mat4 projectionMatrix;

out mediump vec2 vTexCoord;

void main(void ) {
    gl_Position = projectionMatrix * cameraMatrix * modelMatrix * vertexPos;
    vTexCoord = texCoord;
}
