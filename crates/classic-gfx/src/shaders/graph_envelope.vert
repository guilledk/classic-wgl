#version 300 es

// Graph min/max envelope (`classic-gfx/src/graph.rs`): one instance per pair
// of consecutive pyramid buckets `a_b0`, `a_b1` (`x0, x1, y_min, y_max`), a
// 4-vertex `TRIANGLE_STRIP` trapezoid from the first bucket's centre to the
// second's.  The band is widened by the line half-width (so a flat signal
// still reads as a line) plus one pixel of AA fringe.  Physical pixels.

precision highp float;

in vec4 a_b0;
in vec4 a_b1;

uniform vec2 u_origin;
uniform vec2 u_scale;
uniform vec2 u_offset;
uniform vec2 u_target;
uniform float u_half_width;

out float vDist; // signed px from the band's centre line
out float vEdge; // px from the centre line to the coverage edge
out float vBand; // px half-height of the min/max band itself

vec2 toPx(vec2 p) {
    return u_offset + (p - u_origin) * u_scale;
}

void main(void) {
    int vid = gl_VertexID;
    vec4 b = vid < 2 ? a_b0 : a_b1;
    float x = 0.5 * (b.x + b.y);
    vec2 lo = toPx(vec2(x, b.z));
    vec2 hi = toPx(vec2(x, b.w));
    float c = 0.5 * (lo.y + hi.y);
    float h = 0.5 * abs(lo.y - hi.y);
    float e = h + u_half_width;
    float side = (vid == 1 || vid == 3) ? 1.0 : -1.0;
    float ext = e + 1.0;
    vec2 pos = vec2(lo.x, c + side * ext);
    vDist = side * ext;
    vEdge = e;
    vBand = h;
    gl_Position = vec4(pos.x / u_target.x * 2.0 - 1.0, 1.0 - pos.y / u_target.y * 2.0, 0.0, 1.0);
}
