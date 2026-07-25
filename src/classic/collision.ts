import { Quadtree } from '/lib/quadtree.js';
import { GJKContext, type GJKShape } from '/lib/gjk.js';
import { mat4, vec3 } from 'gl-matrix';

import { Buffer } from '/classic/utils.js';
import { Component } from '/classic/ecs.js';
import { registerComponent } from '/classic/registry.js';
import type {
  IEntity,
  IGameState,
  IShape,
  ICollider,
  IVirtualCollider,
  Rect,
  ColliderHandlerName,
  ColliderHandler,
  ComponentData,
} from './types.js';

type Vec3Like = vec3 | [number, number, number] | number[];

export class Shape implements IShape, GJKShape {
  game: IGameState;
  gl: WebGLRenderingContext;
  position: vec3;
  scale: vec3;
  rotation: number;

  constructor(
    game: IGameState,
    position: Vec3Like,
    scale: Vec3Like,
    rotation: number
  ) {
    this.game = game;
    this.gl = game.gl;
    this.position = vec3.clone(position as vec3);
    this.scale = vec3.clone(scale as vec3);
    this.rotation = rotation;
  }

  modelMatrix(): mat4 {
    const modelMatrix = mat4.create();
    mat4.translate(modelMatrix, modelMatrix, [
      this.position[0],
      this.position[1],
      0,
    ]);
    mat4.scale(modelMatrix, modelMatrix, this.scale);
    mat4.rotate(modelMatrix, modelMatrix, this.rotation, [0, 0, 1]);

    return modelMatrix;
  }

  rectangle(): Rect {
    throw new Error('Abstract method must be overridden');
  }

  center(): vec3 {
    throw new Error('Abstract method must be overridden');
  }

  support(_dir: vec3): vec3 | null {
    throw new Error('Abstract method must be overridden');
  }

  rawDebugDraw(): void {
    this.game.buffers.quad.verts.bind();
    this.gl.vertexAttribPointer(
      this.game.shaders.solid.attr.vertexPos,
      3,
      this.gl.FLOAT,
      false,
      0,
      0
    );
    this.gl.enableVertexAttribArray(this.game.shaders.solid.attr.vertexPos);

    // Indices
    this.game.buffers.quad.indices.bind();

    this.game.shaders.solid.bind();

    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.projectionMatrix,
      false,
      this.game.projectionMatrix
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.cameraMatrix,
      false,
      mat4.create()
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.modelMatrix,
      false,
      this.modelMatrix()
    );
    this.gl.uniform4fv(this.game.shaders.solid.unif.color, [1, 0, 0, 0.2]);

    this.gl.drawElements(this.gl.TRIANGLES, 6, this.gl.UNSIGNED_SHORT, 0);
  }
}

export class Circle extends Shape {
  constructor(game: IGameState, position: Vec3Like, diameter: number) {
    super(game, position, [diameter, diameter, 1], 0);
  }

  rectangle(): Rect {
    return {
      x: this.position[0] - this.scale[0] / 2,
      y: this.position[1] - this.scale[1] / 2,
      width: this.scale[0],
      height: this.scale[1],
    };
  }

  center(): vec3 {
    return vec3.clone(this.position);
  }

  support(dir: vec3): vec3 {
    const furthest = vec3.clone(dir);
    vec3.normalize(furthest, furthest);
    vec3.transformMat4(furthest, furthest, this.modelMatrix());
    return furthest;
  }
}

export class Polygon extends Shape {
  rawVerts: vec3[];
  _rawCenter: vec3;
  _rawMin: vec3;
  _rawMax: vec3;
  _flatVertArray: number[];
  _rawVertBuffer: Buffer;

  constructor(
    game: IGameState,
    position: Vec3Like,
    scale: Vec3Like,
    rotation: number,
    rawVerts: Vec3Like[]
  ) {
    super(game, position, scale, rotation);
    this.rawVerts = rawVerts.map((v) => vec3.clone(v as vec3));

    this._rawCenter = vec3.create();
    this._rawMin = vec3.create();
    this._rawMax = vec3.create();

    let i = 0;
    for (const vert of this.rawVerts) {
      vec3.add(this._rawCenter, this._rawCenter, vert);
      vec3.min(this._rawMin, this._rawMin, vert);
      vec3.max(this._rawMax, this._rawMax, vert);
      i++;
    }
    vec3.scale(this._rawCenter, this._rawCenter, 1 / i);

    // debug draw stuff
    // upload raw verts to gpu

    // flatten vert array
    this._flatVertArray = [];
    for (const vert of this.rawVerts) {
      this._flatVertArray.push(...vert);
    }

    this._rawVertBuffer = new Buffer(
      this.gl,
      this.gl.ARRAY_BUFFER,
      this._flatVertArray,
      Float32Array,
      this.gl.STATIC_DRAW
    );
  }

  rectangle(): Rect {
    const vMin = vec3.clone(this._rawMin);
    const vMax = vec3.clone(this._rawMax);
    vec3.transformMat4(vMin, vMin, this.modelMatrix());
    vec3.transformMat4(vMax, vMax, this.modelMatrix());
    return {
      x: vMin[0],
      y: vMin[1],
      width: Math.abs(vMax[0] - vMin[0]),
      height: Math.abs(vMax[1] - vMin[1]),
    };
  }

  center(): vec3 {
    const center = vec3.create();
    vec3.transformMat4(center, this._rawCenter, this.modelMatrix());
    return center;
  }

  support(dir: vec3): vec3 | null {
    let d = Number.NEGATIVE_INFINITY;
    let furthest: vec3 | null = null;

    const modelMat = this.modelMatrix();

    for (const rawVert of this.rawVerts) {
      const vert = vec3.clone(rawVert);
      vec3.transformMat4(vert, vert, modelMat);
      const cd = vec3.dot(dir, vert);
      if (cd > d) {
        d = cd;
        furthest = vert;
      }
    }

    return furthest;
  }

  rawDebugDraw(): void {
    this._rawVertBuffer.bind();
    this.gl.vertexAttribPointer(
      this.game.shaders.solid.attr.vertexPos,
      3,
      this.gl.FLOAT,
      false,
      0,
      0
    );
    this.gl.enableVertexAttribArray(this.game.shaders.solid.attr.vertexPos);

    this.game.shaders.solid.bind();

    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.projectionMatrix,
      false,
      this.game.projectionMatrix
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.cameraMatrix,
      false,
      mat4.create()
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.modelMatrix,
      false,
      this.modelMatrix()
    );
    this.gl.uniform4fv(this.game.shaders.solid.unif.color, [1.0, 1.0, 0.0, 1.0]);

    this.gl.drawArrays(this.gl.LINE_LOOP, 0, this.rawVerts.length);
  }
}

export class VirtualCollider implements IVirtualCollider {
  _pid: number;
  shape: IShape;
  position: vec3;
  scale: vec3;
  x: number = 0;
  y: number = 0;
  width: number = 0;
  height: number = 0;

  constructor(pid: number, shape: IShape) {
    this._pid = pid;
    this.shape = shape;
    this.position = shape.position as vec3;
    this.scale = shape.scale as vec3;
    this.updateRect();
  }

  updateRect(): void {
    const rect = this.shape.rectangle();
    this.x = rect.x;
    this.y = rect.y;
    this.width = rect.width;
    this.height = rect.height;
  }

  intersects(other: Rect): boolean {
    return (
      this.x <= other.x + other.width &&
      this.x + this.width >= other.x &&
      this.y <= other.y + other.height &&
      this.y + this.height >= other.y
    );
  }

  rawDebugDraw(): void {
    this.shape.rawDebugDraw();
  }
}

export class Collider extends Component implements ICollider {
  shape: IShape;
  position: vec3;
  scale: vec3;
  _pid: number = 0;
  x: number = 0;
  y: number = 0;
  width: number = 0;
  height: number = 0;

  _handlerNames: ColliderHandlerName[];
  _handlers: Record<ColliderHandlerName, ColliderHandler[]>;

  constructor(entity: IEntity, shape: IShape) {
    super(entity);
    this.shape = shape;
    this.position = shape.position as vec3;
    this.scale = shape.scale as vec3;
    this.updateRect();

    this._handlerNames = ['enter', 'exit', 'click', 'selection', 'selectionTemp'];
    this._handlers = {
      enter: [],
      exit: [],
      click: [],
      selection: [],
      selectionTemp: [],
    };

    this.game.physics!.registerCollider(this);
    entity.registerForCleanup(this.cleanup.bind(this));
  }

  updateRect(): void {
    const rect = this.shape.rectangle();
    this.x = rect.x;
    this.y = rect.y;
    this.width = rect.width;
    this.height = rect.height;
  }

  addHandler(name: ColliderHandlerName, fn: ColliderHandler): void {
    console.assert(
      this._handlerNames.indexOf(name) > -1,
      'handler not found ' + name
    );
    this._handlers[name].push(fn);
  }

  callHandler(name: ColliderHandlerName, ...params: unknown[]): boolean {
    let result = false;
    for (const fn of this._handlers[name]) {
      const fnResult = fn(...params);
      if (fnResult) {
        result = true;
        break;
      }
    }
    return result;
  }

  hasHandlers(name: ColliderHandlerName): boolean {
    return this._handlers[name].length > 0;
  }

  intersects(other: Rect): boolean {
    return (
      this.x <= other.x + other.width &&
      this.x + this.width >= other.x &&
      this.y <= other.y + other.height &&
      this.y + this.height >= other.y
    );
  }

  cleanup(): void {
    this.game.physics!.unregisterCollider(this);
  }

  rawDebugDraw(): void {
    this.shape.rawDebugDraw();
  }
}

export class PhysicsProvider {
  game: IGameState;
  gl: WebGLRenderingContext;

  _rectVerts: number[][];
  _rawRectVerts: number[];
  _vertBuffer: Buffer;

  mouse: VirtualCollider;
  selection: VirtualCollider;

  collided: Record<number, Record<number, boolean>>;
  colliding: Record<number, Record<number, boolean>>;

  _autoIdBegin: number;
  _nextId: number;
  _registry: Record<number, Collider | VirtualCollider>;

  screenCollider!: Rect;
  screen!: Quadtree<Collider | VirtualCollider>;

  constructor(game: IGameState) {
    this.game = game;
    this.gl = game.gl;

    this._rectVerts = [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
    ];
    this._rawRectVerts = [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0];
    this._vertBuffer = new Buffer(
      this.gl,
      this.gl.ARRAY_BUFFER,
      this._rawRectVerts,
      Float32Array,
      this.gl.STATIC_DRAW
    );

    this.mouse = new VirtualCollider(0, new Circle(game, [0, 0, 0], 1));

    this.selection = new VirtualCollider(
      1,
      new Polygon(game, [-1, -1, 0], [1, 1, 1], 0, this._rectVerts)
    );

    this.collided = {};
    this.colliding = {};

    this._autoIdBegin = 2;
    this._nextId = this._autoIdBegin;
    this._registry = {
      0: this.mouse,
      1: this.selection,
    };
  }

  resizeScreen(): void {
    this.screenCollider = {
      x: 0,
      y: 0,
      width: this.game.canvas!.width,
      height: this.game.canvas!.height,
    };
    this.screen = new Quadtree(this.screenCollider);
  }

  gjk(a: Collider | VirtualCollider, b: Collider | VirtualCollider): boolean {
    return new GJKContext(a.shape as GJKShape, b.shape as GJKShape).performTest();
  }

  beginSelection(): void {
    vec3.set(
      this.selection.position,
      this.game.mousePos[0],
      this.game.mousePos[1],
      0
    );
    this.selection.updateRect();
  }

  updateSelection(): void {
    const min = vec3.create();
    const max = vec3.create();
    vec3.min(min, this.game.selectionBegin, this.game.mousePos);
    vec3.max(max, this.game.mousePos, this.game.selectionBegin);
    const delta = vec3.create();
    vec3.sub(delta, max, min);

    vec3.set(this.selection.position, min[0], min[1], min[2]);
    vec3.set(this.selection.scale, delta[0], delta[1], 1);
    this.selection.updateRect();
  }

  endSelection(): void {
    for (const c of this.screen.retrieve(this.selection)) {
      if (c._pid === 0 || !('entity' in c) || !c.entity.enabled) continue;
      const collider = c as Collider;
      if (collider.hasHandlers('selection') && this.gjk(this.selection, c)) {
        collider.callHandler('selection');
      }
    }

    vec3.set(this.selection.position, -1, -1, 0);
    vec3.set(this.selection.scale, 1, 1, 1);
    this.selection.updateRect();
  }

  beginFrame(): void {
    this.screen.clear();
    for (let id = this._autoIdBegin; id < this._nextId; id++) {
      const c = this._registry[id];
      if (c && c.intersects(this.screenCollider)) {
        this.screen.insert(c);
      }
    }

    // mouse collider
    vec3.copy(this.mouse.position, this.game.mousePos);
    this.mouse.updateRect();
    this.screen.insert(this.mouse);
  }

  registerCollider(c: Collider): void {
    const id = this._nextId++;
    this._registry[id] = c;
    c._pid = id;
  }

  unregisterCollider(c: Collider): void {
    delete this._registry[c._pid];
  }

  performCalls(): void {
    // First update collided and colliding dictionaries
    this.collided = {};
    Object.assign(this.collided, this.colliding);

    this.colliding = {};
    for (let id = this._autoIdBegin; id < this._nextId; id++) {
      const c = this._registry[id];
      if (!c || !('entity' in c) || !c.entity.enabled) continue;

      for (const other of this.screen.retrieve(c)) {
        if (other._pid === id) continue;

        if (this.gjk(c, other)) {
          if (this.colliding[id] === undefined) {
            this.colliding[id] = {};
          }
          this.colliding[id][other._pid] = true;
        }
      }
    }

    if (this.game.wasMouseButtonPressed(0)) {
      for (const c of this.screen.retrieve(this.mouse)) {
        if (c._pid === 0 || !('entity' in c) || !c.entity.enabled) continue;
        const collider = c as Collider;
        if (collider.hasHandlers('click') && this.gjk(this.mouse, c)) {
          if (collider.callHandler('click')) break;
        }
      }
    }

    // For each collision this frame check if it wasn't colliding last
    // frame and if so call handleEnter
    for (const id in this.colliding) {
      const c = this._registry[Number(id)];
      if (!c || !('hasHandlers' in c)) continue;
      const collider = c as Collider;
      if (collider.hasHandlers('enter')) {
        for (const otherId in this.colliding[Number(id)]) {
          if (!(id in this.collided)) {
            const other = this._registry[Number(otherId)];
            collider.callHandler('enter', other);
          }
        }
      }
    }

    // For each collision last frame check if it isn't colliding now
    // and if so call handleExit
    for (const id in this.collided) {
      const c = this._registry[Number(id)];
      if (!c || !('hasHandlers' in c)) continue;
      const collider = c as Collider;
      if (collider.hasHandlers('exit')) {
        for (const otherId in this.collided[Number(id)]) {
          if (!(id in this.colliding)) {
            const other = this._registry[Number(otherId)];
            collider.callHandler('exit', other);
          }
        }
      }
    }

    // Selection temporal calls
    for (const c of this.screen.retrieve(this.selection)) {
      if (c._pid === 0 || !('entity' in c) || !c.entity.enabled) continue;
      const collider = c as Collider;
      if (
        collider.hasHandlers('selectionTemp') &&
        this.gjk(this.selection, c)
      ) {
        collider.callHandler('selectionTemp');
      }
    }
  }

  rawDebugQuadtreeDraw(currentNode: Quadtree<Collider | VirtualCollider>): void {
    for (const node of currentNode.nodes) {
      this.rawDebugQuadtreeDraw(node);
    }

    const colors = [
      [1, 0, 0, 1],
      [0, 1, 0, 1],
      [0, 0, 1, 1],
    ];

    const bounds = currentNode.bounds;
    const modelMatrix = mat4.create();
    mat4.translate(modelMatrix, modelMatrix, [bounds.x, bounds.y, 0]);
    mat4.scale(modelMatrix, modelMatrix, [bounds.width, bounds.height, 0]);

    this._vertBuffer.bind();
    this.gl.vertexAttribPointer(
      this.game.shaders.solid.attr.vertexPos,
      3,
      this.gl.FLOAT,
      false,
      0,
      0
    );
    this.gl.enableVertexAttribArray(this.game.shaders.solid.attr.vertexPos);

    this.game.shaders.solid.bind();

    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.projectionMatrix,
      false,
      this.game.projectionMatrix
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.cameraMatrix,
      false,
      mat4.create()
    );
    this.gl.uniformMatrix4fv(
      this.game.shaders.solid.unif.modelMatrix,
      false,
      modelMatrix
    );
    this.gl.uniform4fv(
      this.game.shaders.solid.unif.color,
      colors[currentNode.level % colors.length]
    );

    this.gl.drawArrays(this.gl.LINE_LOOP, 0, 4);
  }

  debugDraw(): void {
    for (let id = this._autoIdBegin - 1; id < this._nextId; id++) {
      const c = this._registry[id];
      if (c) {
        c.rawDebugDraw();
      }
    }

    this.rawDebugQuadtreeDraw(this.screen);
  }
}

// Register component
registerComponent('Collider', Collider);
