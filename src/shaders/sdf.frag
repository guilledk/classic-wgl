precision mediump float;

#ifdef GL_OES_standard_derivatives
#extension GL_OES_standard_derivatives : enable
#endif

varying mediump vec2 vTexCoord;

uniform sampler2D texSampler;
uniform vec4 color;
uniform vec4 outlineColor;
uniform float outlineWidth;
uniform float softEdge;
uniform float spread;
uniform vec2 atlasSize;
uniform float weight;
uniform float gamma;

void main(void ) {
    float distance = texture2D(texSampler, vTexCoord).r;
    float edge = 0.5 - weight;

    float w;

    #ifdef GL_OES_standard_derivatives
    vec2 uvPx = fwidth(vTexCoord) * atlasSize;
    float pxRange = 2.0 * spread / max(length(uvPx), 1e-5);
    w = clamp(0.5 / pxRange, 0.0001, 0.5);
    #else
    w = softEdge;
    #endif

    float alpha = smoothstep(edge - w, edge + w, distance);

    float outlineAlpha = 0.0;
    if (outlineWidth > 0.001 || outlineWidth < -0.001) {
        float outlineSDF = outlineWidth / (2.0 * spread);
        float outlineEdge = edge - outlineSDF;
        outlineAlpha = smoothstep(outlineEdge - w, outlineEdge + w, distance);
    }

    float fillAlpha = alpha * color.a;
    float outAlpha = outlineAlpha * outlineColor.a;

    vec4 result;

    result.rgb = mix(outlineColor.rgb, color.rgb, alpha) * (outAlpha + fillAlpha);
    result.a = outAlpha + fillAlpha * (1.0 - outAlpha);

    // per-frame alpha is controlled from JS side via color.a / outlineColor.a

    float finalA = clamp(result.a, 0.0, 1.0);
    if (gamma > 0.001) {
        result.rgb *= pow(finalA, gamma - 1.0);
    }

    gl_FragColor = vec4(result.rgb, finalA);
}
