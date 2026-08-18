//! Editor HUD and tool prefabs: menu/button panel, height/tile/nav palettes,
//! widget visibility dispatch, and the selection-paint routine.
//!
//! These are demo content — they build the editor UI and drive tile/height/nav
//! edits through the engine's generic `rebuild_*` / `sync_nav_heights`
//! primitives.  They share `DemoState` via `Rc<RefCell<DemoState>>`.

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::components::{NavMesh, SdfTextRender, TextJustify, Tilemap, Transform, UiAnchor};
use classic_core::instrument::Chan;
use classic_engine::ui;
use classic_engine::Engine;

use crate::state::{DemoStateRef, EditorState};

/// Spawn HUD text entities using the UI layout system.
pub fn init_ui(engine: &mut Engine) {
    let vp_w = 1280.0_f32;
    let vp_h = 720.0_f32;
    let mut ui = ui::UIManager::new(vp_w, vp_h, &mut engine.world);

    // Top bar
    let top_bar = ui.spawn_container(&mut engine.world, vp_w, 68.0, [0.0, 0.0, 0.0, 0.5]);
    ui.root_add_child(&mut engine.world, top_bar, UiAnchor::TopCenter, UiAnchor::TopCenter);

    // FPS counter (left)
    let fps_text = ui.spawn_sdf_text(
        &mut engine.world,
        "0",
        1.4,
        100.0,
        [0.0, 0.6, 0.0, 1.0],
        TextJustify::Left,
    );
    if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(fps_text) {
        sdf.weight = 0.15;
    }
    ui.container_add_child(
        &mut engine.world,
        top_bar,
        fps_text,
        UiAnchor::MidLeft,
        UiAnchor::MidLeft,
    );

    // Banner (center)
    let banner = ui.spawn_sdf_text(
        &mut engine.world,
        "CLASSIC-ISO",
        1.5,
        600.0,
        [1.0, 0.53, 0.3, 1.0],
        TextJustify::Center,
    );
    if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(banner) {
        sdf.outline_color = [0.1, 0.05, 0.0, 1.0];
        sdf.outline_width = 0.12;
    }
    ui.container_add_child(
        &mut engine.world,
        top_bar,
        banner,
        UiAnchor::MidCenter,
        UiAnchor::MidCenter,
    );

    // Info text (right)
    let info = ui.spawn_sdf_text(
        &mut engine.world,
        "WASD MOVE\nSCROLL ZOOM",
        1.0,
        300.0,
        [1.0, 0.2, 0.6, 1.0],
        TextJustify::Right,
    );
    ui.container_add_child(
        &mut engine.world,
        top_bar,
        info,
        UiAnchor::MidRight,
        UiAnchor::MidRight,
    );

    // FPS update closure
    let fps_e = fps_text;
    engine.on_update(move |engine| {
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(fps_e) {
            let fps = engine.time.fps;
            sdf.text = fps.to_string();
            sdf.color = if fps >= 30 { [0.0, 0.6, 0.0, 1.0] } else { [0.8, 0.0, 0.0, 1.0] };
        }
    });

    // Refresh layout driven by frame() after resize + before physics.
    engine.ui = Some(ui);
}

/// Spawn the DEV button tool panel with slide-out menu, agent selector,
/// and backdrop for click-outside-to-close.
#[allow(clippy::too_many_lines)]
pub fn init_tool_buttons(engine: &mut Engine, state: &DemoStateRef) {
    use classic_core::components::UiAlign;

    let btn_size: f32 = 128.0;
    let agent_size: f32 = 64.0;
    let menu_item_h: f32 = 28.0;
    let menu_padding: f32 = 6.0;
    let menu_gap: f32 = 2.0;
    let menu_font_scale: f32 = 0.45;
    let menu_panel_gap: f32 = 0.0;
    let agent_pad: f32 = 8.0;

    // Built as a Vec rather than a fixed array so the lunar entry can be
    // conditional: adding it unconditionally would change the menu height
    // (and therefore the demo scene's layout and golden baseline) for a row
    // that does nothing outside the generated scene.
    let menu_targets: Vec<(&str, &str)> = vec![
        ("Tile Editor", "tilemap"),
        ("Nav Editor", "navMesh"),
        ("Height Editor", "height"),
        ("Light Config", "light"),
        ("Footprints", "_footprints"),
        ("Text Demo", "textDemo"),
    ];

    let max_label_len = menu_targets.iter().map(|m| m.0.len()).max().unwrap_or(12);
    let glyph_w = 18.0_f32;
    let menu_w = max_label_len as f32 * glyph_w + menu_padding * 2.0;
    let n = menu_targets.len() as f32;
    let menu_h = n * menu_item_h + menu_gap * (n - 1.0) + menu_padding * 2.0;

    let editor_rc = Rc::new(RefCell::new(EditorState::default()));

    // Spawn all UI entities inside a block so the ui borrow is released
    // before calling set_enabled (which borrows engine).
    let (agent_btn, btn_arr, menu_panel, backdrop, item_rows);
    {
        let Some(ref mut ui) = engine.ui else { return };

        // Transparent vertical array: agent on top, dev below, center-aligned
        let btn_array = ui.spawn_array(
            &mut engine.world,
            true,
            UiAlign::Center,
            agent_pad,
            [0.0, 0.0, 0.0, 0.0],
        );

        // Agent [A] button
        let ag;
        {
            let es = editor_rc.clone();
            ag = ui.spawn_button(
                &mut engine.world,
                &mut engine.physics,
                agent_size,
                agent_size,
                [0.1, 0.6, 0.1, 0.8],
                ui::ButtonOptions {
                    text: Some("A".into()),
                    text_scale: 0.4,
                    sdf_text: true,
                    hover: true,
                    click_priority: 1,
                    click_action: Some(Box::new(move || {
                        let mut s = es.borrow_mut();
                        s.agent_selected = !s.agent_selected;
                        s.target = "none".into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        agent_btn = ag;
        ui.container_add_child(
            &mut engine.world,
            btn_array,
            ag,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        // DEV button sprite
        let dev;
        {
            let es = editor_rc.clone();
            dev = ui.spawn_button(
                &mut engine.world,
                &mut engine.physics,
                btn_size,
                btn_size,
                [0.0, 0.0, 0.0, 0.0],
                ui::ButtonOptions {
                    sprite: Some("editorIcons".into()),
                    sprite_frame: 0.0,
                    sprite_tile_set: [4.0, 4.0],
                    hover: true,
                    click_action: Some(Box::new(move || {
                        let mut s = es.borrow_mut();
                        s.panel_menu_open = !s.panel_menu_open;
                        if !s.panel_menu_open {
                            s.target = "none".into();
                        }
                        s.agent_selected = false;
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        ui.container_add_child(
            &mut engine.world,
            btn_array,
            dev,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        // Menu panel
        menu_panel = ui.spawn_container(&mut engine.world, menu_w, menu_h, [0.1, 0.1, 0.1, 0.95]);

        // Menu item rows
        let mut rows: Vec<(hecs::Entity, usize)> = Vec::new();
        for (idx, (label, target)) in menu_targets.iter().enumerate() {
            let row_w = menu_w - menu_padding * 2.0;
            let t_str = (*target).to_string();
            let es = editor_rc.clone();

            let click_fn: Box<dyn FnMut() -> bool> = if t_str == "_footprints" {
                Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.debug_footprints = !s.debug_footprints;
                    s.panel_menu_open = false;
                    true
                })
            } else {
                Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.target = if s.target == t_str { "none".into() } else { t_str.clone() };
                    s.agent_selected = false;
                    s.panel_menu_open = false;
                    true
                })
            };

            let row = ui.spawn_button(
                &mut engine.world,
                &mut engine.physics,
                row_w,
                menu_item_h,
                [0.15, 0.15, 0.15, 1.0],
                ui::ButtonOptions {
                    text: Some((*label).into()),
                    text_scale: menu_font_scale,
                    sdf_text: true,
                    click_priority: 3,
                    hover: true,
                    click_action: Some(click_fn),
                    ..Default::default()
                },
            );
            ui.container_add_child(
                &mut engine.world,
                menu_panel,
                row,
                UiAnchor::TopLeft,
                UiAnchor::TopLeft,
            );
            rows.push((row, idx));
        }
        item_rows = rows;

        // Set initial menu position to avoid 1-frame glitch on first open.
        // The on_update closure repositions to actual viewport on every frame.
        {
            let init_vh: f32 = 720.0;
            let init_mx = btn_size;
            let init_my = init_vh - btn_size - menu_panel_gap - menu_h;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(menu_panel) {
                tf.position = glam::Vec3::new(init_mx, init_my, -1100.0);
            }
            let mut row_y = init_my + menu_padding;
            for (row_e, _) in &item_rows {
                if let Ok(mut tf) = engine.world.get::<&mut Transform>(*row_e) {
                    tf.position = glam::Vec3::new(init_mx + menu_padding, row_y, -1100.0);
                }
                if let Ok(node) = engine.world.get::<&classic_core::components::UiNode>(*row_e) {
                    for child in &node.children {
                        if let Ok(mut tf) = engine.world.get::<&mut Transform>(child.entity) {
                            tf.position.z = -1100.0;
                        }
                    }
                }
                row_y += menu_item_h + menu_gap;
            }
        }

        // Backdrop with click handler at lowest priority
        let bd;
        {
            let es = editor_rc.clone();
            bd = ui.spawn_container(&mut engine.world, 800.0, 600.0, [0.0, 0.0, 0.0, 0.01]);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(bd) {
                tf.position.z = -1050.0;
            }
            let bp_pid = ui.add_collider_to_elem(&mut engine.world, bd, &mut engine.physics);
            engine.physics.set_collider_consumes_click(bp_pid, true);
            engine.physics.set_collider_click_priority(bp_pid, -1);
            engine.physics.add_collider_handler(
                bp_pid,
                classic_core::collision::HandlerKind::Click,
                move || {
                    es.borrow_mut().panel_menu_open = false;
                    true
                },
            );
        }
        backdrop = bd;
        btn_arr = btn_array;
    }

    // Now safe to call set_enabled (ui borrow released)
    engine.set_enabled(menu_panel, false);
    engine.set_enabled(backdrop, false);
    state.borrow_mut().menu_panel_e = Some(menu_panel);

    let items = item_rows.clone();
    let targets: Vec<String> = menu_targets.iter().map(|t| t.1.to_string()).collect();
    let editor_rc_clone = editor_rc.clone();
    let state = Rc::clone(state);

    // Per-frame: position elements, sync Rc state → engine, toggle visibility
    engine.on_update(move |engine| {
        // Sync shared state → demo (before ui borrow)
        {
            let es = editor_rc_clone.borrow();
            {
                let mut s = state.borrow_mut();
                s.editor.target = es.target.clone();
                s.editor.debug_footprints = es.debug_footprints;
                s.editor.panel_menu_open = es.panel_menu_open;
                s.editor.agent_selected = es.agent_selected;
            }
            engine.agent_selected = es.agent_selected;
            let open = es.panel_menu_open;
            drop(es);
            engine.set_enabled(menu_panel, open);
            engine.set_enabled(backdrop, open);
        }

        let Some(ref mut ui) = engine.ui else { return };
        let vw = ui.viewport_w;
        let vh = ui.viewport_h;

        // Position button array (agent above dev, centered on X)
        let arr_x = btn_size / 2.0;
        let arr_y = vh - btn_size * 0.5 - agent_size - btn_size - agent_pad;
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(btn_arr) {
            tf.position = glam::Vec3::new(arr_x, arr_y, tf.position.z);
        }
        ui.layout_standalone(btn_arr, &mut engine.world);

        // Update agent button color based on selection state
        let ag_color: [f32; 4] = if editor_rc_clone.borrow().agent_selected {
            [0.1, 0.6, 0.1, 0.8]
        } else {
            [0.3, 0.3, 0.3, 0.6]
        };
        ui.set_button_base_color(agent_btn, ag_color);

        // Position menu panel
        let m_x = btn_size;
        let m_y = vh - btn_size - menu_panel_gap - menu_h;
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(menu_panel) {
            tf.position = glam::Vec3::new(m_x, m_y, -1100.0);
        }

        // Position menu item rows
        let mut row_y = m_y + menu_padding;
        for (row_e, _) in &items {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(*row_e) {
                tf.position = glam::Vec3::new(m_x + menu_padding, row_y, -1100.0);
            }
            if let Ok(node) = engine.world.get::<&classic_core::components::UiNode>(*row_e) {
                for child in &node.children {
                    if let Ok(mut tf) = engine.world.get::<&mut Transform>(child.entity) {
                        tf.position.z = -1100.0;
                    }
                }
            }
            ui::UIManager::position_children_of(*row_e, &mut engine.world);
            row_y += menu_item_h + menu_gap;
        }

        // Backdrop
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(backdrop) {
            tf.position = glam::Vec3::new(0.0, 0.0, tf.position.z);
        }
        if let Ok(mut node) = engine.world.get::<&mut classic_core::components::UiNode>(backdrop) {
            node.size.x = vw;
            node.size.y = vh;
        }

        // Active-tool highlighting on menu item rows
        for (row_e, idx) in &items {
            let target = &targets[*idx];
            let color: [f32; 4] = if target == "_footprints" {
                if state.borrow().editor.debug_footprints {
                    [0.2, 0.35, 0.6, 1.0]
                } else {
                    [0.15, 0.15, 0.15, 1.0]
                }
            } else if state.borrow().editor.target == *target {
                [0.2, 0.35, 0.6, 1.0]
            } else {
                [0.15, 0.15, 0.15, 1.0]
            };
            ui.set_button_base_color(*row_e, color);
        }
    });
}

/// Height editing widget: +/- buttons for height delta and scale multiplier,
/// plus a set/blend mode toggle.
#[allow(clippy::too_many_lines)]
pub fn init_height_widget(engine: &mut Engine, state: &DemoStateRef) {
    let btn_sz: f32 = 28.0;
    let label_w: f32 = 60.0;
    let gap: f32 = 4.0;
    let row_h: f32 = btn_sz;
    let widget_w: f32 = gap * 4.0 + btn_sz * 2.0 + label_w;
    let widget_h: f32 = row_h * 3.0 + gap * 4.0;
    let _border: f32 = 0.0;

    let editor_rc = Rc::new(RefCell::new(EditorState::default()));

    let Some(ref mut ui) = engine.ui else { return };

    let container = ui.spawn_container(&mut engine.world, widget_w, widget_h, [0.0, 0.0, 0.0, 0.4]);

    // Row 1: height value +/-
    let h_minus;
    {
        let es = editor_rc.clone();
        h_minus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.6, 0.1, 0.1, 1.0],
            ui::ButtonOptions {
                text: Some("-".into()),
                text_scale: 0.5,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    es.borrow_mut().height -= 1;
                    true
                })),
                ..Default::default()
            },
        );
    }
    let h_label = ui.spawn_sdf_text(
        &mut engine.world,
        "0",
        1.0,
        200.0,
        [1.0, 1.0, 1.0, 1.0],
        TextJustify::Center,
    );
    let h_plus;
    {
        let es = editor_rc.clone();
        h_plus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.1, 0.6, 0.1, 1.0],
            ui::ButtonOptions {
                text: Some("+".into()),
                text_scale: 0.5,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    es.borrow_mut().height += 1;
                    true
                })),
                ..Default::default()
            },
        );
    }

    // Row 2: scale multiplier s-/s+
    let s_minus;
    {
        let es = editor_rc.clone();
        s_minus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.1, 0.1, 0.6, 1.0],
            ui::ButtonOptions {
                text: Some("s-".into()),
                text_scale: 0.4,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.height_scale = (s.height_scale - 1).max(1);
                    true
                })),
                ..Default::default()
            },
        );
    }
    let s_label = ui.spawn_sdf_text(
        &mut engine.world,
        "x1",
        0.9,
        200.0,
        [1.0, 1.0, 1.0, 1.0],
        TextJustify::Center,
    );
    let s_plus;
    {
        let es = editor_rc.clone();
        s_plus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.1, 0.1, 0.6, 1.0],
            ui::ButtonOptions {
                text: Some("s+".into()),
                text_scale: 0.4,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    es.borrow_mut().height_scale += 1;
                    true
                })),
                ..Default::default()
            },
        );
    }

    // Row 3: set/blend mode toggle
    let mode_btn;
    {
        let es = editor_rc.clone();
        mode_btn = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            widget_w - gap * 2.0,
            row_h,
            [0.2, 0.2, 0.2, 1.0],
            ui::ButtonOptions {
                text: Some("blend".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.height_mode =
                        if s.height_mode == "set" { "blend".into() } else { "set".into() };
                    true
                })),
                ..Default::default()
            },
        );
    }

    // Wire children to container so set_enabled propagates.
    ui.add_children(
        &mut engine.world,
        container,
        &[h_minus, h_plus, s_minus, s_plus, mode_btn, h_label, s_label],
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    state.borrow_mut().height_widget_e = Some(container);
    engine.set_enabled(container, false);

    let con_e = container;
    let h_min_e = h_minus;
    let h_pl_e = h_plus;
    let h_lb_e = h_label;
    let s_mi_e = s_minus;
    let s_pl_e = s_plus;
    let s_lb_e = s_label;
    let md_e = mode_btn;
    let editor_rc_clone = editor_rc.clone();
    let state = Rc::clone(state);

    engine.on_update(move |engine| {
        let Some(ref _ui) = engine.ui else { return };
        let cw = _ui.viewport_w;
        let ch = _ui.viewport_h;
        let x0 = cw - _border - widget_w;
        let y0 = ch - _border - widget_h;
        let cx = gap;
        let cy1 = gap;
        let cy2 = row_h + gap * 2.0;
        let cy3 = row_h * 2.0 + gap * 3.0;

        // Sync Rc state → demo
        {
            let es = editor_rc_clone.borrow();
            let mut s = state.borrow_mut();
            s.editor.height = es.height;
            s.editor.height_scale = es.height_scale;
            s.editor.height_mode = es.height_mode.clone();
        }

        // Apply height scale to tilemap when it changes.  Relative to the
        // scale the mesh was actually built with, not to the tile pixel
        // size (which generated scenes deliberately do not use).
        let prev_hs = editor_rc_clone.borrow().height_scale;
        let base_hs = engine.base_height_scale;
        if let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) {
            if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(e) {
                tm.height_scale = base_hs * prev_hs as f32;
            }
        }

        if let Ok(mut tf) = engine.world.get::<&mut Transform>(con_e) {
            tf.position = glam::Vec3::new(x0, y0, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_min_e) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy1, tf.position.z);
        }
        ui::UIManager::position_children_of(h_min_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_lb_e) {
            tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy1, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_pl_e) {
            tf.position =
                glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy1, tf.position.z);
        }
        ui::UIManager::position_children_of(h_pl_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_mi_e) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy2, tf.position.z);
        }
        ui::UIManager::position_children_of(s_mi_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_lb_e) {
            tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy2, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_pl_e) {
            tf.position =
                glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy2, tf.position.z);
        }
        ui::UIManager::position_children_of(s_pl_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(md_e) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy3, tf.position.z);
        }
        ui::UIManager::position_children_of(md_e, &mut engine.world);

        // Update labels
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(h_lb_e) {
            sdf.text = state.borrow().editor.height.to_string();
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(s_lb_e) {
            sdf.text = format!("x{}", state.borrow().editor.height_scale);
        }
        if let Ok(node) = engine.world.get::<&classic_core::components::UiNode>(md_e) {
            if let Some(child) = node.children.first() {
                if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(child.entity) {
                    sdf.text = state.borrow().editor.height_mode.clone();
                }
            }
        }
    });
}

/// Toggle visibility of tool panels based on `editor_target`.
pub fn init_editor_mode_control(engine: &mut Engine, state: &DemoStateRef) {
    let state = Rc::clone(state);
    engine.on_update(move |engine| {
        let target = state.borrow().editor.target.clone();
        if let Some(e) = state.borrow().tile_palette_e {
            engine.set_enabled(e, target == "tilemap");
        }
        if let Some(e) = state.borrow().nav_palette_e {
            engine.set_enabled(e, target == "navMesh");
        }
        if let Some(e) = state.borrow().height_widget_e {
            engine.set_enabled(e, target == "height");
        }
        if let Some(e) = state.borrow().light_widget_e {
            engine.set_enabled(e, target == "light");
        }
        if let Some(e) = state.borrow().text_showcase_e {
            engine.set_enabled(e, target == "textDemo");
        }
        if let Some(e) = engine.entity_by_role(classic_core::RoleKind::NavMesh) {
            engine.set_enabled(e, target == "navMesh");
        }
    });
}

/// Tile palette: shows the tileset texture with click-to-select and a selector overlay.
pub fn init_tile_palette(engine: &mut Engine, state: &DemoStateRef) {
    let Some(tm_entity) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return };
    // Read the tileset name from the component rather than hardcoding
    // "tileSet", so a scene using a different (or procedurally generated)
    // tileset gets a palette showing its own tiles.
    let (tile_px, tile_py, max_tile, tiles_per_row, tile_set) = {
        let Ok(tm) = engine.world.get::<&Tilemap>(tm_entity) else {
            return;
        };
        (
            tm.tile_pixel_size[0],
            tm.tile_pixel_size[1],
            tm.max_tile,
            tm.tiles_per_row,
            tm.tile_set.clone(),
        )
    };
    let Some(ref mut ui) = engine.ui else { return };

    let ts_pixel = [tile_px * tiles_per_row, tile_py * tiles_per_row];
    let palette_w = ts_pixel[0] as f32;
    let palette_h = ts_pixel[1] as f32;
    let t_size = [tile_px as f32, tile_py as f32];

    let container =
        ui.spawn_container(&mut engine.world, palette_w, palette_h, [0.0, 0.0, 0.0, 0.2]);
    let sprite =
        ui.spawn_sprite(&mut engine.world, &tile_set, palette_w, palette_h, 0.0, [1.0, 1.0]);
    ui.container_add_child(
        &mut engine.world,
        container,
        sprite,
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    let selector =
        ui.spawn_container(&mut engine.world, t_size[0], t_size[1], [1.0, 1.0, 1.0, 0.3]);
    ui.container_add_child(
        &mut engine.world,
        container,
        selector,
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    let pid = ui.add_collider_to_elem(&mut engine.world, container, &mut engine.physics);
    engine.physics.set_collider_consumes_click(pid, true);
    // Dummy click handler so consumes_click triggers consumed_click in perform_calls
    engine.physics.add_collider_handler(pid, classic_core::collision::HandlerKind::Click, || true);

    let cp_e = container;
    let sel_e = selector;
    let local_x = Rc::new(std::cell::Cell::new(0u32));
    let local_y = Rc::new(std::cell::Cell::new(0u32));
    let lx2 = local_x.clone();
    let ly2 = local_y.clone();
    let s = Rc::clone(state);

    engine.on_update(move |engine| {
        let Some(ref _ui) = engine.ui else { return };
        if s.borrow().editor.target != "tilemap" {
            return;
        }
        let vw = _ui.viewport_w;
        let vh = _ui.viewport_h;
        let border: f32 = 10.0;
        let px = vw - palette_w - border;
        let py = vh - palette_h - border;
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(cp_e) {
            tf.position = glam::Vec3::new(px, py, tf.position.z);
        }
        ui::UIManager::position_children_of(cp_e, &mut engine.world);

        if engine.input.was_mouse_pressed(0) {
            let mx = engine.input.mouse_pos.x;
            let my = engine.input.mouse_pos.y;
            if mx >= px && mx <= px + palette_w && my >= py && my <= py + palette_h {
                let lx = ((mx - px) / t_size[0]).floor() as u32;
                let ly = ((my - py) / t_size[1]).floor() as u32;
                let tile_idx = lx + ly * tiles_per_row;
                s.borrow_mut().editor.tile = tile_idx.min(max_tile);
                lx2.set(lx);
                ly2.set(ly);
            }
        }

        let sel_x = px + lx2.get() as f32 * t_size[0];
        let sel_y = py + ly2.get() as f32 * t_size[1];
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(sel_e) {
            tf.position = glam::Vec3::new(sel_x, sel_y, tf.position.z);
        }
    });

    state.borrow_mut().tile_palette_e = Some(cp_e);
    engine.set_enabled(cp_e, false);
}

/// Nav palette: shows the nav tileset at 4x scale with click-to-select.
pub fn init_nav_palette(engine: &mut Engine, state: &DemoStateRef) {
    let max_tile: u32 = 2;
    let tiles_per_row: u32 = 2;
    let Some(ref mut ui) = engine.ui else { return };

    let nav_tile_px: f32 = 8.0;
    let ui_scale: f32 = 4.0;
    let tex_h = engine
        .gfx
        .as_ref()
        .and_then(|g| g.textures.get("navTileset"))
        .map(|t| t.size.1 as f32)
        .unwrap_or(16.0);
    let palette_w = nav_tile_px * tiles_per_row as f32 * ui_scale;
    let palette_h = tex_h * ui_scale;
    let ts = [nav_tile_px * ui_scale, nav_tile_px * ui_scale];

    let container =
        ui.spawn_container(&mut engine.world, palette_w, palette_h, [0.0, 0.0, 0.0, 0.2]);
    let sprite =
        ui.spawn_sprite(&mut engine.world, "navTileset", palette_w, palette_h, 0.0, [1.0, 1.0]);
    ui.container_add_child(
        &mut engine.world,
        container,
        sprite,
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    let selector = ui.spawn_container(&mut engine.world, ts[0], ts[1], [1.0, 1.0, 1.0, 0.3]);
    ui.container_add_child(
        &mut engine.world,
        container,
        selector,
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    let cp_e = container;
    let sel_e = selector;
    let local_x = Rc::new(std::cell::Cell::new(0u32));
    let local_y = Rc::new(std::cell::Cell::new(0u32));
    let lx2 = local_x.clone();
    let ly2 = local_y.clone();

    let pid = ui.add_collider_to_elem(&mut engine.world, cp_e, &mut engine.physics);
    engine.physics.set_collider_consumes_click(pid, true);
    engine.physics.add_collider_handler(pid, classic_core::collision::HandlerKind::Click, || true);

    let s = Rc::clone(state);
    engine.on_update(move |engine| {
        let Some(ref _ui) = engine.ui else { return };
        if s.borrow().editor.target != "navMesh" {
            return;
        }
        let vw = _ui.viewport_w;
        let vh = _ui.viewport_h;
        let border: f32 = 10.0;
        let px = vw - palette_w - border;
        let py = vh - palette_h - border;
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(cp_e) {
            tf.position = glam::Vec3::new(px, py, tf.position.z);
        }
        ui::UIManager::position_children_of(cp_e, &mut engine.world);

        if engine.input.was_mouse_pressed(0) {
            let mx = engine.input.mouse_pos.x;
            let my = engine.input.mouse_pos.y;
            if mx >= px && mx <= px + palette_w && my >= py && my <= py + palette_h {
                let lx = ((mx - px) / ts[0]).floor() as u32;
                let ly = ((my - py) / ts[1]).floor() as u32;
                s.borrow_mut().editor.nav_tile = (lx + ly * tiles_per_row).min(max_tile);
                lx2.set(lx);
                ly2.set(ly);
            }
        }

        let sel_x = px + lx2.get() as f32 * ts[0];
        let sel_y = py + ly2.get() as f32 * ts[1];
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(sel_e) {
            tf.position = glam::Vec3::new(sel_x, sel_y, tf.position.z);
        }
    });

    state.borrow_mut().nav_palette_e = Some(container);
    engine.set_enabled(container, false);
}

/// Paint tiles or heights in the selection region after a drag ends.
pub fn apply_editor_selection(engine: &mut Engine, state: &DemoStateRef) {
    let target = state.borrow().editor.target.clone();
    let height = state.borrow().editor.height;
    let height_mode = state.borrow().editor.height_mode.clone();
    let tile = state.borrow().editor.tile;
    let nav_tile = state.borrow().editor.nav_tile;

    let Some(tm_entity) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return };
    let (bx, by, ex, ey, tile_count) = {
        let tm = match engine.world.get::<&Tilemap>(tm_entity) {
            Ok(t) => t,
            Err(_) => {
                classic_core::cl_info!(
                    Chan::Editor,
                    "apply_editor_selection: no Tilemap component on entity"
                );
                return;
            }
        };
        let b = tm.selection_iso_begin;
        let e = tm.selection_iso_end;
        let from_x = b.x.min(e.x).floor().max(0.0) as i32;
        let from_y = b.y.min(e.y).floor().max(0.0) as i32;
        let to_x = b.x.max(e.x).ceil().min(tm.size_x as f32) as i32;
        let to_y = b.y.max(e.y).ceil().min(tm.size_y as f32) as i32;
        let count = (to_x - from_x).max(0) * (to_y - from_y).max(0);
        (from_x, from_y, to_x, to_y, count)
    };
    classic_core::cl_info!(
        Chan::Editor,
        "apply_editor_selection: target={} region=({},{})-({},{}) tile_count={}",
        target,
        bx,
        by,
        ex,
        ey,
        tile_count
    );
    classic_core::cl_debug!(
        Chan::Editor,
        "target={} region=({},{})-({},{})",
        target,
        bx,
        by,
        ex,
        ey
    );
    if tile_count == 0 {
        classic_core::cl_info!(Chan::Editor, "apply_editor_selection: tile_count=0, returning");
        return;
    }

    let updated = if target == "height" {
        let val = height as f32;
        let is_set = height_mode == "set";
        if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(tm_entity) {
            for y in by..ey {
                for x in bx..ex {
                    let idx = (y * (tm.size_x + 1) + x) as usize;
                    if is_set {
                        if let Some(h) = tm.height_data.get_mut(idx) {
                            *h = val.max(0.0);
                        }
                    } else if let Some(h) = tm.height_data.get_mut(idx) {
                        *h = (*h + val).max(0.0);
                    }
                }
            }
        }
        classic_core::cl_debug!(
            Chan::Editor,
            "painted height region ({},{})-({},{}) delta={} mode={}",
            bx,
            by,
            ex,
            ey,
            height,
            height_mode,
        );
        true
    } else if target == "tilemap" {
        let val = tile;
        if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(tm_entity) {
            for y in by..ey {
                for x in bx..ex {
                    let idx = (y * tm.size_x + x) as usize;
                    if let Some(t) = tm.data.get_mut(idx) {
                        *t = val;
                    }
                }
            }
        }
        classic_core::cl_debug!(
            Chan::Editor,
            "painted tile region ({},{})-({},{}) id={}",
            bx,
            by,
            ex,
            ey,
            val
        );
        true
    } else if target == "navMesh" {
        let val = nav_tile;
        if let Some(nav_e) = engine.entity_by_role(classic_core::RoleKind::NavMesh) {
            if let Ok(mut nav) = engine.world.get::<&mut NavMesh>(nav_e) {
                for y in by..ey {
                    for x in bx..ex {
                        let idx = (y * nav.size_x + x) as usize;
                        if let Some(t) = nav.data.get_mut(idx) {
                            *t = val;
                        }
                    }
                }
            }
        }
        classic_core::cl_debug!(
            Chan::Editor,
            "painted nav region ({},{})-({},{}) id={}",
            bx,
            by,
            ex,
            ey,
            val
        );
        true
    } else {
        false
    };

    if updated {
        classic_core::cl_info!(Chan::Editor, "apply_editor_selection: paint done, rebuilding mesh");
        if target == "navMesh" {
            engine.rebuild_nav_gpu();
        } else {
            engine.rebuild_tilemap_mesh();
            if target == "height" {
                engine.sync_nav_heights();
            }
        }
    } else {
        classic_core::cl_info!(
            Chan::Editor,
            "apply_editor_selection: editor_target={}, nothing to paint",
            target
        );
    }
}
