import {
    Buffer, Texture,
    addEvent,
    fetchFile, fetchObject, loadImage,
    deleteLoaderLabel,
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


let Camera = class {
    constructor(position, scale) {
        this.position = position;
        this.scale = scale;
    }

    matrix() {
        var camMatrix = mat4.create();
        const invPos = vec3.fromValues(
            -this.position[0],
            -this.position[1],
            -this.position[2]);
        mat4.translate(
            camMatrix, camMatrix, invPos);
        mat4.scale(
            camMatrix, camMatrix, this.scale);
        return camMatrix;
    }
};

var camera = new Camera([3000, 500, 0], [0.1, 0.1, 1]);

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
            shaders.solid.unif.cameraMatrix,
            false,
            camera.matrix());
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
        position, scale, texture, ignoreCam = false
    ) {
        super(position, scale);
        this.texture = texture;
        this.ignoreCam = ignoreCam;
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
        if (!this.ignoreCam)
            gl.uniformMatrix4fv(
                shaders.image.unif.cameraMatrix,
                false,
                camera.matrix());
        else
            gl.uniformMatrix4fv(
                shaders.image.unif.cameraMatrix,
                false,
                mat4.create());

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
        position, scale,
        sizeX, sizeY,
        mapTileSize,
        tileSet, tilePixelSize
    ) {
        super(position, scale);
        this.sizeX = sizeX;
        this.sizeY = sizeY;

        this.mapTileSize = mapTileSize;
        this.mapSize = [sizeX, sizeY];

        this.tileSet = tileSet;
        this.tilePixelSize = tilePixelSize;
        this.tileSetSize = [
            this.tileSet.image.width / tilePixelSize[0],
            this.tileSet.image.height / tilePixelSize[1]
        ];

        const maxTile = this.tileSetSize[0] * this.tileSetSize[1];

        this.data = Array(sizeX * sizeY);
        for (let y = 0; y < this.sizeY; y++)
            for (let x = 0; x < this.sizeX; x++)
                this.data[x + (sizeX * y)] = Math.floor(
                    getNoiseRange(x, y, 0, maxTile));
        
        console.log(this);

        this.mapDataTexture = null;
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

        this.tileSet.bind(gl.TEXTURE1);

        gl.uniform1i(shaders.isoTilemap.unif.mapData, 0);
        gl.uniform1i(shaders.isoTilemap.unif.tileSet, 1);

        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.projectionMatrix,
            false,
            projectionMatrix);
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.cameraMatrix,
            false,
            camera.matrix());
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.modelMatrix,
            false,
            this.modelMatrix());
        gl.uniformMatrix4fv(
            shaders.isoTilemap.unif.isoMatrix,
            false,
            isoToCartesian4);

        gl.uniform2fv(
            shaders.isoTilemap.unif.tileSetSize, this.tileSetSize);
        gl.uniform2fv(
            shaders.isoTilemap.unif.tilePixelSize, this.tilePixelSize);

        gl.uniform2fv(
            shaders.isoTilemap.unif.mapSize, this.mapSize);
        gl.uniform2fv(
            shaders.isoTilemap.unif.mapTileSize, this.mapTileSize);

        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);
    }
};


var tileMap;
var cursor;

var mouseAxis = vec3.fromValues(0, 0, 0);
var mousePos = vec3.fromValues(-1, -1, 0);
var scrollSpeed = 600;
var scrollDeadZone = .8;

var prevTime = 0;
var deltaTime = 0;
function loop(now) {

    now /= 1000;
    deltaTime = now - prevTime;
    const fps = 1 / deltaTime;
    // console.log(fps);
    

    if (vec3.length(mouseAxis) > scrollDeadZone) {

        const absAxisX = Math.abs(mouseAxis[0]);
        const absAxisY = Math.abs(mouseAxis[1]);

        var scrollAxis = vec3.fromValues(
            Math.min((absAxisX / 100) / -Math.log10(absAxisX), 1) * Math.sign(mouseAxis[0]),
            Math.min((absAxisY / 100) / -Math.log10(absAxisY), 1) * Math.sign(mouseAxis[1]),
            0);

        if (scrollAxis[0] > 1)
            scrollAxis[0] = -1;
        if (scrollAxis[1] > 1)
            scrollAxis[1] = -1;

        if (scrollAxis[0] < -1)
            scrollAxis[0] = 1;

        if (scrollAxis[1] < -1)
            scrollAxis[1] = 1;

        var scrollDelta = vec3.clone(scrollAxis);
        vec3.scale(scrollDelta, scrollDelta, scrollSpeed * deltaTime);

        vec3.add(
            camera.position,
            camera.position,
            scrollDelta);
    }


    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    tileMap.draw();
    cursor.draw();

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
    window.addEventListener("keypress", keyPressHandler, false);
    canvas.addEventListener("click", mouseClickHandler, false);
    document.addEventListener('pointerlockchange', lockChangeAlert, false);
    function lockChangeAlert() {
        if(document.pointerLockElement === canvas) {
            console.log('The pointer lock status is now locked');
            canvas.addEventListener("mousemove", mouseMoveHandler, false);
        } else {
            console.log('The pointer lock status is now unlocked');  
            canvas.removeEventListener("mousemove", mouseMoveHandler, false);
        }
    }
    gl = canvas.getContext("webgl", {
        desynchronized: true,
        preserveDrawingBuffer: true
    });

    if (gl === null) {
        setLoaderLabel(
            "Unable to initialize WebGL. Your browser or machine may not support it.");
        return;
    }

    var vertCode = await fetchFile('/shaders/direct.vert');
    var fragCode = await fetchFile('/shaders/image.frag');

    manifest = await fetchObject('/manifest.json', {cache: "no-store"});

    shaders = await initShaders(gl, manifest.shaders);

    buffers = initBuffers(gl);

    textures = await initTextures(gl, manifest.textures);

    tileMap = new Tilemap(
        [0, 0, 0], [4, 4, 1],
        1000, 1000, [16, 16],
        textures.tileSet, [16, 16]);
    tileMap.uploadToGPU();

    cursor = new Sprite([0, 0, 0], [32, 32, 1], textures.cursor, true);

    deleteLoaderLabel()

    requestAnimationFrame(loop);
}

window.addEventListener("load", initContext, false);
//addEvent(window, 'resize', resizeCanvas);

function mouseClickHandler(event) {

    canvas.requestPointerLock = canvas.requestPointerLock ||
        canvas.mozRequestPointerLock ||
        canvas.webkitRequestPointerLock;
    canvas.requestPointerLock();

}


function mouseMoveHandler(event) {

    if (mousePos[0] == -1)
        mousePos[0] = event.pageX;
    if (mousePos[1] == -1)
        mousePos[1] = event.pageY;

    mousePos[0] += event.movementX;
    mousePos[1] += event.movementY;

    if (mousePos[0] < 0)
        mousePos[0] = 0;
    if (mousePos[0] > canvas.width)
        mousePos[0] = canvas.width;

    if (mousePos[1] < 0)
        mousePos[1] = 0;
    if (mousePos[1] > canvas.height)
        mousePos[1] = canvas.height;

    vec3.copy(cursor.position, mousePos);

    mouseAxis[0] = ((mousePos[0] / canvas.width) - .5) * 2;
    mouseAxis[1] = ((mousePos[1] / canvas.height) - .5) * 2;

    if (mouseAxis[0] > 1)
        mouseAxis[0] = 1;
    if (mouseAxis[1] > 1)
        mouseAxis[1] = 1;

    if (mouseAxis[0] < -1)
        mouseAxis[0] = -1;
    if (mouseAxis[1] < -1)
        mouseAxis[1] = -1;

}


function keyPressHandler(event) {
    console.log(event);

    if (event.key === ',')
        vec3.add(camera.scale, camera.scale, [-0.1, -0.1, 0]);
    if (event.key === '.')
        vec3.add(camera.scale, camera.scale, [0.1, 0.1, 0]);
}

