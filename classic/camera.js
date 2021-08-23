import { mat4 } from '/lib/gl-matrix/index.js';


export let Camera = class {
    constructor(position, scale) {
        this.position = position;
        this.scale = scale;
        this.size = [0, 0];
    }

    resize(size) {
        this.size = size;
    }

    matrix() {
        var camMatrix = mat4.create();
        const invPos = [
            -(this.position[0] - ((this.size[0]) / 2)),
            -(this.position[1] - ((this.size[1]) / 2)),
            -this.position[2]];
        mat4.translate(
            camMatrix, camMatrix, invPos);
        mat4.scale(
            camMatrix, camMatrix, this.scale);
        return camMatrix;
    }
};
