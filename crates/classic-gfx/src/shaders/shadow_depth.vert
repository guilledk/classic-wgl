#version 300 es

precision mediump float;

in vec3 vertex_pos;

// World metres -> light clip space: `light_view_proj` maps the world-space
// terrain mesh (offset by the tilemap's `model_matrix`) straight into the
// shadow map's clip space — there is no separate light-space transform.
uniform mat4 model_matrix;
uniform mat4 light_view_proj;

void main(void ) {
    gl_Position = light_view_proj * model_matrix * vec4(vertex_pos, 1.0);
}
