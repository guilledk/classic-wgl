#version 300 es

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
// World metres -> squashed-cartesian screen pixels (before the shear).
uniform mat4 world_matrix;
// World metres -> light space (px, metric +Z up).
uniform mat4 light_matrix;
uniform float ppm;
uniform float use_iso_depth;
uniform vec4 iso_depth_corners;

out highp vec2 vTexCoord;
out highp vec3 vLightPos;

void main(void ) {
    vTexCoord = tex_coord;
    if (use_iso_depth > 0.5) {
        // Iso sprites are authored in Blender-world metres and share the
        // tilemap's world/light matrices, so sprites and terrain live in one
        // screen and one light space.
        highp vec3 world = (model_matrix * vertex_pos).xyz;
        highp vec4 worldPos = world_matrix * vec4(world, 1.0);
        worldPos.y -= ppm * world.z;
        vec4 clipPos = projection_matrix * camera_matrix * worldPos;
        float bottomDepth = mix(iso_depth_corners.x, iso_depth_corners.y, vertex_pos.x);
        float topDepth = mix(iso_depth_corners.z, iso_depth_corners.w, vertex_pos.x);
        float cornerDepth = mix(topDepth, bottomDepth, vertex_pos.y);
        // `cornerDepth` is window-space `[0, 1]`; map to clip z.
        clipPos.z = cornerDepth * 2.0 - 1.0;
        gl_Position = clipPos;
        vLightPos = (light_matrix * vec4(world, 1.0)).xyz;
    } else {
        // 2D screen-space sprites (cursor, HUD, UI): plain screen transform.
        gl_Position = projection_matrix * camera_matrix * model_matrix * vertex_pos;
        vLightPos = vec3(0.0);
    }
}
