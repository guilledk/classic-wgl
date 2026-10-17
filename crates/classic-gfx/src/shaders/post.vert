#version 300 es

precision highp float;

// Fullscreen pass over the shared unit quad (corners 0..1).
in vec3 vertex_pos;

out vec2 vUv;

void main(void ) {
    vUv = vertex_pos.xy;
    gl_Position = vec4(vertex_pos.xy * 2.0 - 1.0, 0.0, 1.0);
}
