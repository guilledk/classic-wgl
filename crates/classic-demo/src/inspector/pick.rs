//! Click-to-pick in the world while the inspector is open, and the outline
//! of the selected entity.
//!
//! The open inspector lays a full-viewport, lowest-priority backdrop
//! collider under its panels: it consumes every click that no panel or game
//! UI button took, so the editor's rubber band, tile paint and RTS select do
//! not see it, and turns the click into an [`Engine::pick_at`] here.

use classic_core::components::Transform;
use classic_core::math::iso_camera_px;
use classic_engine::Engine;
use classic_gfx::GlBuffer;
use glam::{Mat4, Vec3};

use super::{InspectorState, Selection};
use crate::state::DemoStateRef;

/// Pick the top gameplay entity under `(x, y)`; on a hit select it and
/// reveal it in the tree.  Returns the picked name.
pub fn pick(engine: &Engine, st: &mut InspectorState, x: f32, y: f32) -> Option<String> {
    let name = engine.pick_at(x, y, "")?;
    st.select(Selection::Entity(name.clone()));
    st.reveal(engine, &name);
    Some(name)
}

/// The selected entity's footprint, as screen-space camera-view vertices
/// (`iso_camera_px`), or `None` when it is not on the map.
fn outline(engine: &Engine, name: &str) -> Option<(Vec<f32>, Vec3)> {
    let &e = engine.names.get(name)?;
    let p = engine.world.get::<&Transform>(e).ok()?.position;
    engine.entity_by_role(classic_core::RoleKind::Tilemap)?;
    let footprint = engine.selectable_footprint(e);
    let mut verts = Vec::with_capacity(footprint.len() * 3);
    for o in footprint {
        let v = iso_camera_px(engine.iso_to_world(p.x + o.x, p.y + o.y, 0.0)?);
        verts.extend_from_slice(&[v.x, v.y, v.z]);
    }
    let anchor = iso_camera_px(engine.iso_to_world(p.x, p.y, 0.0)?);
    Some((verts, anchor))
}

/// Overlay hook: outline the selected entity's footprint and mark its
/// ground anchor while the inspector is open.
pub fn draw_selection(engine: &mut Engine, state: &DemoStateRef) {
    let name = {
        let s = state.borrow();
        match (&s.inspector.selection, s.inspector.open) {
            (Some(Selection::Entity(n)), true) => n.clone(),
            _ => return,
        }
    };
    let Some((verts, anchor)) = outline(engine, &name) else { return };
    let cam = engine.camera.matrix();
    let Some(gfx) = engine.gfx.as_mut() else { return };
    let color = [1.0, 0.85, 0.3, 0.95];
    if verts.len() >= 6 {
        let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STREAM_DRAW);
        gfx.draw_line_loop(&buf, (verts.len() / 3) as i32, &Mat4::IDENTITY, &cam, &color);
        buf.delete(&gfx.gl);
    }
    let r = 10.0;
    let cross = [-r, -r, 0.0, r, r, 0.0, -r, r, 0.0, r, -r, 0.0];
    let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &cross, glow::STREAM_DRAW);
    let m = Mat4::from_translation(anchor);
    gfx.draw_line_strip(&buf, 0, 2, &m, &cam, &color);
    gfx.draw_line_strip(&buf, 2, 2, &m, &cam, &color);
    buf.delete(&gfx.gl);
}

#[cfg(test)]
/// The screen point of an entity's ground anchor (for tests and scripted
/// captures): `None` off the map.
pub fn anchor_screen(engine: &Engine, name: &str) -> Option<glam::Vec2> {
    let &e = engine.names.get(name)?;
    let p = engine.world.get::<&Transform>(e).ok()?.position;
    let (x, y) = engine.world_to_screen(engine.iso_to_world(p.x, p.y, 0.0)?);
    Some(glam::Vec2::new(x, y))
}
