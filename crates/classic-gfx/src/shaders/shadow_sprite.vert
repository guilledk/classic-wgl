#version 300 es

precision mediump float;

in vec4 vertex_pos;
in vec2 tex_coord;

uniform mat4 model_matrix;
uniform mat4 light_view_proj;
// The sprite's ground anchor as `(sheared y, light-space height)`.
uniform vec2 sprite_anchor;

out highp vec2 vTexCoord;

// Cast the billboard as *standing* geometry, not as a decal.
//
// `model_matrix` places a screen-aligned quad at constant model z, so read
// literally in light space it lies flat on the ground and casts a shadow the
// shape of a puddle.  A sprite represents geometry standing up out of the
// terrain, so screen up maps to world +Z about the ground anchor.  This must
// stay identical to the `vLightPos` reconstruction in `direct_tex.vert`, or a
// sprite will not lie in its own shadow.
void main(void ) {
    highp vec3 screenPos = (model_matrix * vertex_pos).xyz;
    highp float upFromAnchor = sprite_anchor.x - screenPos.y;
    highp vec3 lightPos = vec3(
        screenPos.x,
        sprite_anchor.x + sprite_anchor.y,
        sprite_anchor.y + upFromAnchor
    );
    gl_Position = light_view_proj * vec4(lightPos, 1.0);
    vTexCoord = tex_coord;
}
