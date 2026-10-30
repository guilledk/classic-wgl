#version 300 es

// Graph polyline coverage: signed distance across the line, antialiased over
// one pixel with `fwidth` (`graph_line.vert`).

precision highp float;

in float vDist;

uniform float u_half_width; // physical px, >= 0.5
uniform vec4 u_color;       // linear RGBA, alpha already scaled for thin lines

out vec4 fragColor;

void main(void) {
    float fw = max(fwidth(vDist), 1e-3);
    float a = clamp((u_half_width - abs(vDist)) / fw + 0.5, 0.0, 1.0);
    if (a <= 0.0) {
        discard;
    }
    fragColor = vec4(u_color.rgb, u_color.a * a);
}
