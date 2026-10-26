#version 300 es

in vec4 vertex_pos;
in vec2 tex_coord;

#ifdef INSTANCED
// Instanced iso sprites (`sprite_batch.rs`): the per-sprite uniforms of the
// one-draw-per-sprite path arrive as per-instance attributes instead, and
// the fragment values travel as flat varyings.  The maths below is the same
// source as the uniform path (the names are macros), so the two paths
// rasterise the same quad.
in vec4 i_m0;
in vec4 i_m1;
in vec4 i_m3;
in vec4 i_corners;
in highp vec4 i_uv;
in highp vec4 i_content;
in vec4 i_tint;
in highp vec4 i_misc;
in highp vec4 i_outline;
in vec4 i_team;
flat out highp vec4 vUvRect;
flat out highp vec4 vContent;
flat out vec4 vTint;
flat out highp vec4 vMisc;
flat out highp vec4 vOutline;
flat out vec4 vTeam;
#ifdef INTERP
// Direction blend (`sheet.frag#INTERP`): direction B's region, the anchors
// and rotations, and the drawn quad.
in highp vec4 i_uv_b;
in highp vec4 i_content_b;
in highp vec4 i_interp;
in highp vec4 i_quad;
flat out highp vec4 vUvB;
flat out highp vec4 vContentB;
flat out highp vec4 vInterp;
flat out highp vec4 vQuad;
// The world-frame tilt residual quaternion `xyz` (`w >= 0`), from the
// model columns' w.
flat out highp vec3 vTilt;
#endif
// The quad has z = 0, so the model's z column never contributes.
// The model is affine: its w row is rebuilt (a direction blend carries its
// tilt residual in the columns' w, see `IsoSpriteInstance::m`).
#define model_matrix mat4(vec4(i_m0.xyz, 0.0), vec4(i_m1.xyz, 0.0), vec4(0.0), vec4(i_m3.xyz, 1.0))
#define iso_depth_corners i_corners
#define use_iso_depth 1.0
#else
uniform mat4 model_matrix;
uniform float use_iso_depth;
uniform vec4 iso_depth_corners;
#endif

uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
// World metres -> camera view space (metres): `iso_camera_matrix`.
uniform mat4 world_matrix;
uniform float ppm;

out highp vec2 vTexCoord;
out highp vec3 vLightPos;

void main(void ) {
    vTexCoord = tex_coord;
#ifdef INSTANCED
    vUvRect = i_uv;
    vContent = i_content;
    vTint = i_tint;
    vMisc = i_misc;
    vOutline = i_outline;
    vTeam = i_team;
#ifdef INTERP
    vUvB = i_uv_b;
    vContentB = i_content_b;
    vInterp = i_interp;
    vQuad = i_quad;
    vTilt = vec3(i_m0.w, i_m1.w, i_m3.w);
#endif
#endif
    if (use_iso_depth > 0.5) {
        // Iso sprites are authored in Blender-world metres and share the
        // tilemap's world matrix, so sprites and terrain live in one screen
        // and one lighting (world) space.
        highp vec3 world = (model_matrix * vertex_pos).xyz;
        highp vec4 view = world_matrix * vec4(world, 1.0);
        highp vec4 screenPos = vec4(view.x * ppm, -view.y * ppm, 0.0, 1.0);
        vec4 clipPos = projection_matrix * camera_matrix * screenPos;
        float bottomDepth = mix(iso_depth_corners.x, iso_depth_corners.y, vertex_pos.x);
        float topDepth = mix(iso_depth_corners.z, iso_depth_corners.w, vertex_pos.x);
        float cornerDepth = mix(topDepth, bottomDepth, vertex_pos.y);
        // `cornerDepth` is window-space `[0, 1]`; map to clip z.
        clipPos.z = cornerDepth * 2.0 - 1.0;
        gl_Position = clipPos;
        vLightPos = world;
    } else {
        // 2D screen-space sprites (cursor, HUD, UI): plain screen transform.
        gl_Position = projection_matrix * camera_matrix * model_matrix * vertex_pos;
        vLightPos = vec3(0.0);
    }
}
