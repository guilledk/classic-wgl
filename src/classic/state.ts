import { mat4, vec3 } from 'gl-matrix';

import { Entity } from '/classic/ecs.js';
import { Camera } from '/classic/camera.js';
import { PhysicsProvider } from '/classic/collision.js';
import { getComponentConstructor } from '/classic/registry.js';
import {
    getObjectValues,
    getVideoCardInfo,
    fetchObject,
    deleteLoaderLabel,
    initShaders,
    initBuffers,
    initTextures,
    initAnimations,
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
} from './types.js';

// Type for the game state object
interface GameState extends IGameState {
    init(): void;
    getTexture(name: string): ITexture;
    download(url: string): void;
    load(url: string): Promise<void>;
    registerCall(callName: CallName, entity: IEntity, fn: CallFunction): void;
    unregisterCall(callName: CallName, entity: IEntity, fn: CallFunction): void;
    performCall(callName: CallName): void;
    getEntity(name: string): IEntity | undefined;
    getEntityOrSpawn(name: string): IEntity;
    spawnEntity(name: string): IEntity;
    destroyEntity(entity: IEntity): void;
    getGameObject(cmd: string | IComponent): IEntity | IComponent;
    resizeCanvas(): void;
    loadResources(): Promise<void>;
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
        });

        if (gl === null) {
            throw new Error('Classic requires WebGL');
        }

        this.gl = gl;

        console.log(getVideoCardInfo(this.gl));

        this.physics = new PhysicsProvider(this) as IPhysicsProvider;

        this.resizeCanvas();
        window.addEventListener('resize', this.resizeCanvas.bind(this), false);
    },

    getTexture(name: string): ITexture {
        return this.textures[name];
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

    async load(url: string) {
        const state = await fetchObject<StateData>(url);
        if (!state) {
            throw new Error(`Failed to load state from ${url}`);
        }

        for (const entityName in state.entities) {
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

    async loadResources() {
        const manifest = await fetchObject<Manifest>('/manifest.json');
        if (!manifest) {
            throw new Error('Failed to load manifest.json');
        }
        this.manifest = manifest;

        this.shaders = await initShaders(this.gl, this.manifest.shaders);

        this.buffers = initBuffers(this.gl);

        this.textures = await initTextures(this.gl, this.manifest.textures);

        this.animations = initAnimations(this.manifest.animations);
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
        gl.viewport(0, 0, this.canvas!.width, this.canvas!.height);
        gl.clearColor(0.0, 0.0, 0.0, 1.0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

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
            drawable.rawDraw();
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
            this.performCall('selectionEnd');
            (this.physics as PhysicsProvider).endSelection();
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
            this.selectionMode = 1;

            vec3.copy(this.selectionBegin, this.mousePos);
            this.performCall('selectionBegin');
            (this.physics as PhysicsProvider).beginSelection();
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

        if (this.selectionMode === 1) {
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
