
export function addEvent(element, eventName, fn) {
    /*
     * Cross browser attach callback to window event
     */

    if (element.addEventListener)
        element.addEventListener(eventName, fn, false);
    else if (element.attachEvent)
        element.attachEvent('on' + eventName, fn);
}

export async function fetchFile(url) {
    try {
        const response = await fetch(url);
        return await response.text(); 
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


/*
 *  Buffers
 */

export function initBuffers(gl) {

    const vertexBuffer = gl.createBuffer();

    gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffer);

    const points = [
        0.0, 0.0,  1.0,
        1.0, 0.0,  1.0,
        0.0, 1.0,  1.0,
        1.1, 1.0,  1.0,
    ];

    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(points), gl.STATIC_DRAW);

    const texCoordBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, texCoordBuffer);

    const texCoord = [
        0.0,  0.0,
        1.0,  0.0,
        0.0,  1.0,
        1.0,  1.0,
    ]

    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(texCoord),
        gl.STATIC_DRAW);

    const indexBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);

    const indices = [
        0,  1,  2,      1,  3,  2,    
    ];

    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,
        new Uint16Array(indices), gl.STATIC_DRAW);

    return {
        vertex: vertexBuffer,
        tex: texCoordBuffer,
        indices: indexBuffer
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


