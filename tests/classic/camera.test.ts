import { describe, it, expect } from 'vitest';
import { mat4, vec3 } from 'gl-matrix';
import { Camera } from '/classic/camera.js';

describe('Camera', () => {
  it('resize() updates the stored size', () => {
    const camera = new Camera([0, 0, 0], [1, 1, 1]);
    camera.resize([800, 600, 1]);
    expect(camera.size).toEqual(vec3.fromValues(800, 600, 1));
  });

  it('getFix() centers the view around the scaled camera position', () => {
    const camera = new Camera([100, 50, 0], [2, 2, 1]);
    camera.resize([800, 600, 1]);

    // getFix = position * scale - size / 2
    const expected = vec3.fromValues(100 * 2 - 400, 50 * 2 - 300, -1);
    expect(camera.getFix()).toEqual(expected);
  });

  it('getFix() with no offset and unit scale is just -size/2', () => {
    const camera = new Camera([0, 0, 0], [1, 1, 1]);
    camera.resize([200, 100, 1]);
    expect(camera.getFix()).toEqual(vec3.fromValues(-100, -50, -1));
  });

  it('matrix() translates by -getFix() and scales by camera scale', () => {
    const camera = new Camera([0, 0, 0], [2, 3, 1]);
    camera.resize([0, 0, 0]);

    const expected = mat4.create();
    mat4.translate(expected, expected, [0, 0, 0]);
    mat4.scale(expected, expected, [2, 3, 1]);

    expect(camera.matrix()).toEqual(expected);
  });

  it('matrix() reflects a non-zero camera position', () => {
    const camera = new Camera([10, 20, 0], [1, 1, 1]);
    camera.resize([0, 0, 0]);

    const fix = camera.getFix();
    const negFix = vec3.clone(fix);
    vec3.negate(negFix, negFix);

    const expected = mat4.create();
    mat4.translate(expected, expected, negFix);
    mat4.scale(expected, expected, [1, 1, 1]);

    expect(camera.matrix()).toEqual(expected);
  });
});
