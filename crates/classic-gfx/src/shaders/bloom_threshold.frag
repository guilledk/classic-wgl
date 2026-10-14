#version 300 es

// Bloom bright-pass + 2x downsample: a 4-tap box over the source (bilinear,
// so 16 texels), then a soft-knee luminance threshold.  `threshold < 0`
// is a plain downsample (used between blur levels).

precision mediump float;

in vec2 vUv;

uniform sampler2D src_tex;
uniform vec2 src_texel;
uniform float threshold;
uniform float knee;

out vec4 fragColor;

void main(void ) {
    vec2 o = src_texel;
    vec3 c = 0.25 * (texture(src_tex, vUv + vec2(-o.x, -o.y)).rgb
        + texture(src_tex, vUv + vec2(o.x, -o.y)).rgb
        + texture(src_tex, vUv + vec2(-o.x, o.y)).rgb
        + texture(src_tex, vUv + vec2(o.x, o.y)).rgb);
    if (threshold >= 0.0) {
        float l = max(c.r, max(c.g, c.b));
        c *= smoothstep(threshold, threshold + knee, l);
    }
    fragColor = vec4(c, 1.0);
}
