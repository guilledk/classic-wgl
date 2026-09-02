#version 300 es

precision mediump float;

in vec3 vertex_pos;
in vec2 map_coord;
in float tile_id;
in vec3 normal;

// World metres -> squashed-cartesian screen pixels (before the shear).
// `vertex_pos` is now authored in world metres, so this replaces the old
// tile-space `iso_matrix` (see `classic_core::math::iso_world_matrix`).
uniform mat4 world_matrix;
uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
uniform mat3 normal_matrix;
// World metres -> light space (px, metric +Z up).  Replaces the old tile-space
// `light_matrix` (see `classic_core::math::iso_world_light_matrix`).
uniform mat4 light_matrix;

uniform vec2 map_size;
uniform vec2 tile_pixel_size;
uniform vec2 depth_scale;
uniform float ppm;

out mediump vec2 vMapCoord;
out mediump float vTileId;
out highp vec3 vNormal;
out highp vec3 vLightPos;

void main(void ) {
    vec3 world = (model_matrix * vec4(vertex_pos, 1.0)).xyz;
    // Screen space: the isometric projection plus the shear that makes height
    // visible.  Height is carried in world metres, so the shear is `- ppm * z`
    // (the old `- z` on a pixel-space vertex).
    vec4 worldPos = world_matrix * vec4(world, 1.0);
    worldPos.y -= ppm * vertex_pos.z;
    vec4 clipPos = projection_matrix * camera_matrix * worldPos;
    // Canonical iso depth in window space `[0, 1]`, re-expressed for world
    // metres: `vertex_pos.x + vertex_pos.y = TILE_M · (tx − ty)`, and
    // `vertex_pos.z` is already metres — so `depth_scale.x` carries
    // `TILE_M · horizontal_depth_scale`.
    highp float isoDepth = (vertex_pos.x + vertex_pos.y) / depth_scale.x + 0.5 + vertex_pos.z / depth_scale.y;
    clipPos.z = isoDepth * 2.0 - 1.0;
    gl_Position = clipPos;
    vMapCoord = map_coord;
    vTileId = tile_id;
    vNormal = normalize(normal_matrix * normal);
    vec4 lightPos = light_matrix * vec4(world, 1.0);
    vLightPos = lightPos.xyz;
}
