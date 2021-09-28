import { vec3, mat4 } from '/lib/gl-matrix/index.js';


export let Camera = class {
    constructor(position, scale) {
        this.position = position;
        this.scale = scale;
        this.size = vec3.create();
    }

    resize(size) {
        this.size = size;
    }

    getFix() {
        let camFixed = vec3.clone(this.position);
        camFixed[0] -= this.size[0] / 2;
        camFixed[1] -= this.size[1] / 2;
        return camFixed;
    }

    matrix() {
        var camMatrix = mat4.create();
        const pos = vec3.clone(this.position);
        vec3.negate(pos, pos);
        vec3.add(pos, pos, [this.size[0] / 2, this.size[1] / 2, 0]);
        
        mat4.translate(
            camMatrix, camMatrix, pos);
        mat4.scale(
            camMatrix, camMatrix, this.scale);
        return camMatrix;
    }
};
