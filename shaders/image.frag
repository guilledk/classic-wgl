varying mediump vec2 vTexCoord;

uniform sampler2D texSampler;

void main(void) {
    gl_FragColor = texture2D(texSampler, vTexCoord);
}
