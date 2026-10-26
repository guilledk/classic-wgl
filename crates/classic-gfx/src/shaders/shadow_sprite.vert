#version 300 es

precision highp float;

in vec4 vertex_pos;
in vec2 tex_coord;

#ifdef INSTANCED
// Instanced casters (`sprite_batch.rs`): the model matrix and the frame's
// region arrive per instance.
in vec4 i_m0;
in vec4 i_m1;
in vec4 i_m3;
in vec4 i_uv;
in vec4 i_content;
in vec4 i_outline;
in vec4 i_misc;
flat out highp vec4 vUvRect;
flat out highp vec4 vContent;
flat out highp vec2 vSource;
flat out float vGrid;
// The model is affine: its w row is rebuilt (a direction blend carries its
// tilt residual in the columns' w, see `IsoSpriteInstance::m`).
#define model_matrix mat4(vec4(i_m0.xyz, 0.0), vec4(i_m1.xyz, 0.0), vec4(0.0), vec4(i_m3.xyz, 1.0))
#else
uniform mat4 model_matrix;
#endif
// World metres -> light clip space (the shadow map's own world-space view/proj).
uniform mat4 light_view_proj;

out highp vec2 vTexCoord;

// The sprite is authored as a world-metre quad standing up out of the terrain
// (width along the isometric right direction, height along world Z), so it
// casts into the shadow map as standing geometry — no billboard unproject.
// This must stay identical to the `vLightPos` reconstruction in
// `direct_tex.vert`, or a sprite will not lie in its own shadow.
void main(void ) {
    highp vec3 world = (model_matrix * vertex_pos).xyz;
    gl_Position = light_view_proj * vec4(world, 1.0);
    vTexCoord = tex_coord;
#ifdef INSTANCED
    vUvRect = i_uv;
    vContent = i_content;
    vSource = i_outline.zw;
    vGrid = i_misc.w;
#endif
}
