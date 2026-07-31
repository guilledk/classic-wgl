import { describe, it, expect, vi } from 'vitest';
import { vec3 } from 'gl-matrix';
import { Circle, Polygon, VirtualCollider, Collider, PhysicsProvider } from '/classic/collision.js';
import { Entity } from '/classic/ecs.js';
import { createMockGame } from '../helpers/mockGame.js';

const unitSquareVerts = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
];

describe('Circle', () => {
    it('rectangle() is centered on position with side length = diameter', () => {
        const game = createMockGame();
        const circle = new Circle(game, [10, 20, 0], 4);

        expect(circle.rectangle()).toEqual({
            x: 8,
            y: 18,
            width: 4,
            height: 4,
        });
    });

    it('center() returns the circle position', () => {
        const game = createMockGame();
        const circle = new Circle(game, [10, 20, 0], 4);
        expect(circle.center()).toEqual(vec3.fromValues(10, 20, 0));
    });

    it('support() returns a point at scale distance from center along the given direction', () => {
        // Note: support() applies the full `scale` (the diameter argument) to the
        // normalized direction rather than half of it, so the returned point sits
        // at `diameter` units from the center, not `diameter / 2`.
        const game = createMockGame();
        const circle = new Circle(game, [0, 0, 0], 2);
        const p = circle.support(vec3.fromValues(1, 0, 0));
        expect(p[0]).toBeCloseTo(2, 5);
        expect(p[1]).toBeCloseTo(0, 5);
    });
});

describe('Polygon', () => {
    it('rectangle() computes the axis-aligned bounding box', () => {
        const game = createMockGame();
        const poly = new Polygon(game, [0, 0, 0], [10, 10, 1], 0, unitSquareVerts);

        const rect = poly.rectangle();
        expect(rect.x).toBeCloseTo(0, 5);
        expect(rect.y).toBeCloseTo(0, 5);
        expect(rect.width).toBeCloseTo(10, 5);
        expect(rect.height).toBeCloseTo(10, 5);
    });

    it('center() returns the transformed centroid', () => {
        const game = createMockGame();
        const poly = new Polygon(game, [5, 5, 0], [1, 1, 1], 0, unitSquareVerts);

        const center = poly.center();
        expect(center[0]).toBeCloseTo(5.5, 5);
        expect(center[1]).toBeCloseTo(5.5, 5);
    });

    it('support() returns the furthest transformed vertex along a direction', () => {
        const game = createMockGame();
        const poly = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);

        const p = poly.support(vec3.fromValues(1, 1, 0));
        expect(p).toEqual(vec3.fromValues(1, 1, 0));
    });
});

describe('VirtualCollider', () => {
    it('intersects() detects overlap between two rects', () => {
        const game = createMockGame();
        const a = new VirtualCollider(0, new Circle(game, [0, 0, 0], 2));
        const b = { x: 0.5, y: 0.5, width: 2, height: 2 };
        expect(a.intersects(b)).toBe(true);
    });

    it('intersects() returns false for disjoint rects', () => {
        const game = createMockGame();
        const a = new VirtualCollider(0, new Circle(game, [0, 0, 0], 2));
        const b = { x: 100, y: 100, width: 2, height: 2 };
        expect(a.intersects(b)).toBe(false);
    });

    it('intersects() is true for exactly touching edges', () => {
        const game = createMockGame();
        const a = new VirtualCollider(0, new Circle(game, [0, 0, 0], 2)); // rect: x:-1..1, y:-1..1
        const b = { x: 1, y: -1, width: 2, height: 2 }; // starts exactly where a ends
        expect(a.intersects(b)).toBe(true);
    });
});

describe('Collider', () => {
    it('registers itself with game.physics on construction', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'wall');
        const shape = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);

        const collider = entity.addComponent(Collider, shape);

        expect(game.physics!.registerCollider).toHaveBeenCalledWith(collider);
    });

    it('unregisters itself from game.physics on cleanup', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'wall');
        const shape = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const collider = entity.addComponent(Collider, shape);

        entity.cleanup();

        expect(game.physics!.unregisterCollider).toHaveBeenCalledWith(collider);
    });

    it('addHandler + callHandler invokes registered handlers by name', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'button');
        const shape = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const collider = entity.addComponent(Collider, shape);

        const handler = vi.fn(() => true);
        collider.addHandler('click', handler);

        expect(collider.hasHandlers('click')).toBe(true);
        expect(collider.hasHandlers('enter')).toBe(false);

        const result = collider.callHandler('click', 'arg1');
        expect(result).toBe(true);
        expect(handler).toHaveBeenCalledWith('arg1');
    });

    it('callHandler stops at the first handler that returns truthy', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'button');
        const shape = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const collider = entity.addComponent(Collider, shape);

        const first = vi.fn(() => true);
        const second = vi.fn(() => true);
        collider.addHandler('click', first);
        collider.addHandler('click', second);

        collider.callHandler('click');

        expect(first).toHaveBeenCalled();
        expect(second).not.toHaveBeenCalled();
    });
});

describe('PhysicsProvider', () => {
    it('registerCollider assigns increasing ids starting after reserved ids', () => {
        const game = createMockGame();
        const physics = new PhysicsProvider(game);

        const entity = new Entity(game, 1, 'a');
        const shape = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const collider = new Collider(entity, shape);
        // Collider's own constructor calls game.physics.registerCollider (the mock),
        // so register it directly against our real PhysicsProvider instance too.
        physics.registerCollider(collider);

        expect(collider._pid).toBeGreaterThanOrEqual(2);
    });

    it('gjk() detects collision between two overlapping polygon colliders', () => {
        const game = createMockGame();
        const physics = new PhysicsProvider(game);

        const entityA = new Entity(game, 1, 'a');
        const shapeA = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const colliderA = new Collider(entityA, shapeA);

        const entityB = new Entity(game, 2, 'b');
        const shapeB = new Polygon(game, [0.5, 0.5, 0], [1, 1, 1], 0, unitSquareVerts);
        const colliderB = new Collider(entityB, shapeB);

        expect(physics.gjk(colliderA, colliderB)).toBe(true);
    });

    it('gjk() returns false for disjoint polygon colliders', () => {
        const game = createMockGame();
        const physics = new PhysicsProvider(game);

        const entityA = new Entity(game, 1, 'a');
        const shapeA = new Polygon(game, [0, 0, 0], [1, 1, 1], 0, unitSquareVerts);
        const colliderA = new Collider(entityA, shapeA);

        const entityB = new Entity(game, 2, 'b');
        const shapeB = new Polygon(game, [50, 50, 0], [1, 1, 1], 0, unitSquareVerts);
        const colliderB = new Collider(entityB, shapeB);

        expect(physics.gjk(colliderA, colliderB)).toBe(false);
    });

    it('resizeScreen() builds a quadtree covering the canvas bounds', () => {
        const game = createMockGame({
            canvas: { width: 1024, height: 768 } as unknown as HTMLCanvasElement,
        });
        const physics = new PhysicsProvider(game);

        physics.resizeScreen();

        expect(physics.screenCollider).toEqual({
            x: 0,
            y: 0,
            width: 1024,
            height: 768,
        });
    });

    it('beginFrame() inserts in-bounds colliders into the broad-phase screen quadtree', () => {
        const game = createMockGame();
        const physics = new PhysicsProvider(game);
        physics.resizeScreen();

        const entity = new Entity(game, 1, 'a');
        const shape = new Polygon(game, [10, 10, 0], [1, 1, 1], 0, unitSquareVerts);
        const collider = new Collider(entity, shape);
        physics.registerCollider(collider);

        physics.beginFrame();

        const found = physics.screen.retrieve(physics.screenCollider);
        expect(found).toContain(collider);
    });
});
