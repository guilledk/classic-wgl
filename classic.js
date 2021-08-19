import {
    Buffer, Texture,
    addEvent,
    fetchFile, fetchObject, loadImage,
    initShaders,
    initBuffers,
    initTextures,
    loadTexture,
    isoToCartesian4,
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
        position, scale, sizeX, sizeY, tileSize
    ) {
        super(position, scale);
        this.sizeX = sizeX;
        this.sizeY = sizeY;

        this.tileSize = tileSize;
        this.mapTileSize = [sizeX, sizeY];

        this.data = Array(sizeX * sizeY);
        for (let y = 0; y < this.sizeY; y++)
            for (let x = 0; x < this.sizeX; x++)
                this.data[x + (sizeX * y)] = Math.floor(
                    getNoiseRange(x, y, 0, 255));
        
        this.mapDataTexture = null;

        this.debugSprite = new Sprite(
            [10, 10, 0], this.mapTileSize, null);
        
        console.log(this.data);
    }

    uploadToGPU() {
        if (this.mapDataTexture != null)
            gl.deleteTexture(this.mapDataTexture);

        var pixelData = new Uint8Array(this.sizeX * this.sizeY * 4);
        for (let i = 0; i < (this.sizeX * this.sizeY * 4); i += 4) {
            const val = this.data[Math.floor(i / 4)];
            pixelData[i]     = val; 
            pixelData[i + 1] = val;
            pixelData[i + 2] = val;
            pixelData[i + 3] = 255;
        }

        console.log(pixelData);

        this.mapDataTexture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, this.mapDataTexture);
        gl.texImage2D(
            gl.TEXTURE_2D,
            0,
            gl.RGBA,
            this.sizeX, this.sizeY, 0,
            gl.RGBA,
            gl.UNSIGNED_BYTE,
            pixelData);
        
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

        this.debugSprite.texture = new Texture(
            gl, "mapData");
        this.debugSprite.texture.texture = this.mapDataTexture;
    }

    draw() {
        // Verts
        buffers.quad.verts.bind();
        gl.vertexAttribPointer(
            shaders.isoTilemap.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(
            shaders.isoTilemap.attr.vertexPos);

        // UVs
        buffers.quad.uvs.bind();
        gl.vertexAttribPointer(
            shaders.isoTilemap.attr.mapCoord,
            2,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(shaders.isoTilemap.attr.mapCoord);
        
        // Indices
        buffers.quad.indices.bind();

        shaders.isoTilemap.bind();

        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.mapDataTexture);

        gl.uniform1i(shaders.isoTilemap.unif.texSampler, 0);
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.projectionMatrix,
            false,
            projectionMatrix);
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.modelMatrix,
            false,
            this.modelMatrix());
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.isoMatrix,
            false,
            isoToCartesian4);

        gl.uniform2fv(
            shaders.isoTilemap.unif.mapTileSize, this.mapTileSize);


        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);

        this.debugSprite.draw();
    }
};


var rect = new Rectangle([200, 200, 0], [100, 100, 1], [1.0, 1.0, 0.0, .99]);
var coolSnek;

var tileMap = new Tilemap(
    [0, 0, 0], [1, 1, 1], 100, 100);

var prevTime = 0;
function loop(now) {

    now /= 1000;
    const deltaTime = now - prevTime;
    const fps = 1 / deltaTime;
    console.log(Math.floor(fps));

    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    //rect.draw();
    coolSnek.draw();

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

    coolSnek = new Sprite([200, 200, 0], [64, 64, 1], textures.coolSnake);
    tileMap.uploadToGPU();
    console.log(tileMap);

    // Take out loading text
    var loadingLabel = document.getElementById("loader");
    loadingLabel.remove();

    requestAnimationFrame(loop);
}

addEvent(window, 'load', initContext);
//addEvent(window, 'resize', resizeCanvas);

