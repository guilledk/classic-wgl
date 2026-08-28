#version 300 es

precision mediump float;

in vec3 vertex_pos;

uniform mat4 model_matrix;
uniform mat4 iso_matrix;
uniform mat4 light_view_proj;

// Casters are rendered in *light space* (+Z up), not the sheared screen space
// the main pass rasterises in.  Applying the isometric `y -= vertex_pos.z`
// shear here would carry height in both y and z, presenting the sun at a
// near-degenerate grazing angle.  Must stay in step with `vLightPos` in
// `iso_tilemap.vert` and with `world_corner` in `classic-engine/src/shadow.rs`.
void main(void ) {
    vec4 lightPos = model_matrix * iso_matrix * vec4(vertex_pos, 1.0);
    gl_Position = light_view_proj * lightPos;
}
