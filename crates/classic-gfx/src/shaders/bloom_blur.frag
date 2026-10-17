#version 300 es

// Separable 9-tap Gaussian as 5 bilinear taps (weights/offsets from the
// classic linear-sampling decomposition).

precision mediump float;

in vec2 vUv;

uniform sampler2D src_tex;
// One texel along the blur axis.
uniform vec2 blur_dir;

out vec4 fragColor;

void main(void ) {
    vec3 c = texture(src_tex, vUv).rgb * 0.2270270270;
    c += texture(src_tex, vUv + blur_dir * 1.3846153846).rgb * 0.3162162162;
    c += texture(src_tex, vUv - blur_dir * 1.3846153846).rgb * 0.3162162162;
    c += texture(src_tex, vUv + blur_dir * 3.2307692308).rgb * 0.0702702703;
    c += texture(src_tex, vUv - blur_dir * 3.2307692308).rgb * 0.0702702703;
    fragColor = vec4(c, 1.0);
}
