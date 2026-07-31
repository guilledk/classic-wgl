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
        const camFixed = vec3.clone(this.position);
        const size = vec3.clone(this.size);

        vec3.mul(camFixed, camFixed, this.scale);

        vec3.div(size, size, [2, 2, 1]);

        vec3.sub(camFixed, camFixed, size);

        return camFixed;
    }

    matrix() {
        const pos = this.getFix();
        vec3.negate(pos, pos);
        var camMatrix = mat4.create();
        mat4.translate(
            camMatrix, camMatrix, pos);
        mat4.scale(
            camMatrix, camMatrix, this.scale);
        return camMatrix;
    }
};
