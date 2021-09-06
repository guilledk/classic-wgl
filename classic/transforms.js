import { Component } from "/classic/ecs.js";
import game from "/classic/state.js";

import { mat4, vec2, vec3 } from "/lib/gl-matrix/index.js";
import { isoToCartesian4 } from "/classic/utils.js";


class Transform extends Component {
    constructor(
        entity, position, scale
    ) {
        super(entity);
        this.position = position;
        this.scale = scale;
    }

    dump() {
        const minObj = super.dump();
        minObj.position = this.position;
        minObj.scale = this.scale;
        return minObj;
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

class Drawable extends Transform {
    constructor(
        entity, position, scale
    ) {
        super(entity, position, scale);

        entity.registerCall("renderList", this.renderOrder.bind(this));
    }

    renderOrder() { this.game.renderList.push(this); }

    rawDraw() {
        throw "Abstract method must be overwritten";
    }

    order() {
        return this.position[2];
    }
};

class Rectangle extends Drawable {
    constructor(
        entity, position, scale, color, ignoreCam
    ) {
        super(entity, position, scale);
        this.color = color;
        this.ignoreCam = ignoreCam;
    }

    dump() {
        const minObj = super.dump();
        minObj.color = this.color;
        minObj.ignoreCam = this.ignoreCam;
        return minObj;
    }

    rawDraw() {
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.solid.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // perform normalization 
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.solid.attr.vertexPos);
        
        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.solid.bind();

        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        if (!this.ignoreCam)
            this.gl.uniformMatrix4fv(
                this.game.shaders.solid.unif.cameraMatrix,
                false,
                this.game.camera.matrix());
        else
            this.gl.uniformMatrix4fv(
                this.game.shaders.solid.unif.cameraMatrix,
                false,
                mat4.create());
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.modelMatrix,
            false,
            this.modelMatrix());
        this.gl.uniform4fv(this.game.shaders.solid.unif.color, this.color);
            
        this.gl.drawElements(
            this.gl.TRIANGLES,
            6,                  // vertex count
            this.gl.UNSIGNED_SHORT,  // type
            0);                 //start offset

    }
};

class Sprite extends Drawable {
    constructor(
        entity, position, scale, texture, ignoreCam,
        frame,
        tileSetSize,
        anchor
    ) {
        super(entity, position, scale);
        this.texture = this.game.getTexture(texture);
        this.ignoreCam = ignoreCam;
        this.frame = frame;
        this.tileSetSize = tileSetSize;
        this.anchor = anchor;
    }

    dump() {
        const minObj = super.dump();
        minObj.texture = this.texture.name;
        minObj.ignoreCam = this.ignoreCam;
        minObj.frame = this.frame;
        minObj.tileSetSize = this.tileSetSize;
        minObj.anchor = this.anchor;
        return minObj;
    }

    modelMatrix() {
        var modelMatrix = mat4.create();
        const texDimension = [
            this.texture.image.width, this.texture.image.height];
        const texAnchorDelta = [
            texDimension[0] * this.anchor[0] * this.scale[0],
            texDimension[1] * this.anchor[1] * this.scale[1]];
        
        var anchoredPos = vec3.clone(this.position);
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

    rawDraw() {
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
        if (!this.ignoreCam)
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.cameraMatrix,
                false,
                this.game.camera.matrix());
        else
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.cameraMatrix,
                false,
                mat4.create());

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


class Text extends Drawable {
    constructor(
        entity,
        position, scale, textureFont,
        maxCharSize, fontSize, glyphSize, glyphStr,
        color, bgcolor,
        ignoreCam 
    ) {
        super(entity, position, scale);
        this.textureFont = this.game.getTexture(textureFont);
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
        this.bgcolor = bgcolor;

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

        this.targetTexture = this.gl.createTexture();
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.targetTexture);
        this.gl.texImage2D(
            this.gl.TEXTURE_2D,
            0,        // mipmap levels
            this.gl.RGBA,  // internal format
            this.targetTextureWidth,
            this.targetTextureHeight,
            0,                 // border
            this.gl.RGBA,           // source format,
            this.gl.UNSIGNED_BYTE,  // buffer type
            null);             // data pointer
        
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);

        this.frameBuffer = this.gl.createFramebuffer();

        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.framebufferTexture2D(
            this.gl.FRAMEBUFFER,
            this.gl.COLOR_ATTACHMENT0,  // attatchment point
            this.gl.TEXTURE_2D,
            this.targetTexture,
            0);  // level
    }

    dump() {
        const minObj = super.dump();
        minObj.textureFont = this.textureFont.name;
        minObj.maxCharSize = this.maxCharSize;
        minObj.fontSize = this.fontSize;
        minObj.glyphSize = this.glyphSize;
        minObj.glyphStr = this.glyphStr;
        minObj.color = this.color;
        minObj.bgcolor = this.bgcolor;
        minObj.ignoreCam = this.ignoreCam;
        return minObj;
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
        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.enable(this.gl.BLEND);
        this.gl.blendFunc(this.gl.SRC_ALPHA, this.gl.ONE_MINUS_SRC_ALPHA);
        this.gl.viewport(0, 0, this.targetTextureWidth, this.targetTextureHeight);
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

            this.textureFont.bind(this.gl.TEXTURE0);

            this.gl.uniform1i(this.game.shaders.imageSheet.unif.texSampler, 0);
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.projectionMatrix,
                false,
                this.internalProjMatrix);
            if (!this.ignoreCam)
                this.gl.uniformMatrix4fv(
                    this.game.shaders.imageSheet.unif.cameraMatrix,
                    false,
                    this.game.camera.matrix());
            else
                this.gl.uniformMatrix4fv(
                    this.game.shaders.imageSheet.unif.cameraMatrix,
                    false,
                    mat4.create());

            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.modelMatrix,
                false,
                modelMatrix);

            this.gl.uniform1f(
                this.game.shaders.imageSheet.unif.tileIdFlat,
                glyphIndex);
            this.gl.uniform2fv(
                this.game.shaders.imageSheet.unif.tileSetSize,
                this.fontSize);

            this.gl.drawElements(
                this.gl.TRIANGLES,
                6,                  // vertex count
                this.gl.UNSIGNED_SHORT,  // type
                0);                 // start offset
            
            this.advanceCursor();
        }
    }

    setText(str) {
        this.cursorPos = [0, 0];
        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.clearColor(
            this.bgcolor[0],
            this.bgcolor[1],
            this.bgcolor[2],
            this.bgcolor[3]);
        this.gl.clear(this.gl.COLOR_BUFFER_BIT);
        this.appendText(str);
    }

    rawDraw() {

        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageColorize.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.imageColorize.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageColorize.attr.texCoord,
            2,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // normalize,
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(this.game.shaders.imageColorize.attr.texCoord);
        
        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.imageColorize.bind();

        this.gl.activeTexture(this.gl.TEXTURE0);
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.targetTexture);

        this.gl.uniform1i(this.game.shaders.imageColorize.unif.texSampler, 0);
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageColorize.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        if (!this.ignoreCam)
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageColorize.unif.cameraMatrix,
                false,
                this.game.camera.matrix());
        else
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageColorize.unif.cameraMatrix,
                false,
                mat4.create());

        this.gl.uniformMatrix4fv(
            this.game.shaders.imageColorize.unif.modelMatrix,
            false,
            this.modelMatrix());

        this.gl.uniform4fv(
            this.game.shaders.imageColorize.unif.color, this.color);

        this.gl.drawElements(
            this.gl.TRIANGLES,
            6,                  // vertex count
            this.gl.UNSIGNED_SHORT,  // type
            0);                 // start offset

        this.advanceCursor();
    }

};

export { Transform, Drawable, Rectangle, Sprite, Text };

window.Transform = Transform;
window.Drawable = Drawable;
window.Rectangle = Rectangle;
window.Sprite = Sprite;
window.Text = Text;
