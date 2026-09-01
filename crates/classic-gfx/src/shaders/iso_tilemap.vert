#version 300 es

precision mediump float;

in vec3 vertex_pos;
in vec2 map_coord;
in float tile_id;
in vec3 normal;

uniform mat4 iso_matrix;
uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
uniform mat3 normal_matrix;
// Tile -> light space: `T(origin) * S(tile_scale) * Rz(-45deg)`.  NOT
// `model_matrix * iso_matrix` — that carries the isometric `diag(1, 0.5, 1)`
// squash, which makes lighting non-metric.  See `classic-engine::light_matrix`.
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
    // Light space (+Z up, metric): the space `light_direction`, `vNormal`,
    // `Light::position` and the shadow map all live in.  Isotropic — one metre
    // is `ppm` pixels along every axis — so `length`/`normalize`/`dot` mean
    // what they say.
    vec4 lightPos = light_matrix * vec4(vertex_pos, 1.0);
    // Screen space: the isometric projection (`diag(1, 0.5, 1)`) plus the shear
    // that makes height visible.  It halves y and carries height in both y and
    // z, so it must never be used for lighting or shadows.
    vec4 worldPos = model_matrix * iso_matrix * vec4(vertex_pos, 1.0);
    worldPos.y -= vertex_pos.z;
    vec4 clipPos = projection_matrix * camera_matrix * worldPos;
    // Canonical iso depth in window space `[0, 1]`:
    //   iso_depth = (tx - ty) / depth_scale.x + 0.5 + z / depth_scale.y
    // with depth_scale = (horizontal_depth_scale, HEIGHT_DEPTH_SCALE_M) from
    // classic-core.  The mesh `vertex_pos.z` is carried in tileset pixels, so
    // it is converted to metres (`/ ppm`) before dividing by the metre-space
    // height divisor.  The `+ z` term reflects that taller terrain is farther
    // (the camera basis `back.z = +0.5`).  Window depth maps to clip z via
    // `d * 2.0 - 1.0`.  Computed in highp to match the sprite/depth-map path.
    highp float isoDepth = (vertex_pos.x - vertex_pos.y) / depth_scale.x + 0.5 + (vertex_pos.z / ppm) / depth_scale.y;
    clipPos.z = isoDepth * 2.0 - 1.0;
    gl_Position = clipPos;
    vMapCoord = map_coord;
    vTileId = tile_id;
    vNormal = normalize(normal_matrix * normal);
    vLightPos = lightPos.xyz;
}
