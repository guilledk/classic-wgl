import { describe, it, expect } from 'vitest';
import { SimplexNoise } from '/lib/simplex-noise.js';

describe('SimplexNoise', () => {
  it('is deterministic for a given numeric seed', () => {
    const a = new SimplexNoise(42);
    const b = new SimplexNoise(42);

    for (let i = 0; i < 20; i++) {
      const x = i * 0.37;
      const y = i * 0.71;
      expect(a.noise2D(x, y)).toBe(b.noise2D(x, y));
    }
  });

  it('is deterministic for a given string seed', () => {
    const a = new SimplexNoise('classic-wgl');
    const b = new SimplexNoise('classic-wgl');
    expect(a.noise3D(1, 2, 3)).toBe(b.noise3D(1, 2, 3));
  });

  it('produces different sequences for different seeds', () => {
    const a = new SimplexNoise(1);
    const b = new SimplexNoise(2);

    // Compare a handful of samples rather than a single point, since two
    // different seeds can coincidentally agree at any one coordinate.
    const samples = [0.1, 0.5, 1.3, 2.7, 4.2].map(
      (v) => [a.noise2D(v, v * 0.5), b.noise2D(v, v * 0.5)] as const
    );
    expect(samples.some(([av, bv]) => av !== bv)).toBe(true);
  });

  it('keeps noise2D output within [-1, 1]', () => {
    const noise = new SimplexNoise(7);
    for (let x = -5; x <= 5; x += 0.5) {
      for (let y = -5; y <= 5; y += 0.5) {
        const v = noise.noise2D(x, y);
        expect(v).toBeGreaterThanOrEqual(-1);
        expect(v).toBeLessThanOrEqual(1);
      }
    }
  });

  it('keeps noise3D output within [-1, 1]', () => {
    const noise = new SimplexNoise(7);
    for (let x = -3; x <= 3; x += 1) {
      for (let y = -3; y <= 3; y += 1) {
        for (let z = -3; z <= 3; z += 1) {
          const v = noise.noise3D(x, y, z);
          expect(v).toBeGreaterThanOrEqual(-1);
          expect(v).toBeLessThanOrEqual(1);
        }
      }
    }
  });

  it('keeps noise4D output within [-1, 1]', () => {
    const noise = new SimplexNoise(7);
    for (let x = -2; x <= 2; x += 1) {
      for (let y = -2; y <= 2; y += 1) {
        for (let z = -2; z <= 2; z += 1) {
          for (let w = -2; w <= 2; w += 1) {
            const v = noise.noise4D(x, y, z, w);
            expect(v).toBeGreaterThanOrEqual(-1);
            expect(v).toBeLessThanOrEqual(1);
          }
        }
      }
    }
  });
});
