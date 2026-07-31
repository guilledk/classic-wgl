import { vec3, mat4 } from 'gl-matrix';
import type { ICamera } from './types.js';

type Vec3Like = vec3 | [number, number, number] | number[];

export class Camera implements ICamera {
    position: vec3;
    scale: vec3;
    size: vec3;

    constructor(position: Vec3Like, scale: Vec3Like) {
        this.position = vec3.clone(position as vec3);
        this.scale = vec3.clone(scale as vec3);
        this.size = vec3.create();
    }

    resize(size: Vec3Like): void {
        this.size = vec3.clone(size as vec3);
    }

    getFix(): vec3 {
        const camFixed = vec3.clone(this.position);
        const size = vec3.clone(this.size);

        vec3.mul(camFixed, camFixed, this.scale);
        vec3.div(size, size, [2, 2, 1]);
        vec3.sub(camFixed, camFixed, size);

        return camFixed;
    }

    matrix(): mat4 {
        const pos = this.getFix();
        vec3.negate(pos, pos);
        const camMatrix = mat4.create();
        mat4.translate(camMatrix, camMatrix, pos);
        mat4.scale(camMatrix, camMatrix, this.scale);
        return camMatrix;
    }
}
