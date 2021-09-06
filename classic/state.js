import { mat4, vec3 } from "/lib/gl-matrix/index.js";

import { Entity } from "/classic/ecs.js";
import { Camera } from "/classic/camera.js";
import {
    getObjectValues,
    getVideoCardInfo,
    fetchFile, fetchObject, loadImage,
    deleteLoaderLabel,
    initShaders,
    initBuffers,
    initTextures,
    initAnimations,
    loadTexture,
    cartesianToIso4
} from '/classic/utils.js';



export default {
    isFirefox: navigator.userAgent.toLowerCase().indexOf('firefox') > -1,
    projectionMatrix: mat4.create(),
    calls: {},
    nextEntityId: 0,
    nameToId: {},

    manifest: {},
    shaders: {},
    buffers: {},
    textures: {},
    animations: {},

    entities: {},

    prevTime: 0.0,
    deltaTime: 0.0,

    focused: false,    

    mouseSensibility: 0.8,
    mouseAxis: vec3.fromValues(0, 0, 0),
    mousePos: vec3.fromValues(-1, -1, -10000),
    
    mouseIsoPos: vec3.fromValues(-1, -1, 0),
    mouseWheel: 0,

    mouseDown: {},
    mousePressed: {},
    mouseReleased: {},

    keysDown: {},
    keysPressed: {},
    keysReleased: {},

    selectionBegin: vec3.fromValues(-1, -1, -1),
    selectionEnd: vec3.fromValues(-1, -1, -1),
    selectionMode: -1,
    selectionColor: [0, 1, 1, 1],

    scrollSpeed: 600,
    scrollDeadZone: .8,

    canvas: null,
    gl: null,
    renderList: [],

    camera: new Camera([0, 0, 0], [.1, .1, 1]),

    init() {

        document.addEventListener(
            "pointerlockchange",
            this.pointerLockChangeHandler.bind(this),
            false);
    
        this.canvas = document.getElementById("glCanvas");
        this.canvas.addEventListener(
            "click",
            this.mouseClickHandler.bind(this),
            false);
        this.canvas.addEventListener(
            "wheel",
            this.mouseWheelHandler.bind(this),
            false);

        this.gl = this.canvas.getContext("webgl", {
            desynchronized: true,
            preserveDrawingBuffer: true
        });

        if (this.gl === null)
            throw "Classic requires WebGL";

        console.log(getVideoCardInfo(this.gl));

        this.resizeCanvas();
        window.addEventListener("resize", this.resizeCanvas.bind(this), false);

    },

    getTexture(name) {
        return this.textures[name];
    },

    download(url) {
        const entities = {};
        for (let entityId in this.entities) {
            const entity = this.entities[entityId];
            let components = [];
            for (let component of entity.components)
                components.push(component.dump());

            entities[entity.name] = {
                components: components
            }
        }

        const minState = {
            entities: entities
        };

        let link = document.createElement('a');
        link.download = url;

        const blob = new Blob(
            [JSON.stringify(minState, null, 4)],
            {type: "text/plain;charset=utf-8"});

        link.href = URL.createObjectURL(blob);

        link.click();

        URL.revokeObjectURL(link.href);
    },

    async load(url) {
        const state = await fetchObject(url);

        for (let entityName in state.entities) {
            const entity = state.entities[entityName];
            const instance = this.spawnEntity(entityName);

            // TODO: sanitize evals
            for (let component of entity.components) {
                let args = getObjectValues(component);

                args.splice(args.indexOf(component.type), 1);
                instance.addComponent(
                    eval("window." + component.type),
                    ...args);
            }
        }
    },

    registerCall(callName, entity, fn) {
        if (this.calls[callName] === undefined)
            this.calls[callName] = {};

        if (this.calls[callName][entity.id] === undefined)
            this.calls[callName][entity.id] = {};

        this.calls[callName][entity.id][fn.id] = fn;
    },

    unregisterCall(callName, entity, fn) {
        delete this.calls[callName][entity.id][fn.id];
    },

    performCall(callName) {
        if (this.calls[callName] === undefined)
            return;

        for (const entityId in this.calls[callName])
            if (this.entities[entityId].enabled)
                for (const fnId in this.calls[callName][entityId])
                    this.calls[callName][entityId][fnId]();

    },

    getEntity(name) {
        return this.entities[this.nameToId[name]];
    },

    getEntityOrSpawn(name) {
        return this.entities[this.nameToId[name]] || this.spawnEntity(name);
    },

    spawnEntity(name) {
        var entity = new Entity(
            this, this.nextEntityId++, name);

        this.nameToId[name] = entity.id;

        this.entities[entity.id] = entity;
        return entity;
    },

    destroyEntity(entity) {
        for (const callName of entity.callRegistry)
            delete this.calls[callName][entity.id];

        delete this.nameToId[entity.name]
        delete this.entities[entity.id];
    },

    /*
     *
     * Takes a string with the formats:
     *  - {entity.name} => return entity
     *  - {entity.name}.{component type} => return component
     *
     */
    getGameObject(cmd) {
        if (typeof cmd === "string") {
            const words = cmd.split('.');
            if (words.length == 1)
                return this.getEntity(cmd);
            else
                return this.getEntity(words[0]).getComponent(window[words[1]]);
        } else
            return cmd;
    },

    resizeCanvas() {
        const vw = Math.max(
            document.documentElement.clientWidth || 0,
            window.innerWidth || 0
        )
        const vh = Math.max(
            document.documentElement.clientHeight || 0,
            window.innerHeight || 0
        )

        this.canvas.width = vw;
        this.canvas.height = vh;

        this.projectionMatrix = mat4.create();
        mat4.ortho(
            this.projectionMatrix,
            0,     // left
            vw,    // right
            vh,    // bottom
            0,     // top
            -10000,     // near
            10000);  // far

        this.camera.resize([vw, vh]);
    },

    pushEventHandlers() {
        this.focused = true;
        window.addEventListener("keydown", this.keyDownHandler.bind(this), false);
        window.addEventListener("keyup", this.keyUpHandler.bind(this), false);

        this.canvas.addEventListener("mousemove", this.mouseMoveHandler.bind(this), false);
        this.canvas.addEventListener("mousedown", this.mouseDownHandler.bind(this), false);
        this.canvas.addEventListener("mouseup", this.mouseUpHandler.bind(this), false);
    },

    popEventHandlers() {
        this.focused = false;
        this.canvas.removeEventListener("mousemove", this.mouseMoveHandler.bind(this), false);
        this.canvas.removeEventListener("mousedown", this.mouseDownHandler.bind(this), false);
        this.canvas.removeEventListener("mouseup", this.mouseUpHandler.bind(this), false);

        window.removeEventListener("keydown", this.keyDownHandler.bind(this), false);
        window.removeEventListener("keyup", this.keyUpHandler.bind(this), false);
    },

    async loadResources() {
        this.manifest = await fetchObject('/manifest.json');

        this.shaders = await initShaders(
            this.gl, this.manifest.shaders);

        this.buffers = initBuffers(
            this.gl);

        this.textures = await initTextures(
            this.gl, this.manifest.textures);

        this.animations = initAnimations(this.manifest.animations);
    },

    launch() {
        deleteLoaderLabel()
        requestAnimationFrame(this.draw.bind(this));
    },

    draw(now) {

        now /= 1000;
        this.deltaTime = now - this.prevTime;
        this.fps = Math.floor(1 / this.deltaTime);

        this.performCall("update");

        this.mouseWheel = (Math.abs(this.mouseWheel) - (1.4 * this.deltaTime)) * Math.sign(this.mouseWheel); 
        this.mouseWheel = Math.min(this.mouseWheel, 1);
        this.mouseWheel = Math.max(this.mouseWheel, -1);
        if (Math.abs(this.mouseWheel) < .01)
            this.mouseWheel = 0;

        // if (vec3.length(this.mouseAxis) > this.scrollDeadZone) {

        //     const absAxisX = Math.abs(this.mouseAxis[0]);
        //     const absAxisY = Math.abs(this.mouseAxis[1]);

        //     var scrollAxis = vec3.fromValues(
        //         Math.min((absAxisX / 100) / -Math.log10(absAxisX), 1) * Math.sign(this.mouseAxis[0]),
        //         Math.min((absAxisY / 100) / -Math.log10(absAxisY), 1) * Math.sign(this.mouseAxis[1]),
        //         0);

        //     if (scrollAxis[0] > 1)
        //         scrollAxis[0] = -1;
        //     if (scrollAxis[1] > 1)
        //         scrollAxis[1] = -1;

        //     if (scrollAxis[0] < -1)
        //         scrollAxis[0] = 1;

        //     if (scrollAxis[1] < -1)
        //         scrollAxis[1] = 1;

        //     var scrollDelta = vec3.clone(scrollAxis);
        //     vec3.scale(
        //         scrollDelta,
        //         scrollDelta,
        //         this.scrollSpeed * this.deltaTime);

        //     vec3.add(
        //         this.camera.position,
        //         this.camera.position,
        //         scrollDelta);
        // }

        const gl = this.gl;

        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, this.canvas.width, this.canvas.height);
        gl.clearColor(0.0, 0.0, 0.0, 1.0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

        this.renderList.length = 0;
        this.performCall("renderList");
        this.renderList.sort((a, b) => {
            const aOrder = a.order();
            const bOrder = b.order();
            if (aOrder > bOrder)
                return -1;
            else if (aOrder < bOrder)
                return 1;
            else
                return 0;
        });

        for (const drawable of this.renderList)
            drawable.rawDraw();

        this.prevTime = now;
        this.clearKeys();
        this.clearMouseButtons();
        requestAnimationFrame(this.draw.bind(this));
    },

    // EVENT HANDLERS

    pointerLockChangeHandler (event) {
        if(document.pointerLockElement === this.canvas && !this.focused)
            this.pushEventHandlers();
        else
            this.popEventHandlers();
    },

    clearMouseButtons() {
        this.mousePressed = {};
        this.mouseReleased = {};
    },

    isMouseButtonDown(button) {
        if (button in this.mouseDown)
            return this.mouseDown[button];

        return false;
    },

    wasMouseButtonPressed(button) {
        if (button in this.mousePressed)
            return this.mousePressed[button];

        return false;
    },
    wasMouseButtonReleased(button) {
        if (button in this.mouseReleased)
            return this.mouseReleased[button];

        return false;
    },

    mouseClickHandler(event) {
        if (!this.focused) {
            this.canvas.requestPointerLock = this.canvas.requestPointerLock ||
                this.canvas.mozRequestPointerLock ||
                this.canvas.webkitRequestPointerLock;
            this.canvas.requestPointerLock();
        }
        if (this.mousePos[0] == -1)
            this.mousePos[0] = event.pageX;
        if (this.mousePos[1] == -1)
            this.mousePos[1] = event.pageY;
    },

    mouseWheelHandler(event) {
        event.preventDefault();

        this.mouseWheel -= event.deltaY / this.canvas.height;
    },

    mouseUpHandler(event) {
        this.mouseDown[event.button] = false;
        this.mouseReleased[event.button] = true;

        if (event.button == 0) {
            this.selectionMode = -1;
        
            vec3.copy(this.selectionEnd, this.mouseIsoPos);
            this.performCall("selectionEnd");
        }
    },

    mouseDownHandler(event) {
        this.mouseDown[event.button] = true;
        this.mousePressed[event.button] = true;

        if (this.mousePos[0] == -1)
            return;

        if (event.button == 0) {
            this.selectionMode = 1;

            vec3.copy(this.selectionBegin, this.mouseIsoPos);
            this.performCall("selectionBegin");
        }
    },

    mouseMoveHandler(event) {
        let xFix = 0;
        if (this.isFirefox)
            xFix = 2;
        this.mousePos[0] += ((event.movementX + xFix) * this.mouseSensibility);
        this.mousePos[1] += (event.movementY * this.mouseSensibility);

        if (this.mousePos[0] < 0)
            this.mousePos[0] = 0;
        if (this.mousePos[0] > this.canvas.width)
            this.mousePos[0] = this.canvas.width;

        if (this.mousePos[0] == -1)
            return;

        if (event.button == 0) {
            this.selectionMode = 1;

            vec3.copy(this.selectionBegin, this.mouseIsoPos);
            this.performCall("selectionBegin");
        }
    },

    mouseMoveHandler(event) {
        let xFix = 0;
        if (this.isFirefox)
            xFix = 2;
        this.mousePos[0] += ((event.movementX + xFix) * this.mouseSensibility);
        this.mousePos[1] += (event.movementY * this.mouseSensibility);

        if (this.mousePos[0] < 0)
            this.mousePos[0] = 0;
        if (this.mousePos[0] > this.canvas.width)
            this.mousePos[0] = this.canvas.width;

        if (this.mousePos[1] < 0)
            this.mousePos[1] = 0;
        if (this.mousePos[1] > this.canvas.height)
            this.mousePos[1] = this.canvas.height;

        let mouseGlobal = vec3.clone(this.mousePos);
        vec3.add(mouseGlobal, mouseGlobal, this.camera.getFix());
        vec3.transformMat4(this.mouseIsoPos, mouseGlobal, cartesianToIso4);

        this.mouseAxis[0] = ((this.mousePos[0] / this.canvas.width) - .5) * 2;
        this.mouseAxis[1] = ((this.mousePos[1] / this.canvas.height) - .5) * 2;

        if (this.mouseAxis[0] > 1)
            this.mouseAxis[0] = 1;
        if (this.mouseAxis[1] > 1)
            this.mouseAxis[1] = 1;

        if (this.mouseAxis[0] < -1)
            this.mouseAxis[0] = -1;
        if (this.mouseAxis[1] < -1)
            this.mouseAxis[1] = -1;

    },

    isKeyDown(code) {
        if (code in this.keysDown)
            return this.keysDown[code];

        return false;
    },

    wasKeyPressed(code) {
        if (code in this.keysPressed)
            return this.keysPressed[code];

        return false;
    },
    wasKeyReleased(code) {
        if (code in this.keysReleased)
            return this.keysReleased[code];

        return false;
    },

    clearKeys() {
        this.keysPressed = {};
        this.keysReleased = {};
    },

    keyDownHandler(event) {
        this.keysDown[event.code] = true;
        this.keysPressed[event.code] = true;
    },

    keyUpHandler(event) {
        this.keysDown[event.code] = false;
        this.keysReleased[event.code] = true;
    }
    
};
