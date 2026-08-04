/**
 * SdfText - Signed Distance Field text renderer for classic-wgl
 *
 * Pure-GL non-monospaced font rendering using a pre-generated SDF atlas.
 * Supports outlines, drop shadows, kerning, and pixel-precise spacing.
 */

import { Drawable } from '/classic/transforms.js';
import { registerComponent } from '/classic/registry.js';
import type { IEntity, ITexture, ComponentData, SdfFontMetrics } from './types.js';
import { mat4, vec3 } from 'gl-matrix';

type Vec3Like = vec3 | [number, number, number] | number[];
type Color = [number, number, number, number] | number[];

export type { SdfFontMetrics };

export class SdfText extends Drawable {
    atlasTexture: ITexture;
    metrics: SdfFontMetrics;
    atlasName: string;
    ignoreCam: boolean;
    color: Color;
    bgcolor: Color;
    outlineColor: Color;
    outlineWidth: number;
    shadowOffset: [number, number];
    shadowColor: Color;
    shadowBlur: number;
    text: string;
    textWidth: number;
    textHeight: number;
    justify: 'left' | 'center' | 'right' = 'left';
    weight: number;
    gamma: number;
    glowRadius: number;
    glowColor: Color;
    snapToPixel: boolean;
    showNotdef: boolean;
    vertexBuffer: WebGLBuffer | null = null;
    vertexCount: number;
    glyphData: Float32Array;

    private _cpuBufferSize: number;
    private _bufferDirty: boolean;
    protected _scale: number;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        atlasName: string,
        color: Color,
        bgcolor: Color,
        ignoreCam: boolean,
    ) {
        super(entity, position, scale);

        this.atlasTexture = this.game.getTexture(`${atlasName}-sdf`);
        this.metrics = this.game.getSdfFont(atlasName);

        this.gl.bindTexture(this.gl.TEXTURE_2D, this.atlasTexture.texture);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.LINEAR);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.LINEAR);

        this.atlasName = atlasName;
        this.ignoreCam = ignoreCam;
        this.color = color;
        this.bgcolor = bgcolor;
        this.outlineColor = [0, 0, 0, 1];
        this.outlineWidth = 0;
        this.shadowOffset = [0, 0];
        this.shadowColor = [0, 0, 0, 0.5];
        this.shadowBlur = 0;
        this.weight = 0;
        this.gamma = 1.4;
        this.glowRadius = 0;
        this.glowColor = [1, 1, 1, 0.3];
        this.snapToPixel = false;
        this.showNotdef = true;
        this.text = '';
        this.textWidth = 0;
        this.textHeight = 0;
        this.vertexCount = 0;
        this._cpuBufferSize = 0;
        this._bufferDirty = false;
        this._scale = (scale as number[])[0] || 1;
        this.glyphData = new Float32Array(0);
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.atlasName = this.atlasName;
        minObj.color = this.color;
        minObj.bgcolor = this.bgcolor;
        minObj.outlineColor = this.outlineColor;
        minObj.outlineWidth = this.outlineWidth;
        minObj.shadowOffset = this.shadowOffset;
        minObj.shadowColor = this.shadowColor;
        minObj.ignoreCam = this.ignoreCam;
        return minObj;
    }

    modelMatrix(offset?: readonly [number, number]): mat4 {
        const m = mat4.create();
        const ox = offset?.[0] ?? 0;
        const oy = offset?.[1] ?? 0;
        let px = this.position[0] + ox;
        let py = this.position[1] + oy;
        if (this.snapToPixel && this.ignoreCam) {
            px = Math.round(px);
            py = Math.round(py);
        }
        mat4.translate(m, m, [px, py, this.position[2]]);
        mat4.scale(m, m, [this.textWidth, this.textHeight, this.scale[2]]);
        return m;
    }

    setScale(s: number): this {
        this._scale = s;
        this.scale = vec3.fromValues(s, s, this.scale[2]);
        if (this.text) this._buildGlyphBuffer(this.text);
        return this;
    }

    setColor(rgba: Color): this {
        this.color = rgba;
        return this;
    }

    setOutline(width: number, rgba: Color = this.outlineColor): this {
        this.outlineWidth = width;
        this.outlineColor = rgba;
        return this;
    }

    setShadow(
        offsetX: number,
        offsetY: number,
        rgba: Color = [0, 0, 0, 0.5],
        blur: number = 0,
    ): this {
        this.shadowOffset = [offsetX, offsetY];
        this.shadowColor = rgba;
        this.shadowBlur = blur;
        return this;
    }

    setGlow(radius: number, rgba: Color = [1, 1, 1, 0.3]): this {
        this.glowRadius = radius;
        this.glowColor = rgba;
        return this;
    }

    setWeight(w: number): this {
        this.weight = w;
        return this;
    }

    setGamma(g: number): this {
        this.gamma = g;
        return this;
    }

    setText(str: string): this {
        this.text = str;
        this._buildGlyphBuffer(str);
        return this;
    }

    setTextSync(str: string): void {
        this.text = str;
        this._buildGlyphBuffer(str);
    }

    advanceFor(ch: string): number {
        const g = this.metrics.glyphs[ch];
        if (g) return g.xAdvance;
        if (ch === ' ') return this.metrics.glyphs[' ']?.xAdvance || this.metrics.glyphSize * 0.5;
        if (ch === '\t')
            return (this.metrics.glyphs[' ']?.xAdvance || this.metrics.glyphSize * 0.5) * 4;
        return this.metrics.glyphSize * 0.5;
    }

    private _buildGlyphBuffer(str: string): void {
        const m = this.metrics;
        const atlasW = m.atlasSize[0];
        const atlasH = m.atlasSize[1];
        const scale = this._scale;

        let penX = 0;
        let maxWidth = 0;
        const maxH = m.lineHeight * scale;

        const perLine: Array<{ char: string; x: number; y: number; adv: number }> = [];

        let lineIndex = 0;
        for (const line of str.split('\n')) {
            let lineX = 0;
            for (const ch of line) {
                const g = m.glyphs[ch];
                const adv = this.advanceFor(ch) * scale;
                if (g) {
                    perLine.push({
                        char: ch,
                        x: lineX + g.xOffset * scale,
                        y: lineIndex,
                        adv,
                    });
                }
                lineX += adv;
            }
            if (lineX > maxWidth) maxWidth = lineX;
            lineIndex++;
        }

        if (this.justify !== 'left') {
            const lineWidths: Record<number, number> = {};
            for (const pg of perLine) {
                lineWidths[pg.y] = (lineWidths[pg.y] || 0) + pg.adv;
            }
            for (const pg of perLine) {
                const lw = Math.max(1, lineWidths[pg.y] || 0);
                const extra = maxWidth - lw;
                if (this.justify === 'center') pg.x += extra / 2;
                else if (this.justify === 'right') pg.x += extra;
            }
        }

        this.textWidth = maxWidth;
        this.textHeight = maxH * (lineIndex || 1);

        let glyphExtentMin = Infinity;
        let glyphExtentMax = -Infinity;
        for (const pg of perLine) {
            const g = m.glyphs[pg.char];
            if (!g) continue;
            const gy = m.baseline * scale + g.yOffset * scale + pg.y * m.lineHeight * scale;
            if (gy < glyphExtentMin) glyphExtentMin = gy;
            if (gy + g.h * scale > glyphExtentMax) glyphExtentMax = gy + g.h * scale;
        }
        if (glyphExtentMin < glyphExtentMax) {
            this.textHeight = glyphExtentMax - glyphExtentMin;
        }

        const vertsPerGlyph = 6;
        const floatsPerVert = 4;
        const totalFloats = perLine.length * vertsPerGlyph * floatsPerVert;
        const data = new Float32Array(totalFloats);

        let vi = 0;
        for (const pg of perLine) {
            const g = m.glyphs[pg.char]!;
            const gx = pg.x;
            const gy = m.baseline * scale + g.yOffset * scale + pg.y * m.lineHeight * scale;
            const gw = g.w * scale;
            const gh = g.h * scale;
            const tw = this.textWidth || 1;
            const th = this.textHeight || 1;

            const lx0 = gx / tw;
            const lx1 = (gx + gw) / tw;
            const ly0 = gy / th;
            const ly1 = (gy + gh) / th;

            const ux0 = g.x / atlasW;
            const ux1 = (g.x + g.w) / atlasW;
            const uy0 = g.y / atlasH;
            const uy1 = (g.y + g.h) / atlasH;

            data[vi + 0] = lx0;
            data[vi + 1] = ly0;
            data[vi + 2] = ux0;
            data[vi + 3] = uy0;
            data[vi + 4] = lx1;
            data[vi + 5] = ly0;
            data[vi + 6] = ux1;
            data[vi + 7] = uy0;
            data[vi + 8] = lx1;
            data[vi + 9] = ly1;
            data[vi + 10] = ux1;
            data[vi + 11] = uy1;

            data[vi + 12] = lx0;
            data[vi + 13] = ly0;
            data[vi + 14] = ux0;
            data[vi + 15] = uy0;
            data[vi + 16] = lx1;
            data[vi + 17] = ly1;
            data[vi + 18] = ux1;
            data[vi + 19] = uy1;
            data[vi + 20] = lx0;
            data[vi + 21] = ly1;
            data[vi + 22] = ux0;
            data[vi + 23] = uy1;

            vi += vertsPerGlyph * floatsPerVert;
        }

        this.glyphData = data;
        this.vertexCount = perLine.length * vertsPerGlyph;
        this._bufferDirty = true;
    }

    rawDraw(): void {
        if (this.vertexCount === 0) return;

        if (!this.vertexBuffer) {
            this.vertexBuffer = this.gl.createBuffer();
        }

        if (this._bufferDirty) {
            this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this.vertexBuffer);
            this.gl.bufferData(this.gl.ARRAY_BUFFER, this.glyphData, this.gl.DYNAMIC_DRAW);
            this._bufferDirty = false;
        }

        const shader = this.game.shaders.sdf;
        shader.bind();

        const stride = 4 * 4;
        this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this.vertexBuffer);
        this.gl.vertexAttribPointer(shader.attr.vertexPos, 2, this.gl.FLOAT, false, stride, 0);
        this.gl.enableVertexAttribArray(shader.attr.vertexPos);

        this.gl.vertexAttribPointer(shader.attr.texCoord, 2, this.gl.FLOAT, false, stride, 2 * 4);
        this.gl.enableVertexAttribArray(shader.attr.texCoord);

        this.gl.activeTexture(this.gl.TEXTURE0);
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.atlasTexture.texture);
        this.gl.uniform1i(shader.unif.texSampler, 0);

        this.gl.uniformMatrix4fv(shader.unif.projectionMatrix, false, this.game.projectionMatrix);

        if (!this.ignoreCam) {
            this.gl.uniformMatrix4fv(shader.unif.cameraMatrix, false, this.game.camera.matrix());
        } else {
            this.gl.uniformMatrix4fv(shader.unif.cameraMatrix, false, mat4.create());
        }

        this.gl.uniform1f(shader.unif.softEdge, 0.08);
        this.gl.uniform1f(shader.unif.spread, this.metrics.spread || 2);
        this.gl.uniform2f(
            shader.unif.atlasSize,
            this.metrics.atlasSize[0],
            this.metrics.atlasSize[1],
        );
        this.gl.uniform1f(shader.unif.weight, this.weight);
        this.gl.uniform1f(shader.unif.gamma, this.gamma);

        const drawPass = (passColor: Color, outlineW: number, offsetX: number, offsetY: number) => {
            const passModel = this.modelMatrix(
                offsetX !== 0 || offsetY !== 0 ? [offsetX, offsetY] : undefined,
            );
            this.gl.uniformMatrix4fv(shader.unif.modelMatrix, false, passModel);

            this.gl.uniform4fv(shader.unif.color, passColor as number[]);

            const oc = outlineW !== 0 ? this.outlineColor : [0, 0, 0, 0];
            this.gl.uniform4fv(shader.unif.outlineColor, oc as number[]);
            this.gl.uniform1f(shader.unif.outlineWidth, outlineW);

            this.gl.drawArrays(this.gl.TRIANGLES, 0, this.vertexCount);
        };

        const hasShadow =
            (this.shadowOffset[0] !== 0 || this.shadowOffset[1] !== 0) &&
            (this.shadowColor[3] as number) > 0;

        if (hasShadow) {
            drawPass(this.shadowColor, this.shadowBlur, this.shadowOffset[0], this.shadowOffset[1]);
        }

        if (this.glowRadius > 0 && (this.glowColor[3] as number) > 0) {
            drawPass(this.glowColor, this.glowRadius, 0, 0);
        }

        drawPass(this.color, this.outlineWidth, 0, 0);
    }
}

export interface SdfText extends Drawable {
    setPosition(x: number, y: number): this;
}

Object.defineProperty(SdfText.prototype, 'setPosition', {
    value: function (this: SdfText, x: number, y: number): SdfText {
        this.position[0] = x;
        this.position[1] = y;
        return this;
    },
    writable: true,
});

registerComponent(
    'SdfText',
    SdfText as unknown as new (entity: IEntity, ...args: unknown[]) => Drawable,
);
