#version 300 es

// Additive bloom composite: the half-res (tight) and quarter-res (wide) blur
// levels, added onto the scene (blend ONE, ONE).

precision mediump float;

in vec2 vUv;

uniform sampler2D near_tex;
uniform sampler2D far_tex;
uniform float intensity;

out vec4 fragColor;

void main(void ) {
    vec3 c = texture(near_tex, vUv).rgb + 1.4 * texture(far_tex, vUv).rgb;
    fragColor = vec4(c * intensity, 0.0);
}
