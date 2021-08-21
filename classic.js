import {
    Buffer, Texture,
    getVideoCardInfo,
    fetchFile, fetchObject, loadImage,
    deleteLoaderLabel,
    initShaders,
    initBuffers,
    initTextures,
    loadTexture,
    isoToCartesian4,
    cartesianToIso4,
    getNoiseRange
} from '/utils.js';

import { mat4, vec4, vec3, vec2 } from '/lib/gl-matrix/index.js';

var running = false;
var projectionMatrix = mat4.create();

var manifest;

var shaders;
var buffers;
var textures;

var canvas = document.getElementById('glCanvas');
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

var camera = new Camera([3000, 0, 0], [0.3, 0.3, 1]);

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
        position, scale, texture, ignoreCam
    ) {
        super(position, scale);
        this.texture = texture;
        this.ignoreCam = ignoreCam;
        this.frame = 0;
        this.tileSetSize = [1, 1];
    }

    draw() {
        // Verts
        buffers.quad.verts.bind();
        gl.vertexAttribPointer(
            shaders.imageSheet.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(
            shaders.imageSheet.attr.vertexPos);

        // UVs
        buffers.quad.uvs.bind();
        gl.vertexAttribPointer(
            shaders.imageSheet.attr.texCoord,
            2,         // num of values to pull from array per iteration
            gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        gl.enableVertexAttribArray(shaders.imageSheet.attr.texCoord);
        
        // Indices
        buffers.quad.indices.bind();

        shaders.imageSheet.bind();

        this.texture.bind(gl.TEXTURE0);

        gl.uniform1i(shaders.imageSheet.unif.texSampler, 0);
        gl.uniformMatrix4fv(
            shaders.imageSheet.unif.projectionMatrix,
            false,
            projectionMatrix);
        if (!this.ignoreCam)
            gl.uniformMatrix4fv(
                shaders.imageSheet.unif.cameraMatrix,
                false,
                camera.matrix());
        else
            gl.uniformMatrix4fv(
                shaders.imageSheet.unif.cameraMatrix,
                false,
                mat4.create());

        gl.uniformMatrix4fv(
            shaders.imageSheet.unif.modelMatrix,
            false,
            this.modelMatrix());

        gl.uniform1f(
            shaders.imageSheet.unif.tileIdFlat,
            this.frame);
        gl.uniform2fv(
            shaders.imageSheet.unif.tileSetSize,
            this.tileSetSize);

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
        tileSet, tilePixelSize, maxTile
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
        this.maxTile = maxTile;
        this.tileRatio = [
            mapTileSize[0] / tilePixelSize[0],
            mapTileSize[1] / tilePixelSize[1],
        ]

        this.data = Array(sizeX * sizeY);
        for (let y = 0; y < this.sizeY; y++)
            for (let x = 0; x < this.sizeX; x++)
                this.data[x + (sizeX * y)] = Math.floor(
                    getNoiseRange(x, y, 0, maxTile));
        
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

        gl.uniform2fv(
            shaders.isoTilemap.unif.selectedTile,
            [mouseIsoPos[0] / this.tileRatio[0] / camera.scale[0],
            mouseIsoPos[1] / this.tileRatio[1] / camera.scale[1]]);

        gl.uniform2fv(
            shaders.isoTilemap.unif.selectionBegin,
            [selectionBegin[0] / this.tileRatio[0] / camera.scale[0],
            selectionBegin[1] / this.tileRatio[1] / camera.scale[1]]);
    
        gl.uniform1i(shaders.isoTilemap.unif.selectionMode, selectionMode);
        gl.uniform4fv(shaders.isoTilemap.unif.selectionColor, selectionColor);

        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);
    }
};


class Text extends Transform {
    constructor(
        position, scale, textureFont,
        maxCharSize, fontSize, glyphSize, glyphStr,
        color,
        ignoreCam 
    ) {
        super(position, scale);
        this.textureFont = textureFont;
        this.ignoreCam = ignoreCam;

        // max number of rows and columns of chars
        this.maxCharSize = maxCharSize;

        // number of glyphs in sheet
        this.fontSize = fontSize;
        // gylph size in pixels
        this.glyphSize = glyphSize;
        this.glyphStr = glyphStr;

        this.cursorPos = vec2.create();
        this.text = "";
        this.color = color;

        // init target texture
        this.targetTextureWidth = glyphSize[0] * maxCharSize[0];
        this.targetTextureHeight = glyphSize[1] * maxCharSize[1];

        this.internalProjMatrix = mat4.create();
        mat4.ortho(
            this.internalProjMatrix,
            0,  // left
            this.targetTextureWidth,   // right
            0,      // bottom
            this.targetTextureHeight,  // top
            0,      // near
            10000); // far

        this.targetTexture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, this.targetTexture);
        gl.texImage2D(
            gl.TEXTURE_2D,
            0,        // mipmap levels
            gl.RGBA,  // internal format
            this.targetTextureWidth,
            this.targetTextureHeight,
            0,                 // border
            gl.RGBA,           // source format,
            gl.UNSIGNED_BYTE,  // buffer type
            null);             // data pointer
        
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

        this.frameBuffer = gl.createFramebuffer();

        gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
        gl.framebufferTexture2D(
            gl.FRAMEBUFFER,
            gl.COLOR_ATTACHMENT0,  // attatchment point
            gl.TEXTURE_2D,
            this.targetTexture,
            0);  // level
    }

    modelMatrix() {
        var modelMatrix = mat4.create();
        var scale = vec3.clone(this.scale);
        scale[0] *= this.maxCharSize[0] * this.glyphSize[0];
        scale[1] *= this.maxCharSize[1] * this.glyphSize[1];
        mat4.translate(
            modelMatrix, modelMatrix, this.position);
        mat4.scale(
            modelMatrix, modelMatrix, scale);
        return modelMatrix;
    }

    getChrIndex(chr) {
        for (var i = 0; i < this.glyphStr.length; i++)
            if (this.glyphStr[i] === chr)
                return i;
        return -1;
    }

    advanceCursor() {
        this.cursorPos[0] += this.glyphSize[0];
        if (this.cursorPos[0] >= (this.maxCharSize[0] * this.glyphSize[0])) {
            this.cursorPos[0] = 0;
            this.cursorPos[1] += this.glyphSize[1];
        }

        if (this.cursorPos[1] >= (this.maxCharSize[1] * this.glyphSize[1]))
            this.cursorPos[1] = 0;
    }

    appendText(str) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.viewport(0, 0, this.targetTextureWidth, this.targetTextureHeight);
        for (const chr of str) {
            const glyphIndex = this.getChrIndex(chr);
            if (glyphIndex < 0)
                if (chr === " ") {
                    this.advanceCursor();
                    continue;
                } else
                    throw "Char \'" + chr + "\' not in font glyph string";

            var modelMatrix = mat4.create();
            mat4.translate(
                modelMatrix, modelMatrix,
                [this.cursorPos[0], this.cursorPos[1], 0]);
            mat4.scale(
                modelMatrix, modelMatrix,
                [this.glyphSize[0], this.glyphSize[1], 1]);

            // Verts
            buffers.quad.verts.bind();
            gl.vertexAttribPointer(
                shaders.imageSheet.attr.vertexPos,
                3,         // num of values to pull from array per iteration
                gl.FLOAT,  // type
                false,     // normalize,
                0,         // stride
                0);        // start offset
            gl.enableVertexAttribArray(
                shaders.imageSheet.attr.vertexPos);

            // UVs
            buffers.quad.uvs.bind();
            gl.vertexAttribPointer(
                shaders.imageSheet.attr.texCoord,
                2,         // num of values to pull from array per iteration
                gl.FLOAT,  // type
                false,     // normalize,
                0,         // stride
                0);        // start offset
            gl.enableVertexAttribArray(shaders.imageSheet.attr.texCoord);
            
            // Indices
            buffers.quad.indices.bind();

            shaders.imageSheet.bind();

            this.textureFont.bind(gl.TEXTURE0);

            gl.uniform1i(shaders.imageSheet.unif.texSampler, 0);
            gl.uniformMatrix4fv(
                shaders.imageSheet.unif.projectionMatrix,
                false,
                this.internalProjMatrix);
            if (!this.ignoreCam)
                gl.uniformMatrix4fv(
                    shaders.imageSheet.unif.cameraMatrix,
                    false,
                    camera.matrix());
            else
                gl.uniformMatrix4fv(
                    shaders.imageSheet.unif.cameraMatrix,
                    false,
                    mat4.create());

            gl.uniformMatrix4fv(
                shaders.imageSheet.unif.modelMatrix,
                false,
                modelMatrix);

            gl.uniform1f(
                shaders.imageSheet.unif.tileIdFlat,
                glyphIndex);
            gl.uniform2fv(
                shaders.imageSheet.unif.tileSetSize,
                this.fontSize);

            gl.drawElements(
                gl.TRIANGLES,
                6,                  // vertex count
                gl.UNSIGNED_SHORT,  // type
                0);                 // start offset
            
            this.advanceCursor();
        }
    }

    setText(str) {
        this.cursorPos = [0, 0];
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
        gl.clearColor(0.0, 0.0, 0.0, 0.0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        this.appendText(str);
    }

    draw() {

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

        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.targetTexture);

        gl.uniform1i(shaders.imageColorize.unif.texSampler, 0);
        gl.uniformMatrix4fv(
            shaders.imageColorize.unif.projectionMatrix,
            false,
            projectionMatrix);
        if (!this.ignoreCam)
            gl.uniformMatrix4fv(
                shaders.imageColorize.unif.cameraMatrix,
                false,
                camera.matrix());
        else
            gl.uniformMatrix4fv(
                shaders.imageColorize.unif.cameraMatrix,
                false,
                mat4.create());

        gl.uniformMatrix4fv(
            shaders.imageColorize.unif.modelMatrix,
            false,
            this.modelMatrix());

        gl.uniform4fv(
            shaders.imageColorize.unif.color, this.color);

        gl.drawElements(
            gl.TRIANGLES,
            6,                  // vertex count
            gl.UNSIGNED_SHORT,  // type
            0);                 // start offset

        this.advanceCursor();
    }

};


var tileMap;
var cursor;
var counter = 0;

var font;
var fpsCounter;

var mouseAxis = vec3.fromValues(0, 0, 0);
var mousePos = vec3.fromValues(-1, -1, 0);
var mouseIsoPos = vec3.fromValues(-1, -1, 0);

var selectionBegin = vec3.fromValues(-1, -1, -1);
var selectionMode = -1;
var selectionColor = [0, 1, 1, 1];

var scrollSpeed = 600;
var scrollDeadZone = .8;

var prevTime = 0;
var deltaTime = 0;
function loop(now) {

    now /= 1000;
    deltaTime = now - prevTime;
    counter += deltaTime;
    const fps = Math.floor(1 / deltaTime);
    fpsCounter.setText(fps.toString());
    
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

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0.0, 0.0, 0.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    tileMap.draw();
    cursor.draw();
    fpsCounter.draw();
    font.draw();

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
    canvas.addEventListener("click", mouseClickHandler, false);
    document.addEventListener(
        "pointerlockchange",
        function () {
            if(document.pointerLockElement === canvas) {
                window.addEventListener("keypress", keyPressHandler, false);
                canvas.addEventListener("mousemove", mouseMoveHandler, false);
                canvas.addEventListener("mousedown", mouseDownHandler, false);
                canvas.addEventListener("mouseup", mouseUpHandler, false);
            } else {
                canvas.removeEventListener("mousemove", mouseMoveHandler, false);
                canvas.removeEventListener("mousedown", mouseDownHandler, false);
                canvas.removeEventListener("mouseup", mouseUpHandler, false);
                window.removeEventListener("keypress", keyPressHandler, false);
            }
        }, false);

    gl = canvas.getContext("webgl", {
        desynchronized: true,
        preserveDrawingBuffer: true
    });

    if (gl === null) {
        setLoaderLabel(
            "Unable to initialize WebGL. Your browser or machine may not support it.");
        return;
    }

    resizeCanvas()

    console.log(getVideoCardInfo(gl));

    manifest = await fetchObject('/manifest.json', {cache: "no-store"});

    shaders = await initShaders(gl, manifest.shaders);

    buffers = initBuffers(gl);

    textures = await initTextures(gl, manifest.textures);

    tileMap = new Tilemap(
        [0, 0, -100], [1, 1, 1],
        1000, 1000,  // size in tiles
        [64, 64],    // map pixel size
        textures.tileSet,
        [16, 16],    // tileset tile pixel size
        10);          // max tile flat id
    tileMap.uploadToGPU();

    cursor = new Sprite([0, 0, 0], [32, 32, 1], textures.cursor, true);

    fpsCounter = new Text(
        [0, 0, 0], [1, 1, 1],
        textures.font,
        [10, 3],  // max char size
        [16, 16],   // font size
        [32, 32],   // glyph pixel size
        // glyph str
        "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
        [1, 1, 1, 1],  // color
        true);     // ignore cam

    font = new Text(
        [100, 0, 0], [.8, .8, 1],
        textures.font,
        [24, 1],  // max char size
        [16, 16],   // font size
        [32, 32],   // glyph pixel size
        // glyph str
        "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
        [1.00,0.53,0.30, 1],  // color
        true);     // ignore cam
    font.setText("CLASSIC ENGINE V0.1A0 ;)");

    deleteLoaderLabel()

    requestAnimationFrame(loop);
}

window.addEventListener("load", initContext, false);
window.addEventListener("resize", resizeCanvas, false);

function mouseUpHandler(event) {
    selectionMode = -1;
}

function mouseDownHandler(event) {
    if (mousePos[0] == -1)
        return;

    selectionMode = 1;

    vec3.copy(selectionBegin, mouseIsoPos);
}

function mouseClickHandler(event) {

    canvas.requestPointerLock = canvas.requestPointerLock ||
        canvas.mozRequestPointerLock ||
        canvas.webkitRequestPointerLock;
    canvas.requestPointerLock();
    if (mousePos[0] == -1)
        mousePos[0] = event.pageX;
    if (mousePos[1] == -1)
        mousePos[1] = event.pageY;

}
var isFirefox = navigator.userAgent.toLowerCase().indexOf('firefox') > -1;

function mouseMoveHandler(event) {

    mousePos[0] += event.movementX;
    if (isFirefox)
        mousePos[0] += 2;
    mousePos[1] += event.movementY;

    if (mousePos[0] < 0)
        mousePos[0] = 0;
    if (mousePos[0] > canvas.width)
        mousePos[0] = canvas.width;

    if (mousePos[1] < 0)
        mousePos[1] = 0;
    if (mousePos[1] > canvas.height)
        mousePos[1] = canvas.height;

    var mouseGlobal = vec3.clone(mousePos);
    vec3.add(mouseGlobal, mouseGlobal, camera.position);
    vec3.transformMat4(mouseIsoPos, mouseGlobal, cartesianToIso4);

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
    if (event.key === ',' && camera.scale[0] > 0.02)
        vec3.add(camera.scale, camera.scale, [-0.01, -0.01, 0]);
    if (event.key === '.')
        vec3.add(camera.scale, camera.scale, [0.01, 0.01, 0]);
}
