#version 300 es

precision mediump float;

in vec3 vertex_pos;
in vec2 map_coord;
in float tile_id;
in vec3 normal;

// World metres -> camera view space (metres): `iso_camera_matrix`.
uniform mat4 world_matrix;
uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;

uniform vec2 map_size;
uniform vec2 tile_pixel_size;
// Camera view-depth bounds `[near, far]` (metres).  `near` is the closest view
// depth (most positive `dot(back, world)`), `far` the farthest; `near > far`
// numerically.
uniform vec2 depth_span;
// Pixels per metre (`PPM_TARGET`) — the raster scale of the camera view.
uniform float ppm;

out mediump vec2 vMapCoord;
out mediump float vTileId;
out highp vec3 vNormal;
out highp vec3 vLightPos;

void main(void ) {
    vec3 world = (model_matrix * vec4(vertex_pos, 1.0)).xyz;
    // Camera view: `(right·w, up·w, back·w)` in metres.
    vec4 view = world_matrix * vec4(world, 1.0);
    // Screen pixels before pan/zoom.  The camera `up` axis projects to the
    // negative old-cartesian y, hence the `-view.y`.
    vec4 screenPos = vec4(view.x * ppm, -view.y * ppm, 0.0, 1.0);
    vec4 clipPos = projection_matrix * camera_matrix * screenPos;
    // Camera view depth in window space `[0, 1]` (0 = nearest, 1 = farthest).
    highp float isoDepth = (depth_span.x - view.z) / (depth_span.x - depth_span.y);
    clipPos.z = isoDepth * 2.0 - 1.0;
    gl_Position = clipPos;
    vMapCoord = map_coord;
    vTileId = tile_id;
    // Terrain normals are baked in world space (metres, +Z up); lighting is
    // done in that same world space, so there is no normal transform here.
    vNormal = normalize(normal);
    vLightPos = world;
}
