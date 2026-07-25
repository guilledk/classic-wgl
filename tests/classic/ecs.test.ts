import { describe, it, expect, vi } from 'vitest';
import { Component, Entity } from '/classic/ecs.js';
import type { IEntity, CallFunction } from '/classic/types.js';
import { createMockGame } from '../helpers/mockGame.js';

class Health extends Component {
    hp: number;
    constructor(entity: IEntity, hp: number = 100) {
        super(entity);
        this.hp = hp;
    }
}

class Position extends Component {}

describe('Entity', () => {
    it('addComponent constructs and stores the component', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');

        const health = entity.addComponent(Health, 50);

        expect(health).toBeInstanceOf(Health);
        expect(health.hp).toBe(50);
        expect(entity.components).toContain(health);
        expect(health.entity).toBe(entity);
        expect(health.game).toBe(game);
        expect(health.gl).toBe(game.gl);
    });

    it('getComponent returns a previously added component', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');
        const health = entity.addComponent(Health);

        expect(entity.getComponent(Health)).toBe(health);
    });

    it('getComponent returns null for a component type never added', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');
        entity.addComponent(Health);

        expect(entity.getComponent(Position)).toBeNull();
    });

    it('registerForCleanup + cleanup() invokes all registered callbacks', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');

        const cleanupA = vi.fn();
        const cleanupB = vi.fn();
        entity.registerForCleanup(cleanupA);
        entity.registerForCleanup(cleanupB);

        entity.cleanup();

        expect(cleanupA).toHaveBeenCalledTimes(1);
        expect(cleanupB).toHaveBeenCalledTimes(1);
    });

    it('registerCall delegates to game.registerCall and assigns an id once', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');
        const fn = vi.fn() as unknown as CallFunction;

        entity.registerCall('update', fn);
        expect(fn.id).toBe(0);
        expect(game.registerCall).toHaveBeenCalledWith('update', entity, fn);

        entity.registerCall('canvasResize', fn);
        // id should not be reassigned on subsequent registrations
        expect(fn.id).toBe(0);
        expect(entity.nextCallId).toBe(1);
    });

    it('starts enabled with an empty component list', () => {
        const game = createMockGame();
        const entity = new Entity(game, 42, 'enemy');

        expect(entity.enabled).toBe(true);
        expect(entity.id).toBe(42);
        expect(entity.name).toBe('enemy');
        expect(entity.components).toEqual([]);
    });
});

describe('Component', () => {
    it('dump() returns the constructor name as type', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');
        const health = entity.addComponent(Health);

        expect(health.dump()).toEqual({ type: 'Health' });
    });

    it('toGameObjectString() formats as entityName.ComponentName', () => {
        const game = createMockGame();
        const entity = new Entity(game, 1, 'player');
        const health = entity.addComponent(Health);

        expect(health.toGameObjectString()).toBe('player.Health');
    });
});
