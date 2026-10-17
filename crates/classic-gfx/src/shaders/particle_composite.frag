#version 300 es

// Half-res particle composite: the target already holds premultiplied
// colour + coverage, so the blend (ONE, ONE_MINUS_SRC_ALPHA) is exact "over".

precision mediump float;

in vec2 vUv;

uniform sampler2D color_tex;

out vec4 fragColor;

void main(void ) {
    fragColor = texture(color_tex, vUv);
}
