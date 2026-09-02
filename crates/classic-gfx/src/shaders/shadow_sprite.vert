#version 300 es

precision mediump float;

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
// World metres -> light space (px, metric +Z up).
uniform mat4 light_matrix;
uniform mat4 light_view_proj;

out highp vec2 vTexCoord;

// The sprite is authored as a world-metre quad standing up out of the terrain
// (width along the isometric right direction, height along world Z), so it
// casts into the shadow map as standing geometry — no billboard unproject.
// This must stay identical to the `vLightPos` reconstruction in
// `direct_tex.vert`, or a sprite will not lie in its own shadow.
void main(void ) {
    highp vec3 world = (model_matrix * vertex_pos).xyz;
    highp vec3 lightPos = (light_matrix * vec4(world, 1.0)).xyz;
    gl_Position = light_view_proj * vec4(lightPos, 1.0);
    vTexCoord = tex_coord;
}
