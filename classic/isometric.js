import { fetchObject, getNoiseRange } from "/classic/utils.js";
import { Drawable } from "/classic/transforms.js";
import { isoToCartesian4, isoToCartesian3 } from "/classic/utils.js";

import { mat4, vec2, vec3 } from "/lib/gl-matrix/index.js";


class Tilemap extends Drawable {
    constructor(
        entity,
        position, scale,
        sizeX, sizeY,
        tileSet, tilePixelSize, maxTile, data
    ) {
        super(entity, position, scale);
        this.sizeX = sizeX;
        this.sizeY = sizeY;

        this.mapSize = [sizeX, sizeY];

        this.tileSet = this.game.getTexture(tileSet);
        this.tilePixelSize = tilePixelSize;
        this.tileSetSize = [
            this.tileSet.image.width / tilePixelSize[0],
            this.tileSet.image.height / tilePixelSize[1]
        ];
        this.maxTile = maxTile;

        if (data == null) {
            this.data = Array(sizeX * sizeY);
            for (let y = 0; y < this.sizeY; y++)
                for (let x = 0; x < this.sizeX; x++)
                    this.data[x + (sizeX * y)] = 0;
        } else
            this.data = data;
        
        this.mapDataTexture = null;
    }

    async loadMap(url) {
        const data = await fetchObject(url);
        this.data = data;
        this.uploadToGPU();
    }

    dump() {
        const minObj = super.dump();
        minObj.sizeX = this.sizeX;
        minObj.sizeY = this.sizeY;
        minObj.tileSet = this.tileSet.name;
        minObj.tilePixelSize = this.tilePixelSize;
        minObj.maxTile = this.maxTile;
        minObj.data = this.data;
        return minObj;
    }

    generateNoiseMap() {
        for (let y = 0; y < this.sizeY; y++)
            for (let x = 0; x < this.sizeX; x++)
                this.data[x + (this.sizeX * y)] = Math.floor(
                    getNoiseRange(x, y, 0, this.maxTile));
    }

    getSelection() {
        var begin = vec2.fromValues(
            this.game.selectionBegin[0] / this.game.camera.scale[0],
            this.game.selectionBegin[1] / this.game.camera.scale[1]);
        var end = vec2.fromValues(
            this.game.selectionEnd[0] / this.game.camera.scale[0],
            this.game.selectionEnd[1] / this.game.camera.scale[1]);

        var from = vec2.create();
        var to = vec2.create();
        vec2.min(from, begin, end);
        vec2.max(to, begin, end);
        vec2.floor(from, from);
        vec2.ceil(to, to);
        return [from, to];
    }

    fillRegion(from, to, value) {
        const [fromX, fromY] = from;
        const [toX, toY] = to;

        for (let y = fromY; y < toY; y++)
            for (let x = fromX; x < toX; x++)
                this.data[x + (this.sizeX * y)] = value;
    }

    uploadToGPU() {
        if (this.mapDataTexture != null)
            this.gl.deleteTexture(this.mapDataTexture);

        var pixelData = new Uint8Array(this.sizeX * this.sizeY * 4);
        for (let i = 0; i < (this.sizeX * this.sizeY * 4); i += 4) {
            const val = this.data[Math.floor(i / 4)];
            pixelData[i]     = val; 
            pixelData[i + 1] = val;
            pixelData[i + 2] = val;
            pixelData[i + 3] = 255;
        }

        this.mapDataTexture = this.gl.createTexture();
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.mapDataTexture);
        this.gl.texImage2D(
            this.gl.TEXTURE_2D,
            0,
            this.gl.RGBA,
            this.sizeX, this.sizeY, 0,
            this.gl.RGBA,
            this.gl.UNSIGNED_BYTE,
            pixelData);
        
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);
    }

    draw() {
        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.isoTilemap.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.isoTilemap.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.isoTilemap.attr.mapCoord,
            2,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(this.game.shaders.isoTilemap.attr.mapCoord);
        
        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.isoTilemap.bind();

        this.gl.activeTexture(this.gl.TEXTURE0);
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.mapDataTexture);

        this.tileSet.bind(this.gl.TEXTURE1);

        this.gl.uniform1i(this.game.shaders.isoTilemap.unif.mapData, 0);
        this.gl.uniform1i(this.game.shaders.isoTilemap.unif.tileSet, 1);

        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.cameraMatrix,
            false,
            this.game.camera.matrix());
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.modelMatrix,
            false,
            this.modelMatrix());
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.isoMatrix,
            false,
            isoToCartesian4);

        this.gl.uniform2fv(
            this.game.shaders.isoTilemap.unif.tileSetSize, this.tileSetSize);
        this.gl.uniform2fv(
            this.game.shaders.isoTilemap.unif.tilePixelSize, this.tilePixelSize);

        this.gl.uniform2fv(
            this.game.shaders.isoTilemap.unif.mapSize, this.mapSize);

        this.gl.uniform2fv(
            this.game.shaders.isoTilemap.unif.selectedTile,
            [this.game.mouseIsoPos[0] / this.game.camera.scale[0],
            this.game.mouseIsoPos[1] / this.game.camera.scale[1]]);

        this.gl.uniform2fv(
            this.game.shaders.isoTilemap.unif.selectionBegin,
            [this.game.selectionBegin[0] / this.game.camera.scale[0],
            this.game.selectionBegin[1] / this.game.camera.scale[1]]);
    
        this.gl.uniform1i(this.game.shaders.isoTilemap.unif.selectionMode, this.game.selectionMode);
        this.gl.uniform4fv(this.game.shaders.isoTilemap.unif.selectionColor, this.game.selectionColor);

        this.gl.drawElements(
            this.gl.TRIANGLES,
            6,                  // vertex count
            this.gl.UNSIGNED_SHORT,  // type
            0);
    }
};


class IsometricDrawable extends Drawable {
    constructor(
        entity, position, scale
    ) {
        super(entity, position, scale);
        this.direction = 0;
    }

    modelMatrix() {
        let modelMatrix = mat4.create();
        let cartPos = vec3.clone(this.position);
        vec3.transformMat3(cartPos, cartPos, isoToCartesian3);
        mat4.translate(
            modelMatrix, modelMatrix, cartPos);
        mat4.scale(
            modelMatrix, modelMatrix, this.scale);
        return modelMatrix;
    }
};

class IsoSprite extends IsometricDrawable {
    constructor(
        entity, position, scale, texture
    ) {
        super(entity, position, scale);
        this.texture = this.game.getTexture(texture);
        this.frame = 0;
        this.tileSetSize = [1, 1];
        this.anchor = [0, 0];
    }

    dump() {
        const minObj = super.dump();
        minObj.texture = this.texture.name;
        return minObj;
    }

    modelMatrix() {
        var modelMatrix = super.modelMatrix();
        const texDimension = [
            this.texture.image.width, this.texture.image.height];
        const texAnchorDelta = [
            texDimension[0] * this.anchor[0] * this.scale[0],
            texDimension[1] * this.anchor[1] * this.scale[1]];
        
        var anchoredPos = vec3.fromValues(0, 0, 0);
        anchoredPos[0] -= texAnchorDelta[0];
        anchoredPos[1] -= texAnchorDelta[1];
        mat4.translate(
            modelMatrix, modelMatrix, anchoredPos);

        var sizeInPixels = vec3.clone(this.scale);
        sizeInPixels[0] *= texDimension[0];
        sizeInPixels[1] *= texDimension[1];
        mat4.scale(
            modelMatrix, modelMatrix, sizeInPixels);
        return modelMatrix;
    }

    draw() {
        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageSheet.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.imageSheet.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageSheet.attr.texCoord,
            2,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(this.game.shaders.imageSheet.attr.texCoord);
        
        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.imageSheet.bind();

        this.texture.bind(this.gl.TEXTURE0);

        this.gl.uniform1i(this.game.shaders.imageSheet.unif.texSampler, 0);
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.cameraMatrix,
            false,
            this.game.camera.matrix());
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.modelMatrix,
            false,
            this.modelMatrix());

        this.gl.uniform1f(
            this.game.shaders.imageSheet.unif.tileIdFlat,
            this.frame);
        this.gl.uniform2fv(
            this.game.shaders.imageSheet.unif.tileSetSize,
            this.tileSetSize);

        this.gl.drawElements(
            this.gl.TRIANGLES,
            6,                  // vertex count
            this.gl.UNSIGNED_SHORT,  // type
            0);                 // start offset
    }
};

export { Tilemap, IsoSprite };

if (window.gameClasses === undefined)
    window.gameClasses = {};

window.gameClasses.Tilemap = Tilemap;
window.gameClasses.IsoSprite = IsoSprite;
