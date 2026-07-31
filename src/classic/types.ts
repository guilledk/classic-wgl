/**
 * Core type definitions for classic-wgl game engine
 */

import type { mat4, vec2, vec3 } from 'gl-matrix';

// ============================================================================
// Forward declarations for circular dependencies
// ============================================================================

export interface IEntity {
    game: IGameState;
    id: number;
    name: string;
    enabled: boolean;
    components: IComponent[];
    registerCall(callName: CallName, fn: CallFunction): void;
    addComponent<T extends IComponent>(type: ComponentConstructor<T>, ...args: unknown[]): T;
    getComponent<T extends IComponent>(type: ComponentConstructor<T>): T | null;
    registerForCleanup(fn: () => void): void;
    cleanup(): void;
}

export interface IComponent {
    entity: IEntity;
    game: IGameState;
    gl: WebGLRenderingContext;
    dump(): ComponentData;
    toGameObjectString(): string;
}

// ============================================================================
// Component System Types
// ============================================================================

export interface ComponentData {
    type: string;
    [key: string]: unknown;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export interface ComponentConstructor<T extends IComponent = IComponent> {
    new (entity: IEntity, ...args: any[]): T;
}

// ============================================================================
// Call System Types
// ============================================================================

export type CallName =
    'update' | 'renderList' | 'canvasResize' | 'selectionBegin' | 'selectionEnd' | string; // Allow custom call names

export interface CallFunction {
    (): void;
    id?: number;
}

export type CallRegistry = Record<string, Record<number, Record<number, CallFunction>>>;

// ============================================================================
// Rendering Types
// ============================================================================

export interface IDrawable {
    position: vec3;
    scale: vec3;
    order(): number;
    rawDraw(): void;
}

export interface ShaderInfo {
    name: string;
    vertex: string;
    fragment: string;
    attr: string[];
    unif: string[];
}

export interface IShader {
    gl: WebGLRenderingContext;
    name: string;
    program: WebGLProgram | null;
    attr: Record<string, number>;
    unif: Record<string, WebGLUniformLocation | null>;
    bind(): void;
    unbind(): void;
}

export interface ITexture {
    gl: WebGLRenderingContext;
    name: string;
    texture: WebGLTexture | null;
    image: HTMLImageElement;
    bind(texUnit: number): void;
}

export interface IBuffer {
    gl: WebGLRenderingContext;
    buffer: WebGLBuffer | null;
    bind(): void;
    unbind(): void;
}

export interface QuadBuffers {
    verts: IBuffer;
    indices: IBuffer;
    uvs: IBuffer;
}

export interface GameBuffers {
    quad: QuadBuffers;
}

// ============================================================================
// Physics/Collision Types
// ============================================================================

export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

export interface IShape {
    game: IGameState;
    gl: WebGLRenderingContext;
    position: vec3 | number[];
    scale: vec3 | number[];
    rotation: number;
    modelMatrix(): mat4;
    rectangle(): Rect;
    center(): vec3;
    support(dir: vec3): vec3 | null;
    rawDebugDraw(): void;
}

export type ColliderHandlerName = 'enter' | 'exit' | 'click' | 'selection' | 'selectionTemp';

export type ColliderHandler = (...params: unknown[]) => boolean | void;

export interface ICollider extends IComponent, Rect {
    shape: IShape;
    position: vec3 | number[];
    scale: vec3 | number[];
    _pid: number;
    updateRect(): void;
    addHandler(name: ColliderHandlerName, fn: ColliderHandler): void;
    callHandler(name: ColliderHandlerName, ...params: unknown[]): boolean;
    hasHandlers(name: ColliderHandlerName): boolean;
    intersects(other: Rect): boolean;
    rawDebugDraw(): void;
}

export interface IPhysicsProvider {
    game: IGameState;
    gl: WebGLRenderingContext;
    mouse: IVirtualCollider;
    selection: IVirtualCollider;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    gjk: (a: any, b: any) => boolean;
    resizeScreen(): void;
    beginFrame(): void;
    registerCollider(c: ICollider): void;
    unregisterCollider(c: ICollider): void;
    performCalls(): void;
    beginSelection(): void;
    updateSelection(): void;
    endSelection(): void;
}

export interface IVirtualCollider extends Rect {
    _pid: number;
    shape: IShape;
    position: vec3 | number[];
    scale: vec3 | number[];
    updateRect(): void;
    intersects(other: Rect): boolean;
    rawDebugDraw(): void;
}

// ============================================================================
// Camera Types
// ============================================================================

export interface ICamera {
    position: vec3 | number[];
    scale: vec3 | number[];
    size: vec3;
    resize(size: vec3 | number[]): void;
    getFix(): vec3;
    matrix(): mat4;
}

// ============================================================================
// Animation Types
// ============================================================================

export interface AnimationData {
    name: string;
    src: string;
    rate: number;
    sequence: number[];
}

export interface IAnimation {
    name: string;
    src: string;
    rate: number;
    sequence: number[];
}

// ============================================================================
// Isometric Types
// ============================================================================

export interface ITilemap extends IDrawable {
    sizeX: number;
    sizeY: number;
    tileScale: vec3 | number[];
    cartesianToIsoTile(pos: vec3 | number[]): vec3;
    isoTileToCartesian(pos: vec3 | number[]): vec3;
}

export interface INavMesh {
    sendMsg(type: string, payload?: unknown): void;
}

// ============================================================================
// UI Types
// ============================================================================

export interface IUIElement {
    position: vec3;
    width: number;
    height: number;
    enabled: boolean;
    setPosition(x: number, y: number, z?: number): void;
    setSize(w: number, h: number): void;
    setEnabled(enabled: boolean): void;
    getChildren(): IUIElement[];
}

export interface IUIManager {
    game: IGameState;
    root: IUIElement | null;
    markDirty(): void;
    refreshLayout(): void;
}

// ============================================================================
// Resource Manifest Types
// ============================================================================

export interface TextureManifestEntry {
    name: string;
    src: string;
}

export interface Manifest {
    shaders: ShaderInfo[];
    textures: TextureManifestEntry[];
    animations: AnimationData[];
}

// ============================================================================
// State JSON Types (for loading entities from JSON)
// ============================================================================

export interface EntityComponentData {
    type: string;
    [key: string]: unknown;
}

export interface EntityData {
    components: EntityComponentData[];
}

export interface StateData {
    entities: Record<string, EntityData>;
}

// ============================================================================
// Game State Interface
// ============================================================================

export interface IGameState {
    // Browser detection
    isFirefox: boolean;

    // Matrices
    projectionMatrix: mat4;

    // Call system
    calls: CallRegistry;

    // Entity management
    nextEntityId: number;
    nameToId: Record<string, number>;
    entities: Record<number, IEntity>;

    // Resources
    manifest: Manifest;
    shaders: Record<string, IShader>;
    buffers: GameBuffers;
    textures: Record<string, ITexture>;
    animations: Record<string, IAnimation>;

    // Timing
    prevTime: number;
    deltaTime: number;
    fps: number;

    // Focus
    focused: boolean;

    // Mouse state
    mouseSensibility: number;
    mouseAxis: vec3;
    mousePos: vec3;
    mouseWheel: number;
    mouseDown: Record<number, boolean>;
    mousePressed: Record<number, boolean>;
    mouseReleased: Record<number, boolean>;

    // Keyboard state
    keysDown: Record<string, boolean>;
    keysPressed: Record<string, boolean>;
    keysReleased: Record<string, boolean>;

    // Selection
    selectionBegin: vec3;
    selectionEnd: vec3;
    selectionIsoBegin: vec3;
    selectionIsoEnd: vec3;
    selectionMode: number;
    selectionColor: number[];

    // Scrolling
    scrollSpeed: number;
    scrollDeadZone: number;

    // WebGL
    canvas: HTMLCanvasElement | null;
    gl: WebGLRenderingContext;
    renderList: IDrawable[];

    // Camera
    camera: ICamera;

    // Physics
    physics: IPhysicsProvider | null;

    // Methods
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

    // Input methods
    isMouseButtonDown(button: number): boolean;
    wasMouseButtonPressed(button: number): boolean;
    wasMouseButtonReleased(button: number): boolean;
    isKeyDown(code: string): boolean;
    wasKeyPressed(code: string): boolean;
    wasKeyReleased(code: string): boolean;
}

// ============================================================================
// Quadtree Types (for the lib)
// ============================================================================

export interface QuadtreeNode<T extends Rect = Rect> {
    bounds: Rect;
    level: number;
    objects: T[];
    nodes: QuadtreeNode<T>[];
    insert(obj: T): void;
    retrieve(obj: Rect): T[];
    clear(): void;
}

export interface QuadtreeConstructor {
    new <T extends Rect = Rect>(
        bounds: Rect,
        maxObjects?: number,
        maxLevels?: number,
        level?: number,
    ): QuadtreeNode<T>;
}

// ============================================================================
// SimplexNoise Types (for the lib)
// ============================================================================

export interface SimplexNoiseInstance {
    noise2D(x: number, y: number): number;
    noise3D(x: number, y: number, z: number): number;
    noise4D(x: number, y: number, z: number, w: number): number;
}

export interface SimplexNoiseConstructor {
    new (randomOrSeed?: (() => number) | string | number): SimplexNoiseInstance;
}

// ============================================================================
// Global declarations for libraries that attach to window
// ============================================================================

declare global {
    interface Window {
        Quadtree: QuadtreeConstructor;
        SimplexNoise: SimplexNoiseConstructor;
        game: IGameState;
    }

    // Make Quadtree available globally
    const Quadtree: QuadtreeConstructor;
}
