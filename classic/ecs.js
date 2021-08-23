export let Component = class {
    constructor(entity) {
        this.entity = entity;
        this.game = entity.game;
        this.gl = this.game.gl;
    }
};

export let Entity = class {
    constructor(game, id, name) {
        this.game = game;
        this.id = id;
        this.name = name;

        this.nextCallId = 0;

        this.components = new Array();
        this.callRegistry = new Set();
    }

    registerCall(callName, fn) {
        if (fn.id === undefined)
            fn.id = this.nextCallId++;

        this.game.registerCall(callName, this, fn);
        this.callRegistry.add(callName);
    }

    addComponent(type, ...args) {
        var component = new type(this, ...args);
        this.components.push(component);
        return component;
    }
};
