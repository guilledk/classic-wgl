#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D tex_sampler;

out vec4 fragColor;

void main(void ) {
    fragColor = texture(tex_sampler, vTexCoord);
}
