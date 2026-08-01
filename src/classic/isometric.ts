import game from '/classic/state.js';
import { fetchBase64Object, getNoiseRange } from '/classic/utils.js';
import { Component } from '/classic/ecs.js';
import { Drawable } from '/classic/transforms.js';
import { isoToCartesian4, cartesianToIso4 } from '/classic/utils.js';
import { Animator } from '/classic/animator.js';
import { registerComponent } from '/classic/registry.js';
import type { IEntity, ITexture, ComponentData, IAnimation } from './types.js';

import { mat4, vec2, vec3 } from 'gl-matrix';

type Vec3Like = vec3 | [number, number, number] | number[];
type Vec2Like = vec2 | [number, number] | number[];

export class Tilemap extends Drawable {
    sizeX: number;
    sizeY: number;
    mapSize: [number, number];
    tileSet: ITexture;
    tilePixelSize: [number, number];
    tileSetSize: [number, number];
    tileSetPixelSize: [number, number];
    maxTile: number;
    dataUrl: string | null;
    data: number[];
    mapDataTexture: WebGLTexture | null = null;
    mouseIsoPos: vec3;
    selectionIsoBegin: vec3;
    selectionIsoEnd: vec3;
    invScale: vec3;
    _isoToCartesian: mat4;
    _cartesianToIso: mat4;
    heightData: number[];
    heightScale: number;
    _meshVertBuffer: WebGLBuffer | null = null;
    _meshVertCount: number = 0;
    _meshDirty: boolean = true;
    _needsBufferResize: boolean = true;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        sizeX: number,
        sizeY: number,
        tileSet: string,
        tilePixelSize: [number, number],
        maxTile: number,
        dataUrl: string | null,
    ) {
        super(entity, position, scale);
        this.sizeX = sizeX;
        this.sizeY = sizeY;
        this.mapSize = [sizeX, sizeY];

        this.tileSet = this.game.getTexture(tileSet);
        this.tilePixelSize = tilePixelSize;

        this.invScale = vec3.create();
        this._isoToCartesian = mat4.create();
        this._cartesianToIso = mat4.create();
        this.setScale(scale);

        this.tileSetSize = [
            this.tileSet.image.width / tilePixelSize[0],
            this.tileSet.image.height / tilePixelSize[1],
        ];

        this.tileSetPixelSize = [this.tileSet.image.width, this.tileSet.image.height];

        this.maxTile = maxTile;
        this.dataUrl = dataUrl;

        this.heightData = Array(sizeX * sizeY).fill(1);
        this.heightScale = tilePixelSize[0];

        if (dataUrl != null) {
            this.data = [];
            this.loadMap(dataUrl);
        } else {
            this.data = Array(sizeX * sizeY);
            for (let y = 0; y < this.sizeY; y++) {
                for (let x = 0; x < this.sizeX; x++) {
                    this.data[x + sizeX * y] = 0;
                }
            }
            this._meshDirty = true;
        }

        this.mouseIsoPos = vec3.fromValues(-1, -1, -1);
        this.selectionIsoBegin = vec3.fromValues(-1, -1, -1);
        this.selectionIsoEnd = vec3.fromValues(-1, -1, -1);

        entity.registerCall('update', this.updateMousePos.bind(this));
        entity.registerCall('selectionBegin', this.selectionBegin.bind(this));
        entity.registerCall('selectionEnd', this.selectionEnd.bind(this));
    }

    selectionBegin(): void {
        this.selectionIsoBegin = vec3.clone(this.mouseIsoPos);
    }

    selectionEnd(): void {
        this.selectionIsoEnd = vec3.clone(this.mouseIsoPos);
    }

    updateMousePos(): void {
        this.mouseIsoPos = vec3.clone(this.game.mousePos);
        vec3.add(this.mouseIsoPos, this.mouseIsoPos, this.game.camera.getFix());
        vec3.div(this.mouseIsoPos, this.mouseIsoPos, this.game.camera.scale as vec3);
        this.cartesianToIso(this.mouseIsoPos);

        const hd = this.heightData;
        const sX = this.sizeX;
        const sY = this.sizeY;
        const at = (tx: number, ty: number) =>
            hd[
                Math.min(Math.max(Math.floor(tx), 0), sX - 1) +
                    Math.min(Math.max(Math.floor(ty), 0), sY - 1) * sX
            ] ?? 0;
        const tileStep = (this.scale[0] as number) * 0.7071;

        let isoX = this.mouseIsoPos[0];
        let isoY = this.mouseIsoPos[1];

        for (let i = 0; i < 3; i++) {
            const ftx = Math.floor(isoX);
            const fty = Math.floor(isoY);
            const fx = isoX - ftx;
            const fy = isoY - fty;
            const hNW = at(ftx, fty);
            const hNE = at(ftx + 1, fty);
            const hSW = at(ftx, fty + 1);
            const hSE = at(ftx + 1, fty + 1);
            const h = hNW + (hNE - hNW) * fx + (hSW - hNW) * fy + (hNW - hNE - hSW + hSE) * fx * fy;
            if (h <= 0) break;
            const zOffset = (h * this.heightScale) / tileStep;
            isoX = this.mouseIsoPos[0] - zOffset;
            isoY = this.mouseIsoPos[1] + zOffset;
        }

        this.mouseIsoPos[0] = isoX;
        this.mouseIsoPos[1] = isoY;
    }

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        mat4.translate(modelMatrix, modelMatrix, this.position);
        return modelMatrix;
    }

    downloadMap(url: string): void {
        const link = document.createElement('a');
        link.download = url;

        const blob = new Blob([btoa(JSON.stringify(this.data))], {
            type: 'text/plain;charset=utf-8',
        });

        link.href = URL.createObjectURL(blob);
        link.click();
        URL.revokeObjectURL(link.href);
    }

    loadMap(url: string): void {
        console.log('[loadMap] Fetching map from:', url);
        fetch(url)
            .then((res) => res.text())
            .then((text) => {
                this.data = JSON.parse(atob(text.trim()));
                this.uploadToGPU();
            })
            .catch((err) => {
                console.error('[Tilemap] Failed to load map data:', err);
            });
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.sizeX = this.sizeX;
        minObj.sizeY = this.sizeY;
        minObj.tileSet = this.tileSet.name;
        minObj.tilePixelSize = this.tilePixelSize;
        minObj.maxTile = this.maxTile;
        minObj.data = this.dataUrl;
        minObj.heightScale = this.heightScale;
        return minObj;
    }

    setScale(scale: Vec3Like): void {
        this.scale = vec3.clone(scale as vec3);
        this.invScale = vec3.create();
        vec3.inverse(this.invScale, this.scale);

        this._isoToCartesian = mat4.clone(isoToCartesian4);
        mat4.scale(this._isoToCartesian, this._isoToCartesian, this.scale);

        this._cartesianToIso = mat4.clone(cartesianToIso4);
        mat4.scale(this._cartesianToIso, this._cartesianToIso, this.invScale);
    }

    cartesianToIso(v: vec3): void {
        vec3.transformMat4(v, v, this._cartesianToIso);
    }

    isoToCartesian(v: vec3 | number[]): void {
        vec3.transformMat4(v as vec3, v as vec3, this._isoToCartesian);
    }

    generateNoiseMap(): void {
        for (let y = 0; y < this.sizeY; y++) {
            for (let x = 0; x < this.sizeX; x++) {
                this.data[x + this.sizeX * y] = Math.floor(getNoiseRange(x, y, 0, this.maxTile));
            }
        }
        this._meshDirty = true;
    }

    getSelection(): [vec2, vec2] {
        const from = vec2.create();
        const to = vec2.create();
        vec2.min(from, this.selectionIsoBegin as vec2, this.selectionIsoEnd as vec2);
        vec2.max(to, this.selectionIsoBegin as vec2, this.selectionIsoEnd as vec2);
        vec2.floor(from, from);
        vec2.ceil(to, to);
        return [from, to];
    }

    fillRegion(from: Vec2Like, to: Vec2Like, value: number): void {
        const [fromX, fromY] = from as [number, number];
        const [toX, toY] = to as [number, number];

        for (let y = fromY; y < toY; y++) {
            for (let x = fromX; x < toX; x++) {
                this.data[x + this.sizeX * y] = value;
            }
        }
        this._meshDirty = true;
    }

    uploadToGPU(): void {
        if (this.mapDataTexture != null) {
            this.gl.deleteTexture(this.mapDataTexture);
        }

        const pixelData = new Uint8Array(this.sizeX * this.sizeY * 4);
        for (let i = 0; i < this.sizeX * this.sizeY * 4; i += 4) {
            const val = this.data[Math.floor(i / 4)];
            pixelData[i] = val;
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
            this.sizeX,
            this.sizeY,
            0,
            this.gl.RGBA,
            this.gl.UNSIGNED_BYTE,
            pixelData,
        );

        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.NEAREST);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_S, this.gl.CLAMP_TO_EDGE);
        this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_WRAP_T, this.gl.CLAMP_TO_EDGE);

        this._meshDirty = true;
    }

    setHeight(x: number, y: number, value: number): void {
        if (x < 0 || x >= this.sizeX || y < 0 || y >= this.sizeY) return;
        this.heightData[x + this.sizeX * y] = value;
        this._meshDirty = true;
    }

    buildMesh(): void {
        const sX = this.sizeX;
        const sY = this.sizeY;
        const hs = this.heightScale;

        const maxVerts = sX * sY * 180; // worst case: face + 4 walls = 30 vertices = 180 floats
        const verts = new Float32Array(maxVerts);
        let vi = 0;

        const mx = new Float32Array(sX + 1);
        const my = new Float32Array(sY + 1);
        for (let i = 0; i <= sX; i++) mx[i] = i / sX;
        for (let i = 0; i <= sY; i++) my[i] = i / sY;

        for (let ty = 0; ty < sY; ty++) {
            for (let tx = 0; tx < sX; tx++) {
                const idx = tx + ty * sX;
                const tid = this.data[idx];
                const hThis = this.heightData[idx];

                const hNW = hThis;
                const hNE = tx + 1 < sX ? this.heightData[tx + 1 + ty * sX] : hThis;
                const hSW = ty + 1 < sY ? this.heightData[tx + (ty + 1) * sX] : hThis;
                const hSE =
                    tx + 1 < sX && ty + 1 < sY ? this.heightData[tx + 1 + (ty + 1) * sX] : hThis;

                const hasFace = tid !== 0 || hNW !== 0 || hNE !== 0 || hSW !== 0 || hSE !== 0;
                if (!hasFace) continue;

                const zNW = hNW * hs;
                const zNE = hNE * hs;
                const zSW = hSW * hs;
                const zSE = hSE * hs;

                const zMax = Math.max(zNW, zNE, zSW, zSE);
                const zMin = Math.min(zNW, zNE, zSW, zSE);
                const steepness = Math.min((zMax - zMin) / hs, 1.0);
                const faceTileId = -steepness;

                const mxNW = mx[tx];
                const myNW = my[ty];
                const mxNE = mx[tx + 1];
                const myNE = my[ty];
                const mxSW = mx[tx];
                const mySW = my[ty + 1];
                const mxSE = mx[tx + 1];
                const mySE = my[ty + 1];

                verts[vi++] = tx;
                verts[vi++] = ty;
                verts[vi++] = zNW;
                verts[vi++] = mxNW;
                verts[vi++] = myNW;
                verts[vi++] = faceTileId;
                verts[vi++] = tx + 1;
                verts[vi++] = ty;
                verts[vi++] = zNE;
                verts[vi++] = mxNE;
                verts[vi++] = myNE;
                verts[vi++] = faceTileId;
                verts[vi++] = tx;
                verts[vi++] = ty + 1;
                verts[vi++] = zSW;
                verts[vi++] = mxSW;
                verts[vi++] = mySW;
                verts[vi++] = faceTileId;
                verts[vi++] = tx + 1;
                verts[vi++] = ty;
                verts[vi++] = zNE;
                verts[vi++] = mxNE;
                verts[vi++] = myNE;
                verts[vi++] = faceTileId;
                verts[vi++] = tx + 1;
                verts[vi++] = ty + 1;
                verts[vi++] = zSE;
                verts[vi++] = mxSE;
                verts[vi++] = mySE;
                verts[vi++] = faceTileId;
                verts[vi++] = tx;
                verts[vi++] = ty + 1;
                verts[vi++] = zSW;
                verts[vi++] = mxSW;
                verts[vi++] = mySW;
                verts[vi++] = faceTileId;

                const wallTileId = tid || 1;

                if (tx + 1 >= sX && hThis > 0) {
                    const zTop = hThis * hs;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                }

                if (ty + 1 >= sY && hThis > 0) {
                    const zTop = hThis * hs;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSE;
                    verts[vi++] = mySE;
                    verts[vi++] = wallTileId;
                }

                if (tx === 0 && hThis > 0) {
                    const zTop = hThis * hs;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = 0;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty + 1;
                    verts[vi++] = zTop;
                    verts[vi++] = mxSW;
                    verts[vi++] = mySW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                }

                if (ty === 0 && hThis > 0) {
                    const zTop = hThis * hs;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx + 1;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNE;
                    verts[vi++] = myNE;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = zTop;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                    verts[vi++] = tx;
                    verts[vi++] = ty;
                    verts[vi++] = 0;
                    verts[vi++] = mxNW;
                    verts[vi++] = myNW;
                    verts[vi++] = wallTileId;
                }
            }
        }

        const floatCount = vi;
        this._meshVertCount = floatCount / 6;

        if (this._meshVertBuffer == null || this._needsBufferResize) {
            if (this._meshVertBuffer != null) this.gl.deleteBuffer(this._meshVertBuffer);
            this._meshVertBuffer = this.gl.createBuffer();
            this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this._meshVertBuffer);
            this.gl.bufferData(this.gl.ARRAY_BUFFER, maxVerts * 4, this.gl.DYNAMIC_DRAW);
            this._needsBufferResize = false;
        }

        this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this._meshVertBuffer);
        this.gl.bufferSubData(this.gl.ARRAY_BUFFER, 0, verts.subarray(0, floatCount));
        this.gl.bindBuffer(this.gl.ARRAY_BUFFER, null);
        this._meshDirty = false;
    }

    rawDraw(): void {
        if (this._meshDirty) {
            this.buildMesh();
        }

        if (this._meshVertBuffer == null || this._meshVertCount === 0) return;
        if (this.mapDataTexture == null) return;

        this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this._meshVertBuffer);
        const stride = 6 * 4;
        const shader = this.game.shaders.isoTilemap;

        this.gl.vertexAttribPointer(shader.attr.vertexPos, 3, this.gl.FLOAT, false, stride, 0);
        this.gl.enableVertexAttribArray(shader.attr.vertexPos);

        this.gl.vertexAttribPointer(shader.attr.mapCoord, 2, this.gl.FLOAT, false, stride, 12);
        this.gl.enableVertexAttribArray(shader.attr.mapCoord);

        this.gl.vertexAttribPointer(shader.attr.tileId, 1, this.gl.FLOAT, false, stride, 20);
        this.gl.enableVertexAttribArray(shader.attr.tileId);

        shader.bind();

        this.gl.activeTexture(this.gl.TEXTURE0);
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.mapDataTexture);

        this.tileSet.bind(this.gl.TEXTURE1);

        this.gl.uniform1i(shader.unif.mapData, 0);
        this.gl.uniform1i(shader.unif.tileSet, 1);

        this.gl.uniformMatrix4fv(shader.unif.projectionMatrix, false, this.game.projectionMatrix);
        this.gl.uniformMatrix4fv(shader.unif.cameraMatrix, false, this.game.camera.matrix());
        this.gl.uniformMatrix4fv(shader.unif.modelMatrix, false, this.modelMatrix());
        this.gl.uniformMatrix4fv(shader.unif.isoMatrix, false, this._isoToCartesian);

        this.gl.uniform2fv(shader.unif.tileSetSize, this.tileSetSize);
        this.gl.uniform2fv(shader.unif.tilePixelSize, this.tilePixelSize);
        this.gl.uniform2fv(shader.unif.mapSize, this.mapSize);

        this.gl.uniform2fv(shader.unif.selectedTile, [this.mouseIsoPos[0], this.mouseIsoPos[1]]);
        this.gl.uniform2fv(shader.unif.selectionBegin, [
            this.selectionIsoBegin[0],
            this.selectionIsoBegin[1],
        ]);
        this.gl.uniform1i(shader.unif.selectionMode, this.game.selectionMode);
        this.gl.uniform4fv(shader.unif.selectionColor, this.game.selectionColor);

        this.gl.uniform4fv(shader.unif.wallColor, [0.3, 0.2, 0.15, 1.0]);
        this.gl.uniform1f(shader.unif.slopeDarken, 0.4);

        // Agent transparency: project agent world position to screen coords
        const agentEntity = this.game.getEntity('navAgent');
        const agent = agentEntity?.getComponent(IsoAgent);
        if (agent && (this.game.agentSelected ?? false)) {
            const aPos = vec3.clone(agent.position);
            const sX = this.sizeX;
            const sY = this.sizeY;
            this.isoToCartesian(aPos);
            vec3.add(aPos, aPos, this.position);
            const hd = this.heightData;
            const apx = agent.position[0];
            const apy = agent.position[1];
            const aftx = Math.floor(apx);
            const afty = Math.floor(apy);
            const afx = apx - aftx;
            const afy = apy - afty;
            const atH = (tx: number, ty: number) =>
                hd[
                    Math.min(Math.max(Math.floor(tx), 0), sX - 1) +
                        Math.min(Math.max(Math.floor(ty), 0), sY - 1) * sX
                ] ?? 0;
            const ahNW = atH(aftx, afty);
            const ahNE = atH(aftx + 1, afty);
            const ahSW = atH(aftx, afty + 1);
            const ahSE = atH(aftx + 1, afty + 1);
            const ah =
                ahNW +
                (ahNE - ahNW) * afx +
                (ahSW - ahNW) * afy +
                (ahNW - ahNE - ahSW + ahSE) * afx * afy;
            aPos[1] -= ah * this.heightScale;

            const clip = vec3.create();
            vec3.transformMat4(clip, aPos, this.game.camera.matrix() as mat4);
            vec3.transformMat4(clip, clip, this.game.projectionMatrix as mat4);
            const cw = this.game.canvas?.width ?? 1;
            const ch = this.game.canvas?.height ?? 1;
            const screenX = (clip[0] + 1.0) * 0.5 * cw;
            const screenY =
                (clip[1] + 1.0) * 0.5 * ch +
                (agent as unknown as { tilePixelSize: [number, number] }).tilePixelSize[1] *
                    0.75 *
                    (this.game.camera.scale[1] as number);

            const agentDepth =
                (agent.position[0] - agent.position[1]) / 400.0 + 0.5 - agent.position[2] / 14500.0;

            this.gl.uniform2f(shader.unif.agentScreenPos, screenX, screenY);
            this.gl.uniform1f(shader.unif.agentIsoDepth, agentDepth);

            const tps = (agent as unknown as { tilePixelSize: [number, number] }).tilePixelSize;
            const sc = (this.game.camera.scale[1] as number) || 1;
            const boxMinX = screenX - tps[0] * 0.5 * sc;
            const boxMinY = screenY - tps[1] * 0.02 * sc;
            const boxMaxX = screenX + tps[0] * 0.5 * sc;
            const boxMaxY = screenY + tps[1] * 0.98 * sc;

            this.gl.uniform2f(shader.unif.agentBoxMin, boxMinX, boxMinY);
            this.gl.uniform2f(shader.unif.agentBoxMax, boxMaxX, boxMaxY);
        } else {
            this.gl.uniform2f(shader.unif.agentScreenPos, -999, -999);
            this.gl.uniform1f(shader.unif.agentIsoDepth, 0.0);
            this.gl.uniform2f(shader.unif.agentBoxMin, 0, 0);
            this.gl.uniform2f(shader.unif.agentBoxMax, 0, 0);
        }

        this.gl.enable(this.gl.DEPTH_TEST);
        this.gl.drawArrays(this.gl.TRIANGLES, 0, this._meshVertCount);
        this.gl.disable(this.gl.DEPTH_TEST);
    }
}

export class IsometricNavMesh extends Tilemap {
    map: Tilemap;
    _msgId: number;
    _resolves: Record<number, (value: unknown) => void>;
    _rejects: Record<number, (reason?: unknown) => void>;
    _worker: Worker;

    constructor(
        entity: IEntity,
        map: string,
        sizeX: number,
        sizeY: number,
        dataUrl: string | null,
    ) {
        const mapComponent = entity.game
            .getEntity(map)!
            .getComponent(Tilemap as unknown as new (...args: unknown[]) => Tilemap)!;

        super(
            entity,
            mapComponent.position,
            mapComponent.scale,
            sizeX,
            sizeY,
            'navTileset',
            [8, 8],
            2,
            dataUrl,
        );

        this.map = mapComponent;

        this._msgId = 0;
        this._resolves = {};
        this._rejects = {};
        // new URL(..., import.meta.url) lets vite bundle the worker on build
        this._worker = new Worker(new URL('./pathfinder.ts', import.meta.url), { type: 'module' });
        this._worker.onmessage = this.pathfinderMessageHandler.bind(this);
    }

    loadMap(url: string): void {
        console.log('[loadMap] Fetching map from:', url);
        fetch(url)
            .then((res) => res.text())
            .then((text) => {
                this.data = JSON.parse(atob(text));
                this.uploadToGPU();
                this.sendMsg('initmap', {
                    name: this.entity.name,
                    size: [this.sizeX, this.sizeY],
                    data: this.data,
                }).then((ret) => {
                    console.assert(ret === 'ok', 'Isometric Nav Mesh initialization error');
                });
            });
    }

    updateMap(corner: [number, number], size: [number, number], data: number[]): Promise<unknown> {
        return this.sendMsg('updatemap', {
            name: this.entity.name,
            corner: corner,
            size: size,
            data: data,
        });
    }

    findPath(from: Vec2Like, to: Vec2Like): Promise<unknown> {
        return this.sendMsg('findpath', {
            name: this.entity.name,
            from: from,
            to: to,
        });
    }

    dump(): ComponentData {
        const minObj: ComponentData = { type: this.constructor.name };
        minObj.map = this.map.entity.name;
        minObj.sizeX = this.sizeX;
        minObj.sizeY = this.sizeY;
        minObj.data = this.dataUrl;
        return minObj;
    }

    sendMsg(op: string, args: unknown): Promise<unknown> {
        const msgId = this._msgId++;
        const msg = {
            op: op,
            args: args,
            id: msgId,
        };

        return new Promise((resolve, reject) => {
            this._resolves[msgId] = resolve;
            this._rejects[msgId] = reject;
            this._worker.postMessage(msg);
        });
    }

    pathfinderMessageHandler(msg: MessageEvent): void {
        const { id, data } = msg.data;

        const resolve = this._resolves[id];
        if (resolve) {
            resolve(data);
        }

        // purge used callbacks
        delete this._resolves[id];
        delete this._rejects[id];
    }
}

class IsometricDrawable extends Drawable {
    tilemap: Tilemap;
    direction: number;

    constructor(entity: IEntity, position: Vec3Like, scale: Vec3Like, tilemap: string) {
        super(entity, position, scale);
        this.tilemap = this.game
            .getEntity(tilemap)!
            .getComponent(Tilemap as unknown as new (...args: unknown[]) => Tilemap)!;
        this.direction = 0;
    }

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        const cartPos = vec3.clone(this.position);
        this.tilemap.isoToCartesian(cartPos);
        vec3.add(cartPos, cartPos, this.tilemap.position);

        const px = this.position[0];
        const py = this.position[1];
        const ftx = Math.floor(px);
        const fty = Math.floor(py);
        const fx = px - ftx;
        const fy = py - fty;
        const hd = this.tilemap.heightData;
        const sX = this.tilemap.sizeX;
        const sY = this.tilemap.sizeY;
        const at = (tx: number, ty: number) =>
            hd[Math.min(Math.max(tx, 0), sX - 1) + Math.min(Math.max(ty, 0), sY - 1) * sX] ?? 0;
        const hNW = at(ftx, fty);
        const hNE = at(ftx + 1, fty);
        const hSW = at(ftx, fty + 1);
        const hSE = at(ftx + 1, fty + 1);
        const h = hNW + (hNE - hNW) * fx + (hSW - hNW) * fy + (hNW - hNE - hSW + hSE) * fx * fy;
        cartPos[1] -= h * this.tilemap.heightScale;

        mat4.translate(modelMatrix, modelMatrix, cartPos);
        mat4.scale(modelMatrix, modelMatrix, this.scale);
        return modelMatrix;
    }

    order(): number {
        return this.position[0] - this.position[1];
    }
}

export class IsoSprite extends IsometricDrawable {
    texture: ITexture;
    frame: number;
    tileSetSize: Vec2Like;
    anchor: Vec2Like;
    tilePixelSize: [number, number];

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        texture: string,
        tilemap: string,
        frame: number,
        tileSetSize: Vec2Like,
        anchor: Vec2Like,
    ) {
        super(entity, position, scale, tilemap);
        this.texture = this.game.getTexture(texture);
        this.frame = frame;
        this.tileSetSize = tileSetSize;
        this.anchor = anchor;

        this.tilePixelSize = [
            this.texture.image.width / (tileSetSize[0] as number),
            this.texture.image.height / (tileSetSize[1] as number),
        ];
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.texture = this.texture.name;
        minObj.tilemap = this.tilemap.entity.name;
        minObj.frame = this.frame;
        minObj.tileSetSize = this.tileSetSize;
        minObj.anchor = this.anchor;
        return minObj;
    }

    modelMatrix(): mat4 {
        const modelMatrix = super.modelMatrix();
        const texDimension = vec3.clone(this.tilePixelSize as vec3);
        const texAnchorDelta = [
            texDimension[0] * (this.anchor[0] as number),
            texDimension[1] * (this.anchor[1] as number),
        ];

        const anchoredPos = vec3.create();
        anchoredPos[0] -= texAnchorDelta[0];
        anchoredPos[1] -= texAnchorDelta[1];
        mat4.translate(modelMatrix, modelMatrix, anchoredPos);

        const sizeInPixels: vec3 = [texDimension[0], texDimension[1], 1];
        mat4.scale(modelMatrix, modelMatrix, sizeInPixels);
        return modelMatrix;
    }

    rawDraw(): void {
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

        this.texture.bind(this.gl.TEXTURE0);

        this.gl.uniform1i(this.game.shaders.imageSheet.unif.texSampler, 0);
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.projectionMatrix,
            false,
            this.game.projectionMatrix,
        );
        this.gl.uniformMatrix4fv(
            this.game.shaders.imageSheet.unif.cameraMatrix,
            false,
            this.game.camera.matrix(),
        );
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

        this.gl.uniform1f(this.game.shaders.imageSheet.unif.useIsoDepth, 1.0);

        const depth =
            (this.position[0] - this.position[1]) / 400.0 +
            0.5 -
            this.position[2] / 14500.0 -
            0.001;
        this.gl.uniform1f(this.game.shaders.imageSheet.unif.isoDepth, depth);

        this.gl.enable(this.gl.DEPTH_TEST);
        this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);
        this.gl.disable(this.gl.DEPTH_TEST);
    }
}

const animDirs = [
    'East',
    'SouthEast',
    'South',
    'SouthWest',
    'West',
    'NorthWest',
    'North',
    'NorthEast',
];

const AgentStates = {
    idle: 0,
    followPath: 1,
} as const;

type AgentState = (typeof AgentStates)[keyof typeof AgentStates];

export class IsoAgent extends IsoSprite {
    anim!: Animator;
    speed: number;
    animIndex: number;
    _state: AgentState = AgentStates.idle;
    _path: Vec2Like[] = [];
    _start_index: number = 0;
    _target_index: number = 1;
    _delta: number = 0;
    _init_dist: number;

    constructor(
        entity: IEntity,
        position: Vec3Like,
        scale: Vec3Like,
        texture: string,
        tilemap: string,
        frame: number,
        tileSetSize: Vec2Like,
        anchor: Vec2Like,
        speed: number,
        animSpeed: number,
    ) {
        super(entity, position, scale, texture, tilemap, frame, tileSetSize, anchor);

        this.anim = entity.addComponent(Animator, this, animSpeed) as Animator;
        this.idle();

        this.speed = speed; // tiles per second
        this.animIndex = 2;
        this._init_dist = 0;

        entity.registerCall('update', this.update.bind(this));
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.speed = this.speed;
        minObj.animSpeed = this.anim.speed;
        return minObj;
    }

    idle(): void {
        this._state = AgentStates.idle;
        this._path = [];
        this._start_index = 0;
        this._target_index = 1;
        this._delta = 0;
    }

    followPath(path: Vec2Like[]): void {
        if (!path || path.length < 2) return;

        this._path = path;

        this._init_dist = vec2.distance(this.position as vec2, this._path[1] as vec2);

        this._path[0] = [this.position[0], this.position[1]];
        this._state = AgentStates.followPath;
        this._start_index = 0;
        this._target_index = 1;
        this._delta = 0;
    }

    nextTarget(): void {
        this._start_index = this._target_index++;
    }

    update(): void {
        switch (this._state) {
            case AgentStates.idle:
                this.anim.play(
                    this.game.animations['idle' + animDirs[this.animIndex]] as IAnimation,
                    true,
                );
                break;

            case AgentStates.followPath:
                if (
                    this._target_index >= this._path.length ||
                    this._start_index >= this._path.length ||
                    !this._path[this._target_index] ||
                    !this._path[this._start_index]
                ) {
                    this.idle();
                    return;
                }

                this._delta += (this.speed * this.game.deltaTime) / this._init_dist;
                if (this._delta >= 1) {
                    this.nextTarget();
                    this._delta = 0;

                    if (this._target_index === this._path.length) {
                        this.idle();
                        return;
                    }

                    this._init_dist = vec2.distance(
                        this.position as vec2,
                        this._path[this._target_index] as vec2,
                    );
                }

                const delta = vec2.create();
                vec2.sub(
                    delta,
                    this._path[this._target_index] as vec2,
                    this._path[this._start_index] as vec2,
                );
                const radians = Math.atan2(delta[1], delta[0]);

                this.direction = radians * (180 / Math.PI);
                this.animIndex = Math.floor(this.direction / 45);

                if (this.animIndex < 0) {
                    this.animIndex = 8 + this.animIndex;
                }

                this.anim.play(
                    this.game.animations['walk' + animDirs[this.animIndex]] as IAnimation,
                    true,
                );

                vec3.lerp(
                    this.position,
                    [...this._path[this._start_index], this.position[2]] as vec3,
                    [...this._path[this._target_index], this.position[2]] as vec3,
                    this._delta,
                );

                const px = this.position[0];
                const py = this.position[1];
                const tx = Math.floor(px);
                const ty = Math.floor(py);
                const fx = px - tx;
                const fy = py - ty;
                const sX = this.tilemap.sizeX;
                const sY = this.tilemap.sizeY;
                const hd = this.tilemap.heightData;
                const cx = Math.min(tx, sX - 1);
                const cy = Math.min(ty, sY - 1);
                const cx1 = Math.min(tx + 1, sX - 1);
                const cy1 = Math.min(ty + 1, sY - 1);
                const hTop =
                    (hd[cx + cy * sX] ?? 0) +
                    ((hd[cx1 + cy * sX] ?? 0) - (hd[cx + cy * sX] ?? 0)) * fx;
                const hBot =
                    (hd[cx + cy1 * sX] ?? 0) +
                    ((hd[cx1 + cy1 * sX] ?? 0) - (hd[cx + cy1 * sX] ?? 0)) * fx;
                const hi = hTop + (hBot - hTop) * fy;
                const targetZ = hi * this.tilemap.heightScale;
                const zSpeed = Math.min(1, this.game.deltaTime * 4);
                this.position[2] += (targetZ - this.position[2]) * zSpeed;
                break;
        }
    }
}

// Register components
registerComponent('Tilemap', Tilemap);
registerComponent('IsometricNavMesh', IsometricNavMesh);
registerComponent('IsoSprite', IsoSprite);
registerComponent('IsoAgent', IsoAgent);
