#version 300 es

// Graph polyline (`classic-gfx/src/graph.rs`): one instance per segment
// `a_p1 -> a_p2` of the level-0 sample ring, with its neighbours `a_p0` /
// `a_p3` for the joins.  Nine vertices per instance (`TRIANGLES`): the
// segment quad (0-5) and a bevel triangle at `a_p2` (6-8), collapsed to a
// point when the join there is mitred.  Everything is in physical pixels.

precision highp float;

in vec2 a_p0;
in vec2 a_p1;
in vec2 a_p2;
in vec2 a_p3;

uniform vec2 u_origin;   // data point drawn at u_offset (the view's x0, y0)
uniform vec2 u_scale;    // physical px per data unit (y negative: up)
uniform vec2 u_offset;   // physical px of u_origin (plot bottom-left)
uniform vec2 u_target;   // physical size of the render target
uniform float u_half_width; // physical px, >= 0.5
uniform int u_first;     // 1: instance 0 has no previous sample
uniform int u_last;      // instance with no next sample (-1: none)

out float vDist;         // signed px distance from the segment centre line

// cos(turn) above which a join is mitred: a miter limit of 4 half-widths
// (cos(turn / 2) >= 1 / 4).  Decided from the two directions alone, so
// both segments of a join agree.
const float MITER_COS = 2.0 / 16.0 - 1.0;

vec2 toPx(vec2 p) {
    return u_offset + (p - u_origin) * u_scale;
}

vec2 dirOr(vec2 v, vec2 fallback) {
    float l = length(v);
    return l > 1e-4 ? v / l : fallback;
}

vec2 perp(vec2 d) {
    return vec2(-d.y, d.x);
}

// Offset (in half-widths) of the corner on the +n side of a join between
// directions `din` and `dout`, for a segment with normal `n`: the miter, or
// `n` itself when the join is too sharp (the bevel triangle fills the gap).
vec2 joinOffset(vec2 din, vec2 dout, vec2 n) {
    if (dot(din, dout) < MITER_COS) {
        return n;
    }
    vec2 m = perp(normalize(din + dout));
    return m / max(dot(m, n), 0.25);
}

void main(void) {
    vec2 p1 = toPx(a_p1);
    vec2 p2 = toPx(a_p2);
    vec2 d = dirOr(p2 - p1, vec2(1.0, 0.0));
    vec2 p0 = (u_first == 1 && gl_InstanceID == 0) ? p1 - d : toPx(a_p0);
    vec2 p3 = (gl_InstanceID == u_last) ? p2 + d : toPx(a_p3);
    vec2 da = dirOr(p1 - p0, d);
    vec2 dc = dirOr(p3 - p2, d);
    vec2 n = perp(d);
    // The quad reaches one physical pixel past the edge for the AA fringe.
    float w = u_half_width + 1.0;

    int vid = gl_VertexID;
    vec2 pos;
    if (vid < 6) {
        bool atEnd = vid == 2 || vid == 4 || vid == 5;
        float side = (vid == 1 || vid == 3 || vid == 4) ? 1.0 : -1.0;
        vec2 off = atEnd ? joinOffset(d, dc, n) : joinOffset(da, d, n);
        pos = (atEnd ? p2 : p1) + side * off * w;
        vDist = side * w;
    } else {
        // Bevel at p2 on the outer side of the turn (only when not mitred).
        float turn = d.x * dc.y - d.y * dc.x;
        float side = turn > 0.0 ? -1.0 : 1.0;
        bool mitred = dot(d, dc) >= MITER_COS;
        if (mitred || vid == 6) {
            pos = p2;
            vDist = 0.0;
        } else {
            pos = p2 + side * (vid == 7 ? n : perp(dc)) * w;
            vDist = w;
        }
    }
    gl_Position = vec4(pos.x / u_target.x * 2.0 - 1.0, 1.0 - pos.y / u_target.y * 2.0, 0.0, 1.0);
}
