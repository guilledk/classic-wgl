#version 300 es

precision mediump float;

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
uniform mat4 light_view_proj;

out highp vec2 vTexCoord;

void main(void ) {
    vec4 worldPos = model_matrix * vertex_pos;
    gl_Position = light_view_proj * worldPos;
    vTexCoord = tex_coord;
}
