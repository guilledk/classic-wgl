//! Text showcase, iso coordinate overlay, and the debug overlays (footprint
//! polygons, agent ring, compass rose) that were previously inlined in
//! `Engine::frame`.  These draw via the engine's overlay hook.

use classic_core::components::{
    DebugName, IsoSprite, SdfTextRender, TextJustify, Tilemap, Transform, UiAnchor, UiNode,
};
use classic_core::instrument::Chan;
use classic_core::math::iso_to_cartesian_4;
use classic_core::tilemap::bilinear_height;
use classic_engine::Engine;
use classic_gfx::GlBuffer;
use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::state::DemoStateRef;

/// Route the mouse wheel to the text-demo panel when it is open and under the
/// cursor, before camera zoom (which runs first in registration order).
pub fn route_text_scroll(engine: &mut Engine, state: &DemoStateRef) {
    if engine.input.mouse_wheel.abs() <= 0.01 {
        return;
    }
    if state.borrow().editor.target != "textDemo" || state.borrow().text_showcase_e.is_none() {
        return;
    }
    if let Some(ref ui) = engine.ui {
        let panel_w: f32 = 520.0;
        let panel_h: f32 = 440.0;
        let border: f32 = 10.0;
        let px = ui.viewport_w - panel_w - border;
        let py = ui.viewport_h - panel_h - border;
        let mouse = engine.input.mouse_pos;
        let in_bounds =
            mouse.x >= px && mouse.x <= px + panel_w && mouse.y >= py && mouse.y <= py + panel_h;
        if in_bounds {
            let ds = engine.input.mouse_wheel * 30.0;
            let max_scroll = (state.borrow().text_demo_content_h - panel_h).max(0.0);
            if let Some(e) = state.borrow().text_showcase_e {
                if let Ok(mut node) = engine.world.get::<&mut UiNode>(e) {
                    node.scroll_y = (node.scroll_y - ds).clamp(0.0, max_scroll);
                }
            }
            engine.input.mouse_wheel = 0.0;
        }
    }
}

/// Text showcase panel: demonstrates SDF text features with
/// scrollable, scissor-clipped container.
pub fn init_text_showcase(engine: &mut Engine, state: &DemoStateRef) {
    let Some(ref mut ui) = engine.ui else { return };
    let border: f32 = 10.0;
    let panel_w: f32 = 520.0;
    let panel_h: f32 = 440.0;
    let init_px = ui.viewport_w - panel_w - border;
    let init_py = ui.viewport_h - panel_h - border;

    let container =
        ui.spawn_container(&mut engine.world, panel_w, panel_h, [0.05, 0.05, 0.08, 0.92]);
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(container) {
        tf.position = Vec3::new(init_px, init_py, tf.position.z);
    }
    if let Ok(mut node) = engine.world.get::<&mut UiNode>(container) {
        node.clip_children = true;
    }

    let text_scale: f32 = 0.7;
    let line_h: f32 = 28.0;
    let line_gap: f32 = 4.0;
    let section_gap: f32 = 16.0;
    let indent: f32 = 6.0;
    let mut cy: f32 = 6.0;

    #[rustfmt::skip]
    let lines: Vec<(&str, f32, TextJustify, [f32; 4])> = vec![
        ("SDF Font Rendering", 1.3, TextJustify::Left, [1.0, 1.0, 1.0, 1.0]),
        ("Tiny text (0.3)", 0.3, TextJustify::Left, [0.7, 0.7, 0.7, 1.0]),
        ("Small text (0.5)", 0.5, TextJustify::Left, [0.7, 0.7, 0.7, 1.0]),
        ("Medium text (1.0)", 1.0, TextJustify::Center, [0.8, 0.8, 0.9, 1.0]),
        ("Large text (1.8)", 1.8, TextJustify::Left, [0.7, 0.9, 1.0, 1.0]),
        ("Extra large (2.5)", 2.5, TextJustify::Left, [1.0, 0.6, 0.2, 1.0]),
        ("Maximum (3.5)", 3.5, TextJustify::Left, [0.2, 1.0, 0.3, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("Weight 0.0 — thinner strokes", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
        ("Weight 0.15 — medium", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
        ("Weight 0.3 — bolder strokes", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
        ("Gamma 0.5 — sharper edges", text_scale, TextJustify::Left, [0.8, 0.8, 0.9, 1.0]),
        ("Gamma 2.5 — softer edges", text_scale, TextJustify::Left, [0.8, 0.8, 0.9, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Left, [0.6, 1.0, 0.6, 1.0]),
        ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Center, [0.6, 1.0, 0.6, 1.0]),
        ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Right, [0.6, 1.0, 0.6, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("Red text", text_scale, TextJustify::Left, [1.0, 0.2, 0.2, 1.0]),
        ("Green text", text_scale, TextJustify::Left, [0.2, 1.0, 0.2, 1.0]),
        ("Blue text", text_scale, TextJustify::Left, [0.3, 0.5, 1.0, 1.0]),
        ("Yellow text", text_scale, TextJustify::Left, [1.0, 0.9, 0.1, 1.0]),
        ("Cyan text", text_scale, TextJustify::Left, [0.2, 0.9, 1.0, 1.0]),
        ("Magenta text", text_scale, TextJustify::Left, [1.0, 0.3, 0.8, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("Thin outline (0.08)", 1.2, TextJustify::Left, [0.9, 0.5, 0.2, 1.0]),
        ("Thick outline (0.2)", 1.2, TextJustify::Left, [0.9, 0.5, 0.2, 1.0]),
        ("Blue glow", 1.4, TextJustify::Left, [0.2, 0.6, 1.0, 1.0]),
        ("Orange glow", 1.4, TextJustify::Left, [1.0, 0.4, 0.1, 1.0]),
        ("Drop shadow", 1.0, TextJustify::Left, [0.9, 0.9, 0.9, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("Chess: \u{2654}\u{2655}\u{2656}\u{2657}\u{2658}\u{2659}", text_scale, TextJustify::Left, [1.0, 0.8, 0.4, 1.0]),
        ("Suits: \u{2660}\u{2663}\u{2665}\u{2666}", text_scale, TextJustify::Center, [0.8, 0.8, 0.8, 1.0]),
        ("Arrows: \u{2190}\u{2191}\u{2192}\u{2193}\u{2194}\u{21C4}\u{21BA}", text_scale, TextJustify::Left, [0.6, 0.7, 1.0, 1.0]),
        ("Shapes: \u{25A0}\u{25B2}\u{25C6}\u{25CF}\u{2605}\u{2713}\u{2717}", text_scale, TextJustify::Left, [0.7, 0.9, 0.6, 1.0]),
        ("Greek: \u{0391}\u{0392}\u{0393}\u{0394}\u{03A3}\u{03A9}\u{03B1}\u{03B2}\u{03B3}\u{03C0}", text_scale, TextJustify::Left, [0.7, 0.7, 1.0, 1.0]),
        ("Japanese: \u{65E5}\u{672C}\u{8A9E} \u{6F22}\u{5B57} \u{30AB}\u{30BF}\u{30AB}\u{30CA}", text_scale, TextJustify::Left, [0.8, 0.6, 0.9, 1.0]),
        ("Math: \u{2211} x\u{00B2}  \u{222B}\u{221E}  \u{221A}(-1)  \u{2200}\u{2203}  \u{2260}\u{2264}\u{2265}", text_scale, TextJustify::Left, [0.7, 0.7, 1.0, 1.0]),

        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("min", 0.4, TextJustify::Left, [0.6, 0.6, 0.6, 1.0]),
        ("Very very very very very very long single line to test overflow behavior", text_scale, TextJustify::Left, [0.7, 0.5, 0.5, 1.0]),
        ("", 0.0, TextJustify::Left, [0.0; 4]),
        ("Scroll with mouse wheel ...", 0.5, TextJustify::Right, [0.4, 0.4, 0.4, 1.0]),
    ];

    let mut sdf_entities: Vec<(hecs::Entity, f32, f32, TextJustify)> = Vec::new();
    for (text, font_scale, justify, color) in &lines {
        if text.is_empty() {
            cy += section_gap;
            continue;
        }
        let e = ui.spawn_sdf_text(
            &mut engine.world,
            text,
            *font_scale,
            panel_w - indent * 2.0,
            *color,
            *justify,
        );
        ui.container_add_child(
            &mut engine.world,
            container,
            e,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(e) {
            match *text {
                "Weight 0.0 — thinner strokes" => sdf.weight = 0.0,
                "Weight 0.15 — medium" => sdf.weight = 0.15,
                "Weight 0.3 — bolder strokes" => sdf.weight = 0.3,
                "Gamma 0.5 — sharper edges" => sdf.gamma = 0.5,
                "Gamma 2.5 — softer edges" => sdf.gamma = 2.5,
                "Thin outline (0.08)" => {
                    sdf.outline_width = 0.08;
                    sdf.outline_color = [0.1, 0.08, 0.0, 1.0];
                }
                "Thick outline (0.2)" => {
                    sdf.outline_width = 0.2;
                    sdf.outline_color = [0.1, 0.08, 0.0, 1.0];
                }
                "Blue glow" => {
                    sdf.outline_width = 0.25;
                    sdf.outline_color = [0.0, 0.3, 0.8, 1.0];
                }
                "Orange glow" => {
                    sdf.outline_width = 0.25;
                    sdf.outline_color = [0.8, 0.3, 0.0, 1.0];
                }
                "Drop shadow" => {
                    sdf.shadow_offset = [3.0, 3.0];
                    sdf.shadow_color = [0.0, 0.0, 0.0, 0.6];
                    sdf.shadow_blur = 0.05;
                }
                _ => {}
            }
        }
        sdf_entities.push((e, *font_scale, cy, *justify));
        let line_count = text.matches('\n').count() as f32 + 1.0;
        cy += line_count * font_scale.max(0.5) * line_h + line_gap * line_count;
    }
    let content_h = cy + 6.0;
    state.borrow_mut().text_demo_content_h = content_h;

    // Scrollbar thumb
    let thumb_w: f32 = 6.0;
    let thumb_e = ui.spawn_container(&mut engine.world, thumb_w, 30.0, [0.4, 0.4, 0.4, 0.8]);

    let ce = container;
    let thumb = thumb_e;

    engine.on_update(move |engine| {
        let Some(ref _ui) = engine.ui else { return };
        let vw = _ui.viewport_w;
        let vh = _ui.viewport_h;
        let px2 = vw - panel_w - border;
        let py2 = vh - panel_h - border;

        if let Ok(mut tf) = engine.world.get::<&mut Transform>(ce) {
            tf.position = Vec3::new(px2, py2, tf.position.z);
        }
        if let Ok(mut node) = engine.world.get::<&mut UiNode>(ce) {
            node.size = Vec2::new(panel_w, panel_h);
        }

        let sy = engine.world.get::<&UiNode>(ce).map(|n| n.scroll_y).unwrap_or(0.0);
        let max_scroll = (content_h - panel_h).max(0.0);

        let clip = Vec4::new(px2, py2, panel_w, panel_h);
        for &(sdf_e, _font_scale, base_y, justify) in &sdf_entities {
            let text_w = engine.world.get::<&UiNode>(sdf_e).map(|n| n.size.x).unwrap_or(0.0);
            let pos_x = match justify {
                TextJustify::Left => px2 + indent,
                TextJustify::Center => px2 + panel_w / 2.0 - text_w / 2.0,
                TextJustify::Right => px2 + panel_w - indent - text_w,
            };
            if let Ok(mut cn) = engine.world.get::<&mut UiNode>(sdf_e) {
                cn.clip_rect = clip;
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(sdf_e) {
                tf.position.x = pos_x;
                tf.position.y = py2 + base_y - sy;
            }
        }

        let thumb_h =
            if max_scroll > 0.0 { (panel_h / content_h * panel_h).max(20.0) } else { panel_h };
        let thumb_y_off =
            if max_scroll > 0.0 { (sy / max_scroll) * (panel_h - thumb_h - 4.0) } else { 0.0 };
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(thumb) {
            tf.position.x = px2 + panel_w - thumb_w - 2.0;
            tf.position.y = py2 + 2.0 + thumb_y_off;
        }
        if let Ok(mut tn) = engine.world.get::<&mut UiNode>(thumb) {
            tn.size = Vec2::new(thumb_w, thumb_h);
        }
    });

    state.borrow_mut().text_showcase_e = Some(container);
    engine.set_enabled(container, false);
}

/// Iso coord overlay — cardinal compass rose + XYZ axes + live iso coords.
/// Always visible, positioned top-left below the top bar, outside the UI tree.
pub fn init_iso_coord_overlay(engine: &mut Engine, state: &DemoStateRef) {
    log::info!("iso_debug: creating compass overlay (14 labels + GL lines)");
    let z_layer = -1500.0_f32;

    let mut spawn = |text: &str, scale: f32, color: [f32; 4]| -> hecs::Entity {
        engine.world.spawn((
            Transform::new(Vec3::new(0.0, 0.0, z_layer), Vec3::new(scale, scale, 1.0)),
            SdfTextRender {
                atlas_name: "dejavusans".into(),
                color,
                text: text.to_string(),
                ignore_cam: true,
                justify: TextJustify::Left,
                weight: 0.0,
                gamma: 1.0,
                bgcolor: [0.0; 4],
                outline_color: [0.0; 4],
                outline_width: 0.0,
                shadow_offset: [1.0, 1.0],
                shadow_color: [0.0, 0.0, 0.0, 0.5],
                shadow_blur: 0.0,
            },
            DebugName(format!("iso_debug_{}", text.replace([':', ' '], ""))),
        ))
    };

    let n_e = spawn("N", 1.2, [1.0, 1.0, 0.8, 1.0]);
    let e_e = spawn("E", 1.2, [1.0, 1.0, 0.8, 1.0]);
    let s_e = spawn("S", 1.2, [1.0, 1.0, 0.8, 1.0]);
    let w_e = spawn("W", 1.2, [1.0, 1.0, 0.8, 1.0]);

    let ne_e = spawn("NE", 0.8, [1.0, 1.0, 0.8, 0.5]);
    let se_e = spawn("SE", 0.8, [1.0, 1.0, 0.8, 0.5]);
    let sw_e = spawn("SW", 0.8, [1.0, 1.0, 0.8, 0.5]);
    let nw_e = spawn("NW", 0.8, [1.0, 1.0, 0.8, 0.5]);

    let ax_e = spawn("X", 1.2, [1.0, 0.2, 0.2, 1.0]);
    let ay_e = spawn("Y", 1.2, [0.2, 1.0, 0.2, 1.0]);
    let az_e = spawn("Z", 1.2, [0.2, 0.2, 1.0, 1.0]);

    let cx_e = spawn("X: 0.0", 1.0, [1.0, 0.3, 0.3, 1.0]);
    let cy_e = spawn("Y: 0.0", 1.0, [0.3, 1.0, 0.3, 1.0]);
    let cz_e = spawn("Z: 0", 1.0, [0.4, 0.4, 1.0, 1.0]);

    state.borrow_mut().iso_coord_x_e = Some(cx_e);
    state.borrow_mut().iso_coord_y_e = Some(cy_e);
    state.borrow_mut().iso_coord_z_e = Some(cz_e);

    // Build combined GL line buffer for compass rose.
    let r: f32 = 30.0;
    let al: f32 = 35.0;
    let si =
        |dtx: f32, dty: f32, s: f32| -> (f32, f32) { (s * (dtx + dty), s * (dty - dtx) / 2.0) };

    let mut verts: Vec<f32> = Vec::with_capacity(54);
    verts.extend_from_slice(&[-2.0 * r, 0.0, 0.0, 2.0 * r, 0.0, 0.0]);
    verts.extend_from_slice(&[0.0, -r, 0.0, 0.0, r, 0.0]);

    let (nx, ny) = si(0.0, -1.0, r);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, nx, ny, 0.0]);
    let (ex, ey) = si(1.0, 0.0, r);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, ex, ey, 0.0]);
    let (sx, sy) = si(0.0, 1.0, r);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, sx, sy, 0.0]);
    let (wx, wy) = si(-1.0, 0.0, r);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, wx, wy, 0.0]);

    let (axx, axy) = si(1.0, 0.0, al);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, axx, axy, 0.0]);
    let (ayx, ayy) = si(0.0, 1.0, al);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, ayx, ayy, 0.0]);
    verts.extend_from_slice(&[0.0, 0.0, 0.0, 0.0, -al, 0.0]);

    if let Some(ref gfx) = engine.gfx {
        state.borrow_mut().iso_compass_buf =
            Some(GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STATIC_DRAW));
    }

    // on_update: reposition labels + update coord text every frame
    let tilemap_name = "tilemap".to_string();
    let coord_x = cx_e;
    let coord_y = cy_e;
    let coord_z = cz_e;

    engine.on_update(move |engine| {
        let Some(&tm_entity) = engine.names.get(&tilemap_name) else { return };
        let Ok(tm) = engine.world.get::<&Tilemap>(tm_entity) else { return };

        let mx = tm.mouse_iso_pos.x;
        let my = tm.mouse_iso_pos.y;
        let h = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, mx, my);

        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_x) {
            sdf.text = format!("X: {:.1}", mx);
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_y) {
            sdf.text = format!("Y: {:.1}", my);
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_z) {
            sdf.text = format!("Z: {:.0}", h * tm.height_scale);
        }

        let cx: f32 = 100.0;
        let cy: f32 = 155.0;
        let ax_ox: f32 = 220.0;
        let ax_oy: f32 = 155.0;
        let coord_x_pos: f32 = 340.0;
        let coord_y_base: f32 = 130.0;
        let gap: f32 = 22.0;

        let set_pos = |e: hecs::Entity, x: f32, y: f32| {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                tf.position = Vec3::new(x, y, z_layer);
            }
        };

        set_pos(n_e, cx - 30.0 - 14.0, cy - 15.0 - 26.0);
        set_pos(e_e, cx + 30.0 + 6.0, cy - 15.0 - 10.0);
        set_pos(s_e, cx + 30.0 + 7.0, cy + 15.0 + 2.0);
        set_pos(w_e, cx - 30.0 - 35.0, cy + 15.0 - 10.0);

        set_pos(ne_e, cx - 6.0, cy - 30.0 - 14.0 - 6.0);
        set_pos(se_e, cx + 60.0 + 6.0, cy - 7.0);
        set_pos(sw_e, cx - 6.0, cy + 30.0 + 6.0);
        set_pos(nw_e, cx - 60.0 - 6.0 - 22.0, cy - 7.0);

        set_pos(ax_e, ax_ox + 35.0 + 8.0, ax_oy - 17.5 - 10.0);
        set_pos(ay_e, ax_ox + 35.0 + 8.0, ax_oy + 17.5 - 10.0);
        set_pos(az_e, ax_ox - 7.0, ax_oy - 35.0 - 20.0 - 10.0);

        set_pos(coord_x, coord_x_pos, coord_y_base);
        set_pos(coord_y, coord_x_pos, coord_y_base + gap);
        set_pos(coord_z, coord_x_pos, coord_y_base + gap * 2.0);

        classic_core::cl_first!(
            Chan::Iso,
            5,
            log::Level::Info,
            "iso overlay: compass=({},{}), coords=({},{})  r={} al={}",
            cx,
            cy,
            coord_x_pos,
            coord_y_base,
            r,
            al,
        );
    });
}

/// Debug overlay: footprint polygons + anchor crosshairs + agent selection
/// ring.  Registered on the engine's overlay hook.
pub fn draw_debug_overlay(engine: &mut Engine, state: &DemoStateRef) {
    // Footprints + agent ring, gated on the demo's footprint toggle.
    if state.borrow().editor.debug_footprints {
        // Resolve roles before mutably borrowing gfx (entity_by_role borrows &Engine).
        let tm_entity = engine.entity_by_role(classic_core::RoleKind::Tilemap);
        let agent_entity = engine.entity_by_role(classic_core::RoleKind::Agent);
        let Some(gfx) = engine.gfx.as_mut() else { return };
        let cam = engine.camera.matrix();
        let x_cross: [f32; 12] = [-8.0, -8.0, 0.0, 8.0, 8.0, 0.0, -8.0, 8.0, 0.0, 8.0, -8.0, 0.0];
        let x_cross_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &x_cross, glow::STATIC_DRAW);

        if let Some(tm_e) = tm_entity {
            let (iso_to_cart_world, tilemap_pos, size_x, size_y, hd, hs) = {
                let tm = engine.world.get::<&Tilemap>(tm_e).unwrap();
                let tm_tf = engine.world.get::<&Transform>(tm_e).unwrap();
                let iso_to_cart = iso_to_cartesian_4() * Mat4::from_scale(tm_tf.scale);
                (
                    iso_to_cart,
                    tm_tf.position,
                    tm.size_x,
                    tm.size_y,
                    tm.height_data.clone(),
                    tm.height_scale,
                )
            };
            for (_e, (iso_sprite, tf)) in engine.world.query::<(&IsoSprite, &Transform)>().iter() {
                let mut world_fp: Vec<f32> = Vec::with_capacity(iso_sprite.footprint.len() * 3);
                for pt in &iso_sprite.footprint {
                    let px = tf.position.x + pt.x;
                    let py = tf.position.y + pt.y;
                    let h = bilinear_height(&hd, size_x, size_y, px, py);

                    let mut v = Vec3::new(px, py, 0.0);
                    v = iso_to_cart_world.transform_point3(v);
                    v += tilemap_pos;
                    v.y -= h * hs;
                    world_fp.extend_from_slice(&[v.x, v.y, v.z]);
                }

                if world_fp.is_empty() {
                    continue;
                }
                let fp_buf =
                    GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &world_fp, glow::STREAM_DRAW);
                let vcount = (world_fp.len() / 3) as i32;

                gfx.draw_line_loop(&fp_buf, vcount, &Mat4::IDENTITY, &cam, &[0.0, 1.0, 0.5, 0.7]);

                let ax = tf.position.x;
                let ay = tf.position.y;
                let ah = bilinear_height(&hd, size_x, size_y, ax, ay);

                let mut anchor_world = Vec3::new(ax, ay, 0.0);
                anchor_world = iso_to_cart_world.transform_point3(anchor_world);
                anchor_world += tilemap_pos;
                anchor_world.y -= ah * hs;

                let anchor_m = Mat4::from_translation(anchor_world);
                gfx.draw_line_strip(&x_cross_buf, 0, 2, &anchor_m, &cam, &[1.0, 0.0, 1.0, 0.9]);
                gfx.draw_line_strip(&x_cross_buf, 2, 2, &anchor_m, &cam, &[1.0, 0.0, 1.0, 0.9]);
            }

            // Selection ring around selected agent (yellow diamond).
            if engine.agent_selected {
                if let Some(agent_e) = agent_entity {
                    if let Ok(agent_tf) = engine.world.get::<&Transform>(agent_e) {
                        let pos = agent_tf.position;
                        let ring_iso: [(f32, f32); 4] = [
                            (pos.x - 1.0, pos.y),
                            (pos.x, pos.y - 1.0),
                            (pos.x + 1.0, pos.y),
                            (pos.x, pos.y + 1.0),
                        ];
                        let mut ring_verts: Vec<f32> = Vec::with_capacity(12);
                        for &(ix, iy) in &ring_iso {
                            let mut v = Vec3::new(ix, iy, 0.0);
                            v = iso_to_cart_world.transform_point3(v);
                            v += tilemap_pos;
                            let h = bilinear_height(&hd, size_x, size_y, ix, iy);
                            v.y -= h * hs;
                            ring_verts.extend_from_slice(&[v.x, v.y, v.z]);
                        }
                        let rb = GlBuffer::from_slice(
                            &gfx.gl,
                            glow::ARRAY_BUFFER,
                            &ring_verts,
                            glow::STREAM_DRAW,
                        );
                        gfx.draw_line_loop(&rb, 4, &Mat4::IDENTITY, &cam, &[1.0, 1.0, 0.0, 0.8]);
                    }
                }
            }
        }
    }

    // Iso compass rose (always visible).
    let sguard = state.borrow();
    if let Some(buf) = sguard.iso_compass_buf.as_ref() {
        let Some(gfx) = engine.gfx.as_mut() else { return };
        let cx: f32 = 100.0;
        let cy: f32 = 155.0;
        let ax_ox: f32 = 220.0;
        let model = Mat4::from_translation(Vec3::new(cx, cy, -1500.0));
        let ax_model = Mat4::from_translation(Vec3::new(ax_ox, cy, -1500.0));
        let gcol = [0.6, 0.6, 0.5, 0.4];
        let scol = [1.0, 1.0, 0.8, 0.85];
        gfx.draw_line_strip(buf, 0, 2, &model, &Mat4::IDENTITY, &gcol);
        gfx.draw_line_strip(buf, 2, 2, &model, &Mat4::IDENTITY, &gcol);
        for i in 0..4 {
            gfx.draw_line_strip(buf, 4 + i * 2, 2, &model, &Mat4::IDENTITY, &scol);
        }
        gfx.draw_line_strip(buf, 12, 2, &ax_model, &Mat4::IDENTITY, &[1.0, 0.2, 0.2, 1.0]);
        gfx.draw_line_strip(buf, 14, 2, &ax_model, &Mat4::IDENTITY, &[0.2, 1.0, 0.2, 1.0]);
        gfx.draw_line_strip(buf, 16, 2, &ax_model, &Mat4::IDENTITY, &[0.2, 0.2, 1.0, 1.0]);
    }
}
