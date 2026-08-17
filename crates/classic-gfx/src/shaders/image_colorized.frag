#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D tex_sampler;
uniform vec4 color;

out vec4 fragColor;

vec4 grayscale(vec4 v) {
    float average = (v.r + v.g + v.b) / 3.0;
    return vec4(average, average, average, v.a);
}

vec4 colorize(vec4 grayscale, vec4 c) {
    return grayscale * c;
}

void main(void ) {
    vec4 texColor = texture(tex_sampler, vTexCoord);
    vec4 grayScale = grayscale(texColor);
    fragColor = colorize(grayScale, color);
}
