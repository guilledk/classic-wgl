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

out mediump vec2 vMapCoord;
out mediump float vTileId;
out mediump vec3 vNormal;

void main(void ) {
    vec4 worldPos = model_matrix * iso_matrix * vec4(vertex_pos, 1.0);
    worldPos.y -= vertex_pos.z;
    vec4 clipPos = projection_matrix * camera_matrix * worldPos;
    float isoDepth = clamp(
        (vertex_pos.x - vertex_pos.y) / 400.0 + 0.5 - vertex_pos.z / 14500.0,
        0.0,
        1.0
    );
    clipPos.z = isoDepth;
    gl_Position = clipPos;
    vMapCoord = map_coord;
    vTileId = tile_id;
    vNormal = normalize(normal_matrix * normal);
}
