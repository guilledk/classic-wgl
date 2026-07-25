import type {
  IEntity,
  IComponent,
  IGameState,
  ComponentConstructor,
  CallName,
  CallFunction,
  ComponentData,
} from './types.js';

export class Component implements IComponent {
  entity: IEntity;
  game: IGameState;
  gl: WebGLRenderingContext;

  constructor(entity: IEntity) {
    this.entity = entity;
    this.game = entity.game;
    this.gl = this.game.gl;
  }

  dump(): ComponentData {
    return { type: this.constructor.name };
  }

  toGameObjectString(): string {
    return this.entity.name + '.' + this.constructor.name;
  }
}

export class Entity implements IEntity {
  game: IGameState;
  id: number;
  name: string;
  enabled: boolean;
  nextCallId: number;
  components: IComponent[];
  _callRegistry: Set<string>;
  _toCleanup: Array<() => void>;

  constructor(game: IGameState, id: number, name: string) {
    this.game = game;
    this.id = id;
    this.name = name;

    this.enabled = true;
    this.nextCallId = 0;

    this.components = [];
    this._callRegistry = new Set();
    this._toCleanup = [];
  }

  registerCall(callName: CallName, fn: CallFunction): void {
    if (fn.id === undefined) {
      fn.id = this.nextCallId++;
    }

    this.game.registerCall(callName, this, fn);
    this._callRegistry.add(callName);
  }

  addComponent<T extends IComponent>(
    type: ComponentConstructor<T>,
    ...args: unknown[]
  ): T {
    const component = new type(this, ...args);
    this.components.push(component);
    return component;
  }

  getComponent<T extends IComponent>(type: ComponentConstructor<T>): T | null {
    for (const component of this.components) {
      if (component.constructor === type) {
        return component as T;
      }
    }
    return null;
  }

  registerForCleanup(fn: () => void): void {
    this._toCleanup.push(fn);
  }

  cleanup(): void {
    for (const fn of this._toCleanup) {
      fn();
    }
  }
}
