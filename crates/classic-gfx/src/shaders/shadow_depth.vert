#version 300 es

precision mediump float;

in vec3 vertex_pos;

uniform mat4 model_matrix;
uniform mat4 iso_matrix;
uniform mat4 light_view_proj;

void main(void ) {
    vec4 worldPos = model_matrix * iso_matrix * vec4(vertex_pos, 1.0);
    worldPos.y -= vertex_pos.z;
    gl_Position = light_view_proj * worldPos;
}
