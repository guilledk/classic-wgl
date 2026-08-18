#version 300 es

in vec4 vertex_pos;

uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;

void main(void ) {
    gl_Position = projection_matrix * camera_matrix * model_matrix * vertex_pos;
}
