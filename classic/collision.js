import "/lib/quadtree.js";
import { GJKContext } from "/lib/gjk.js";
import { mat4, vec3 } from "/lib/gl-matrix/index.js"

import { Buffer } from "/classic/utils.js";
import { Component } from "/classic/ecs.js";


class Shape {
    constructor(game, position, scale, rotation) {
        this.game = game;
        this.gl = game.gl;
        this.position = position;
        this.scale = scale;
        this.rotation = rotation;
    }

    modelMatrix() {
        let modelMatrix = mat4.create();
        mat4.translate(
            modelMatrix, modelMatrix, [this.position[0], this.position[1], 0]);
        mat4.scale(
            modelMatrix, modelMatrix, this.scale);
        mat4.rotate(
            modelMatrix, modelMatrix, this.rotation, [0, 0, 1]);

        return modelMatrix;
    }

    rectangle() {
        throw "Abstract method must be overriden";
    }

    center() {
        throw "Abstract method must be overriden";
    }

    support(dir) {
        throw "Abstract method must be overriden";
    }

    rawDebugDraw() {
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
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.cameraMatrix,
            false,
            mat4.create());
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.modelMatrix,
            false,
            this.modelMatrix());
        this.gl.uniform4fv(this.game.shaders.solid.unif.color, [1, 0, 0, 0.2]);
            
        this.gl.drawElements(
            this.gl.TRIANGLES,
            6,                  // vertex count
            this.gl.UNSIGNED_SHORT,  // type
            0);                 //start offse
    }
};


class Circle extends Shape {
    constructor(
        game,
        position,
        diameter
    ) {
        super(game, position, [diameter, diameter, 1], 0);
    }

    rectangle() {
        return {
            x: this.position[0] - (this.scale[0] / 2),
            y: this.position[1] - (this.scale[1] / 2),
            width: this.scale[0],
            height: this.scale[1]
        };
    }

    center() {
        return vec3.clone(this.position);
    }

    support(dir) {
        let d = Number.NEGATIVE_INFINITY;
        let furthest = vec3.clone(dir);
        vec3.normalize(furthest, furthest);
        vec3.transformMat4(furthest, furthest, this.modelMatrix());
        return furthest;
    }
};


class Polygon extends Shape {
    constructor(
        game,
        position,
        scale,
        rotation,
        rawVerts
    ) {
        super(game, position, scale, rotation);
        this.rawVerts = rawVerts;

        this._rawCenter = vec3.create();
        this._rawMin = vec3.create();
        this._rawMax = vec3.create();
        let i = 0;
        for (const vert of rawVerts) {
            vec3.add(this._rawCenter, this._rawCenter, vert);
            vec3.min(this._rawMin, this._rawMin, vert);
            vec3.max(this._rawMax, this._rawMax, vert);
            i++;
        }
        vec3.scale(this._rawCenter, this._rawCenter, 1 / i);

        // debug draw stuff
        // upload raw verts to gpu
        
        // flatten vert array
        this._flatVertArray = []
        for (const vert of this.rawVerts)
            this._flatVertArray.push(...vert);
       
        this._rawVertBuffer = new Buffer(
            this.gl, this.gl.ARRAY_BUFFER,
            this._flatVertArray, Float32Array, this.gl.STATIC_DRAW);
    }

    rectangle() {
        let vMin = vec3.clone(this._rawMin);
        let vMax = vec3.clone(this._rawMax);
        vec3.transformMat4(vMin, vMin, this.modelMatrix());
        vec3.transformMat4(vMax, vMax, this.modelMatrix());
        return {
            x: vMin[0],
            y: vMin[1],
            width: Math.abs(vMax[0] - vMin[0]),
            height: Math.abs(vMax[1] - vMin[1])
        };
    }

    center() {
        let center = vec3.create();
        vec3.transformMat4(center, this._rawCenter, this.modelMatrix());
        return center;
    }

    support(dir) {
        let d = Number.NEGATIVE_INFINITY;
        let furthest = null;

        const modelMat = this.modelMatrix();

        for (const rawVert of this.rawVerts) {
            let vert = vec3.clone(rawVert);
            vec3.transformMat4(vert, vert, modelMat);
            let cd = vec3.dot(dir, vert);
            if (cd > d) {
                d = cd;
                furthest = vert;
            }
        }

        return furthest;
    }

    rawDebugDraw() {

        this._rawVertBuffer.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.solid.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // perform normalization 
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.solid.attr.vertexPos);

        this.game.shaders.solid.bind();

        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.cameraMatrix,
            false,
            mat4.create());
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.modelMatrix,
            false,
            this.modelMatrix());
        this.gl.uniform4fv(this.game.shaders.solid.unif.color, [1.0, 1.0, 0.0, 1.0]);
            
        this.gl.drawArrays(
            this.gl.LINE_LOOP,
            0,
            this.rawVerts.length);

    }
};


class VirtualCollider {
    constructor(pid, shape) {
        this._pid = pid;
        this.shape = shape;
        this.position = this.shape.position;
        this.scale = this.shape.scale;
        this.updateRect();
    }

    updateRect() {
        Object.assign(
            this,
            this.shape.rectangle());
    }

    intersects(other) {
        return (
            this.x <= other.x + other.width &&
            this.x + this.width >= other.x &&
            this.y <= other.y + other.height &&
            this.y + this.height >= other.y);
    }

    rawDebugDraw() {
        this.shape.rawDebugDraw();
    }
};


class Collider extends Component  {
    constructor(entity, shape) {
        super(entity);
        this.shape = shape;
        this.position = this.shape.position;
        this.scale = this.shape.scale;
        this.updateRect();

        this.game.physics.registerCollider(this);
        entity.registerForCleanup(this.cleanup.bind(this));
    }

    updateRect() {
        Object.assign(
            this,
            this.shape.rectangle());
    }

    intersects(other) {
        return (
            this.x <= other.x + other.width &&
            this.x + this.width >= other.x &&
            this.y <= other.y + other.height &&
            this.y + this.height >= other.y);
    }

    cleanup() {
        this.game.physics.unregisterCollider(this);
    }

    rawDebugDraw() {
        this.shape.rawDebugDraw();
    }
};


class PhysicsProvider {

    constructor(game) {
        this.game = game;
        this.gl = game.gl;

        this._rawRectVerts = [
            0, 0, 0,
            1, 0, 0,
            1, 1, 0,
            0, 1, 0
        ];
        this._vertBuffer = new Buffer(
            this.gl, this.gl.ARRAY_BUFFER,
            this._rawRectVerts, Float32Array, this.gl.STATIC_DRAW);
        this.mouse = new VirtualCollider(
            0, new Circle(game, [0, 0, 0], 1));

        this.collided = {}; 
        this.colliding = {};

        this._nextId = 1;
        this._registry = {
            0: this.mouse
        };
    }

    resizeScreen() {
        this.screenCollider = {
            x: 0,
            y: 0,
            width: this.game.canvas.width,
            height: this.game.canvas.height
        };
        this.screen = new Quadtree(this.screenCollider);
    }

    gjk(a, b) {
        return (new GJKContext(a.shape, b.shape).performTest());
    }

    beginFrame() {
        this.screen.clear();
        for (let id = 1; id < this._nextId; id++) {
            const c = this._registry[id];
            if (c.intersects(this.screenCollider))
                this.screen.insert(c);
        }

        // mouse collider
        vec3.copy(this.mouse.position, this.game.mousePos);
        this.mouse.updateRect();
        this.screen.insert(this.mouse);
    }

    registerCollider(c) {
        const id = this._nextId++;
        this._registry[id] = c;
        c._pid = id;
    }

    unregisterCollider(c) {
        delete this._registry[c._pid];
    }

    performCalls() {

        // First update collided and collding dictionaries
        this.collided = {};
        Object.assign(this.collided, this.colliding);

        this.colliding = {};
        for (let id = 1; id < this._nextId; id++) {
            const c = this._registry[id];
            if (!c.entity.enabled)
                continue;

            for (const other of this.screen.retrieve(c)) {
                if (other._pid == id)
                    continue;

                if (this.gjk(c, other)) {
                    if (this.colliding[id] === undefined)
                        this.colliding[id] = {};

                    this.colliding[id][other._pid] = true;
                }
            }
        }

        if (game.wasMouseButtonPressed(0)) {
            for (const c of this.screen.retrieve(this.mouse)) {
                if (c._pid == 0) continue;
                if (c.clickHandler != null && this.gjk(this.mouse, c))
                    if (c.clickHandler()) break;
            }
        }
        
        // For each collision this frame check if it wasn't colliding last
        // frame and if so call handleEnter
        for (const id in this.colliding) {
            const c = this._registry[id];
            if (c.handleEnter != null) {
                for (const otherId in this.colliding[id]) {
                    const other = this._registry[otherId];
                    if (!(id in this.collided))
                        c.handleEnter(other);
                }
            }
        }

        // For each collision last frame check if it isn't colliding now
        // and if so call handleExit
        for (const id in this.collided) {
            const c = this._registry[id];
            if (c.handleExit != null) {
                for (const otherId in this.collided[id]) {
                    const other = this._registry[otherId];
                    if (!(id in this.colliding))
                        c.handleExit(other);
                }
            }
        }
    }

    rawDebugQuadtreeDraw(currentNode) {
        for(const node of currentNode.nodes)
            this.rawDebugQuadtreeDraw(node);

        const colors = [
            [1, 0, 0, 1],
            [0, 1, 0, 1],
            [0, 0, 1, 1]
        ];

        const bounds = currentNode.bounds;
        let modelMatrix = mat4.create();
        mat4.translate(
            modelMatrix, modelMatrix, [bounds.x, bounds.y, 0]);
        mat4.scale(
            modelMatrix, modelMatrix, [bounds.width, bounds.height, 0]);

        this._vertBuffer.bind();
        this.gl.vertexAttribPointer(
            this.game.shaders.solid.attr.vertexPos,
            3,         // num of values to pull from array per iteration
            this.gl.FLOAT,  // type
            false,     // perform normalization 
            0,         // stride
            0);        // start offset
        this.gl.enableVertexAttribArray(
            this.game.shaders.solid.attr.vertexPos);
        
        this.game.shaders.solid.bind();

        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.projectionMatrix,
            false,
            this.game.projectionMatrix);
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.cameraMatrix,
            false,
            mat4.create());
        this.gl.uniformMatrix4fv(
            this.game.shaders.solid.unif.modelMatrix,
            false,
            modelMatrix);
        this.gl.uniform4fv(this.game.shaders.solid.unif.color, colors[currentNode.level % colors.length]);
            
        this.gl.drawArrays(
            this.gl.LINE_LOOP,
            0,
            4);
    }

    debugDraw() {
        for (let id = 0; id < this._nextId; id++)
            this._registry[id].rawDebugDraw();

        this.rawDebugQuadtreeDraw(this.screen);
    }

};


export { PhysicsProvider, Collider, Circle, Polygon };

window.Collider = Collider;
