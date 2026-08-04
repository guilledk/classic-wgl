import { describe, it, expect } from 'vitest';
import { IsoSprite } from '/classic/isometric.js';

describe('IsoSprite.defaultFootprint', () => {
    it('returns a 4-vertex diamond [NE, SE, SW, NW]', () => {
        const fp = IsoSprite.defaultFootprint();
        expect(fp).toHaveLength(4);
        expect(fp[0]).toEqual([0.5, -0.5]);
        expect(fp[1]).toEqual([0.5, 0.5]);
        expect(fp[2]).toEqual([-0.5, 0.5]);
        expect(fp[3]).toEqual([-0.5, -0.5]);
    });
});

describe('IsoSprite static interface', () => {
    it('exposes static defaultFootprint method', () => {
        expect(typeof IsoSprite.defaultFootprint).toBe('function');
    });
});
