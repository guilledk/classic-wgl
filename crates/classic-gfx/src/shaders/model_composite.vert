#version 300 es

precision highp float;

// The shared unit quad (`QuadBuffers`, corners 0..1) stretched over the whole
// render target.  `vUv` addresses the model pixelation target directly: it was
// rendered with the same projection, so no flip or offset is needed.
in vec3 vertex_pos;

out vec2 vUv;

void main(void ) {
    vUv = vertex_pos.xy;
    gl_Position = vec4(vertex_pos.xy * 2.0 - 1.0, 0.0, 1.0);
}
