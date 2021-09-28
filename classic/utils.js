export function getObjectValues(obj) {
    let l = [];
    for (let key in obj)
        l.push(obj[key]);

    return l;
}


export function getVideoCardInfo(gl) {
    const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
    return debugInfo ? {
        vendor: gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL),
        renderer:  gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL),
    } : {
        error: "no WEBGL_debug_renderer_info",
    };
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

var loadingLabel = document.getElementById("loader");
export function setLoaderLabel(msg) {
    loadingLabel.innerHTML = msg;
}

export function deleteLoaderLabel() {
    loadingLabel.remove();
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
        setLoaderLabel(
            'An error occurred compiling the shaders: ' +
            gl.getShaderInfoLog(shader));
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
        setLoaderLabel(
            'Unable to initialize the shader program: ' +
            gl.getProgramInfoLog(shaderProgram));
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
        this.vertexCode = await fetchFile(this.vertexSrc);
        this.fragmentCode = await fetchFile(this.fragmentSrc);
    }

    compile() {
        console.log("Compilling", this.name, "...");
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
            0.0,  1.0,  0.0,
            1.0,  1.0,  0.0,
            0.0,  0.0,  0.0,
            1.0,  0.0,  0.0
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
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

    return [texture, image];
}

export let Texture = class {
    constructor(
        gl, name, src) {
        this.gl = gl;
        this.name = name;
        this.src = src;
    }

    async load() {
        var [tex, img] = await loadTexture(this.gl, this.src);
        this.texture = tex;
        this.image = img;
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
 *  Animations
 */

export let Animation = class {
    constructor(name, src, rate, sequence) {
        this.name = name;
        this.src = src;
        this.rate = rate;
        this.sequence = sequence;
    }
};

export function initAnimations(animationManifest) {
    var animations = {};
    
    for (const anim of animationManifest) {
        animations[anim.name] = new Animation(
            anim.name, anim.src, anim.rate, anim.sequence);
    }

    return animations;
}


/*
 *  Isometric tools
 */
import { mat3, vec3, mat4, vec4 } from '/lib/gl-matrix/index.js';


var _cartesianToIso3 = mat3.create();

mat3.rotate(_cartesianToIso3, Math.PI / 4);
mat3.scale(_cartesianToIso3, _cartesianToIso3, [1, 2]);

export const cartesianToIso3 = _cartesianToIso3;


var _isoToCartesian3 = mat3.create();
mat3.invert(_isoToCartesian3, _cartesianToIso3);

export const isoToCartesian3 = _isoToCartesian3;


var _cartesianToIso4 = mat4.create();

mat4.rotateZ(_cartesianToIso4, _cartesianToIso4, Math.PI / 4);
mat4.scale(_cartesianToIso4, _cartesianToIso4, [1, 2, 1]);

export const cartesianToIso4 = _cartesianToIso4;


var _isoToCartesian4 = mat4.create();
mat4.invert(_isoToCartesian4, _cartesianToIso4);

export const isoToCartesian4 = _isoToCartesian4;




/*
 *  Simplex Noise
 */
import '/lib/simplex-noise.js';

const factor = 50.0;

export const noiseGen = new window.SimplexNoise();

export function getNoiseRange(x, y, from, to) {
    return (((noiseGen.noise2D(
        x / factor, y / factor) + 1) / 2) * (to - from)) + from;
}

/*
 * Math
 */

export function degreeToRadian(deg) {
    return deg * (Math.PI / 180);
}

export function radianToDegree(rad) {
    return rad * (180 / Math.PI);
}
