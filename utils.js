export function addEvent(element, eventName, fn) {
    /*
     * Cross browser attach callback to window event
     */

    if (element.addEventListener)
        element.addEventListener(eventName, fn, false);
    else if (element.attachEvent)
        element.attachEvent('on' + eventName, fn);
}

export async function fetchFile(url, config = {}) {
    try {
        const response = await fetch(url, config);
        return await response.text(); 
    } catch (err) {
        console.error(err);
    }
}

export async function fetchObject(url, config = {}) {
    try {
        const response = await fetch(url, config);
        return await response.json(); 
    } catch (err) {
        console.error(err);
    }
}

export function loadImage(src) {
    return new Promise((resolve, reject) => {
        let img = new Image()
        img.onload = () => resolve(img)
        img.onerror = reject
        img.src = src
    });
}


/*
 *
 *  Shaders
 *
 */

export function loadShader(gl, type, source) {
    const shader = gl.createShader(type);

    gl.shaderSource(shader, source);
    gl.compileShader(shader);

    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        alert('An error occurred compiling the shaders: ' + gl.getShaderInfoLog(shader));
        gl.deleteShader(shader);
        return null;
    }

    return shader;
}

export function initShaderProgram(gl, vsSource, fsSource) {
    const vertexShader = loadShader(gl, gl.VERTEX_SHADER, vsSource);
    const fragmentShader = loadShader(gl, gl.FRAGMENT_SHADER, fsSource);

    const shaderProgram = gl.createProgram();
    gl.attachShader(shaderProgram, vertexShader);
    gl.attachShader(shaderProgram, fragmentShader);
    gl.linkProgram(shaderProgram);

    if (!gl.getProgramParameter(shaderProgram, gl.LINK_STATUS)) {
        alert('Unable to initialize the shader program: ' + gl.getProgramInfoLog(shaderProgram));
        return null;
    }

    return shaderProgram;
}


export let Shader = class {
    constructor(
        gl, name, vertexSrc, fragmentSrc, attributes, uniforms
    ) {
        this.gl = gl;
        this.name = name;
        this.vertexSrc = vertexSrc;
        this.fragmentSrc = fragmentSrc;
        this.attributes = attributes;
        this.uniforms = uniforms;
    }

    async fetchCode() {
        this.vertexCode = await fetchFile(this.vertexSrc, {cache: 'no-store'});
        this.fragmentCode = await fetchFile(this.fragmentSrc, {cache: 'no-store'});
    }

    compile() {
        this.program = initShaderProgram(
            this.gl, this.vertexCode, this.fragmentCode);

        this.attr = {};
        for (const attr of this.attributes)
            this.attr[attr] = this.gl.getAttribLocation(this.program, attr);
       
        this.unif = {};
        for (const unif of this.uniforms)
            this.unif[unif] = this.gl.getUniformLocation(this.program, unif);
    }

    bind() {
        this.gl.useProgram(this.program);
    }

    unbind() {
        this.gl.useProgram(null);
    }
}


export async function initShaders(gl, shaderManifest) {
    var shaders = {};

    for (const shaderInfo of shaderManifest) {
        const name = shaderInfo.name;

        shaders[name] = new Shader(
            gl, name,
            shaderInfo.vertex, shaderInfo.fragment,
            shaderInfo.attr, shaderInfo.unif
        );

        await shaders[name].fetchCode();
        shaders[name].compile();
    }
    return shaders;
}


/*
 *  Buffers
 */

export let Buffer = class {
    constructor(
        gl, type, data, data_type, usage
    ) {
        this.gl = gl;
        this.buffer = gl.createBuffer();
        this.type = type;
        this.data = data;
        this.array = new data_type(data);
        this.usage = usage;

        gl.bindBuffer(type, this.buffer);
        gl.bufferData(type, this.array, usage);
        gl.bindBuffer(type, null);
    }

    bind() {
        this.gl.bindBuffer(this.type, this.buffer);
    }

    unbind() {
        this.gl.bindBuffer(this.type, null);
    }
};

export function initBuffers(gl) {

    // Verts
    var vertBuffer = new Buffer(
        gl, gl.ARRAY_BUFFER,
        [
            -1.0,  1.0,  0.0,
            1.0,  1.0,  0.0,
            -1.0, -1.0,  0.0,
            1.0, -1.0,  0.0
        ],
        Float32Array,
        gl.STATIC_DRAW
    );

    // Indices
    var indexBuffer = new Buffer(
        gl, gl.ELEMENT_ARRAY_BUFFER,
        [
            0,  1,  2,
            1,  2,  3
        ],
        Uint16Array,
        gl.STATIC_DRAW
    );

    // UVs
    var texCoordBuffer = new Buffer(
        gl, gl.ARRAY_BUFFER,
        [
            0.0,  1.0,
            1.0,  1.0,
            0.0,  0.0,
            1.0,  0.0
        ],
        Float32Array,
        gl.STATIC_DRAW
    );

    return {
        quad: {
            verts: vertBuffer,
            indices: indexBuffer,
            uvs: texCoordBuffer
        }
    };
}


/*
 *  Textures
 */

export async function loadTexture(gl, url) {
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);

    const level = 0;
    const internalFormat = gl.RGBA;
    const srcFormat = gl.RGBA;
    const srcType = gl.UNSIGNED_BYTE;

    const image = await loadImage(url);

    gl.texImage2D(
        gl.TEXTURE_2D, level, internalFormat,
        srcFormat, srcType, image);
    
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);

    return texture;
}

export let Texture = class {
    constructor(
        gl, name, src) {
        this.gl = gl;
        this.name = name;
        this.src = src;
    }

    async load() {
        this.texture = await loadTexture(this.gl, this.src);
    }

    bind(tex_core) {
        this.gl.activeTexture(tex_core);
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.texture);
    }
};

export async function initTextures(gl, textureManifest) {
    var textures = {};

    for (const tex of textureManifest) {
        textures[tex.name] = new Texture(gl, tex.name, tex.src);
        await textures[tex.name].load();
    }
     
    return textures;
}


/*
 *  Isometric tools
 */
import { mat3, vec3, mat4, vec4 } from '/lib/gl-matrix/index.js';


var _cartesianToIso3 = mat3.fromValues(
    1, 1, 0,
   -2, 2, 0,
    0, 0, 1);
const isoScaling3 = mat3.fromValues(
    1/64, 0,    0,
    0,    1/64, 0,
    0,    0,    1);
mat3.multiply(_cartesianToIso3, _cartesianToIso3, isoScaling3);
export const cartesianToIso3 = _cartesianToIso3;


var _isoToCartesian3 = mat3.fromValues(
    0.5,-0.25, 0,
    0.5, 0.25, 0,
    0,   0,    1);
const cartesianScaling3 = mat3.fromValues(
    64,  0,  0,
    0,  64,  0,
    0,  0,  1);
mat3.multiply(_isoToCartesian3, _isoToCartesian3, cartesianScaling3);
export const isoToCartesian3 = _isoToCartesian3;

var testIdentity3 = mat3.create();
mat3.multiply(testIdentity3, isoToCartesian3, cartesianToIso3);

console.assert(
    mat3.equals(testIdentity3, mat3.create()),
    "Isometric coord transformation mismatch");


var _cartesianToIso4 = mat4.fromValues(
    1, 1, 0, 0,
   -2, 2, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1);
const isoScaling4 = mat4.fromValues(
    1/64, 0,    0, 0,
    0,    1/64, 0, 0,
    0,    0,    1, 0,
    0,    0,    0, 1);
mat4.multiply(_cartesianToIso4, _cartesianToIso4, isoScaling4);
export const cartesianToIso4 = _cartesianToIso4;


var _isoToCartesian4 = mat4.fromValues(
    0.5,-0.25, 0, 0,
    0.5, 0.25, 0, 0,
    0,   0,    1, 0,
    0,   0,    0, 1);
const cartesianScaling4 = mat4.fromValues(
    64,  0,  0, 0,
    0,  64,  0, 0,
    0,  0,   1, 0,
    0,  0,   0, 1);
mat4.multiply(_isoToCartesian4, _isoToCartesian4, cartesianScaling4);
export const isoToCartesian4 = _isoToCartesian4;

var testIdentity4 = mat4.create();
mat4.multiply(testIdentity4, isoToCartesian4, cartesianToIso4);

console.assert(
    mat4.equals(testIdentity4, mat4.create()),
    "Isometric coord transformation mismatch");

/*
 *  Simplex Noise
 */
import '/lib/simplex-noise.js';

const factor = 10.0;

export const noiseGen = new window.SimplexNoise();

export function getNoiseRange(x, y, from, to) {
    return (((noiseGen.noise2D(
        x / factor, y / factor) + 1) / 2) * (to - from)) + from;
}
