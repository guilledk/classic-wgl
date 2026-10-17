#version 300 es

// A white-hot core inside a coloured glow halo, additive (premultiplied with
// zero alpha so it never darkens what is behind it).

precision mediump float;

in float vAcross;
in float vCore;
in float vIntensity;
in vec3 vColor;

out vec4 fragColor;

void main(void ) {
    float d = abs(vAcross);
    float core = 1.0 - smoothstep(vCore * 0.5, vCore, d);
    float glow = exp(-d * d * 6.0) * (1.0 - d);
    vec3 c = vColor * glow * 0.6 + mix(vColor, vec3(1.0), 0.7) * core * 1.4;
    fragColor = vec4(c * vIntensity, 0.0);
}
