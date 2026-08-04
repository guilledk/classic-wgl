---
name: classic-collision
description: >
    Collision detection and interaction dispatch for classic-wgl.
    Covers `PhysicsProvider` lifecycle, `Collider`/`VirtualCollider`,
    shapes (Circle/Polygon), GJK spatial queries, Quadtree spatial
    partition, enter/exit events, click dispatch with `clickPriority`
    sorting, `consumesClick` prescan, selection rectangle, and UI
    collider integration via `addColliderToElem`.  Use when debugging
    button overlap clicks, hover not firing, selection not working,
    `enter`/`exit` transitions, or collider enabled/disabled state.
    Trigger phrases: "enter handler", "exit handler", "clickPriority",
    "consumesClick", "performCalls", "Quadtree", "GJK", "beginFrame",
    "beginSelection", "colliding", "collided", "VirtualCollider",
    "collider enabled", "collider PID", "addColliderToElem",
    "_syncColliders".
compatibility: All colliders are 2D shapes in screen space.  GJKContext
    from `lib/gjk.js`, Quadtree from `lib/quadtree.js`.
metadata:
    author: classic-wgl
    version: '0.1'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

## Scope

This skill covers the collision system: shapes, collider lifecycle,
handler dispatch (enter/exit/click/selection), quadtree spatial
partitioning, GJK intersection queries, and how the `PhysicsProvider`
orchestrates the per-frame collision loop.

---

## 1. ARCHITECTURE OVERVIEW

`PhysicsProvider` is the central hub, stored as `game.physics`.  It
owns:

- **Quadtree** (`this.screen`) — spatial index rebuilt every
  `beginFrame()`
- **Collider registry** (`_registry`) — `Collider | VirtualCollider`
  keyed by monotonically-increasing `_pid`
- **Collision state** — `collided` (last frame) and `colliding` (this
  frame), `{ [pid]: { [otherPid]: true } }` dictionaries
- **Virtual colliders** — mouse (PID 0, Circle radius 1) and selection
  (PID 1, Polygon rectangle)

### Class hierarchy

```
Shape (abstract)
  ├─ Circle                   diameter-based, support() normalizes direction
  └─ Polygon                  convex vertex hull, GJK-compatible

Collider extends Component    entity-bound, registered in _registry, has handlers
VirtualCollider               bare-minimum Rect interface, no entity, no handlers
```

Every `Collider` is attached to an `Entity` and registered with
`this.game.physics!.registerCollider(this)` during construction.
`VirtualCollider` covers system colliders (mouse, selection) that
don't belong to an entity.

---

## 2. COLLIDER LIFECYCLE

### Registration

```typescript
registerCollider(c: Collider): void {
    const id = this._nextId++;      // monotonically increasing, never reused
    this._registry[id] = c;
    c._pid = id;
}
```

`_autoIdBegin = 2` (fixed), reserving PID 0 for mouse and PID 1 for
selection.  The first user-created `Collider` gets `_pid = 2`.

### Unregistration

```typescript
unregisterCollider(c: Collider): void {
    delete this._registry[c._pid];
}
```

**`_nextId` never decrements.** Unregistration deletes the registry
slot but does not reuse the PID.  Long-running sessions accumulate
IDs — no practical limit at 2⁵³ entries.

### Cleanup

`Collider` constructor calls `entity.registerForCleanup(this.cleanup)`.
When the entity is destroyed, `cleanup()` unregisters the collider.

---

## 3. HANDLER SYSTEM

`Collider` supports five handler names:

| Name | When called |
|------|-------------|
| `'enter'` | First frame a GJK intersection with `other` begins |
| `'exit'` | First frame a GJK intersection with `other` ends |
| `'click'` | Mouse button 0 pressed while intersecting mouse |
| `'selection'` | User drag-selection rectangle intersects collider |
| `'selectionTemp'` | Per-frame while selection rectangle is active and intersects |

```typescript
collider.addHandler('click', (other?) => { ... return true; });
collider.callHandler('click', arg);   // returns true if any handler returned truthy
collider.hasHandlers('enter');        // boolean
```

`callHandler` iterates all registered handlers for the name and stops
at the first one that returns a truthy value (**"consumed"**).  This
is how click dispatch implements mutual exclusion.

---

## 4. FRAME LIFECYCLE

The main `draw()` loop calls `beginFrame()` then `performCalls()` with
no user-code interleaving — enabled/disabled state cannot change
between quadtree insertion and collision detection within a single frame.

### `beginFrame()`

```typescript
beginFrame(): void {
    this.screen.clear();
    for (let id = this._autoIdBegin; id < this._nextId; id++) {
        const c = this._registry[id];
        if (c && c.intersects(this.screenCollider)) {   // ← NO enabled check
            this.screen.insert(c);
        }
    }
    vec3.copy(this.mouse.position, this.game.mousePos);
    this.mouse.updateRect();
    this.screen.insert(this.mouse);
}
```

**Critical rule:** all colliders (including disabled ones) are inserted
into the quadtree.  The only guard is `intersects(screen)`.  This means
disabled colliders can appear as `other` in `retrieve()` calls made by
*enabled* colliders during `performCalls()`.

### `performCalls()` — execution order

```
1. collided ← copy of previous frame's colliding
2. colliding ← fresh GJK computation (skips disabled in outer loop)
3. Click prescan  (consumesClick → uiConsumedClick flag)
4. Click dispatch (sorted by clickPriority desc, _pid asc)
5. enter dispatch (per-pair new collisions)
6. exit dispatch  (per-pair ended collisions)
7. selectionTemp dispatch
```

---

## 5. ENTER/EXIT EVENTS

Enter/exit use per-pair collision tracking between frames.  The data
structure is:

```
collided  = { [pid]: { [otherPid]: true, ... }, ... }    // last frame
colliding = { [pid]: { [otherPid]: true, ... }, ... }    // this frame (only enabled colliders as keys)
```

### Per-pair enter check

```typescript
if (!this.collided[Number(id)]?.[Number(otherId)]) {
    collider.callHandler('enter', other);
}
```

Checks whether this specific pair was colliding **last frame**.  If
not → new collision → `enter` fires.  The `?.` handles the case where
`collided[id]` is `undefined` (collider had no collisions last frame).

### Per-pair exit check

```typescript
if (!this.colliding[Number(id)]?.[Number(otherId)]) {
    collider.callHandler('exit', other);
}
```

Checks whether this specific pair is STILL colliding **this frame**.
If not → collision ended → `exit` fires.

### Key invariant

`colliding` is only keyed by **enabled** colliders (outer loop
skips disabled).  `collided` propagates from last frame's `colliding`,
so exiting collided entries are ONLY for colliders that were enabled
last frame.  This means:

- Disabled colliders never fire `enter`
- A collider disabled this frame correctly fires `exit` for any
  ongoing collisions (it WAS in collided from last frame)

### Caveat: disabled as "other"

Since `beginFrame()` inserts disabled colliders into the quadtree, an
ENABLED collider can GJK-hit a DISABLED one.  The enabled collider's
`enter` receives the disabled collider as the `other` argument.  UI
hover handlers should check `other.entity?.enabled` if they do
anything with the `other` reference.

---

## 6. CLICK DISPATCH

Two-phase system running only when `wasMouseButtonPressed(0)`:

### Phase 1: consumesClick prescan

```typescript
for (const c of this.screen.retrieve(this.mouse)) {
    if (collider.consumesClick && this.gjk(this.mouse, c)) {
        this.game.uiConsumedClick = true;
        break;
    }
}
```

Sets `game.uiConsumedClick = true` if ANY intersecting collider has
`consumesClick = true`.  This flag gates `beginSelection()` and the
tilemap click path — one frame BEFORE the handler dispatch loop runs.

### Phase 2: Sorted handler dispatch

All GJK-intersecting colliders with `'click'` handlers are collected,
sorted by `clickPriority` **descending** (tiebroken by `_pid`
ascending), then dispatched.  First handler returning truthy breaks
the loop ("consumes" the click).

```typescript
clickCandidates.sort((a, b) => b.clickPriority - a.clickPriority || a._pid - b._pid);
for (const collider of clickCandidates) {
    if (collider.callHandler('click')) break;
}
```

**Default `clickPriority = 0`.**  When two colliders overlap visually
(e.g. popup menu over toggle button), give the top element a higher
priority:

```typescript
col.clickPriority = 1;  // dispatches before parent toggle (priority 0)
```

---

## 7. SELECTION

The selection system uses `VirtualCollider` PID 1 as a rectangle
shape:

- `beginSelection()` — anchors a point at `game.mousePos`
- `updateSelection()` — grows the rectangle toward current mouse
- `endSelection()` — fires `'selection'` on all overlapped colliders,
  then resets rectangle to `(-1, -1)`

`updateSelection()` is guarded by `!this.game.uiConsumedClick` —
the `consumesClick` prescan suppresses rectangle growth during UI
clicks.

---

## 8. ENABLED/DISABLED BEHAVIOR

| Operation | Skips disabled? | Notes |
|-----------|:---:|-------|
| `beginFrame()` quadtree insert | **No** | All colliders inserted; disabled ones visible to `retrieve()` |
| `performCalls()` outer loop (colliding keys) | **Yes** | `!c.entity.enabled` → skipped; disabled never become `colliding` keys |
| `performCalls()` as `other` target | **No** | Enabled collider can GJK-hit a disabled collider via quadtree retrieval |
| enter dispatch | n/a | Only keys in `colliding` (enabled) → safe |
| exit dispatch | n/a | Keys from `collided` (previously enabled) → safe |
| click prescan + dispatch | **Yes** | Both check `!c.entity.enabled` |

### Common transition: disable → re-enable

When a widget is `setEnabled(true)` after being disabled:

- `collided` has **no** stale entry (collider wasn't in `colliding`
  while disabled, so never propagated to `collided`)
- New collisions are detected cleanly, `enter` fires correctly
- The one-frame null-collision period while disabled serves as an
  implicit "clean slate"

---

## 9. UI COLLIDER INTEGRATION

`UIManager.addColliderToElem(elem)` creates a `Polygon` shape from
`elem.width × elem.height` at `elem.position`, registers a `Collider`,
and stores the `(elem, shape, collider)` triple in
`this._elementColliders`.

### `_syncColliders()`

Called by `UI.refreshLayout()` every frame (or when dirty).  Updates
each tracked collider's shape position and vertices to match the
current element position and dimensions.  Runs regardless of element
enabled state.

### Positioning rules

- **Always call `UI.refreshLayout()` after manual `setPosition()`**
  in widget update loops — the next frame's `beginFrame()` uses
  `_syncColliders`'s output to place colliders in the quadtree
- **UIManager's own update handler** calls `refreshLayout()` only when
  `dirty` — do NOT rely on it alone; widget handlers must call
  `UI.refreshLayout()` explicitly after repositioning

### Hover via spawnButton

`UIManager.spawnButton()` with `hover: true` stores `_btnBase` (base
color array) and `_btnCollider` on the container element.  A global
per-frame updater in the UIManager constructor walks
`_elementColliders`, discovers elements with `_btnBase`, and does
per-frame GJK against the mouse to set hover or base color.  Click
flash (`clickFeedback` frames) takes priority over hover.

---

## 10. COMMON PITFALLS

| Symptom | Cause | Fix |
|---------|-------|-----|
| Button click fires on wrong element in overlap | Lower-PID collider dispatched first by default | Set `collider.clickPriority` higher on the visually-top element |
| Hover changes persist or don't reset | Old enter/exit used per-collider guards (`id in collided`) instead of per-pair; stale pairs suppressed new ones | Switched to `collided[id]?.[otherId]` — fixed in this codebase |
| `enter` fires for disabled collider | `beginFrame()` inserts all colliders; enabled collider can GJK-hit disabled one | Check `other.entity?.enabled` in enter handlers that act on `other` |
| Collider position stale after repositioning | Widget update loop positions elements but doesn't call `UI.refreshLayout()` | Call `UI.refreshLayout()` at end of widget update handler |
| `enter` fires with disabled collider as `other` arg | Same as above — the enabled collider's `enter` receives whatever `_registry[otherId]` returns | Guard `other` usage or don't act on disabled `other` objects |
| Long-running session leaks IDs | `_nextId` never decrements, monotic increase | Negligible — 2⁵³ limit; unregister just deletes the slot |

---

## 11. QUICK REFERENCE

```
Collider(entity, shape) → Collider          auto-registers via game.physics
collider.addHandler('click', fn)            push handler, fn returns bool
collider.callHandler('click', ...args)       call all, stop on first truthy
collider.hasHandlers('enter')               boolean check
collider.consumesClick = true               pre-flags uiConsumedClick on click
collider.clickPriority = number             higher dispatches first
collider.intersects(rect)                   AABB test
collider.updateRect()                       sync from shape.rectangle()

PhysicsProvider.gjk(a, b)                   boolean GJK intersection
PhysicsProvider.beginFrame()                clear quadtree, insert all, sync mouse
PhysicsProvider.performCalls()              full collision + handler loop
PhysicsProvider.beginSelection()            anchor point at mousePos
PhysicsProvider.updateSelection()           grow rectangle to current mouse
PhysicsProvider.endSelection()              fire 'selection', reset rect

UIManager.addColliderToElem(elem) → Collider  track in _elementColliders
UIManager.spawnButton(w, h, c, onClick, opts?)  unified button factory
  opts: { text?, textScale?, textColor?, sprite?, spriteFrame?,
          spriteTileSet?, priority?, hover?, clickFeedback? }
```
