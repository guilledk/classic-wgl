#version 300 es

precision mediump float;

in mediump vec2 vTexCoord;

uniform sampler2D tex_sampler;
uniform vec4 color;
uniform vec4 outline_color;
uniform float outline_width;
uniform float soft_edge;
uniform float spread;
uniform vec2 atlas_size;
uniform float weight;
uniform float gamma;

out vec4 fragColor;

void main(void ) {
    float distance = texture(tex_sampler, vTexCoord).r;
    float edge = 0.5 - weight;

    float w;

    vec2 uvPx = fwidth(vTexCoord) * atlas_size;
    float pxRange = 2.0 * spread / max(length(uvPx), 1e-5);
    w = clamp(0.5 / pxRange, 0.0001, 0.5);

    float alpha = smoothstep(edge - w, edge + w, distance);

    float outlineAlpha = 0.0;
    if (outline_width > 0.001 || outline_width < -0.001) {
        float outlineSDF = outline_width / (2.0 * spread);
        float outlineEdge = edge - outlineSDF;
        outlineAlpha = smoothstep(outlineEdge - w, outlineEdge + w, distance);
    }

    float fillAlpha = alpha * color.a;
    float outAlpha = outlineAlpha * outline_color.a;

    vec4 result;

    result.rgb = mix(outline_color.rgb, color.rgb, alpha) * (outAlpha + fillAlpha);
    result.a = outAlpha + fillAlpha * (1.0 - outAlpha);

    float finalA = clamp(result.a, 0.0, 1.0);
    if (gamma > 0.001) {
        result.rgb *= pow(finalA, gamma - 1.0);
    }

    fragColor = vec4(result.rgb, finalA);
}
