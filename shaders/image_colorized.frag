precision highp float;
varying highp vec2 vTexCoord;

uniform sampler2D texSampler;
uniform vec4 color;

vec4 grayscale(in vec4 v) {
  float average = (v.r + v.g + v.b) / 3.0;
  return vec4(average, average, average, v.a);
}

vec4 colorize(in vec4 grayscale, in vec4 c) {
    return (grayscale * c);
}

void main(void) {
    vec4 texColor = texture2D(texSampler, vTexCoord);
    vec4 grayScale = grayscale(texColor);
    gl_FragColor = colorize(grayScale, color);
}
