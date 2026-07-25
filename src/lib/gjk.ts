/**
 * GJK (Gilbert-Johnson-Keerthi) Collision Detection Algorithm
 *
 * TypeScript conversion for classic-wgl
 */

import { vec3, mat4 } from 'gl-matrix';

function tripleProduct(out: vec3, a: vec3, b: vec3, c: vec3): void {
  const tmp = vec3.create();
  vec3.cross(tmp, a, b);
  vec3.cross(out, tmp, c);
}

/**
 * Interface for shapes that can be used with GJK
 */
export interface GJKShape {
  center(): vec3;
  support(dir: vec3): vec3 | null;
}

type Vec3Like = vec3 | [number, number, number] | number[];

/**
 * Basic polygon shape for GJK testing
 */
export class Shape implements GJKShape {
  pos: vec3;
  scale: vec3;
  rotation: number;
  rawVerts: vec3[];
  _debugColor: string;

  constructor(pos: Vec3Like, scale: Vec3Like, rawVerts: Vec3Like[]) {
    this.pos = vec3.clone(pos as vec3);
    this.scale = vec3.clone(scale as vec3);
    this.rotation = 0;
    this.rawVerts = rawVerts.map((v) => vec3.clone(v as vec3));
    this._debugColor = '#C6E6FB';
  }

  modelMatrix(): mat4 {
    const modelMatrix = mat4.create();
    mat4.translate(modelMatrix, modelMatrix, this.pos);
    mat4.scale(modelMatrix, modelMatrix, this.scale);
    mat4.rotate(modelMatrix, modelMatrix, this.rotation, [0, 0, 1]);
    return modelMatrix;
  }

  vertices(): vec3[] {
    const verts: vec3[] = [];
    const model = this.modelMatrix();
    for (let i = 0; i < this.rawVerts.length; i++) {
      const tmp = vec3.clone(this.rawVerts[i]);
      vec3.transformMat4(tmp, tmp, model);
      verts.push(tmp);
    }
    return verts;
  }

  center(): vec3 {
    const center = vec3.create();
    const verts = this.vertices();
    for (const vert of verts) {
      vec3.add(center, center, vert);
    }
    vec3.scale(center, center, 1 / verts.length);
    return center;
  }

  support(dir: vec3): vec3 | null {
    let d = Number.NEGATIVE_INFINITY;
    let furthest: vec3 | null = null;

    for (const vert of this.vertices()) {
      const cd = vec3.dot(dir, vert);
      if (cd > d) {
        d = cd;
        furthest = vert;
      }
    }

    return furthest;
  }

  // Debug drawing for canvas 2D context
  draw(ctx: CanvasRenderingContext2D): void {
    const verts = this.vertices();
    ctx.beginPath();
    ctx.fillStyle = this._debugColor;
    ctx.moveTo(verts[0][0], verts[0][1]);
    for (let i = 1; i < verts.length; i++) {
      ctx.lineTo(verts[i][0], verts[i][1]);
    }
    ctx.closePath();
    ctx.fill();
  }
}

export const EvolveResult = {
  NoIntersection: 0,
  Intersection: 1,
  StillEvolving: 2,
} as const;

export type EvolveResultType = (typeof EvolveResult)[keyof typeof EvolveResult];

/**
 * GJK Algorithm Context
 */
export class GJKContext {
  shapeA: GJKShape;
  shapeB: GJKShape;
  direction: vec3;
  verts: vec3[];

  constructor(shapeA: GJKShape, shapeB: GJKShape) {
    this.shapeA = shapeA;
    this.shapeB = shapeB;
    this.direction = vec3.create();
    this.verts = [];
  }

  addSupport(dir: vec3): boolean {
    const nDir = vec3.create();
    vec3.negate(nDir, dir);

    const supA = this.shapeA.support(dir);
    const supB = this.shapeB.support(nDir);

    if (!supA || !supB) {
      return false;
    }

    const tmp = vec3.create();
    vec3.sub(tmp, supA, supB);

    this.verts.push(tmp);

    return vec3.dot(dir, tmp) >= 0;
  }

  evolveSimplex(): EvolveResultType {
    let a: vec3, b: vec3, c: vec3;

    switch (this.verts.length) {
      case 0:
        vec3.sub(this.direction, this.shapeA.center(), this.shapeB.center());
        break;

      case 1:
        // flip direction
        vec3.negate(this.direction, this.direction);
        break;

      case 2:
        b = this.verts[1];
        c = this.verts[0];

        const cb = vec3.create();
        const c0 = vec3.create();

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

        const a0 = vec3.create();
        const ab = vec3.create();
        const ac = vec3.create();

        vec3.negate(a0, a); // a to origin
        vec3.sub(ab, b, a); // a to b
        vec3.sub(ac, c, a); // a to c

        const abPerp = vec3.create();
        const acPerp = vec3.create();

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
          this.verts.splice(1, 1);
          this.direction = acPerp;
        } else {
          return EvolveResult.Intersection;
        }
        break;

      default:
        throw new Error('Only 2D simplex supported');
    }

    if (this.addSupport(this.direction)) {
      return EvolveResult.StillEvolving;
    } else {
      return EvolveResult.NoIntersection;
    }
  }

  performTest(): boolean {
    let res: EvolveResultType = EvolveResult.StillEvolving;
    const maxIter = 1000;
    let i = 0;

    while (res === EvolveResult.StillEvolving && i++ < maxIter) {
      res = this.evolveSimplex();
    }

    if (i === maxIter) {
      throw new Error('GJK: Max iteration reached');
    }

    return res === EvolveResult.Intersection;
  }
}
