---
name: classic-rust-camera
description: >
    2D camera math for classic-wgl's Rust port.  Covers matrix
    multiplication order (translate-first vs scale-first), the
    `fix()` formula (TS vs initial buggy port), zoom mechanics
    (additive vs proportional), orthographic projection setup,
    and real frame delta impact on camera smoothness.  Use when
    debugging camera drift at high zoom, zoom feeling wrong,
    camera jitter, or "camera position moves when I zoom."
    Trigger phrases: "camera matrix order", "T*S vs S*T",
    "fix formula", "camera fix", "camera drift", "zoom moves camera",
    "zoom sensitivity", "proportional zoom", "additive zoom",
    "orthographic_rh", "camera jitter", "camera position shifts",
    "getFix", "matrix multiply order".
compatibility: glam 0.29, column-major matrix convention
metadata:
    author: classic-wgl
    version: '0.1'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

## Scope

Covers `classic-core/src/camera.rs` — the 2D orthographic camera used by
the engine.  This is a direct port of `ts/classic/camera.ts` and must
produce identical matrices for visual parity.

---

## 1. Camera Matrix Math — The Order is Critical

The TS `camera.ts:matrix()` does:

```ts
mat4.translate(m, m, [-fix.x, -fix.y, -fix.z]);
mat4.scale(m, m, this.scale);
```

Using column-major conventions: `m = T(-fix) * S(scale)`.

**Our Rust port MUST match:**

```rust
pub fn matrix(&self) -> Mat4 {
    let fix = self.fix();
    Mat4::from_translation(-fix) * Mat4::from_scale(self.scale)
    //          T(-fix)          *         S(scale)
}
```

**Wrong (initial bug):**
```rust
Mat4::from_scale(self.scale) * Mat4::from_translation(-fix)  // S * T ← WRONG
```

`S * T` means "translate first, then scale around the origin" — the
translation gets multiplied by `scale`, sending the visible area far
from the intended camera position at high zoom.

**Same bug exists in `Transform::model_matrix()`:**
```rust
// Correct:
Mat4::from_translation(self.position) * Mat4::from_scale(self.scale)
//           T(pos)           *         S(scale)
```

---

## 2. The fix() Formula — Division Distributive Mistake

**TS `camera.ts:getFix()`:**
```ts
vec3.multiply(camFixed, this.position, this.scale);   // pos * scale
vec3.sub(camFixed, camFixed, this.size);               // pos*scale - size
vec3.divide(camFixed, camFixed, [2, 2, 1]);           // (pos*scale - size) / 2
```

This is: `fix = (position * scale - size) / Vec3::new(2, 2, 1)`

**Initial buggy Rust port:**
```rust
let fixed = self.position * self.scale;
fixed - self.size / Vec3::new(2.0, 2.0, 1.0)  // pos*scale - size/2 ← WRONG
```

The division only applies to `size`, not `position*scale`.  At
`scale=50`, this error is `position*25` pixels off — enough to push the
camera completely past the tilemap.

**Correct fix:**
```rust
pub fn fix(&self) -> Vec3 {
    (self.position * self.scale - self.size) / Vec3::new(2.0, 2.0, 1.0)
}
```

**Regression test** (non-zero position, scale, AND size):
```rust
#[test]
fn matrix_with_nonzero_position_and_size() {
    let mut cam = Camera::new(Vec3::new(100., 50., 0.), Vec3::new(2., 2., 1.));
    cam.resize(Vec3::new(800., 600., 1.));
    assert_eq!(cam.fix(), Vec3::new(-300., -250., -1.));
}
```
Previously all tests used `size=(0,0,0)` or `scale=(1,1,1)` where both
formulas produce identical results, masking the bug.

---

## 3. Zoom — Additive, Not Proportional

The TS zoom is **additive** (`scale += wheel * delta`), not proportional
(`scale *= 1 + wheel * delta`).  Additive zoom produces the same
world-space change per scroll notch regardless of current zoom level.

```rust
// Correct (matches TS):
let dz = engine.input.mouse_wheel * engine.time.delta;
engine.camera.scale.x += dz;
engine.camera.scale.y += dz;

// Wrong (feels "off" at extremes):
let f = 1.0 + engine.input.mouse_wheel * engine.time.delta;
engine.camera.scale *= f;  // scale=50: one notch = +0.7, scale=1: one notch = +0.014
```

**Do NOT multiply by `scroll_speed`** — `scroll_speed` (600) is for WASD
panning (pixels/second), not zoom.  The TS never multiplies zoom by it.

---

## 4. Orthographic Projection

Matches `state.ts:355-363`:

```rust
Mat4::orthographic_rh(0.0, viewport_w, viewport_h, 0.0, -10000.0, 10000.0)
```

- `left=0, right=vw` — screen x maps to [-1, +1]
- `bottom=vh, top=0` — screen y maps to [-1, +1] (Y grows downward)
- `near=-10000, far=10000` — deep clipping range

The `orthographic_rh` in glam uses a right-handed coordinate system.
The `-2.0 / (far - near)` for the z-column (RH convention) produces
the correct mapping for the `isoDepth` manual depth override in
`iso_tilemap.vert`.

---

## 5. Real Frame Delta Impact

The camera zoom closure reads `engine.time.delta` which is set from
the platform's real frame timing.  A hardcoded `0.016` causes:
- At 144Hz: zoom decays too fast (0.022 per 7ms instead of per 16ms)
- At 60Hz: zoom is correct
- Frame-to-frame variation causes visible wobble

**Always use real delta from the platform.**  The zoom formula, wheel
decay, and WASD speed all depend on accurate timing.

---

## 6. SDF Text Justify x-offset

The model matrix for SDF text applies an x-offset based on justify
to align text content within the element's pixel box:

```rust
let x_off = match sdf.justify {
    TextJustify::Left   => 0.0,
    TextJustify::Center => -text_width / 2.0,
    TextJustify::Right  => -text_width,
};
let model = T(position + x_off, position_y, position_z) * S(text_width, text_height, scale_z);
```

**For UI-managed text,** the anchor system already positions the
element box at the correct screen location.  The justify x-offset
must be **skipped** (`x_off = 0`) to avoid double-centering:

```rust
let is_ui = world.get::<&UiNode>(*e).map(|n| n.parent.is_some()).unwrap_or(false);
let x_off = if is_ui { 0.0 } else { match sdf.justify { ... } };
```

---

## 7. UI Anchor Math (`set_child_position`)

The anchor system positions children relative to their parent
container using CSS-like anchor offsets:

```
child.x = parent.x + parent_anchor.offset(parent_w, parent_h).x
                  - child_anchor.offset(child_w, child_h).x
child.y = parent.y + parent_anchor.offset(parent_w, parent_h).y
                  - child_anchor.offset(child_w, child_h).y
```

`UiAnchor::offset()` maps the 9 anchor points to (x, y) offsets
from the element's top-left corner.  For `MidCenter` on a w×h box:
`(w/2, h/2)`.

**Pitfall:** `set_child_position` sets the **top-left corner** of
the child.  Do NOT manually add `widget_w/2` to center — the anchor
system already computes the correct offset.

**Vertical centering of SDF text:** The glyph buffer's `text_height`
uses `glyphExtentMin + glyphExtentMax` so the visual center of the
glyph row aligns with the geometric center `ch/2`.  This makes
`MidCenter` anchors correctly center text visually.
