#version 300 es

precision mediump float;

// Depth-only pass: the FBO has no color attachment (`draw_buffers([NONE])`), so
// this output is masked; the hardware writes the depth from `gl_Position`.
out vec4 fragColor;

void main(void ) {
    fragColor = vec4(1.0);
}
