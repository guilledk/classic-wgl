#version 300 es

precision mediump float;

in vec3 vertex_pos;

// Tile -> light space: `T(origin) * S(tile_scale) * Rz(-45deg)`.
uniform mat4 light_matrix;
uniform mat4 light_view_proj;

// Casters are rendered in *light space* (+Z up, metric), not the sheared screen
// space the main pass rasterises in.  Two distortions must stay out of it:
// the `y -= vertex_pos.z` shear (which carries height in both y and z and
// presents the sun at a near-degenerate grazing angle) and the isometric
// `diag(1, 0.5, 1)` squash (which halves y and makes the space non-metric).
// Must stay in step with `vLightPos` in `iso_tilemap.vert` and with
// `world_corner` in `classic-engine/src/shadow.rs`.
void main(void ) {
    gl_Position = light_view_proj * light_matrix * vec4(vertex_pos, 1.0);
}
