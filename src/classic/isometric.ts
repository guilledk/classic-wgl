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
    }

    modelMatrix(): mat4 {
        const modelMatrix = mat4.create();
        mat4.translate(modelMatrix, modelMatrix, this.position);
        mat4.scale(modelMatrix, modelMatrix, [...this.mapSize, 1] as vec3);
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
                this.data = JSON.parse(atob(text));
                this.uploadToGPU();
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

    isoDistanceToCam(pos: vec3): number {
        const camPos = vec3.clone(game.camera.position as vec3);
        vec3.add(camPos, camPos, [0, -game.camera.size[1] / 2, 0]);
        this.cartesianToIso(camPos);
        return vec3.distance(camPos, pos);
    }

    generateNoiseMap(): void {
        for (let y = 0; y < this.sizeY; y++) {
            for (let x = 0; x < this.sizeX; x++) {
                this.data[x + this.sizeX * y] = Math.floor(getNoiseRange(x, y, 0, this.maxTile));
            }
        }
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
    }

    rawDraw(): void {
        // Verts
        this.game.buffers.quad.verts.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.isoTilemap.attr.vertexPos,
            3,
            this.gl.FLOAT,
            false,
            0,
            0,
        );
        this.gl.enableVertexAttribArray(this.game.shaders.isoTilemap.attr.vertexPos);

        // UVs
        this.game.buffers.quad.uvs.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.isoTilemap.attr.mapCoord,
            2,
            this.gl.FLOAT,
            false,
            0,
            0,
        );
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
            this.game.projectionMatrix,
        );
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.cameraMatrix,
            false,
            this.game.camera.matrix(),
        );
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.modelMatrix,
            false,
            this.modelMatrix(),
        );
        this.gl.uniformMatrix4fv(
            this.game.shaders.isoTilemap.unif.isoMatrix,
            false,
            this._isoToCartesian,
        );

        this.gl.uniform2fv(this.game.shaders.isoTilemap.unif.tileSetSize, this.tileSetSize);
        this.gl.uniform2fv(this.game.shaders.isoTilemap.unif.tilePixelSize, this.tilePixelSize);

        this.gl.uniform2fv(this.game.shaders.isoTilemap.unif.mapSize, this.mapSize);

        this.gl.uniform2fv(this.game.shaders.isoTilemap.unif.selectedTile, [
            this.mouseIsoPos[0],
            this.mouseIsoPos[1],
        ]);

        this.gl.uniform2fv(this.game.shaders.isoTilemap.unif.selectionBegin, [
            this.selectionIsoBegin[0],
            this.selectionIsoBegin[1],
        ]);

        this.gl.uniform1i(this.game.shaders.isoTilemap.unif.selectionMode, this.game.selectionMode);
        this.gl.uniform4fv(
            this.game.shaders.isoTilemap.unif.selectionColor,
            this.game.selectionColor,
        );

        this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);
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
        mat4.translate(modelMatrix, modelMatrix, cartPos);
        mat4.scale(modelMatrix, modelMatrix, this.scale);
        return modelMatrix;
    }

    order(): number {
        return this.tilemap.order() - this.tilemap.isoDistanceToCam(this.position);
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

        this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);
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
                break;
        }
    }
}

// Register components
registerComponent('Tilemap', Tilemap);
registerComponent('IsometricNavMesh', IsometricNavMesh);
registerComponent('IsoSprite', IsoSprite);
registerComponent('IsoAgent', IsoAgent);
