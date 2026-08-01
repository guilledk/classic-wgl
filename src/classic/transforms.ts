import { Component } from '/classic/ecs.js';
import { registerComponent } from '/classic/registry.js';
import type { IEntity, IDrawable, ITexture, ComponentData } from './types.js';

import { mat4, vec2, vec3 } from 'gl-matrix';

type Vec3Like = vec3 | [number, number, number] | number[];
type Vec2Like = vec2 | [number, number] | number[];
type Color = [number, number, number, number] | number[];

export class Transform extends Component {
    position: vec3;
    scale: vec3;

    constructor(entity: IEntity, position: Vec3Like, scale: Vec3Like) {
        super(entity);
        this.position = vec3.clone(position as vec3);
        this.scale = vec3.clone(scale as vec3);
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.position = this.position;
        minObj.scale = this.scale;
        return minObj;
    }

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        mat4.translate(modelMatrix, modelMatrix, this.position);
        mat4.scale(modelMatrix, modelMatrix, this.scale);
        return modelMatrix;
    }
}

export class Drawable extends Transform implements IDrawable {
    constructor(entity: IEntity, position: Vec3Like, scale: Vec3Like) {
        super(entity, position, scale);
        entity.registerCall('renderList', this.renderOrder.bind(this));
    }

    renderOrder(): void {
        this.game.renderList.push(this);
    }

    rawDraw(): void {
        throw new Error('Abstract method must be overwritten');
    }

    order(): number {
        return this.position[2];
    }
}

export class Rectangle extends Drawable {
    color: Color;
    ignoreCam: boolean;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        color: Color,
        ignoreCam: boolean,
    ) {
        super(entity, position, scale);
        this.color = color;
        this.ignoreCam = ignoreCam;
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.color = this.color;
        minObj.ignoreCam = this.ignoreCam;
        return minObj;
    }

    rawDraw(): void {
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.solid.attr.vertexPos,
            3, // num of values to pull from array per iteration
            this.gl.FLOAT, // type
            false, // perform normalization
            0, // stride
            0, // start offset
        );
        this.gl.enableVertexAttribArray(this.game.shaders.solid.attr.vertexPos);

        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.solid.bind();

        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.projectionMatrix,
            false,
            this.game.projectionMatrix,
        );
        if (!this.ignoreCam) {
            this.gl.uniformMatrix4fv(
                this.game.shaders.solid.unif.cameraMatrix,
                false,
                this.game.camera.matrix(),
            );
        } else {
            this.gl.uniformMatrix4fv(
                this.game.shaders.solid.unif.cameraMatrix,
                false,
                mat4.create(),
            );
        }
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.modelMatrix,
            false,
            this.modelMatrix(),
        );
        this.gl.uniform4fv(this.game.shaders.solid.unif.color, this.color as number[]);

        this.gl.drawElements(
            this.gl.TRIANGLES,
            6, // vertex count
            this.gl.UNSIGNED_SHORT, // type
            0, // start offset
        );
    }
}

export class Sprite extends Drawable {
    texture: ITexture;
    ignoreCam: boolean;
    frame: number;
    tileSetSize: Vec2Like;
    anchor: Vec2Like;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        texture: string,
        ignoreCam: boolean,
        frame: number,
        tileSetSize: Vec2Like,
        anchor: Vec2Like,
    ) {
        super(entity, position, scale);
        this.texture = this.game.getTexture(texture);
        this.ignoreCam = ignoreCam;
        this.frame = frame;
        this.tileSetSize = tileSetSize;
        this.anchor = anchor;
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.texture = this.texture.name;
        minObj.ignoreCam = this.ignoreCam;
        minObj.frame = this.frame;
        minObj.tileSetSize = this.tileSetSize;
        minObj.anchor = this.anchor;
        return minObj;
    }

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        const texDimension = [this.texture.image.width, this.texture.image.height];
        const texAnchorDelta = [
            texDimension[0] * (this.anchor[0] as number) * this.scale[0],
            texDimension[1] * (this.anchor[1] as number) * this.scale[1],
        ];

        const anchoredPos = vec3.clone(this.position);
        anchoredPos[0] -= texAnchorDelta[0];
        anchoredPos[1] -= texAnchorDelta[1];
        mat4.translate(modelMatrix, modelMatrix, anchoredPos);

        const sizeInPixels = vec3.clone(this.scale);
        sizeInPixels[0] *= texDimension[0];
        sizeInPixels[1] *= texDimension[1];
        mat4.scale(modelMatrix, modelMatrix, sizeInPixels);
        return modelMatrix;
    }

    rawDraw(): void {
        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageSheet.attr.vertexPos,
            3, // num of values to pull from array per iteration
            this.gl.FLOAT, // type
            false, // normalize,
            0, // stride
            0, // start offset
        );
        this.gl.enableVertexAttribArray(this.game.shaders.imageSheet.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageSheet.attr.texCoord,
            2, // num of values to pull from array per iteration
            this.gl.FLOAT, // type
            false, // normalize,
            0, // stride
            0, // start offset
        );
        this.gl.enableVertexAttribArray(this.game.shaders.imageSheet.attr.texCoord);

        // Indices
        this.game.buffers.quad.indices.bind();

        this.game.shaders.imageSheet.bind();

        this.texture.bind(this.gl.TEXTURE0);

        this.gl.uniform1i(this.game.shaders.imageSheet.unif.texSampler, 0);
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.projectionMatrix,
            false,
            this.game.projectionMatrix,
        );
        if (!this.ignoreCam) {
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.cameraMatrix,
                false,
                this.game.camera.matrix(),
            );
        } else {
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.cameraMatrix,
                false,
                mat4.create(),
            );
        }

        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.modelMatrix,
            false,
            this.modelMatrix(),
        );

        this.gl.uniform1f(this.game.shaders.imageSheet.unif.tileIdFlat, this.frame);
        this.gl.uniform2fv(
            this.game.shaders.imageSheet.unif.tileSetSize,
            this.tileSetSize as number[],
        );
        this.gl.uniform1f(this.game.shaders.imageSheet.unif.useIsoDepth, 0.0);
        this.gl.uniform1f(this.game.shaders.imageSheet.unif.isoDepth, 0.0);
        this.gl.uniform1f(this.game.shaders.imageSheet.unif.ghostAlpha, 0.0);

        this.gl.drawElements(
            this.gl.TRIANGLES,
            6, // vertex count
            this.gl.UNSIGNED_SHORT, // type
            0, // start offset
        );
    }
}

export class Text extends Drawable {
    textureFont: ITexture;
    ignoreCam: boolean;
    maxCharSize: [number, number];
    fontSize: [number, number];
    glyphSize: [number, number];
    glyphStr: string;
    cursorPos: vec2;
    text: string;
    color: Color;
    bgcolor: Color;
    targetTextureWidth: number;
    targetTextureHeight: number;
    internalProjMatrix: mat4;
    targetTexture: WebGLTexture | null;
    frameBuffer: WebGLFramebuffer | null;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        textureFont: string,
        maxCharSize: [number, number],
        fontSize: [number, number],
        glyphSize: [number, number],
        glyphStr: string,
        color: Color,
        bgcolor: Color,
        ignoreCam: boolean,
    ) {
        super(entity, position, scale);
        this.textureFont = this.game.getTexture(textureFont);
        this.ignoreCam = ignoreCam;

        // max number of rows and columns of chars
        this.maxCharSize = maxCharSize;

        // number of glyphs in sheet
        this.fontSize = fontSize;
        // glyph size in pixels
        this.glyphSize = glyphSize;
        this.glyphStr = glyphStr;

        this.cursorPos = vec2.create();
        this.text = '';
        this.color = color;
        this.bgcolor = bgcolor;

        // init target texture
        this.targetTextureWidth = glyphSize[0] * maxCharSize[0];
        this.targetTextureHeight = glyphSize[1] * maxCharSize[1];

        this.internalProjMatrix = mat4.create();
        mat4.ortho(
            this.internalProjMatrix,
            0, // left
            this.targetTextureWidth, // right
            0, // bottom
            this.targetTextureHeight, // top
            0, // near
            10000, // far
        );

        this.targetTexture = this.gl.createTexture();
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.targetTexture);
        this.gl.texImage2D(
            this.gl.TEXTURE_2D,
            0, // mipmap levels
            this.gl.RGBA, // internal format
            this.targetTextureWidth,
            this.targetTextureHeight,
            0, // border
            this.gl.RGBA, // source format,
            this.gl.UNSIGNED_BYTE, // buffer type
            null, // data pointer
        );

        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);

        this.frameBuffer = this.gl.createFramebuffer();

        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.framebufferTexture2D(
            this.gl.FRAMEBUFFER,
            this.gl.COLOR_ATTACHMENT0, // attachment point
            this.gl.TEXTURE_2D,
            this.targetTexture,
            0, // level
        );
    }

    dump(): ComponentData {
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

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        const scale = vec3.clone(this.scale);
        scale[0] *= this.maxCharSize[0] * this.glyphSize[0];
        scale[1] *= this.maxCharSize[1] * this.glyphSize[1];
        mat4.translate(modelMatrix, modelMatrix, this.position);
        mat4.scale(modelMatrix, modelMatrix, scale);
        return modelMatrix;
    }

    getChrIndex(chr: string): number {
        for (let i = 0; i < this.glyphStr.length; i++) {
            if (this.glyphStr[i] === chr) {
                return i;
            }
        }
        return -1;
    }

    advanceCursor(): void {
        this.cursorPos[0] += this.glyphSize[0];
        if (this.cursorPos[0] >= this.maxCharSize[0] * this.glyphSize[0]) {
            this.cursorPos[0] = 0;
            this.cursorPos[1] += this.glyphSize[1];
        }

        if (this.cursorPos[1] >= this.maxCharSize[1] * this.glyphSize[1]) {
            this.cursorPos[1] = 0;
        }
    }

    appendText(str: string): void {
        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.enable(this.gl.BLEND);
        this.gl.blendFunc(this.gl.SRC_ALPHA, this.gl.ONE_MINUS_SRC_ALPHA);
        this.gl.viewport(0, 0, this.targetTextureWidth, this.targetTextureHeight);

        for (const chr of str) {
            const glyphIndex = this.getChrIndex(chr);
            if (glyphIndex < 0) {
                if (chr === ' ') {
                    this.advanceCursor();
                    continue;
                } else {
                    throw new Error("Char '" + chr + "' not in font glyph string");
                }
            }

            const modelMatrix = mat4.create();
            mat4.translate(modelMatrix, modelMatrix, [this.cursorPos[0], this.cursorPos[1], 0]);
            mat4.scale(modelMatrix, modelMatrix, [this.glyphSize[0], this.glyphSize[1], 1]);

            // Verts
            this.game.buffers.quad.verts.bind();
            this.gl.vertexAttribPointer(
                this.game.shaders.imageSheet.attr.vertexPos,
                3,
                this.gl.FLOAT,
                false,
                0,
                0,
            );
            this.gl.enableVertexAttribArray(this.game.shaders.imageSheet.attr.vertexPos);

            // UVs
            this.game.buffers.quad.uvs.bind();
            this.gl.vertexAttribPointer(
                this.game.shaders.imageSheet.attr.texCoord,
                2,
                this.gl.FLOAT,
                false,
                0,
                0,
            );
            this.gl.enableVertexAttribArray(this.game.shaders.imageSheet.attr.texCoord);

            // Indices
            this.game.buffers.quad.indices.bind();

            this.game.shaders.imageSheet.bind();

            this.textureFont.bind(this.gl.TEXTURE0);

            this.gl.uniform1i(this.game.shaders.imageSheet.unif.texSampler, 0);
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.projectionMatrix,
                false,
                this.internalProjMatrix,
            );
            if (!this.ignoreCam) {
                this.gl.uniformMatrix4fv(
                    this.game.shaders.imageSheet.unif.cameraMatrix,
                    false,
                    this.game.camera.matrix(),
                );
            } else {
                this.gl.uniformMatrix4fv(
                    this.game.shaders.imageSheet.unif.cameraMatrix,
                    false,
                    mat4.create(),
                );
            }

            this.gl.uniformMatrix4fv(
                this.game.shaders.imageSheet.unif.modelMatrix,
                false,
                modelMatrix,
            );

            this.gl.uniform1f(this.game.shaders.imageSheet.unif.tileIdFlat, glyphIndex);
            this.gl.uniform2fv(this.game.shaders.imageSheet.unif.tileSetSize, this.fontSize);

            this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);

            this.advanceCursor();
        }
    }

    setText(str: string): void {
        this.cursorPos = vec2.fromValues(0, 0);
        this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
        this.gl.clearColor(
            this.bgcolor[0] as number,
            this.bgcolor[1] as number,
            this.bgcolor[2] as number,
            this.bgcolor[3] as number,
        );
        this.gl.clear(this.gl.COLOR_BUFFER_BIT);
        this.appendText(str);
    }

    setMaxCharSize(cols: number, rows: number = 1): void {
        cols = Math.max(1, cols);
        rows = Math.max(1, rows);
        if (this.maxCharSize[0] === cols && this.maxCharSize[1] === rows) return;

        this.maxCharSize = [cols, rows];

        // recompute target size
        this.targetTextureWidth = this.glyphSize[0] * cols;
        this.targetTextureHeight = this.glyphSize[1] * rows;

        // resize existing texture storage
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.targetTexture);
        this.gl.texImage2D(
            this.gl.TEXTURE_2D,
            0,
            this.gl.RGBA,
            this.targetTextureWidth,
            this.targetTextureHeight,
            0,
            this.gl.RGBA,
            this.gl.UNSIGNED_BYTE,
            null,
        );

        // update internal projection
        mat4.ortho(
            this.internalProjMatrix,
            0,
            this.targetTextureWidth,
            0,
            this.targetTextureHeight,
            0,
            10000,
        );
    }

    rawDraw(): void {
        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageColorize.attr.vertexPos,
            3,
            this.gl.FLOAT,
            false,
            0,
            0,
        );
        this.gl.enableVertexAttribArray(this.game.shaders.imageColorize.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.imageColorize.attr.texCoord,
            2,
            this.gl.FLOAT,
            false,
            0,
            0,
        );
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
            this.game.projectionMatrix,
        );
        if (!this.ignoreCam) {
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageColorize.unif.cameraMatrix,
                false,
                this.game.camera.matrix(),
            );
        } else {
            this.gl.uniformMatrix4fv(
                this.game.shaders.imageColorize.unif.cameraMatrix,
                false,
                mat4.create(),
            );
        }

        this.gl.uniformMatrix4fv(
            this.game.shaders.imageColorize.unif.modelMatrix,
            false,
            this.modelMatrix(),
        );

        this.gl.uniform4fv(this.game.shaders.imageColorize.unif.color, this.color as number[]);

        this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);

        this.advanceCursor();
    }
}

// Register components
registerComponent('Transform', Transform);
registerComponent('Drawable', Drawable);
registerComponent('Rectangle', Rectangle);
registerComponent('Sprite', Sprite);
registerComponent('Text', Text);
