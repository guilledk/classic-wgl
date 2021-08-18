import {
    addEvent,
    fetchFile, fetchObject, loadImage,
    initShaders,
    initBuffers,
    initTextures,
    loadTexture
} from '/utils.js';

import { mat4 } from '/lib/gl-matrix/index.js';


var running = false;
var projectionMatrix = mat4.create();

var manifest;

var shaders;
var buffers;
var textures;

var canvas;
var gl;

var modelMatrix = mat4.create();
mat4.translate(
    modelMatrix, modelMatrix, [200, 200, 0]);
mat4.scale(
    modelMatrix, modelMatrix, [100, 100, 1]);


var prevTime = 0;
function loop(now) {

    now /= 1000;
    const deltaTime = now - prevTime;
    
    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
   
    // Verts
    {
        const numComponents = 3;  // pull out 3 values per iteration
        const type = gl.FLOAT;    // the data in the buffer is 32bit floats
        const normalize = false;  // don't normalize
        const stride = 0;         // how many bytes to get from one set of values to the next
                                // 0 = use type and numComponents above
        const offset = 0;         // how many bytes inside the buffer to start from
        gl.bindBuffer(gl.ARRAY_BUFFER, buffers.quad.verts);
        gl.vertexAttribPointer(
            shaders.image.attr.vertexPos,
            numComponents, type, normalize, stride, offset);
        gl.enableVertexAttribArray(
            shaders.image.attr.vertexPos);
    }

    // UVs
    {
        const num = 2; // every coordinate composed of 2 values
        const type = gl.FLOAT; // the data in the buffer is 32 bit float
        const normalize = false; // don't normalize
        const stride = 0; // how many bytes to get from one set to the next
        const offset = 0; // how many bytes inside the buffer to start from
        gl.bindBuffer(gl.ARRAY_BUFFER, buffers.quad.uvs);
        gl.vertexAttribPointer(
            shaders.image.attr.texCoord,
            num, type, normalize, stride, offset);
        gl.enableVertexAttribArray(shaders.image.attr.texCoord);
    }
    
    // Indices
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, buffers.quad.indices);

    gl.useProgram(shaders.image.program);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, textures.coolSnake);

    gl.uniform1i(shaders.image.unif.texSampler, 0);
    gl.uniformMatrix4fv(
        shaders.image.unif.projectionMatrix,
        false,
        projectionMatrix);
    gl.uniformMatrix4fv(
        shaders.image.unif.modelMatrix,
        false,
        modelMatrix);

    {
        const vertexCount = 6;
        const type = gl.UNSIGNED_SHORT;
        const offset = 0;
        gl.drawElements(gl.TRIANGLES, vertexCount, type, offset);
    }

    prevTime = now;
    requestAnimationFrame(loop);
}


function resizeCanvas() {
    const vw = Math.max(
        document.documentElement.clientWidth || 0,
        window.innerWidth || 0
    )
    const vh = Math.max(
        document.documentElement.clientHeight || 0,
        window.innerHeight || 0
    )

    canvas = document.getElementById('glCanvas');
    canvas.width = vw;
    canvas.height = vh;

    const right = canvas.width;
    const bottom = canvas.height;
    const far = 10000;
    projectionMatrix = mat4.create();
    mat4.ortho(
        projectionMatrix,
        0, right, bottom, 0, 0, far);
}

async function initContext() {
    resizeCanvas()

    gl = canvas.getContext("webgl", {
        desynchronized: true,
        preserveDrawingBuffer: true
    });

    if (gl === null) {
        alert("Unable to initialize WebGL. Your browser or machine may not support it.");
        return;
    }

    var vertCode = await fetchFile('/shaders/direct.vert');
    var fragCode = await fetchFile('/shaders/image.frag');

    manifest = await fetchObject('/manifest.json', {cache: "no-store"});

    shaders = await initShaders(gl, manifest.shaders);

    buffers = initBuffers(gl);

    textures = await initTextures(gl, manifest.textures);

    requestAnimationFrame(loop);
}

addEvent(window, 'load', initContext);
addEvent(window, 'resize', resizeCanvas);

