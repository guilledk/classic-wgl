#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D texSampler;

out vec4 fragColor;

void main(void ) {
    fragColor = texture(texSampler, vTexCoord);
}
