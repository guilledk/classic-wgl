import { Camera } from "/classic/camera.js";
import { mat4, vec3 } from "/lib/gl-matrix/index.js";
import {
    getVideoCardInfo,
    fetchFile, fetchObject, loadImage,
    deleteLoaderLabel,
    initShaders,
    initBuffers,
    initTextures,
    loadTexture,
    cartesianToIso4
} from '/classic/utils.js';

import { Entity } from "/classic/ecs.js";


export default {
    isFirefox: navigator.userAgent.toLowerCase().indexOf('firefox') > -1,
    projectionMatrix: mat4.create(),
    calls: {},
    nextEntityId: 0,

    manifest: {},
    shaders: {},
    buffers: {},
    textures: {},

    entities: {},

    prevTime: 0.0,
    deltaTime: 0.0,

    focused: false,    

    mouseSensibility: 0.8,
    mouseAxis: vec3.fromValues(0, 0, 0),
    mousePos: vec3.fromValues(-1, -1, 0),
    mouseIsoPos: vec3.fromValues(-1, -1, 0),
    mouseWheel: 0,

    selectionBegin: vec3.fromValues(-1, -1, -1),
    selectionMode: -1,
    selectionColor: [0, 1, 1, 1],

    scrollSpeed: 600,
    scrollDeadZone: .8,

    canvas: null,
    gl: null,

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
            for (const fnId in this.calls[callName][entityId])
                this.calls[callName][entityId][fnId]();

    },

    spawnEntity(name) {
        var entity = new Entity(
            this, this.nextEntityId++, name);

        this.entities[entity.id] = entity;
        return entity;
    },

    destroyEntity(entity) {
        for (const callName of entity.callRegistry)
            delete this.calls[callName][entity.id];

        delete this.entities[entity.id];
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
            0,     // near
            10000);  // far

        this.camera.resize([vw, vh]);
    },

    pushEventHandlers() {
        this.focused = true;
        window.addEventListener("keypress", this.keyPressHandler.bind(this), false);
        this.canvas.addEventListener("mousemove", this.mouseMoveHandler.bind(this), false);
        this.canvas.addEventListener("mousedown", this.mouseDownHandler.bind(this), false);
        this.canvas.addEventListener("mouseup", this.mouseUpHandler.bind(this), false);
    },

    popEventHandlers() {
        this.focused = false;
        this.canvas.removeEventListener("mousemove", this.mouseMoveHandler.bind(this), false);
        this.canvas.removeEventListener("mousedown", this.mouseDownHandler.bind(this), false);
        this.canvas.removeEventListener("mouseup", this.mouseUpHandler.bind(this), false);
        window.removeEventListener("keypress", this.keyPressHandler.bind(this), false);
    },

    async loadResources() {
        this.manifest = await fetchObject('/manifest.json');

        this.shaders = await initShaders(
            this.gl, this.manifest.shaders);

        this.buffers = initBuffers(
            this.gl);

        this.textures = await initTextures(
            this.gl, this.manifest.textures);
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
        else {
            this.camera.scale[0] += this.mouseWheel * this.deltaTime;
            this.camera.scale[1] += this.mouseWheel * this.deltaTime;

            vec3.max(this.camera.scale, this.camera.scale, [.01, .01, 1]);
        }

        if (vec3.length(this.mouseAxis) > this.scrollDeadZone) {

            const absAxisX = Math.abs(this.mouseAxis[0]);
            const absAxisY = Math.abs(this.mouseAxis[1]);

            var scrollAxis = vec3.fromValues(
                Math.min((absAxisX / 100) / -Math.log10(absAxisX), 1) * Math.sign(this.mouseAxis[0]),
                Math.min((absAxisY / 100) / -Math.log10(absAxisY), 1) * Math.sign(this.mouseAxis[1]),
                0);

            if (scrollAxis[0] > 1)
                scrollAxis[0] = -1;
            if (scrollAxis[1] > 1)
                scrollAxis[1] = -1;

            if (scrollAxis[0] < -1)
                scrollAxis[0] = 1;

            if (scrollAxis[1] < -1)
                scrollAxis[1] = 1;

            var scrollDelta = vec3.clone(scrollAxis);
            vec3.scale(
                scrollDelta,
                scrollDelta,
                this.scrollSpeed * this.deltaTime);

            vec3.add(
                this.camera.position,
                this.camera.position,
                scrollDelta);
        }

        const gl = this.gl;

        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, this.canvas.width, this.canvas.height);
        gl.clearColor(0.0, 0.0, 0.0, 1.0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

        this.performCall("draw"); 

        this.prevTime = now;
        requestAnimationFrame(this.draw.bind(this));
    },

    // EVENT HANDLERS

    pointerLockChangeHandler (event) {
        if(document.pointerLockElement === this.canvas && !this.focused)
            this.pushEventHandlers();
        else
            this.popEventHandlers();
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
        this.selectionMode = -1;
    },

    mouseDownHandler(event) {
        if (this.mousePos[0] == -1)
            return;

        if (event.button == 0)
            this.selectionMode = 0;

        vec3.copy(this.selectionBegin, this.mouseIsoPos);
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
        let camFixed = vec3.clone(this.camera.position);
        camFixed[0] -= this.camera.size[0] / 2;
        camFixed[1] -= this.camera.size[1] / 2;
        vec3.add(
            mouseGlobal,
            mouseGlobal,
            camFixed);
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

    keyPressHandler(event) {
        console.log(event);
        // if (event.key === ',' && camera.scale[0] > 0.02)
        //     vec3.add(camera.scale, camera.scale, [-0.01, -0.01, 0]);
        // if (event.key === '.')
        //     vec3.add(camera.scale, camera.scale, [0.01, 0.01, 0]);
    }
    
};
