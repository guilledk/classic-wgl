#version 300 es

precision highp float;

in vec3 vertex_pos;
in vec3 normal;
in vec2 tex_coord;

// `model_matrix` = node world transform (axis-fixed into world metres) * model
// placement.  `world_matrix` = `iso_camera_matrix` (world -> camera view).
uniform mat4 model_matrix;
uniform mat4 world_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;

// Camera view-depth bounds `[near, far]` (metres).
uniform vec2 depth_span;
// Pixels per metre (`PPM_TARGET`).
uniform float ppm;

out highp vec2 vTexCoord;
out highp vec3 vNormal;
out highp vec3 vLightPos;

void main(void ) {
    // Node world position (metres, +Z up).
    highp vec3 world = (model_matrix * vec4(vertex_pos, 1.0)).xyz;
    // Camera view: `(right·w, up·w, back·w)` in metres.
    highp vec4 view = world_matrix * vec4(world, 1.0);
    highp vec4 screenPos = vec4(view.x * ppm, -view.y * ppm, 0.0, 1.0);
    vec4 clipPos = projection_matrix * camera_matrix * screenPos;
    // Camera view depth in window space `[0, 1]` (0 = nearest, 1 = farthest).
    highp float isoDepth = (depth_span.x - view.z) / (depth_span.x - depth_span.y);
    clipPos.z = isoDepth * 2.0 - 1.0;
    gl_Position = clipPos;

    // Match the sprite convention (direct_tex.vert): the texture is uploaded
    // top-to-bottom (row 0 = image top = GL V 0), so glTF UVs (top-left origin)
    // map 1:1 — no V-flip.
    vTexCoord = tex_coord;
    // The model's node transform is rigid (rotation + translation, ~unit scale),
    // so the world normal is `mat3(model_matrix) * normal` (normalized).
    vNormal = normalize(mat3(model_matrix) * normal);
    vLightPos = world;
}
