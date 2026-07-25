import { describe, it, expect } from 'vitest';
import { mat3, mat4, vec3 } from 'gl-matrix';
import {
  getObjectValues,
  cartesianToIso3,
  isoToCartesian3,
  cartesianToIso4,
  isoToCartesian4,
  getNoiseRange,
  degreeToRadian,
  radianToDegree,
} from '/classic/utils.js';

describe('getObjectValues', () => {
  it('returns the values of a plain object', () => {
    expect(getObjectValues({ a: 1, b: 'two', c: true }).sort()).toEqual(
      [1, 'two', true].sort()
    );
  });

  it('returns an empty array for an empty object', () => {
    expect(getObjectValues({})).toEqual([]);
  });
});

describe('iso/cartesian conversion matrices', () => {
  it('isoToCartesian3 is the inverse of cartesianToIso3', () => {
    const identity = mat3.create();
    const product = mat3.create();
    mat3.multiply(product, cartesianToIso3, isoToCartesian3);

    for (let i = 0; i < 9; i++) {
      expect(product[i]).toBeCloseTo(identity[i], 5);
    }
  });

  it('isoToCartesian4 is the inverse of cartesianToIso4', () => {
    const identity = mat4.create();
    const product = mat4.create();
    mat4.multiply(product, cartesianToIso4, isoToCartesian4);

    for (let i = 0; i < 16; i++) {
      expect(product[i]).toBeCloseTo(identity[i], 5);
    }
  });

  it('cartesianToIso3 round-trips a point through isoToCartesian3', () => {
    const original = vec3.fromValues(3, 7, 1);
    const iso = vec3.create();
    vec3.transformMat3(iso, original, cartesianToIso3);

    const back = vec3.create();
    vec3.transformMat3(back, iso, isoToCartesian3);

    expect(back[0]).toBeCloseTo(original[0], 5);
    expect(back[1]).toBeCloseTo(original[1], 5);
  });

  it('cartesianToIso4 round-trips a point through isoToCartesian4', () => {
    const original = vec3.fromValues(3, 7, 2);
    const iso = vec3.create();
    vec3.transformMat4(iso, original, cartesianToIso4);

    const back = vec3.create();
    vec3.transformMat4(back, iso, isoToCartesian4);

    expect(back[0]).toBeCloseTo(original[0], 5);
    expect(back[1]).toBeCloseTo(original[1], 5);
    expect(back[2]).toBeCloseTo(original[2], 5);
  });
});

describe('degreeToRadian / radianToDegree', () => {
  it('converts known degree values to radians', () => {
    expect(degreeToRadian(0)).toBe(0);
    expect(degreeToRadian(180)).toBeCloseTo(Math.PI, 10);
    expect(degreeToRadian(90)).toBeCloseTo(Math.PI / 2, 10);
    expect(degreeToRadian(360)).toBeCloseTo(Math.PI * 2, 10);
  });

  it('converts known radian values to degrees', () => {
    expect(radianToDegree(0)).toBe(0);
    expect(radianToDegree(Math.PI)).toBeCloseTo(180, 10);
    expect(radianToDegree(Math.PI / 2)).toBeCloseTo(90, 10);
  });

  it('round-trips degrees -> radians -> degrees', () => {
    for (const deg of [0, 45, 90, 180, 270, 360]) {
      expect(radianToDegree(degreeToRadian(deg))).toBeCloseTo(deg, 10);
    }
  });
});

describe('getNoiseRange', () => {
  it('stays within the requested [from, to] bounds', () => {
    const from = -5;
    const to = 5;
    for (let x = 0; x < 20; x++) {
      for (let y = 0; y < 20; y++) {
        const v = getNoiseRange(x, y, from, to);
        expect(v).toBeGreaterThanOrEqual(from);
        expect(v).toBeLessThanOrEqual(to);
      }
    }
  });
});
