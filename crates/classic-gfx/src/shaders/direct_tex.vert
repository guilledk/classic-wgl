#version 300 es

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
uniform float use_iso_depth;
uniform vec4 iso_depth_corners;

out mediump vec2 vTexCoord;

void main(void ) {
    gl_Position = projection_matrix * camera_matrix * model_matrix * vertex_pos;
    if (use_iso_depth > 0.5) {
        float bottomDepth = mix(iso_depth_corners.x, iso_depth_corners.y, vertex_pos.x);
        float topDepth = mix(iso_depth_corners.z, iso_depth_corners.w, vertex_pos.x);
        float cornerDepth = mix(topDepth, bottomDepth, vertex_pos.y);
        gl_Position.z = clamp(cornerDepth, 0.0, 1.0);
    }
    vTexCoord = tex_coord;
}
