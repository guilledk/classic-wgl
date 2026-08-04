---
name: classic-ui
description: >
    The retained-mode UI/layout system for classic-wgl, including dev tool
    button panels and editor widgets. Covers UIManager, UIContainer, UIElement,
    UIText, UISprite, UIArray, the anchor/layout system, widget creation pattern,
    slide-out tool panels, and editor mode control. Use when building or
    debugging UI panels, dev tool buttons, editor palettes, or any canvas‑space
    element positioning. Trigger phrases: "UI", "widget", "dev button", "tool
    panel", "editor panel", "palette", "UIText", "UIContainer", "UIElement",
    "UISprite", "spawnContainer", "spawnText", "spawnSprite", "setEnabled",
    "editorTarget", "slide‑out", "addColliderToElem", "initToolButtons",
    "initHeightWidget", "initLightWidget", "editor mode",
    "start menu", "click outside", "consumesClick", "uiConsumedClick",
    "UIManager", "markUIDirty", "refreshLayout",
    "setChildrenPos", "anchor system".
compatibility: All UI elements are Drawables on the renderList with ignoreCam
    flag (UIElement extends Rectangle extends Drawable). Positioning uses
    canvas‑pixel coordinates (orthographic projection). Text uses a sprite‑sheet
    font with an ASCII‑only glyph string. Colliders are Polygon shapes
    transformed to screen space via camera getFix.
metadata:
    author: classic-wgl
    version: '0.3'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit, Write
---

# classic-wgl UI System

## 1. ARCHITECTURE OVERVIEW

The UI system is a retained‑mode layout layer built on top of the rendering
primitives. All UI components are `Drawable` instances that register themselves
on the `renderList` and participate in the standard draw loop. There is no
separate UI render pass.

### Component hierarchy

```
Drawable (transforms.ts)
  └─ Rectangle (transforms.ts)          solid‑color quad
       └─ UIElement (ui.ts)              positioned in canvas‑pixel space
            ├─ UIContainer (ui.ts)       child container with anchor layout
            └─ UIArray (ui.ts)           flex‑like horizontal/vertical layout
  └─ Sprite (transforms.ts)              textured quad
       └─ UISprite (ui.ts)               sized in canvas pixels
  └─ Text (transforms.ts)                render‑to‑texture text (legacy)
       └─ UIText (ui.ts)                 sized in canvas pixels (legacy)
  └─ SdfText (sdfText.ts)                direct SDF vertex‑buffer text
       └─ UISdfText (ui.ts)              word‑wrap, justify, outline, shadow
```

The `UIManager` singleton is stored as `game.ui`. It owns the root container
and drives layout refresh via a dirty flag.

### Key files

| File                        | Role                                                                    |
| --------------------------- | ----------------------------------------------------------------------- |
| `src/classic/ui.ts`         | UIManager, UIContainer, UIElement, UIText, UISdfText, UISprite, UIArray, UIPadding |
| `src/demo/uiPrefabs.ts`     | Demo UI tree: top bar, tool buttons, palettes, height/light widgets     |
| `src/demo/prefabs.ts`       | Editor logic handlers (tile/nav/height fill‑on‑selection)               |
| `src/classic/transforms.ts` | Drawable, Rectangle, Sprite, Text base classes                          |

### Layout flow

1. `UIManager.markDirty()` sets a flag.
2. Next `'canvasResize'` call, `refreshLayout()` walks the tree calling
   `setChildrenPos()` on every container.
3. `setChildrenPos()` computes child positions relative to the parent
   using anchor offsets and calls `child.setPosition(x, y)`.
4. Each `setPosition()` call also triggers `setChildrenPos()` on the child
   if it's a container — cascading down the tree.

---

## 2. ANCHOR SYSTEM

### 9 anchor points

```
top-left    top-center    top-right
mid-left    mid-center    mid-right
bot-left    bot-center    bot-right
```

Each anchor maps to an offset pair relative to the element's top‑left corner:

| Anchor       | x offset | y offset |
| ------------ | -------- | -------- |
| `top-left`   | 0        | 0        |
| `top-center` | w/2      | 0        |
| `top-right`  | w        | 0        |
| `mid-left`   | 0        | h/2      |
| `mid-center` | w/2      | h/2      |
| `mid-right`  | w        | h/2      |
| `bot-left`   | 0        | h        |
| `bot-center` | w/2      | h        |
| `bot-right`  | w        | h        |

### `addChild(child, selfAnchor?, childAnchor?)`

```typescript
container.addChild(child, 'top-left', 'top-left');
```

- `selfAnchor` — point on the **container** where the child attaches
- `childAnchor` — point on the **child** that attaches

Both default to the container's `this.anchor` (which defaults to `'mid-center'`
for `UIContainer`).

### `setChildrenPos()` formula

```
child.x = panel.x + selfAnchorOffset.x - childAnchorOffset.x
child.y = panel.y + selfAnchorOffset.y - childAnchorOffset.y
```

Where `panel.x` / `panel.y` are the container's `position[0]` / `position[1]`
(top‑left corner of the container).

### `getAnchorOffset(anchor, w, h)`

```typescript
getAnchorOffset('mid-center', w, h) → { x: w/2, y: h/2 }
```

---

## 3. `setPosition` AND POSITIONING

### Critical rule

**`element.setPosition(x, y)` sets the top‑left corner of the element,
always.** It does NOT position the anchor point. The anchor system only
affects how children are placed within a container, not the container's
own screen position.

### The centering pitfall

```typescript
// WRONG — pushes container half‑widget off‑screen right and bottom:
container.setPosition(x0 + widgetW / 2, y0 + widgetH / 2);

// CORRECT — top‑left corner at the natural widget origin:
container.setPosition(x0, y0);
```

This is the most common positioning bug. If you want a container visually
centered, use `addChild(container, 'mid-center', 'mid-center')` on its parent
and let the anchor system compute the offset — do NOT add `widgetW/2` to
the position manually.

### Collider sync timing

Collider shapes are only synced during `refreshLayout()` → `_syncColliders()`,
which runs at the START of the `'update'` cycle (UIManager's handler, registered
first). If your update handler repositions elements AFTER `refreshLayout()`,
their collider shapes stay at old positions — the quadtree still has them at
stale coords. Next frame's `performCalls()` won't find them under the
mouse → clicks don't fire and hover highlights break.

**Fix:** Call `UI.refreshLayout()` at the END of your update handler, after
all `setPosition()` and `setEnabled()` calls. This runs `_syncColliders()`
immediately, updating all collider shapes to match current element positions
before the next `beginFrame()` → quadtree insertion → click dispatch.
Generic widgets (palettes, height/light panels) typically don't need this
because they're initially disabled and only sync after a `setEnabled(true)`
triggers a dirty‑flag refresh — but dynamic elements (like menu items that
appear/disappear) do.

### Manual child positioning

For dev tool widgets, children are typically positioned manually each frame
in a `'update'` call registered on `UI.root.entity`. These `setPosition`
calls use **absolute canvas‑pixel coordinates**, not container‑relative
coordinates.

```typescript
UI.root.entity.registerCall('update', () => {
    const cw = game.canvas!.width;
    const ch = game.canvas!.height;
    const x0 = cw - uiBorder - widgetW;
    const y0 = ch - uiBorder - widgetH;

    container.setPosition(x0, y0);
    button.setPosition(x0 + gap, y0 + gap);
    label.setPosition(x0 + gap + btnSize + gap, y0 + gap);
});
```

After `container.setPosition()` fires, the container's own `setChildrenPos()`
runs and positions children relative to the container. The manual
`button.setPosition()` / `label.setPosition()` calls then **override** those
positions with absolute coordinates.

---

## 4. UIManager FACTORY METHODS

### `spawnContainer(w, h, color)` → UIContainer

Creates a `UIContainer` (solid‑color rectangle) attached to the manager's root
entity. Used for widget backgrounds.

```typescript
const container = UI.spawnContainer(widgetW, widgetH, [0, 0, 0, 0.4]);
```

### `spawnText(text, fontSize, maxChars, color, bgColor)` → UIText

Creates a `UIText` element. `fontSize` is multiplied by `_uiScale` before
passing. `maxChars` determines the render‑texture width.

```typescript
const label = UI.spawnText('Hello', 0.5 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
label.setText('Updated text');
label.setTextColor([1, 0, 0, 1]);
```

### `spawnSprite(textureName, w, h, frame, tileSetSize)` → UISprite

Creates a `UISprite` from the named texture. Frame and tile set size index into
sprite sheets like `editorIcons`.

```typescript
const sprite = UI.spawnSprite('editorIcons', btnPixel, btnPixel, 0, [4, 4]);
```

### `addColliderToElem(elem)` → Collider

Attaches a GJK `Collider` (world‑space `Polygon`) to the element. The
collider must be manually updated each frame to track the element's
screen position via the camera's `getFix()`.

```typescript
const col = UI.addColliderToElem(button);
col.addHandler('click', () => {
    /* handle click */ return true;
});
```

---

## 5. START MENU TOOL PANEL

### Pattern (`initToolButtons` in uiPrefabs.ts)

A DEV button (icon sprite, fixed bottom‑left) toggles a vertical pop‑up menu
above it. Selecting a tool from the menu closes it. Clicking outside or
toggling DEV again also closes.

```
┌──────┐
│ DEV  │  ← icon sprite, bottom‑left
└──────┘
    ↑  appears above DEV when toggled
┌──────────────────┐
│  Tile Editor     │  ← text‑labeled rows
│  Nav Editor      │
│  Height Editor   │
│  Light Config    │
└──────────────────┘
```

### DEV button

```typescript
const devContainer = UI.spawnContainer(btnPixel, btnPixel, [0, 0, 0, 0]);
const devSprite = UI.spawnSprite('editorIcons', btnPixel, btnPixel, 0, [4, 4]);
devContainer.addChild(devSprite, 'mid-center', 'mid-center');
const devCollider = UI.addColliderToElem(devContainer);
let isOpen = false;
```

### Menu items

Each item is a `UIContainer` row with a `UIText` label and a `Collider`.
Configured as an array of `{ label, action, isActive }` objects:

```typescript
const menuItems = [
    {
        label: 'Tile Editor',
        action: () => {
            game.editorTarget = 'tilemap';
            isOpen = false;
        },
        isActive: () => game.editorTarget === 'tilemap',
    },
    // ... nav, height, light
];
```

### Mutual exclusion via `editorTarget`

All tools use a single `editorTarget` field: `'tilemap'`, `'navMesh'`,
`'height'`, `'light'`, or `'none'`. Each menu item sets only its value —
mutual exclusion is automatic. No separate boolean flags needed. Light
Config toggles between `'light'` and `'none'`.

```typescript
// Tile / Nav / Height — direct set:
game.editorTarget = 'tilemap';

// Light Config — toggle:
game.editorTarget = game.editorTarget === 'light' ? 'none' : 'light';
```

### Click‑outside‑to‑close

A full‑canvas transparent backdrop `UIContainer` with a `Collider`.
Its click handler returns `false` so the `performCalls` loop continues past
it to DEV/menu handlers:

```typescript
backdropCol.addHandler('click', () => {
    if (!isOpen) return false;
    const mx = game.mousePos[0], my = game.mousePos[1];
    const onDev = /* AABB check against devContainer bounds */;
    const onMenu = /* AABB check against menuPanel bounds */;
    if (!onDev && !onMenu) isOpen = false;
    return false;         // ← NEVER consume the click
});
```

### Hover and active‑tool visual states

Per‑frame in the update loop, each row's background color is set:

| State                    | Color                   |
| ------------------------ | ----------------------- |
| Hovered (GJK intersect)  | `[0.25, 0.45, 0.75, 1]` |
| Active tool (isActive()) | `[0.20, 0.35, 0.60, 1]` |
| Default                  | `[0.15, 0.15, 0.15, 1]` |

### Panel sizing

Text width in pixels: `glyphPixelW = glyphSize[0] × fontScale × _uiScale`.
With the default glyph size 32 and menu font scale 0.5:
`glyphPixelW = 32 × 0.5 × _uiScale = 16 × _uiScale`.

```
const maxLabelLen = Math.max(...menuItems.map(m => m.label.length));
const glyphPixelW = 16 * _uiScale;
const menuW = maxLabelLen * glyphPixelW + menuPadding * 2;
const rowW = menuW - menuPadding * 2;
```

### Collider sync timing (critical)

Collider shapes are only synced during `refreshLayout()` → `_syncColliders()`,
which runs at the START of the `'update'` cycle. If elements are repositioned
IN the update handler after `refreshLayout()`, their colliders remain at old
positions — the quadtree won't find them under the mouse and clicks won't fire.

**Call `UI.refreshLayout()` at the END of your update handler**, after all
`setPosition()` and `setEnabled()` calls:

```typescript
UI.root.entity.registerCall('update', () => {
    // ... position elements, setEnabled menu/backdrop ...
    // ... hover highlight checks ...

    UI.refreshLayout(); // sync all colliders immediately
});
```

### Full update loop sketch

```typescript
UI.root.entity.registerCall('update', () => {
    const ch = game.canvas!.height;
    const h = btnPixel,
        half = h / 2;

    // DEV
    devContainer.setPosition(h - half, ch - h - half);

    // Agent indicator
    agentBox.setPosition(agentInd / 2, ch - h * 2 - agentInd / 2);

    // Menu panel (above DEV)
    const menuTop = ch - h - menuH;
    menuPanel.setPosition(h, menuTop);
    menuPanel.setEnabled(isOpen);

    // Menu items stacked inside panel
    let y = menuTop + menuPadding;
    for (const { row } of itemRows) {
        row.setPosition(menuPanel.position[0] + menuPadding, y);
        y += menuItemH + menuGap;
    }

    // Backdrop
    backdrop.setSize(game.canvas!.width, game.canvas!.height);
    backdrop.setPosition(0, 0);
    backdrop.setEnabled(isOpen);

    // Hover / active highlights
    for (const { row, col, item } of itemRows) {
        const hovered = game.physics!.gjk(col, game.physics!.mouse);
        row.color = hovered
            ? [0.25, 0.45, 0.75, 1]
            : item.isActive()
              ? [0.2, 0.35, 0.6, 1]
              : [0.15, 0.15, 0.15, 1];
    }

    UI.refreshLayout();
});
```

### Click consumption system

Dev tool panels suppress map interactions (tile selection, agent
pathing, selection overlay) when the user clicks a UI element,
preventing clicks from leaking through to the tilemap.

**Problem:** The tilemap `Collider` has the lowest PID and iterates
first in the quadtree. UI colliders fire after — too late to stop
the tilemap's `'click'` handler (agent pathing). The selection
overlay (`beginSelection`/`updateSelection`) fires synchronously in
mouse event handlers, even before `performCalls()` click dispatch.

**Solution — three layers:**

1. **`Collider.consumesClick` flag + prescan**: Before dispatching
   click handlers, `performCalls()` iterates all mouse‑intersecting
   colliders and sets `game.uiConsumedClick = true` if any collider
   has `consumesClick = true`. Tag all UI colliders (backdrop,
   panel‑menu rows, palette, widget buttons, agent indicator)
   with `consumesClick = true`.

2. **Flag‑guarded selection**: `updateSelection()` (mouse move) and
   `endSelection()` (mouse up) check `game.uiConsumedClick` and
   skip when set. `beginSelection()` is moved from synchronous
   `mouseDownHandler` to `draw()` **after** `performCalls()`,
   so the prescan runs first — no one‑frame overlay glitch.

3. **Panel‑menu pre‑flag**: `game.panelMenuOpen` is synced from
   `isOpen` in the update loop. On mousedown, if the menu is
   open the flag is pre‑set — agent and selection suppressed
   before any handler dispatches.

**Mutual exclusion:** All menu tool items toggle between their
target and `'none'`. Selecting any tool deselects the agent
(`game.agentSelected = false`). Clicking the agent [A] button
sets `editorTarget = 'none'`, closing any open tool panels.

### DEV close resets tool

```typescript
devCollider.addHandler('click', () => {
    isOpen = !isOpen;
    if (!isOpen) game.editorTarget = 'none';
    return true;
});
```

---

## 6. WIDGET CREATION PATTERN

Every editor widget follows the same recipe:

```
1. Compute sizes (btnSize, labelW, gap, widgetW, widgetH, uiBorder)
2. Spawn a container → UI.spawnContainer(widgetW, widgetH, [0,0,0,0.4])
3. Spawn child elements → UI.spawnContainer / UI.spawnText / UI.spawnSprite
4. Attach children to container → container.addChild(child, 'top-left', 'top-left')
5. Add colliders → UI.addColliderToElem(button)
6. Register click handlers → col.addHandler('click', () => { ... })
7. Register update loop → UI.root.entity.registerCall('update', () => { ... })
8. Start disabled → container.setEnabled(false)
```

### Widget row layout (manual positioning)

Each row uses the same horizontal offset pattern:

```
Row 1: [gap] button1  [gap]  button2/label  [gap]  button3  [gap]
Row 2: [gap] label     [gap]  button1        [gap]  button2  [gap]
Row 3: [gap] label     [gap]  button1        [gap]  button2  [gap]
```

### Width calculation

```typescript
// CORRECT — take the maximum row width:
const row1W = gap * 4 + btnSize * 2 + labelW;
const row2W = gap * 4 + dirW + smallBtn * 2 + buttonGap * 2;
const widgetW = Math.max(row1W, row2W);

// WRONG — summing all rows creates an absurdly wide widget:
// const widgetW = btnSize * 2 + smallBtn * 4 + labelW + dirW * 2 + 30;
```

### Right‑justified button row

When a row has expanding text (e.g. `az: 0deg` → `az: 135deg`), pin the
label to the left and the buttons to the right:

```typescript
azLabel.setPosition(x0 + cx, y0 + cy);
azMinus.setPosition(x0 + widgetW - gap - smallBtn * 2 - buttonGap, y0 + cy);
azPlus.setPosition(x0 + widgetW - gap - smallBtn, y0 + cy);
```

This prevents text expansion from pushing buttons or causing clipping.

---

## 7. UNIFIED TOOL STATE

### Single `editorTarget` field

All tool panels (tile palette, nav palette, height widget, light widget) are
controlled by a single `game.editorTarget` field. Valid values:
`'tilemap'`, `'navMesh'`, `'height'`, `'light'`, or `'none'`.

Mutual exclusion is automatic — setting one value inherently clears any
other active panel. No per‑panel boolean flags are needed.

### State augmentation

Extended via `declare module` in `prefabs.ts` and `uiPrefabs.ts`:

```typescript
declare module '/classic/types.js' {
    interface IGameState {
        editorTarget?: string; // 'tilemap' | 'navMesh' | 'height' | 'light' | 'none'
    }
}
```

### Toggling widgets

`initEditorModeControl()` maps `editorTarget` to `setEnabled` on each widget
each frame:

```typescript
UI.root.entity.registerCall('update', () => {
    if (_tilePalette) _tilePalette.setEnabled(game.editorTarget === 'tilemap');
    if (_navPalette) _navPalette.setEnabled(game.editorTarget === 'navMesh');
    if (_heightWidget) _heightWidget.setEnabled(game.editorTarget === 'height');
    if (_lightWidget) _lightWidget.setEnabled(game.editorTarget === 'light');
    navMeshEntity.enabled = game.editorTarget === 'navMesh';
});
```

Note: `lightWidgetVisible` boolean flag was removed in favour of the
`'light'` value. Light Config toggles `editorTarget` between `'light'`
and `'none'`.

### Initialization

```typescript
game.editorTarget = 'none';
_tilePalette = initTilePalette(UI);
_navPalette = initNavPalette(UI);
_heightWidget = initHeightWidget(UI);
_lightWidget = initLightWidget(UI);
initEditorModeControl(UI);
```

### DEV close resets

When the DEV button closes the menu, it resets `editorTarget` to `'none'`:

```typescript
devCollider.addHandler('click', () => {
    isOpen = !isOpen;
    if (!isOpen) game.editorTarget = 'none';
    return true;
});
```

---

## 8. FONT LIMITATIONS

### Glyph set

The `UIText` renderer uses a sprite‑sheet font with a **strictly ASCII‑only**
glyph string (defined in `ui.ts:118`):

```
'!"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~'
```

Characters **NOT** in the font include spaces, letters below `A`, `a`–`z`,
and all Unicode (arrows, degree symbols, emoji, accented chars).

### Common errors

| Attempt                     | Error                                      |
| --------------------------- | ------------------------------------------ |
| `spawnText('◀◀', ...)`      | `Error: Char '◀' not in font glyph string` |
| `spawnText('az: 45°', ...)` | `Error: Char '°' not in font glyph string` |

### ASCII substitutions

| Unicode | ASCII replacement |
| ------- | ----------------- |
| `◀◀`    | `<<`              |
| `▶▶`    | `>>`              |
| `°`     | `deg`             |

---

## 8b. SDF FONT TEXT (`UISdfText`)

The `UISdfText` renderer uses a pre‑generated signed‑distance‑field (SDF)
atlas and supports proportional spacing, word‑wrapping by pixel width,
multi‑line via explicit `\n`, and per‑line justification. It replaces the
legacy `UIText`'s monospaced sprite‑sheet approach.

### Factory method

```typescript
UI.spawnSdfText(text, textScale, maxWidth, color, bgColor) → UISdfText
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `text` | `string` | Initial text content |
| `textScale` | `number` | Scale multiplier (cell‑pixel units; ~2.5× larger than equivalent `UIText` scale) |
| `maxWidth` | `number` | Maximum pixel width before word‑wrapping (screen pixels, *not* char count) |
| `color` | `Color` | Foreground text RGBA |
| `bgColor` | `Color` | Background RGBA (unused in UISdfText; pass `[0,0,0,0]`) |

### Scale conversion from `UIText`

| Legacy `UIText` scale | Equivalent `UISdfText` scale | Notes |
|---|---|---|
| 0.4 | 1.0 | Controls hint "WASD MOVE" |
| 0.5 | 1.25 | Menu items, height widget |
| 0.55 | 1.375 | Agent `[A]` button |
| 0.6 | 1.5 | Classic title |

### Feature methods

| Method | Description |
|---|---|
| `setText(str)` | Sets new text; triggers word‑wrap and layout recalculation (synchronous) |
| `setTextColor(rgba)` | Sets foreground color |
| `setTextScale(scale)` | Changes scale; recalculates layout |
| `setMaxWidth(px)` | Changes pixel max‑width; recalculates wrapping |
| `setJustify('left' \| 'center' \| 'right')` | Per‑line horizontal alignment |
| `setOutline(width, color)` | Adds an outline band around glyph edges (SDF‑unit width, RGBA) |
| `setShadow(ox, oy, color, blur)` | Drop‑shadow with offset and blur |
| `setEnabled(bool)` | Toggles visibility; cascades through children |
| `setPosition(x, y)` | Positions top‑left corner in canvas‑pixel space |

### Multi‑line support

Embed `\n` in the text string for hard line breaks. The `wrapTextAtPixelWidth`
function splits on `\n` first, then applies word‑wrapping within each segment.
`_buildGlyphBuffer` offsets each line's glyphs by `lineIndex * lineHeight * scale`.

### Justification

`setJustify('center')` shifts each line's glyphs by `(maxWidth - linePixelWidth) / 2`.
`setJustify('right')` shifts by `(maxWidth - linePixelWidth)`. Per‑line widths are
computed from actual glyph advance sums, not character counts.

### `spawnButton` integration

```typescript
UI.spawnButton(w, h, color, onClick, {
    sdfText: true,              // uses spawnSdfText instead of spawnText
    text: 'Label',
    textScale: 0.5 * _uiScale,  // legacy scale — auto‑converted ×2.5 internally
});
```

### Common pitfalls specific to SDF text

| Symptom | Cause | Fix |
|---|---|---|
| Text not visible, vertexCount=0 | Metrics not loaded synchronously | Ensure `initSdfFonts` runs in `loadResources()`; `game.getSdfFont(name)` returns data |
| All glyphs collapse to one line | `lineIndex` not added to `gy` | `gy += pg.y * lineHeight * scale` in `_buildGlyphBuffer` |
| Text squashed vertically | `textHeight` changed after vertex loop | Compute glyph extent *before* building vertex data |
| Lines overlapping (multi‑line) | Same as above | Track `lineIndex` in perLine and offset `gy` |
| Text overflow at right edge | `maxWidth` too small, or widget widths not adjusted for scale conversion | Bump `labelW`/`dirW`/`glyphPixelW` by ~1.25–2× |
| `setJustify` has no effect on legacy `UIText` | Legacy text has no justify support | Only use on `UISdfText` instances |
| Text ~3× smaller than expected | Scale not converted from legacy units | Multiply legacy scale by ~2.5 |

---

## 9. COMMON PITFALLS

| Symptom                                                               | Cause                                                                                             | Fix                                                                                                       |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Widget background half off‑screen right/bottom, children shifted left | `container.setPosition(x0 + widgetW/2, y0 + widgetH/2)` — `setPosition` sets top‑left, not centre | Use `container.setPosition(x0, y0)`                                                                       |
| `"Char 'X' not in font glyph string"`                                 | Non‑ASCII character in `spawnText`                                                                | Use ASCII equivalent                                                                                      |
| Widget absurdly wide                                                  | `widgetW` computed as sum of all element widths instead of `Math.max()` per row                   | Take max of row widths                                                                                    |
| Buttons clipped by expanding label text                               | Buttons positioned from label edge rather than widget right edge                                  | Pin buttons to right margin                                                                               |
| Menu item clicks never fire                                           | Collider positions stale — `refreshLayout()` runs before manual positioning                       | Call `UI.refreshLayout()` at end of update handler                                                        |
| Text visible even when parent `setEnabled(false)`                     | Child not added to parent with `addChild` — `setEnabled` cascade follows child tree only          | `parent.addChild(child, ...)`                                                                             |
| Panel background too small for text                                   | Character width miscalculated — `N × 8` instead of `N × glyphPixelW`                              | Use `glyphPixelW = 16 × _uiScale`                                                                         |
| Widget flickers / overlaps on toggle                                  | Multiple `setEnabled` calls in one frame each marking dirty                                       | Consolidate to one `editorTarget` field                                                                   |
| Collider doesn't respond to clicks                                    | Collider position not synced — shape at creation position (0,0)                                   | `UI.refreshLayout()` or ensure dirty flag triggers sync                                                   |
| Slide‑out buttons invisible                                           | `isOpen` not set, or `closedX` pushes past canvas edge                                            | Check open/closed state logic                                                                             |
| Widget has no background                                              | `setEnabled(false)` call before ever showing — background entity is disabled                      |
| `spawnText` empty or clipped                                          | `maxChars` too small for the string; increase the third argument                                  |
| Agent paths to map tile behind open panel‑menu                        | Collider PID ordering — tilemap `'click'` handler fires before UI prescan sets `uiConsumedClick`  | Prescan `consumesClick` colliders in `performCalls()`, pre‑flag via `panelMenuOpen` in `mouseDownHandler` |
| Selection overlay flashes briefly when clicking UI                    | `beginSelection()` ran synchronously before click dispatch could set the flag                     | Defer `beginSelection()` to `draw()` after `performCalls()` prescan                                       |
| Click on popup menu item closes menu instead of activating item       | Overlapping colliders (e.g. bottom menu item overlaps toggle button) — lower-PID collider fires first and consumes click | Set `collider.clickPriority = 1` on menu item rows so they dispatch before the parent toggle button |

### 9.1 Click priority dispatch

When two clickable colliders overlap in screen space (e.g. a popup menu stacked
above a toggle button), the default dispatch order is by collider `_pid`
(creation order). A lower-PID collider handles the click first and can consume
it before the visually‑on‑top element gets a chance.

`Collider` exposes a `clickPriority: number` field (default `0`). In
`performCalls()`, all GJK‑intersecting click‑handler colliders are collected
and sorted **descending** by `clickPriority`, tiebroken by `_pid` ascending.
Higher priority → dispatched first.

**Pattern:** Set `clickPriority = 1` (or higher) on child/popup panel item
colliders so they dispatch before the parent toggle or background button:

```typescript
const col = UI.addColliderToElem(row);
col.consumesClick = true;
col.clickPriority = 1;  // dispatch before DEV toggle button (priority 0)
```

---

## 10. REFERENCE: QUICK SIGNATURES

```
UIManager(game) → UI
UI.spawnContainer(w, h, [r,g,b,a])       → UIContainer
UI.spawnText(str, fontScale, maxChars, [r,g,b,a], [br,bg,bb,ba]) → UIText
UI.spawnSdfText(str, textScale, maxWidth, [r,g,b,a], [br,bg,bb,ba]) → UISdfText
UI.spawnSprite(texName, w, h, frame, [cols,rows]) → UISprite
UI.spawnButton(w, h, color, onClick, opts?) → { container: UIContainer, collider: Collider, child?: UIText|UISdfText|UISprite }
//  opts: { text?, textScale?, textColor?, sdfText?, sprite?, spriteFrame?, spriteTileSet?, priority?, hover?, clickFeedback? }
UI.addColliderToElem(UIElement)           → Collider

container.addChild(child, selfAnchor?, childAnchor?) → this
container.setChildrenPos()               → void
container.setPosition(x, y)              → this (top‑left)
container.setEnabled(bool)               → this (cascades to children)
container.setSize(w, h)                  → this

text.setText(str)                         → this
text.setTextColor([r,g,b,a])              → this

sdftext.setText(str)                      → this
sdftext.setJustify('left'|'center'|'right') → this
sdftext.setOutline(width, [r,g,b,a])      → this
sdftext.setShadow(ox, oy, [r,g,b,a], blur) → this
sdftext.setTextScale(n)                   → this
sdftext.setMaxWidth(n)                    → this

sprite.setSize(w, h)                      → this
sprite.setPosition(x, y)                  → this

col.addHandler('click', () => boolean)    → void
col.addHandler('enter', () => void)       → void
col.addHandler('exit', () => void)        → void
col.clickPriority = number                 → higher dispatches first in overlaps
col.consumesClick = boolean                → true prevents map interactions
```
