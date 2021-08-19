import {
    addEvent,
    fetchFile, fetchObject, loadImage,
    initShaders,
    initBuffers,
    initTextures,
    loadTexture,
    isoToCartesian,
    getNoiseRange
} from '/utils.js';

import { mat4, vec3 } from '/lib/gl-matrix/index.js';


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

        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);                 // start offset
    }
};


class Tilemap extends Transform {
    constructor(
        position, scale, sizeX, sizeY
    ) {
        super(position, scale);
        this.sizeX = sizeX;
        this.sizeY = sizeY;

        this.data = [...Array(sizeY)].map(e => Array(sizeX));
        for (let y = 0; y < this.sizeY; y++) {
            for (let x = 0; x < this.sizeX; x++) {
                this.data[x][y] = getNoiseRange(x, y, 0, 1);
            }
        }
        console.log(this.data);
    }

    draw() {
        for (let y = 0; y < this.sizeY; y++) {
            for (let x = 0; x < this.sizeX; x++) {
                // Verts
                buffers.quad.verts.bind();
                gl.vertexAttribPointer(
                    shaders.imageColorize.attr.vertexPos,
                    3,         // num of values to pull from array per iteration
                    gl.FLOAT,  // type
                    false,     // normalize,
                    0,         // stride
                    0);        // start offset
                gl.enableVertexAttribArray(
                    shaders.imageColorize.attr.vertexPos);

                // UVs
                buffers.quad.uvs.bind();
                gl.vertexAttribPointer(
                    shaders.imageColorize.attr.texCoord,
                    2,         // num of values to pull from array per iteration
                    gl.FLOAT,  // type
                    false,     // normalize,
                    0,         // stride
                    0);        // start offset
                gl.enableVertexAttribArray(shaders.imageColorize.attr.texCoord);
                
                // Indices
                buffers.quad.indices.bind();

                shaders.imageColorize.bind();

                textures.tile.bind(gl.TEXTURE0);

                var tilePos = vec3.fromValues(x, y, 0.0);
                vec3.transformMat3(tilePos, tilePos, isoToCartesian);

                var mMatrix = this.modelMatrix();
                var tileSize = vec3.fromValues(32, 16, 0);
                mat4.translate(mMatrix, mMatrix, tilePos);
                mat4.scale(mMatrix, mMatrix, tileSize);

                gl.uniform1i(shaders.imageColorize.unif.texSampler, 0);
                gl.uniformMatrix4fv(
                    shaders.imageColorize.unif.projectionMatrix,
                    false,
                    projectionMatrix);
                gl.uniformMatrix4fv(
                    shaders.imageColorize.unif.modelMatrix,
                    false,
                    mMatrix);
                gl.uniform4fv(shaders.imageColorize.unif.color, [this.data[x][y], 0.0, 0.0, 1.0]);

                gl.drawElements(
                    gl.TRIANGLES,
                    6,                  // vertex count
                    gl.UNSIGNED_SHORT,  // type
                    0);
            }
        }
    }
};


var rect = new Rectangle([200, 200, 0], [100, 100, 1], [1.0, 1.0, 0.0, .99]);
var coolSnek;

var tileMap = new Tilemap(
    [600, 300, 0], [1, 1, 1], 10, 10);

var prevTime = 0;
function loop(now) {

    now /= 1000;
    const deltaTime = now - prevTime;
    
    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    //rect.draw();
    //coolSnek.draw();

    tileMap.draw();

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

    // Take out loading text
    var loadingLabel = document.getElementById("loader");
    loadingLabel.remove();

    requestAnimationFrame(loop);
}

addEvent(window, 'load', initContext);
addEvent(window, 'resize', resizeCanvas);

