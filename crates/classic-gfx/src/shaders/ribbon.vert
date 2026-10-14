#version 300 es

// Electrical arcs / lightning: CPU-expanded camera-facing strips whose
// vertices are already in pre-camera screen pixels with a window depth
// (`classic-engine/src/particles/ribbon.rs`).

precision highp float;

in vec3 a_pos;    // screen x, y (pre-camera px), window depth
in vec3 a_shape;  // across v in [-1, 1], core fraction, intensity
in vec3 a_color;

uniform mat4 camera_matrix;
uniform mat4 projection_matrix;

out float vAcross;
out float vCore;
out float vIntensity;
out vec3 vColor;

void main(void ) {
    vec4 clipPos = projection_matrix * camera_matrix * vec4(a_pos.xy, 0.0, 1.0);
    clipPos.z = a_pos.z * 2.0 - 1.0;
    gl_Position = clipPos;
    vAcross = a_shape.x;
    vCore = a_shape.y;
    vIntensity = a_shape.z;
    vColor = a_color;
}
