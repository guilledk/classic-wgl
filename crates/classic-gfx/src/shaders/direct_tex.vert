#version 300 es

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
uniform float use_iso_depth;
uniform vec4 iso_depth_corners;
// The sprite's ground anchor, bridging the two spaces:
//   x = sheared screen y   (where the quad sits in the rasterised frame)
//   y = light-space height (terrain height under the sprite, px)
//   z = light-space y      (metric, un-squashed, un-sheared)
// Built by `classic-engine::compute_iso_sprite_model`.
uniform vec3 sprite_anchor;

out highp vec2 vTexCoord;
out highp vec3 vLightPos;

void main(void ) {
    gl_Position = projection_matrix * camera_matrix * model_matrix * vertex_pos;
    if (use_iso_depth > 0.5) {
        float bottomDepth = mix(iso_depth_corners.x, iso_depth_corners.y, vertex_pos.x);
        float topDepth = mix(iso_depth_corners.z, iso_depth_corners.w, vertex_pos.x);
        float cornerDepth = mix(topDepth, bottomDepth, vertex_pos.y);
        // `cornerDepth` is window-space `[0, 1]`; map to clip z.
        gl_Position.z = cornerDepth * 2.0 - 1.0;
    }
    vTexCoord = tex_coord;
    highp vec3 screenPos = (model_matrix * vertex_pos).xyz;

    // Unproject the billboard into light space (+Z up, metric).
    //
    // A billboard is a screen-aligned quad at constant model z, so in the
    // sheared screen space its tall axis runs along -y.  Read literally in
    // light space that is a *horizontal* direction, i.e. a decal lying flat on
    // the ground — which is why sprite shadows were meaningless.  A sprite is
    // meant to represent geometry standing up out of the terrain, so screen up
    // maps to world +Z about the ground anchor.
    //
    // The quad occupies a single light-space y (it does not recede into the
    // scene), and screen-up displacement becomes height.  Screen x needs no
    // conversion: the isometric squash only touches y.
    highp float upFromAnchor = sprite_anchor.x - screenPos.y;
    vLightPos = vec3(
        screenPos.x,
        sprite_anchor.z,
        sprite_anchor.y + upFromAnchor
    );
}
