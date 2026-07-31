import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  registerComponent,
  getComponentConstructor,
  hasComponent,
  createComponent,
  getRegisteredComponents,
  clearRegistry,
} from '/classic/registry.js';
import { Component, Entity } from '/classic/ecs.js';
import { createMockGame } from '../helpers/mockGame.js';

class Dummy extends Component {}
class Other extends Component {}

describe('component registry', () => {
  beforeEach(() => {
    clearRegistry();
  });

  it('registers and looks up a component constructor by name', () => {
    registerComponent('Dummy', Dummy);
    expect(getComponentConstructor('Dummy')).toBe(Dummy);
  });

  it('hasComponent reflects registration state', () => {
    expect(hasComponent('Dummy')).toBe(false);
    registerComponent('Dummy', Dummy);
    expect(hasComponent('Dummy')).toBe(true);
  });

  it('getComponentConstructor returns undefined for unregistered names', () => {
    expect(getComponentConstructor('Nope')).toBeUndefined();
  });

  it('createComponent instantiates a registered component by name', () => {
    registerComponent('Dummy', Dummy);
    const game = createMockGame();
    const entity = new Entity(game, 1, 'e');

    const instance = createComponent('Dummy', entity);
    expect(instance).toBeInstanceOf(Dummy);
    expect((instance as Component).entity).toBe(entity);
  });

  it('createComponent throws for an unregistered component name', () => {
    const game = createMockGame();
    const entity = new Entity(game, 1, 'e');

    expect(() => createComponent('Nope', entity)).toThrow(
      /Unknown component type: "Nope"/
    );
  });

  it('warns but overwrites when registering the same name twice', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    registerComponent('Dummy', Dummy);
    registerComponent('Dummy', Other);

    expect(warnSpy).toHaveBeenCalled();
    expect(getComponentConstructor('Dummy')).toBe(Other);
    warnSpy.mockRestore();
  });

  it('getRegisteredComponents lists all registered names', () => {
    registerComponent('Dummy', Dummy);
    registerComponent('Other', Other);
    expect(getRegisteredComponents().sort()).toEqual(['Dummy', 'Other']);
  });

  it('clearRegistry removes all registrations', () => {
    registerComponent('Dummy', Dummy);
    clearRegistry();
    expect(getRegisteredComponents()).toEqual([]);
    expect(hasComponent('Dummy')).toBe(false);
  });
});
