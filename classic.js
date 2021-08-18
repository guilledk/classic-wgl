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


let Transform = class {
    constructor(
        position, scale
    ) {
        this.position = position;
        this.scale = scale;
    }

    modelMatrix() {
        var modelMatrix = mat4.create();
        mat4.translate(
            modelMatrix, modelMatrix, this.position);
        mat4.scale(
            modelMatrix, modelMatrix, this.scale);
        return modelMatrix;
    }
};

class Rectangle extends Transform {
    constructor(
        position, scale, color
    ) {
        super(position, scale);
        this.color = color;
    }

    draw() {
        buffers.quad.verts.bind();
        gl.vertexAttribPointer(
            shaders.solid.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // perform normalization 
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(
            shaders.solid.attr.vertexPos);
        
        // Indices
        buffers.quad.indices.bind();

        shaders.solid.bind();

        gl.uniformMatrix4fv(
            shaders.solid.unif.projectionMatrix,
            false,
            projectionMatrix);
        gl.uniformMatrix4fv(
            shaders.solid.unif.modelMatrix,
            false,
            this.modelMatrix());
        gl.uniform4fv(shaders.solid.unif.color, this.color);
            
        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);                 //start offset

    }
};

class Sprite extends Transform {
    constructor(
        position, scale, texture
    ) {
        super(position, scale);
        this.texture = texture;
    }

    draw() {
        // Verts
        buffers.quad.verts.bind();
        gl.vertexAttribPointer(
            shaders.image.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(
            shaders.image.attr.vertexPos);

        // UVs
        buffers.quad.uvs.bind();
        gl.vertexAttribPointer(
            shaders.image.attr.texCoord,
            2,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(shaders.image.attr.texCoord);
        
        // Indices
        buffers.quad.indices.bind();

        shaders.image.bind();

        this.texture.bind(gl.TEXTURE0);

        gl.uniform1i(shaders.image.unif.texSampler, 0);
        gl.uniformMatrix4fv(
            shaders.image.unif.projectionMatrix,
            false,
            projectionMatrix);
        gl.uniformMatrix4fv(
            shaders.image.unif.modelMatrix,
            false,
            this.modelMatrix());

        {
            const vertexCount = 6;
            const type = gl.UNSIGNED_SHORT;
            const offset = 0;
            gl.drawElements(gl.TRIANGLES, vertexCount, type, offset);
        }
    }
};


var rect = new Rectangle([200, 200, 0], [100, 100, 1], [1.0, 0.0, 0.0, .99]);
var coolSnek;

var prevTime = 0;
function loop(now) {

    now /= 1000;
    const deltaTime = now - prevTime;
    
    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    rect.draw();
    coolSnek.draw();

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
    const far = 100;
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

    coolSnek = new Sprite([200, 200, 0], [32, 16, 1], textures.tile);

    requestAnimationFrame(loop);
}

addEvent(window, 'load', initContext);
addEvent(window, 'resize', resizeCanvas);

