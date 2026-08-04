import { mat4, vec3 } from 'gl-matrix';

import { Entity } from '/classic/ecs.js';
import { Camera } from '/classic/camera.js';
import { PhysicsProvider, Polygon } from '/classic/collision.js';
import { IsoSprite } from '/classic/isometric.js';
import { getComponentConstructor } from '/classic/registry.js';
import {
    getObjectValues,
    getVideoCardInfo,
    fetchObject,
    deleteLoaderLabel,
    initShaders,
    initBuffers,
    initTextures,
    initSdfFonts,
    initAnimations,
    estimateManifestWeight,
    slowSleep,
    MANIFEST_WEIGHT,
    SHADER_FETCH_WEIGHT,
    SHADER_COMPILE_WEIGHT,
    BUFFERS_WEIGHT,
    TEXTURE_WEIGHT,
    SDF_FONT_WEIGHT,
    ANIMATIONS_WEIGHT,
} from '/classic/utils.js';

import type {
    IGameState,
    IEntity,
    IComponent,
    IDrawable,
    IShader,
    ITexture,
    IAnimation,
    IPhysicsProvider,
    ICamera,
    Manifest,
    GameBuffers,
    CallRegistry,
    CallName,
    CallFunction,
    StateData,
    ProgressCallback,
} from './types.js';

// Type for the game state object
interface GameState extends IGameState {
    init(): void;
    getTexture(name: string): ITexture;
    getSdfFont(name: string): IGameState['sdfFonts'][string];
    download(url: string): void;
    load(url: string, onProgress?: ProgressCallback): Promise<void>;
    registerCall(callName: CallName, entity: IEntity, fn: CallFunction): void;
    unregisterCall(callName: CallName, entity: IEntity, fn: CallFunction): void;
    performCall(callName: CallName): void;
    getEntity(name: string): IEntity | undefined;
    getEntityOrSpawn(name: string): IEntity;
    spawnEntity(name: string): IEntity;
    destroyEntity(entity: IEntity): void;
    getGameObject(cmd: string | IComponent): IEntity | IComponent;
    resizeCanvas(): void;
    loadResources(onProgress?: ProgressCallback): Promise<void>;
    launch(): void;
    draw(now: number): void;
    isMouseButtonDown(button: number): boolean;
    wasMouseButtonPressed(button: number): boolean;
    wasMouseButtonReleased(button: number): boolean;
    isKeyDown(code: string): boolean;
    wasKeyPressed(code: string): boolean;
    wasKeyReleased(code: string): boolean;
    // Private event handlers
    pointerLockChangeHandler(event: Event): void;
    mouseClickHandler(event: MouseEvent): void;
    mouseWheelHandler(event: WheelEvent): void;
    keyDownHandler(event: KeyboardEvent): void;
    keyUpHandler(event: KeyboardEvent): void;
    mouseMoveHandler(event: MouseEvent): void;
    mouseDownHandler(event: MouseEvent): void;
    mouseUpHandler(event: MouseEvent): void;
    clearKeys(): void;
    clearMouseButtons(): void;
    // UI manager reference (typed separately in ui.ts module augmentation)
    ui?: IGameState['ui'];
}

const game: GameState = {
    isFirefox: navigator.userAgent.includes('Firefox'),
    projectionMatrix: mat4.create(),
    calls: {} as CallRegistry,
    nextEntityId: 0,
    nameToId: {},

    manifest: {} as Manifest,
    shaders: {} as Record<string, IShader>,
    buffers: {} as GameBuffers,
    textures: {} as Record<string, ITexture>,
    animations: {} as Record<string, IAnimation>,
    sdfFonts: {} as Record<string, IGameState['sdfFonts'][string]>,

    entities: {},

    prevTime: 0.0,
    deltaTime: 0.0,
    fps: 0,

    focused: false,

    mouseSensibility: 0.8,
    mouseAxis: vec3.fromValues(0, 0, 0),
    mousePos: vec3.fromValues(-1, -1, -10000),

    mouseWheel: 0,

    mouseDown: {},
    mousePressed: {},
    mouseReleased: {},

    keysDown: {},
    keysPressed: {},
    keysReleased: {},

    selectionBegin: vec3.fromValues(-1, -1, -1),
    selectionEnd: vec3.fromValues(-1, -1, -1),

    selectionIsoBegin: vec3.fromValues(-1, -1, -1),
    selectionIsoEnd: vec3.fromValues(-1, -1, -1),
    selectionMode: -1,
    selectionColor: [0, 1, 1, 1],

    scrollSpeed: 600,
    scrollDeadZone: 0.8,

    canvas: null,
    gl: null as unknown as WebGLRenderingContext,
    renderList: [],

    camera: new Camera([0, 0, 0], [1, 1, 1]),

    debugFootprints: false,

    physics: null,

    // UI manager reference (set by UIManager constructor)
    ui: undefined,

    init() {
        document.addEventListener(
            'pointerlockchange',
            this.pointerLockChangeHandler.bind(this),
            false,
        );

        this.canvas = document.getElementById('glCanvas') as HTMLCanvasElement;
        this.canvas.addEventListener('click', this.mouseClickHandler.bind(this), false);
        this.canvas.addEventListener('wheel', this.mouseWheelHandler.bind(this), false);

        window.addEventListener('keydown', this.keyDownHandler.bind(this), false);
        window.addEventListener('keyup', this.keyUpHandler.bind(this), false);

        this.canvas.addEventListener('mousemove', this.mouseMoveHandler.bind(this), false);
        this.canvas.addEventListener('mousedown', this.mouseDownHandler.bind(this), false);
        this.canvas.addEventListener('mouseup', this.mouseUpHandler.bind(this), false);

        const gl = this.canvas.getContext('webgl', {
            preserveDrawingBuffer: true,
            depth: true,
        });

        if (gl === null) {
            throw new Error('Classic requires WebGL');
        }

        this.gl = gl;

        gl.getExtension('OES_standard_derivatives');

        console.log(getVideoCardInfo(this.gl));

        this.physics = new PhysicsProvider(this) as IPhysicsProvider;

        this.resizeCanvas();
        window.addEventListener('resize', this.resizeCanvas.bind(this), false);
    },

    getTexture(name: string): ITexture {
        return this.textures[name];
    },

    getSdfFont(name: string): IGameState['sdfFonts'][string] {
        return this.sdfFonts[name];
    },

    download(url: string) {
        const entities: Record<string, { components: ReturnType<IComponent['dump']>[] }> = {};
        for (const entityId in this.entities) {
            const entity = this.entities[Number(entityId)];
            const components: ReturnType<IComponent['dump']>[] = [];
            for (const component of entity.components) {
                components.push(component.dump());
            }

            entities[entity.name] = {
                components: components,
            };
        }

        const minState = {
            entities: entities,
        };

        const link = document.createElement('a');
        link.download = url;

        const blob = new Blob([JSON.stringify(minState, null, 4)], {
            type: 'text/plain;charset=utf-8',
        });

        link.href = URL.createObjectURL(blob);

        link.click();

        URL.revokeObjectURL(link.href);
    },

    async load(url: string, onProgress?: ProgressCallback) {
        const report = onProgress ?? (() => {});

        report(`Fetching ${url}`, 0);
        const state = await fetchObject<StateData>(url);
        if (!state) {
            throw new Error(`Failed to load state from ${url}`);
        }
        await slowSleep();

        const entityNames = Object.keys(state.entities);
        for (let i = 0; i < entityNames.length; i++) {
            const entityName = entityNames[i];
            report(`Spawning entity: ${entityName}`, (i + 1) / entityNames.length);

            const entity = state.entities[entityName];
            const instance = this.spawnEntity(entityName);

            for (const component of entity.components) {
                const args = getObjectValues(component as Record<string, unknown>);

                // Remove the type from args
                args.splice(args.indexOf(component.type), 1);

                // Use registry instead of eval
                const ComponentClass = getComponentConstructor(component.type);
                if (!ComponentClass) {
                    throw new Error(
                        `Unknown component type: "${component.type}". ` +
                            `Make sure the component is imported and registered.`,
                    );
                }

                instance.addComponent(ComponentClass, ...args);
            }
        }
    },

    registerCall(callName: CallName, entity: IEntity, fn: CallFunction) {
        if (this.calls[callName] === undefined) {
            this.calls[callName] = {};
        }

        if (this.calls[callName][entity.id] === undefined) {
            this.calls[callName][entity.id] = {};
        }

        this.calls[callName][entity.id][fn.id!] = fn;
    },

    unregisterCall(callName: CallName, entity: IEntity, fn: CallFunction) {
        delete this.calls[callName][entity.id][fn.id!];
    },

    performCall(callName: CallName) {
        if (this.calls[callName] === undefined) {
            return;
        }

        for (const entityId in this.calls[callName]) {
            if (this.entities[Number(entityId)]?.enabled) {
                for (const fnId in this.calls[callName][Number(entityId)]) {
                    this.calls[callName][Number(entityId)][Number(fnId)]();
                }
            }
        }
    },

    getEntity(name: string): IEntity | undefined {
        return this.entities[this.nameToId[name]];
    },

    getEntityOrSpawn(name: string): IEntity {
        return this.entities[this.nameToId[name]] || this.spawnEntity(name);
    },

    spawnEntity(name: string): IEntity {
        const entity = new Entity(this, this.nextEntityId++, name);

        this.nameToId[name] = entity.id;

        this.entities[entity.id] = entity;
        return entity;
    },

    destroyEntity(entity: IEntity) {
        for (const callName of (entity as Entity)._callRegistry) {
            delete this.calls[callName][entity.id];
        }

        delete this.nameToId[entity.name];
        delete this.entities[entity.id];
    },

    /*
     * Takes a string with the formats:
     *  - {entity.name} => return entity
     *  - {entity.name}.{component type} => return component
     */
    getGameObject(cmd: string | IComponent): IEntity | IComponent {
        if (typeof cmd === 'string') {
            const words = cmd.split('.');
            if (words.length === 1) {
                return this.getEntity(cmd)!;
            } else {
                const entity = this.getEntity(words[0]);
                if (!entity) {
                    throw new Error(`Entity not found: ${words[0]}`);
                }
                const ComponentClass = getComponentConstructor(words[1]);
                if (!ComponentClass) {
                    throw new Error(`Unknown component type: ${words[1]}`);
                }
                return entity.getComponent(ComponentClass)!;
            }
        } else {
            return cmd;
        }
    },

    resizeCanvas() {
        const vw = Math.max(document.documentElement.clientWidth || 0, window.innerWidth || 0);
        const vh = Math.max(document.documentElement.clientHeight || 0, window.innerHeight || 0);

        this.canvas!.width = vw;
        this.canvas!.height = vh;

        this.projectionMatrix = mat4.create();
        mat4.ortho(
            this.projectionMatrix,
            0, // left
            vw, // right
            vh, // bottom
            0, // top
            -10000, // near
            10000, // far
        );

        this.camera.resize([vw, vh, 0]);
        this.physics!.resizeScreen();

        // notify entities that registered for canvas resize
        // (e.g. the UI system root, to refresh positions and sizes)
        this.performCall('canvasResize');
    },

    async loadResources(onProgress?: ProgressCallback) {
        const report = onProgress ?? (() => {});

        report('Fetching manifest', 0);
        const manifest = await fetchObject<Manifest>('/manifest.json');
        if (!manifest) {
            throw new Error('Failed to load manifest.json');
        }
        this.manifest = manifest;
        await slowSleep();

        const total = estimateManifestWeight(manifest);
        let acc = MANIFEST_WEIGHT;
        const reportPhase = (label: string) => report(label, acc / total);

        reportPhase('Initializing shaders');
        this.shaders = await initShaders(this.gl, this.manifest.shaders, (label, frac) =>
            report(
                label,
                (acc +
                    frac *
                        (this.manifest.shaders.length *
                            (SHADER_FETCH_WEIGHT + SHADER_COMPILE_WEIGHT))) /
                    total,
            ),
        );
        acc += this.manifest.shaders.length * (SHADER_FETCH_WEIGHT + SHADER_COMPILE_WEIGHT);

        report('Initializing buffers', acc / total);
        this.buffers = initBuffers(this.gl);
        acc += BUFFERS_WEIGHT;

        report('Loading textures', acc / total);
        this.textures = await initTextures(this.gl, this.manifest.textures, (label, frac) =>
            report(label, (acc + frac * (this.manifest.textures.length * TEXTURE_WEIGHT)) / total),
        );
        acc += this.manifest.textures.length * TEXTURE_WEIGHT;

        report('Loading SDF fonts', acc / total);
        this.sdfFonts = await initSdfFonts(this.manifest.sdfFonts || [], (label, frac) =>
            report(
                label,
                (acc + frac * ((this.manifest.sdfFonts?.length ?? 0) * SDF_FONT_WEIGHT)) / total,
            ),
        );
        acc += (this.manifest.sdfFonts?.length ?? 0) * SDF_FONT_WEIGHT;

        report('Building animations', acc / total);
        this.animations = initAnimations(this.manifest.animations);
        acc += ANIMATIONS_WEIGHT;
    },

    launch() {
        deleteLoaderLabel();
        requestAnimationFrame(this.draw.bind(this));
    },

    draw(now: number) {
        now /= 1000;
        this.deltaTime = now - this.prevTime;
        this.fps = Math.floor(1 / this.deltaTime);

        this.physics!.beginFrame();
        (this.physics as PhysicsProvider).performCalls();

        if (this.wasMouseButtonPressed(0) && !this.uiConsumedClick) {
            this.selectionMode = 1;
            vec3.copy(this.selectionBegin, this.mousePos);
            this.performCall('selectionBegin');
            (this.physics as PhysicsProvider).beginSelection();
        }

        this.performCall('update');

        this.mouseWheel =
            (Math.abs(this.mouseWheel) - 1.4 * this.deltaTime) * Math.sign(this.mouseWheel);
        this.mouseWheel = Math.min(this.mouseWheel, 1);
        this.mouseWheel = Math.max(this.mouseWheel, -1);
        if (Math.abs(this.mouseWheel) < 0.01) {
            this.mouseWheel = 0;
        }

        const gl = this.gl;

        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
        gl.clearColor(0.0, 0.0, 0.0, 1.0);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.depthFunc(gl.LEQUAL);
        gl.depthMask(true);
        gl.disable(gl.SCISSOR_TEST);

        this.renderList.length = 0;
        this.performCall('renderList');
        this.renderList.sort((a: IDrawable, b: IDrawable) => {
            const aOrder = a.order();
            const bOrder = b.order();
            if (aOrder > bOrder) {
                return -1;
            } else if (aOrder < bOrder) {
                return 1;
            } else {
                return 0;
            }
        });

        for (const drawable of this.renderList) {
            if ((drawable as any).entity && !(drawable as any).entity.enabled) continue;
            drawable.rawDraw();
        }

        if (this.debugFootprints) {
            const xVerts = new Float32Array([-16, -16, 0, 16, 16, 0, -16, 16, 0, 16, -16, 0]);
            const xBuf = this.gl.createBuffer();
            this.gl.bindBuffer(this.gl.ARRAY_BUFFER, xBuf);
            this.gl.bufferData(this.gl.ARRAY_BUFFER, xVerts, this.gl.STATIC_DRAW);

            for (const entity of Object.values(this.entities)) {
                let iso: IsoSprite | null = null;
                for (const c of entity.components) {
                    if (c instanceof IsoSprite) {
                        iso = c as IsoSprite;
                        break;
                    }
                }
                if (!iso) continue;

                this.shaders.solid.bind();
                this.gl.uniformMatrix4fv(
                    this.shaders.solid.unif.projectionMatrix,
                    false,
                    this.projectionMatrix,
                );
                this.gl.uniformMatrix4fv(
                    this.shaders.solid.unif.cameraMatrix,
                    false,
                    this.camera.matrix(),
                );

                this.gl.depthFunc(this.gl.ALWAYS);
                this.gl.depthMask(false);

                // Compute world-space footprint vertices on-the-fly with terrain height
                const hd = iso.tilemap.heightData;
                const sW = iso.tilemap.sizeX;
                const atH = (tx: number, ty: number) =>
                    hd[
                        Math.min(Math.max(tx, 0), sW - 1) +
                            Math.min(Math.max(ty, 0), iso.tilemap.sizeY - 1) * sW
                    ] ?? 0;

                const worldFootprint: [number, number, number][] = iso.footprint.map((pt) => {
                    const px = iso.position[0] + pt[0];
                    const py = iso.position[1] + pt[1];
                    const ftx = Math.floor(px);
                    const fty = Math.floor(py);
                    const fx = px - ftx;
                    const fy = py - fty;
                    const hNW = atH(ftx, fty);
                    const hNE = atH(ftx + 1, fty);
                    const hSW = atH(ftx, fty + 1);
                    const hSE = atH(ftx + 1, fty + 1);
                    const h =
                        hNW +
                        (hNE - hNW) * fx +
                        (hSW - hNW) * fy +
                        (hNW - hNE - hSW + hSE) * fx * fy;

                    const v = vec3.fromValues(px, py, 0);
                    iso.tilemap.isoToCartesian(v);
                    vec3.add(v, v, iso.tilemap.position);
                    v[1] -= h * iso.tilemap.heightScale;
                    return [v[0], v[1], 0] as [number, number, number];
                });
                const fpVerts = new Float32Array(worldFootprint.flatMap((v) => [v[0], v[1], v[2]]));
                const fpBuf = this.gl.createBuffer();
                this.gl.bindBuffer(this.gl.ARRAY_BUFFER, fpBuf);
                this.gl.bufferData(this.gl.ARRAY_BUFFER, fpVerts, this.gl.STATIC_DRAW);
                this.gl.vertexAttribPointer(
                    this.shaders.solid.attr.vertexPos,
                    3,
                    this.gl.FLOAT,
                    false,
                    0,
                    0,
                );
                this.gl.enableVertexAttribArray(this.shaders.solid.attr.vertexPos);
                this.gl.uniformMatrix4fv(this.shaders.solid.unif.modelMatrix, false, mat4.create());
                this.gl.uniform4fv(this.shaders.solid.unif.color, [0.0, 1.0, 0.5, 0.7]);
                this.gl.drawArrays(this.gl.LINE_LOOP, 0, worldFootprint.length);
                this.gl.deleteBuffer(fpBuf);

                // Anchor X with terrain height
                const anchorWorld = vec3.clone(iso.position);
                iso.tilemap.isoToCartesian(anchorWorld);
                vec3.add(anchorWorld, anchorWorld, iso.tilemap.position);

                const ax = iso.position[0];
                const ay = iso.position[1];
                const aftx = Math.floor(ax);
                const afty = Math.floor(ay);
                const afx = ax - aftx;
                const afy = ay - afty;
                const ahNW = atH(aftx, afty);
                const ahNE = atH(aftx + 1, afty);
                const ahSW = atH(aftx, afty + 1);
                const ahSE = atH(aftx + 1, afty + 1);
                const ah =
                    ahNW +
                    (ahNE - ahNW) * afx +
                    (ahSW - ahNW) * afy +
                    (ahNW - ahNE - ahSW + ahSE) * afx * afy;
                anchorWorld[1] -= ah * iso.tilemap.heightScale;

                const anchorModel = mat4.create();
                mat4.translate(anchorModel, anchorModel, anchorWorld);
                this.gl.uniformMatrix4fv(this.shaders.solid.unif.modelMatrix, false, anchorModel);
                this.gl.uniform4fv(this.shaders.solid.unif.color, [1.0, 0.0, 1.0, 0.9]);

                this.gl.bindBuffer(this.gl.ARRAY_BUFFER, xBuf);
                this.gl.vertexAttribPointer(
                    this.shaders.solid.attr.vertexPos,
                    3,
                    this.gl.FLOAT,
                    false,
                    0,
                    0,
                );
                this.gl.drawArrays(this.gl.LINE_STRIP, 0, 2);
                this.gl.drawArrays(this.gl.LINE_STRIP, 2, 2);
            }

            // ---- Compass Rose + XYZ Axes (top-left, below FPS bar) ----
            const ds = Math.max(1, Math.min(3, this.canvas!.height / 1080));
            const roseCx = 100 * ds;
            const roseCy = 65 * ds;
            const roseR = 28 * ds;
            const xyzCx = 200 * ds;
            const xyzCy = 65 * ds;
            const xyzLen = 35 * ds;

            this.shaders.solid.bind();
            this.gl.uniformMatrix4fv(
                this.shaders.solid.unif.projectionMatrix,
                false,
                this.projectionMatrix,
            );
            this.gl.uniformMatrix4fv(this.shaders.solid.unif.cameraMatrix, false, mat4.create());
            this.gl.uniformMatrix4fv(this.shaders.solid.unif.modelMatrix, false, mat4.create());

            const drawLine = (x1: number, y1: number, x2: number, y2: number, color: number[]) => {
                this.gl.uniform4fv(this.shaders.solid.unif.color, color);
                const b = this.gl.createBuffer();
                this.gl.bindBuffer(this.gl.ARRAY_BUFFER, b);
                this.gl.bufferData(
                    this.gl.ARRAY_BUFFER,
                    new Float32Array([x1, y1, 0, x2, y2, 0]),
                    this.gl.STATIC_DRAW,
                );
                this.gl.vertexAttribPointer(
                    this.shaders.solid.attr.vertexPos,
                    3,
                    this.gl.FLOAT,
                    false,
                    0,
                    0,
                );
                this.gl.enableVertexAttribArray(this.shaders.solid.attr.vertexPos);
                this.gl.drawArrays(this.gl.LINE_STRIP, 0, 2);
                this.gl.deleteBuffer(b);
            };

            this.gl.depthMask(false);
            this.gl.depthFunc(this.gl.ALWAYS);

            // 2:1 isometric grid lines through compass center
            const gridColor = [0.6, 0.6, 0.5, 0.4] as number[];
            const gx = roseR * 1.15;
            drawLine(roseCx - gx, roseCy - gx / 2, roseCx + gx, roseCy + gx / 2, gridColor);
            drawLine(roseCx - gx, roseCy + gx / 2, roseCx + gx, roseCy - gx / 2, gridColor);

            // 4 cardinal spokes
            const spokeColor = [1.0, 1.0, 0.8, 0.85] as number[];
            drawLine(roseCx, roseCy, roseCx, roseCy - roseR, spokeColor); // N = straight UP
            drawLine(roseCx, roseCy, roseCx + roseR, roseCy, spokeColor); // E = straight RIGHT
            drawLine(roseCx, roseCy, roseCx, roseCy + roseR, spokeColor); // S = straight DOWN
            drawLine(roseCx, roseCy, roseCx - roseR, roseCy, spokeColor); // W = straight LEFT

            // XYZ axes
            drawLine(xyzCx, xyzCy, xyzCx + xyzLen, xyzCy - xyzLen / 2, [1.0, 0.3, 0.3, 0.9]); // X
            drawLine(xyzCx, xyzCy, xyzCx + xyzLen, xyzCy + xyzLen / 2, [0.3, 1.0, 0.3, 0.9]); // Y
            drawLine(xyzCx, xyzCy, xyzCx, xyzCy - xyzLen, [0.4, 0.4, 1.0, 0.9]); // Z

            this.gl.deleteBuffer(xBuf);
            this.gl.depthFunc(this.gl.LEQUAL);
            this.gl.depthMask(true);
        } else {
            this.gl.drawArrays(this.gl.LINE_STRIP, 0, 0);
            this.gl.depthFunc(this.gl.LEQUAL);
            this.gl.depthMask(true);
        }

        //this.physics.debugDraw();

        this.prevTime = now;
        this.clearKeys();
        this.clearMouseButtons();
        requestAnimationFrame(this.draw.bind(this));
    },

    // EVENT HANDLERS

    pointerLockChangeHandler(_event: Event) {
        if (!this.focused && document.pointerLockElement === this.canvas) {
            this.focused = true;
        }

        if (this.focused && document.pointerLockElement === null) {
            this.focused = false;
        }
    },

    clearMouseButtons() {
        this.mousePressed = {};
        this.mouseReleased = {};
    },

    isMouseButtonDown(button: number): boolean {
        if (button in this.mouseDown) {
            return this.mouseDown[button];
        }
        return false;
    },

    wasMouseButtonPressed(button: number): boolean {
        if (button in this.mousePressed) {
            return this.mousePressed[button];
        }
        return false;
    },

    wasMouseButtonReleased(button: number): boolean {
        if (button in this.mouseReleased) {
            return this.mouseReleased[button];
        }
        return false;
    },

    mouseClickHandler(event: MouseEvent) {
        if (!this.focused) {
            const canvas = this.canvas as HTMLCanvasElement & {
                mozRequestPointerLock?: () => void;
                webkitRequestPointerLock?: () => void;
            };
            const requestPointerLock =
                canvas.requestPointerLock ||
                canvas.mozRequestPointerLock ||
                canvas.webkitRequestPointerLock;
            if (requestPointerLock) {
                requestPointerLock.call(canvas);
            }
        }
        if (this.mousePos[0] === -1) {
            this.mousePos[0] = event.pageX;
        }
        if (this.mousePos[1] === -1) {
            this.mousePos[1] = event.pageY;
        }
    },

    mouseWheelHandler(event: WheelEvent) {
        if (!this.focused) return;
        event.preventDefault();

        this.mouseWheel -= (event.deltaY * 2) / this.canvas!.height;
    },

    mouseUpHandler(event: MouseEvent) {
        if (!this.focused) return;
        this.mouseDown[event.button] = false;
        this.mouseReleased[event.button] = true;

        if (event.button === 0) {
            this.selectionMode = -1;

            vec3.copy(this.selectionEnd, this.mousePos);
            if (!this.uiConsumedClick) {
                this.performCall('selectionEnd');
                (this.physics as PhysicsProvider).endSelection();
            }
        }
    },

    mouseDownHandler(event: MouseEvent) {
        if (!this.focused) return;
        this.mouseDown[event.button] = true;
        this.mousePressed[event.button] = true;

        if (this.mousePos[0] === -1) {
            return;
        }

        if (event.button === 0) {
            this.uiConsumedClick = false;
            if (this.panelMenuOpen) this.uiConsumedClick = true;
        }
    },

    mouseMoveHandler(event: MouseEvent) {
        if (!this.focused) return;
        this.mousePos[0] += event.movementX * this.mouseSensibility;
        this.mousePos[1] += event.movementY * this.mouseSensibility;

        if (this.mousePos[0] < 0) {
            this.mousePos[0] = 0;
        }
        if (this.mousePos[0] > this.canvas!.width) {
            this.mousePos[0] = this.canvas!.width;
        }

        if (this.mousePos[1] < 0) {
            this.mousePos[1] = 0;
        }
        if (this.mousePos[1] > this.canvas!.height) {
            this.mousePos[1] = this.canvas!.height;
        }

        this.mouseAxis[0] = (this.mousePos[0] / this.canvas!.width - 0.5) * 2;
        this.mouseAxis[1] = (this.mousePos[1] / this.canvas!.height - 0.5) * 2;

        if (this.mouseAxis[0] > 1) {
            this.mouseAxis[0] = 1;
        }
        if (this.mouseAxis[1] > 1) {
            this.mouseAxis[1] = 1;
        }

        if (this.mouseAxis[0] < -1) {
            this.mouseAxis[0] = -1;
        }
        if (this.mouseAxis[1] < -1) {
            this.mouseAxis[1] = -1;
        }

        if (this.selectionMode === 1 && !this.uiConsumedClick) {
            (this.physics as PhysicsProvider).updateSelection();
        }
    },

    isKeyDown(code: string): boolean {
        if (code in this.keysDown) {
            return this.keysDown[code];
        }
        return false;
    },

    wasKeyPressed(code: string): boolean {
        if (code in this.keysPressed) {
            return this.keysPressed[code];
        }
        return false;
    },

    wasKeyReleased(code: string): boolean {
        if (code in this.keysReleased) {
            return this.keysReleased[code];
        }
        return false;
    },

    clearKeys() {
        this.keysPressed = {};
        this.keysReleased = {};
    },

    keyDownHandler(event: KeyboardEvent) {
        if (!this.focused) return;
        this.keysDown[event.code] = true;
        this.keysPressed[event.code] = true;
    },

    keyUpHandler(event: KeyboardEvent) {
        if (!this.focused) return;
        this.keysDown[event.code] = false;
        this.keysReleased[event.code] = true;
    },
} as GameState;

export default game;
