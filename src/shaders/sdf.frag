precision mediump float;
varying mediump vec2 vTexCoord;

uniform sampler2D texSampler;
uniform vec4 color;
uniform vec4 outlineColor;
uniform float outlineWidth;
uniform float softEdge;

void main(void ) {
    float distance = texture2D(texSampler, vTexCoord).r;
    float edge = 0.5;

    float alpha = smoothstep(edge - softEdge, edge + softEdge, distance);

    float outlineAlpha = 0.0;
    if (outlineWidth > 0.001 || outlineWidth < -0.001) {
        float absWidth = abs(outlineWidth);
        float outlineEdge = outlineWidth > 0.0 ? edge - absWidth : edge + absWidth;
        outlineAlpha = smoothstep(outlineEdge - softEdge, outlineEdge + softEdge, distance);
    }

    vec4 result;
    result.rgb = mix(outlineColor.rgb, color.rgb, alpha);
    result.a = max(alpha, outlineAlpha);

    gl_FragColor = result;
}
