import { describe, it, expect } from 'vitest';
import { vec3 } from 'gl-matrix';
import { Shape, GJKContext, EvolveResult } from '/lib/gjk.js';

const topLeft: [number, number, number] = [0, 0, 0];
const topRight: [number, number, number] = [1, 0, 0];
const botRight: [number, number, number] = [1, 1, 0];
const botLeft: [number, number, number] = [0, 1, 0];
const unitSquareVerts = [topLeft, topRight, botRight, botLeft];

function unitSquare(pos: [number, number, number]): Shape {
  return new Shape(pos, [1, 1, 1], unitSquareVerts);
}

function versorRad(rad: number): vec3 {
  return vec3.fromValues(Math.cos(rad), -Math.sin(rad), 0);
}

describe('Shape.support', () => {
  it('returns the vertex furthest along the given direction', () => {
    const rect = unitSquare([0, 0, 0]);
    const angle = Math.PI / 4;

    expect(rect.support(versorRad(angle))).toEqual(vec3.fromValues(1, 0, 0));
    expect(rect.support(versorRad(angle * 3))).toEqual(
      vec3.fromValues(0, 0, 0)
    );
    expect(rect.support(versorRad(angle * 5))).toEqual(
      vec3.fromValues(0, 1, 0)
    );
    expect(rect.support(versorRad(angle * 7))).toEqual(
      vec3.fromValues(1, 1, 0)
    );
  });

  it('accounts for the shape position offset', () => {
    const rect = unitSquare([5, 5, 0]);
    expect(rect.support(vec3.fromValues(1, 0, 0))).toEqual(
      vec3.fromValues(6, 5, 0)
    );
  });
});

describe('GJKContext.performTest', () => {
  it('detects collision between overlapping squares', () => {
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([0.5, 0.5, 0]);
    expect(new GJKContext(shapeA, shapeB).performTest()).toBe(true);
  });

  it('detects no collision between disjoint squares', () => {
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([10, 10, 0]);
    expect(new GJKContext(shapeA, shapeB).performTest()).toBe(false);
  });

  it('detects collision when one shape fully contains another', () => {
    const big = new Shape([0, 0, 0], [10, 10, 1], unitSquareVerts);
    const small = new Shape([4, 4, 0], [1, 1, 1], unitSquareVerts);
    expect(new GJKContext(big, small).performTest()).toBe(true);
  });

  it('treats exactly touching (edge-sharing) squares as colliding', () => {
    // shapeB starts exactly where shapeA ends (shared edge at x=1)
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([1, 0, 0]);
    expect(new GJKContext(shapeA, shapeB).performTest()).toBe(true);
  });

  it('is symmetric regardless of shape order', () => {
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([0.5, 0.5, 0]);
    expect(new GJKContext(shapeA, shapeB).performTest()).toBe(
      new GJKContext(shapeB, shapeA).performTest()
    );
  });
});

describe('GJKContext.evolveSimplex', () => {
  it('progresses from an empty simplex without throwing', () => {
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([0.5, 0.5, 0]);
    const ctx = new GJKContext(shapeA, shapeB);

    expect(ctx.verts.length).toBe(0);
    const first = ctx.evolveSimplex();
    expect([EvolveResult.StillEvolving, EvolveResult.NoIntersection]).toContain(
      first
    );
  });

  it('throws once the simplex grows past a triangle (2D-only guard)', () => {
    const shapeA = unitSquare([0, 0, 0]);
    const shapeB = unitSquare([0.5, 0.5, 0]);
    const ctx = new GJKContext(shapeA, shapeB);
    ctx.verts.push(vec3.create(), vec3.create(), vec3.create(), vec3.create());

    expect(() => ctx.evolveSimplex()).toThrow('Only 2D simplex supported');
  });
});
