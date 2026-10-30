#version 300 es

// Graph envelope coverage (`graph_envelope.vert`): AA top/bottom edges via
// `fwidth`.  Where the band is no taller than the line it reads as the line
// (full alpha); a band well past it fades to `u_envelope_alpha`.

precision highp float;

in float vDist;
in float vEdge;
in float vBand;

uniform float u_half_width;
uniform vec4 u_color;
uniform float u_envelope_alpha;

out vec4 fragColor;

void main(void) {
    float fw = max(fwidth(vDist), 1e-3);
    float a = clamp((vEdge - abs(vDist)) / fw + 0.5, 0.0, 1.0);
    if (a <= 0.0) {
        discard;
    }
    float fill = clamp((vBand - u_half_width) / (2.0 * u_half_width + 1.0), 0.0, 1.0);
    fragColor = vec4(u_color.rgb, u_color.a * a * mix(1.0, u_envelope_alpha, fill));
}
