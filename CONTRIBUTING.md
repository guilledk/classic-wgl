# Contributing to classic-wgl

Thanks for taking a look at `classic-wgl`. This is a small, hobby-scale
WebGL2 engine — the process is intentionally lightweight.

## Getting set up

```bash
git clone <repo-url>
cd classic.wgl
nix-shell   # optional, provides Node 22 (see default.nix)
npm i
npm run dev # http://localhost:5173
```

See `AGENTS.md` for a full architecture/conventions overview before making
non-trivial changes — it covers the ECS, the call-registry dispatch
pattern, the component registry, and the code style used throughout.

## Before opening a PR

Always run both of the following (they are also what CI runs):

```bash
npm run typecheck
npm run test:coverage
```

`npm test` / `npm run test:watch` are faster options while iterating on a
single change.

There is no linter or formatter configured in this repo — match the
existing style by hand (2-space indentation, single quotes, semicolons,
`.js` extensions on all import specifiers, `/classic/*` and `/lib/*` path
aliases). See the Conventions section of `AGENTS.md` for details.

## Branching / commits

- Base all work on `master` (the default branch); CI runs on pushes to
  `master` and on every PR.
- Keep commit messages short, lowercase, and imperative (e.g. `fix npx
  vite build`, `add Vitest test harness and CI`) — this repo does not use
  Conventional Commits prefixes.
- Prefer small, focused commits over one large commit per feature.

## Adding a new Component

If you add a new `Component` subclass:

1. Register it with `registerComponent('Name', Class)` at the bottom of
   its defining file (see the bottom of `src/classic/transforms.ts` or
   `src/classic/animator.ts` for examples).
2. Make sure the module is imported somewhere in the load path (usually
   transitively via `src/classic/demo.ts`) — components are only
   deserializable from `public/state.json` once their module has run its
   side-effecting registration.
3. Add or extend a test under `tests/classic/` if the component has
   non-trivial logic, using the helpers in `tests/helpers/mockGame.ts`
   instead of touching real WebGL/DOM.

## Reporting issues

Open a GitHub issue with steps to reproduce, expected vs. actual
behavior, and, if relevant, browser/OS info (this is a WebGL2 app so GPU
driver quirks matter).
