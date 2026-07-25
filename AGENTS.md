# AGENTS.md

Guidance for AI coding agents (and humans) working on `classic-wgl`.

## What this is

`classic.wgl` is a small, dependency-light WebGL2 game engine written in
vanilla TypeScript, plus a retained-mode UI/layout layer built on top of it.
There is no framework (no React/etc.) — the whole app is a single
`<canvas>` bootstrapped by Vite. The only runtime dependency is `gl-matrix`.

## Commands

```bash
npm i                 # install deps (Node 22; see default.nix for nix-shell)
npm run dev           # start Vite dev server (opens browser, port 5173)
npm run build         # production build to dist/
npm run preview       # preview a production build
npm run typecheck     # tsc --noEmit (strict mode) — run before finishing
npm test              # vitest run (single pass)
npm run test:watch    # vitest watch mode
npm run test:coverage # vitest run --coverage (v8, scoped allowlist)
```

CI (`.github/workflows/ci.yml`) runs `npm run typecheck` and
`npm run test:coverage` on every push to `master` and every PR. Always run
both before considering a task done. **There is no lint step in CI and no
ESLint/Prettier config in this repo** — match the existing style by hand
(see Conventions below) rather than introducing new tooling unless asked.

## Directory map

```
src/
  main.ts            entry point (sets document.title, imports demo.ts)
  version.ts          derives APP_NAME/APP_VERSION from package.json
  classic/            the engine + demo application
    ecs.ts             Entity / Component base classes
    types.ts            all shared interfaces (IGameState, IEntity, ...)
    registry.ts          component name -> constructor registry (for JSON load)
    state.ts             the singleton `game` object + main RAF loop
    camera.ts            2D camera
    collision.ts          Shape/Circle/Polygon, Collider, PhysicsProvider (GJK + Quadtree)
    transforms.ts         Transform -> Drawable -> Rectangle/Sprite/Text
    animator.ts           Animator component (sprite-sheet frame stepping)
    isometric.ts          Tilemap, IsometricNavMesh, IsoSprite, IsoAgent
    pathfinder.ts         Web Worker: A* pathfinding over the nav mesh
    prefabs.ts            initX() functions that assemble built-in game objects
    ui.ts                 UIElement/UIContainer/UIArray/UIPadding/UIManager
    uiPrefabs.ts           demo UI tree built from ui.ts primitives
    utils.ts               fetch/shader/buffer/texture/animation helpers
    demo.ts                application entry: init -> load resources -> load
                           state -> run prefabs -> initUI() -> launch()
  lib/                vendored, mostly-standalone algorithms
    gjk.ts               GJK convex collision detection
    quadtree.ts          generic spatial Quadtree<T extends Rect>
    simplex-noise.ts     2D/3D/4D simplex noise
  shaders/            GLSL sources (*.vert / *.frag), served at /shaders/*
                       via a custom Vite plugin (see vite.config.ts)
tests/                mirrors src/ 1:1 (tests/classic/*, tests/lib/*)
  helpers/mockGame.ts   createMockGL / createMockPhysics / createMockGame
public/               static assets + manifest.json (shaders/textures/
                       animations) + state.json (persisted demo entities)
```

## Architecture essentials

- **ECS without a scheduler.** `Entity` (src/classic/ecs.ts) holds a list of
  `Component`s. There is no per-frame "system" that iterates components by
  type. Instead, components call `entity.registerCall(name, boundFn)` (e.g.
  `'update'`, `'renderList'`, `'canvasResize'`) and the singleton `game`
  object (src/classic/state.ts) dispatches these via
  `registerCall`/`performCall`/`unregisterCall`. `destroyEntity` cleans up
  everything an entity registered.
- **The `game` singleton** (`state.ts`) is the "god object": input state,
  timing, WebGL handles, camera, physics provider, entity map, and the main
  `draw(now)` loop (physics -> `update` calls -> `renderList` calls, sorted
  by `order()` -> `rawDraw()`). It's also exposed as `window.game` for
  console debugging.
- **Component registry** (`registry.ts`) maps a string name to a
  constructor so `game.load('/state.json')` can deserialize entities without
  `eval`. Every concrete `Component` subclass self-registers at the bottom
  of its file via `registerComponent('Name', Class)`. **Importing a
  component module for its side effects is required** before that component
  type can be loaded from JSON — if you add a new component, register it and
  make sure its module is imported somewhere in the load path (usually via
  `demo.ts` or another already-imported module).
- **Prefab functions** (`prefabs.ts`) are the idiomatic way to build
  gameplay: plain `initXxx()` functions that call `game.spawnEntity()`,
  `entity.addComponent()`, and `entity.registerCall()`. There is no
  declarative scene format beyond the runtime `state.json` dump.
- **Resource manifest** (`public/manifest.json`) declares shaders (name +
  vertex/fragment paths + attribute/uniform names), textures, and
  animations; `game.loadResources()` / `utils.ts` (`initShaders`,
  `initTextures`, `initAnimations`) consume it.
- **Shaders** live in `src/shaders` as source but are fetched at runtime via
  `/shaders/*` URLs. A custom Vite plugin in `vite.config.ts` serves them
  from `src/shaders` in dev and emits them into `dist/shaders` on build —
  don't try to `import` a shader file directly.
- **UI layer** (`ui.ts`) is a small retained-mode layout system built by
  extending the rendering primitives directly: `UIElement extends
  Rectangle`, `UIText extends Text`, `UISprite extends Sprite`. Layout is
  triggered by `UIManager.markDirty()` / `refreshLayout()`, driven off the
  `'canvasResize'` call.
- **Isometric/pathfinding**: `isometric.ts` owns a `pathfinder.ts` Web
  Worker (spun up via `new Worker(new URL('./pathfinder.ts',
  import.meta.url), { type: 'module' })`) and talks to it with an
  id-correlated `initmap`/`updatemap`/`findpath` message protocol.
- **Feature-local type augmentation**: files extend `IGameState` in place
  via `declare module './types.js' { interface IGameState { ... } }`
  (see `ui.ts`, `prefabs.ts`) instead of editing `types.ts` directly for
  optional/feature-specific state.

## Conventions

- 2-space indentation, single quotes, semicolons everywhere.
- `PascalCase` for classes/components; interfaces representing an abstract
  contract are `I`-prefixed (`IEntity`, `IComponent`, `IGameState`,
  `IShape`, `ICamera`, ...). `camelCase` for functions/methods/variables.
  Private/internal fields are `_`-prefixed (`_callRegistry`, `_toCleanup`).
  Prefab initializers follow `initXxx()`. True constants are
  `SCREAMING_SNAKE_CASE`.
- **All import specifiers use a `.js` extension, even for `.ts` source
  files** (e.g. `import game from '/classic/state.js'`). This is
  intentional and consistent across the whole codebase — don't switch to
  `.ts` extensions.
- Path aliases (defined in both `vite.config.ts` and `tsconfig.json`):
  `/classic/*` -> `src/classic/*`, `/lib/*` -> `src/lib/*`. Convention: use
  the alias for cross-module imports, but use a relative import for a
  module's own directory `types.js` (e.g. `import type { ... } from
  './types.js'` from within `src/classic/`).
- `type`-only imports use `import type { ... }` explicitly
  (`verbatimModuleSyntax: true` in `tsconfig.json` requires this).
- Abstract methods are implemented as base-class methods that `throw new
  Error('Abstract method must be overridden')` rather than using the
  `abstract` keyword.
- JSDoc-style `/** ... */` comments are used for file headers and
  non-trivial exported functions, but not uniformly on every method —
  match the density of the file you're editing.

## Testing

- Vitest with `jsdom` environment; tests live under `tests/` and mirror the
  `src/` tree (`tests/classic/*.test.ts`, `tests/lib/*.test.ts`).
- `test:coverage` scope is intentionally limited to core engine pieces —
  see the `test.coverage.include` allowlist in `vite.config.ts` (currently
  `src/lib/**`, `ecs.ts`, `camera.ts`, `collision.ts`, `utils.ts`,
  `registry.ts`). Not every file needs 100% coverage; the demo/prefab/UI
  layer is out of scope for coverage.
- Use `tests/helpers/mockGame.ts` instead of touching real WebGL/DOM:
  - `createMockGL()` — stub `WebGLRenderingContext` with only the methods
    actually exercised.
  - `createMockPhysics()` — stub `IPhysicsProvider`.
  - `createMockGame(overrides?)` — minimal `IGameState`-shaped object,
    override/spread as needed per test.
  These are intentionally partial mocks force-cast to the real interface —
  keep that pattern rather than building full fakes.
- Typical test shape: define a tiny local `Component` subclass inline,
  construct `Entity`s with a mock game, assert on both behavior and
  internal bookkeeping. Use `toBeCloseTo(...)` for float/vector
  comparisons. `registry.test.ts`-style specs call `clearRegistry()` in
  `beforeEach` for isolation.
- Run `npm run typecheck && npm test` (or `npm run test:coverage` to match
  CI) before considering a change complete.

## Git / PR notes

- Default branch is `master` (CI triggers on push to `master` and all PRs).
- Commit messages in this repo are short, lowercase, imperative
  (`fix npx vite build`, `add Vitest test harness and CI`) — no
  conventional-commit prefixes.
