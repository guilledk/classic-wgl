
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


export async function initShaders(gl, shaderManifest) {
    var shaders = {};

    for (const shaderInfo of shaderManifest) {
        const name = shaderInfo.name;
        const vertCode = await fetchFile(shaderInfo.vertex, {cache: "no-store"});
        const fragCode = await fetchFile(shaderInfo.fragment, {cache: "no-store"});

        shaders[name] = {
            program: initShaderProgram(gl, vertCode, fragCode),
            attr: {},
            unif: {}
        };

        for (const attr of shaderInfo.attr)
            shaders[name].attr[attr] = gl.getAttribLocation(
                shaders[name].program, attr);
        
        for (const unif of shaderInfo.unif)
            shaders[name].unif[unif] = gl.getUniformLocation(
                shaders[name].program, unif);
    }
    return shaders;
}


/*
 *  Buffers
 */

export function initBuffers(gl) {

    // Vertices
    const vertBuffer = gl.createBuffer();

    gl.bindBuffer(gl.ARRAY_BUFFER, vertBuffer);

    const quadVerts = [
        -1.0,  1.0,  0.0,
         1.0,  1.0,  0.0,
        -1.0, -1.0,  0.0,
         1.0, -1.0,  0.0
    ];

    gl.bufferData(
        gl.ARRAY_BUFFER, new Float32Array(quadVerts), gl.STATIC_DRAW);


    // Indices
    const indexBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);

    const indices = [
        0,  1,  2,
        1,  2,  3
    ];

    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,
        new Uint16Array(indices), gl.STATIC_DRAW);

    // UVs
    const texCoordBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, texCoordBuffer);

    const texCoords = [
        0.0,  1.0,
        1.0,  1.0,
        0.0,  0.0,
        1.0,  0.0
    ];

    gl.bufferData(gl.ARRAY_BUFFER,
        new Float32Array(texCoords), gl.STATIC_DRAW);

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

export async function initTextures(gl, textureManifest) {
    var textures = {};

    for (const tex of textureManifest)
        textures[tex.name] = await loadTexture(gl, tex.src);
     
    return textures;
}
