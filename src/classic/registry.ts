/**
 * Component Registry
 *
 * Provides a type-safe alternative to eval("window." + componentType)
 * for dynamic component instantiation from JSON state files.
 */

import type { IComponent, IEntity, ComponentConstructor } from './types.js';

/**
 * Registry mapping component type names to their constructors
 */
const componentRegistry = new Map<string, ComponentConstructor>();

/**
 * Register a component class with the registry
 * @param name - The string name used in state.json (e.g., "Transform", "Sprite")
 * @param ctor - The component constructor class
 */
export function registerComponent<T extends IComponent>(
  name: string,
  ctor: ComponentConstructor<T>
): void {
  if (componentRegistry.has(name)) {
    console.warn(`Component "${name}" is already registered. Overwriting.`);
  }
  componentRegistry.set(name, ctor as ComponentConstructor);
}

/**
 * Get a component constructor by name
 * @param name - The string name of the component
 * @returns The component constructor or undefined if not found
 */
export function getComponentConstructor(
  name: string
): ComponentConstructor | undefined {
  return componentRegistry.get(name);
}

/**
 * Check if a component is registered
 * @param name - The string name of the component
 */
export function hasComponent(name: string): boolean {
  return componentRegistry.has(name);
}

/**
 * Create a component instance by name
 * @param name - The string name of the component
 * @param entity - The entity to attach the component to
 * @param args - Additional constructor arguments
 * @returns The created component
 * @throws Error if the component type is not registered
 */
export function createComponent<T extends IComponent>(
  name: string,
  entity: IEntity,
  ...args: unknown[]
): T {
  const ctor = componentRegistry.get(name);
  if (!ctor) {
    throw new Error(
      `Unknown component type: "${name}". ` +
        `Make sure the component is imported and registered before loading state.`
    );
  }
  return new ctor(entity, ...args) as T;
}

/**
 * Get all registered component names (useful for debugging)
 */
export function getRegisteredComponents(): string[] {
  return Array.from(componentRegistry.keys());
}

/**
 * Clear all registered components (useful for testing)
 */
export function clearRegistry(): void {
  componentRegistry.clear();
}
