#version 300 es

precision highp float;

in vec2 vUv;

// The model pixelation target (`ModelTarget`): lit colour + camera view depth,
// both NEAREST-sampled, so each low-res texel lands as a solid screen block.
uniform sampler2D color_tex;
uniform highp sampler2D depth_tex;
// 0 for the normal composite; the sprite ghost alpha (0.4) for the ghost pass.
uniform float ghost_alpha;

out vec4 fragColor;

void main(void ) {
    vec4 color = texture(color_tex, vUv);
    float depth = texture(depth_tex, vUv).r;
    // Uncovered texels (cleared to alpha 0 / depth 1) are not part of a model.
    if (color.a < 0.5 || depth >= 1.0) {
        discard;
    }
    // The model's own view depth, so terrain and sprites depth-test against it
    // exactly as against a directly drawn mesh.
    gl_FragDepth = depth;
    fragColor = vec4(color.rgb, ghost_alpha > 0.0 ? ghost_alpha : 1.0);
}
