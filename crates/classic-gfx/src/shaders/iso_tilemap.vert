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

uniform vec2 map_size;
uniform vec2 tile_pixel_size;
uniform vec2 depth_scale;
uniform float ppm;

out mediump vec2 vMapCoord;
out mediump float vTileId;
out highp vec3 vNormal;
out highp vec3 vLightPos;

void main(void ) {
    // Light space (+Z up): the space `light_direction` and `vNormal` live in,
    // and the space the shadow map is rendered and sampled in.
    vec4 lightPos = model_matrix * iso_matrix * vec4(vertex_pos, 1.0);
    // Screen space: the isometric shear that makes height visible.  It carries
    // height in both y and z, so it must never be used for lighting or shadows.
    vec4 worldPos = lightPos;
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
