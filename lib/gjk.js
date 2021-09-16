import { vec3, mat4 } from "./gl-matrix/index.js";


function tripleProduct(out, a, b, c) {
    let tmp = vec3.create();
    vec3.cross(tmp, a, b);
    vec3.cross(out, tmp, c);
}


class Shape {
    constructor(pos, scale, rawVerts) {
        this.pos = pos;
        this.scale = scale;
        this.rotation = 0;
        this.rawVerts = rawVerts;

        this._debugColor = "#C6E6FB";
    }

    modelMatrix() {
        let modelMatrix = mat4.create();
        mat4.translate(
            modelMatrix, modelMatrix, this.pos);
        mat4.scale(
            modelMatrix, modelMatrix, this.scale);
        mat4.rotate(
            modelMatrix, modelMatrix, this.rotation, [0, 0, 1]);
        return modelMatrix;
    }

    vertices() {
        let verts = [];
        for(let i = 0; i < this.rawVerts.length; i++) {
            let tmp = vec3.clone(this.rawVerts[i]);
            vec3.transformMat4(tmp, tmp, this.modelMatrix());
            verts.push(tmp);
        }
        return verts;
    }

    center() {
        let center = vec3.create();
        let i = 0;
        for (const vert of this.vertices()) {
            vec3.add(center, center, vert);
            i++;
        }
        vec3.scale(center, center, 1 / i);
        return center;
    }

    support(dir) {
        let d = Number.NEGATIVE_INFINITY;
        let furthest = null;

        for (const vert of this.vertices()) {
            let cd = vec3.dot(dir, vert);
            if (cd > d) {
                d = cd;
                furthest = vert;
            }
        }

        return furthest;
    }

    // debug
    
    draw(ctx) {
        const verts = this.vertices();
        ctx.beginPath();
        ctx.fillStyle = this._debugColor;
        ctx.moveTo(...verts[0]);
        for (let i = 1; i < verts.length; i++)
            ctx.lineTo(...verts[i]);

        ctx.closePath();
        ctx.fill();
    }
};


export const EvolveResult = {
    NoIntersection: 0,
    Intersection: 1,
    StillEvolving: 2
};


class GJKContext {
    constructor(shapeA, shapeB) {
        this.shapeA = shapeA;
        this.shapeB = shapeB;
        this.direction = vec3.create();

        this.verts = [];
    }

    addSupport(dir) {
        let nDir = vec3.create();
        vec3.negate(nDir, dir);

        let supA = this.shapeA.support(dir);
        let supB = this.shapeB.support(nDir);

        let tmp = vec3.create();
        vec3.sub(tmp, supA, supB);

        this.verts.push(tmp);

        return vec3.dot(dir, tmp) >= 0;
    }

    evolveSimplex() {
        let a, b, c;
        switch(this.verts.length) {
            case 0:
                vec3.sub(this.direction, this.shapeA.center(), this.shapeB.center());
                break;

            case 1:
                // filp direction
                vec3.negate(this.direction, this.direction);
                break;

            case 2:
                b = this.verts[1];
                c = this.verts[0];

                let cb = vec3.create();
                let c0 = vec3.create();

                // line cb is the line formed by the first two vertices
                vec3.sub(cb, b, c);
                // line c0 is the line from the first vertex to the origin
                vec3.negate(c0, c);

                // use the triple-cross-product to calculate a direction perpendicular
                // to line cb in the direction of the origin
                tripleProduct(this.direction, cb, c0, cb);

                break;

            case 3:
                // calculate if the simplex contains the origin
                a = this.verts[2];
                b = this.verts[1];
                c = this.verts[0];

                let a0 = vec3.create();
                let ab = vec3.create();
                let ac = vec3.create();

                vec3.negate(a0, a);  // a to origin
                vec3.sub(ab, b, a);  // a to b
                vec3.sub(ac, c, a);  // a to c

                let abPerp = vec3.create();
                let acPerp = vec3.create();

                tripleProduct(abPerp, ac, ab, ab);
                tripleProduct(acPerp, ab, ac, ac);

                if (vec3.dot(abPerp, a0) > 0) {
                    // the origin is outside line ab
                    // get rid of c and add a new support in the direction of abPerp
                    this.verts.shift();
                    this.direction = abPerp;
                } else if (vec3.dot(acPerp, a0) > 0) {
                    // the origin is outside line ac
                    // get rid of b and add a new support in the direction of acPerp
                    this.verts.splice(
                        1,  // index b
                        1);  // one item
                    this.direction = acPerp;
                } else
                    return EvolveResult.Intersection;
                break;

            default:
                throw "Only 2D simplex supported";
        }

        // vec3.normalize(this.direction, this.direction);
        if (this.addSupport(this.direction))
            return EvolveResult.StillEvolving;
        else
            return EvolveResult.NoIntersection;
    }

    performTest() {
        let res = EvolveResult.StillEvolving;
        const maxIter = 1000;
        let i = 0;
        while (res == EvolveResult.StillEvolving && i++ < maxIter)
            res = this.evolveSimplex();

        if (i == maxIter)
            throw "Max iteration";

        return res == EvolveResult.Intersection;
    }
};


export { Shape, GJKContext };
