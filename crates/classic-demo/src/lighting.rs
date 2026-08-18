//! Lighting presets and the light-config widget.
//!
//! The four named presets and the `custom` (azimuth/elevation) override are
//! demo content, so they live here rather than in the engine.  They only touch
//! the engine's public light uniforms (`light_ambient` / `light_dir` /
//! `light_color`) and the demo's own `EditorState`.

use classic_core::components::{SdfTextRender, TextJustify, UiAnchor};
use classic_engine::ui;
use classic_engine::Engine;

use crate::state::{DemoStateRef, EditorState};

/// Apply a named lighting preset (sunny, cloudy, dawn, night).
pub fn apply_light_preset(engine: &mut Engine, state: &DemoStateRef, key: &str) {
    let preset = match key {
        "sunny" => Some(("Sunny Day", [0.15, 0.15, 0.2], [0.453, 0.211, 0.866], [1.0, 0.95, 0.85])),
        "cloudy" => Some(("Cloudy", [0.35, 0.35, 0.4], [0.0, -0.2, 1.0], [0.7, 0.72, 0.78])),
        "dawn" => Some(("Dawn / Dusk", [0.2, 0.15, 0.25], [0.5, 0.2, 0.3], [1.0, 0.4, 0.2])),
        "night" => Some(("Night", [0.1, 0.12, 0.25], [-0.2, -0.5, 0.8], [0.3, 0.4, 0.7])),
        _ => None,
    };
    let Some((_name, ambient, dir_unnorm, color)) = preset else {
        return;
    };
    let d = glam::Vec3::new(dir_unnorm[0], dir_unnorm[1], dir_unnorm[2]).normalize();
    engine.light_ambient = ambient;
    engine.light_dir = [d.x, d.y, d.z];
    engine.light_color = color;
    let mut s = state.borrow_mut();
    s.editor.light_preset = key.into();
    s.editor.light_azimuth = d.x.atan2(-d.y).to_degrees();
    s.editor.light_elevation = d.z.asin().to_degrees();
}

/// Recompute light direction from azimuth/elevation angles.
pub fn update_light_direction(engine: &mut Engine, state: &DemoStateRef) {
    let s = state.borrow();
    let az = s.editor.light_azimuth.to_radians();
    let el = s.editor.light_elevation.to_radians();
    let d = glam::Vec3::new(el.cos() * az.sin(), -el.cos() * az.cos(), el.sin()).normalize();
    engine.light_dir = [d.x, d.y, d.z];
}

/// Initialize lighting defaults.
pub fn init_lighting(engine: &mut Engine, state: &DemoStateRef) {
    apply_light_preset(engine, state, "sunny");
}

/// Light config widget: preset cycle + azimuth/elevation adjustment buttons.
#[allow(clippy::too_many_lines)]
pub fn init_light_widget(engine: &mut Engine, state: &DemoStateRef) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let btn_sz: f32 = 32.0;
    let small_btn: f32 = 24.0;
    let label_w: f32 = 160.0;
    let dir_w: f32 = 160.0;
    let gap: f32 = 4.0;
    let _button_gap: f32 = 10.0;
    let row_h: f32 = btn_sz;
    let preset_row_w: f32 = gap * 4.0 + btn_sz * 2.0 + label_w;
    let adjust_row_w: f32 = gap * 4.0 + dir_w + small_btn * 2.0 + _button_gap * 2.0;
    let widget_w = preset_row_w.max(adjust_row_w);
    let widget_h: f32 = row_h * 3.0 + gap * 4.0;
    let _border: f32 = 0.0;

    const PRESET_ORDER: &[&str] = &["sunny", "cloudy", "dawn", "night"];
    const AZ_STEP: f32 = 15.0;
    const EL_STEP: f32 = 10.0;

    let editor_rc = Rc::new(RefCell::new(EditorState::default()));

    let Some(ref mut ui) = engine.ui else { return };

    let container = ui.spawn_container(&mut engine.world, widget_w, widget_h, [0.0, 0.0, 0.0, 0.4]);

    // Row 1: preset cycle << >>
    let prev_btn;
    {
        let es = editor_rc.clone();
        prev_btn = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.3, 0.3, 0.6, 1.0],
            ui::ButtonOptions {
                text: Some("<<".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    let idx = PRESET_ORDER.iter().position(|&p| p == s.light_preset).unwrap_or(0);
                    let prev = PRESET_ORDER[(idx + PRESET_ORDER.len() - 1) % PRESET_ORDER.len()];
                    s.light_preset = prev.into();
                    true
                })),
                ..Default::default()
            },
        );
    }
    let preset_label = ui.spawn_sdf_text(
        &mut engine.world,
        "Sunny Day",
        0.9,
        300.0,
        [1.0, 1.0, 1.0, 1.0],
        TextJustify::Center,
    );
    let next_btn;
    {
        let es = editor_rc.clone();
        next_btn = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn_sz,
            btn_sz,
            [0.3, 0.3, 0.6, 1.0],
            ui::ButtonOptions {
                text: Some(">>".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    let idx = PRESET_ORDER.iter().position(|&p| p == s.light_preset).unwrap_or(0);
                    let next = PRESET_ORDER[(idx + 1) % PRESET_ORDER.len()];
                    s.light_preset = next.into();
                    true
                })),
                ..Default::default()
            },
        );
    }

    // Row 2: azimuth
    let az_label = ui.spawn_sdf_text(
        &mut engine.world,
        "az: 45deg",
        0.9,
        200.0,
        [1.0, 1.0, 1.0, 1.0],
        TextJustify::Center,
    );
    let az_minus;
    {
        let es = editor_rc.clone();
        az_minus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            small_btn,
            small_btn,
            [0.6, 0.3, 0.1, 1.0],
            ui::ButtonOptions {
                text: Some("-".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.light_azimuth = (s.light_azimuth - AZ_STEP + 360.0) % 360.0;
                    s.light_preset = "custom".into();
                    true
                })),
                ..Default::default()
            },
        );
    }
    let az_plus;
    {
        let es = editor_rc.clone();
        az_plus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            small_btn,
            small_btn,
            [0.1, 0.6, 0.3, 1.0],
            ui::ButtonOptions {
                text: Some("+".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.light_azimuth = (s.light_azimuth + AZ_STEP) % 360.0;
                    s.light_preset = "custom".into();
                    true
                })),
                ..Default::default()
            },
        );
    }

    // Row 3: elevation
    let el_label = ui.spawn_sdf_text(
        &mut engine.world,
        "el: 45deg",
        0.9,
        200.0,
        [1.0, 1.0, 1.0, 1.0],
        TextJustify::Center,
    );
    let el_minus;
    {
        let es = editor_rc.clone();
        el_minus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            small_btn,
            small_btn,
            [0.6, 0.3, 0.1, 1.0],
            ui::ButtonOptions {
                text: Some("-".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.light_elevation = (s.light_elevation - EL_STEP).max(0.0);
                    s.light_preset = "custom".into();
                    true
                })),
                ..Default::default()
            },
        );
    }
    let el_plus;
    {
        let es = editor_rc.clone();
        el_plus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            small_btn,
            small_btn,
            [0.1, 0.6, 0.3, 1.0],
            ui::ButtonOptions {
                text: Some("+".into()),
                text_scale: 0.35,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    let mut s = es.borrow_mut();
                    s.light_elevation = (s.light_elevation + EL_STEP).min(90.0);
                    s.light_preset = "custom".into();
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
        &[
            prev_btn,
            next_btn,
            az_minus,
            az_plus,
            el_minus,
            el_plus,
            preset_label,
            az_label,
            el_label,
        ],
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );

    state.borrow_mut().light_widget_e = Some(container);
    engine.set_enabled(container, false);

    let con_e = container;
    let pv_e = prev_btn;
    let nx_e = next_btn;
    let pl_e = preset_label;
    let az_l = az_label;
    let az_m = az_minus;
    let az_p = az_plus;
    let el_l = el_label;
    let el_m = el_minus;
    let el_p = el_plus;
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

        // Sync Rc state → demo (only when preset actually changes)
        let es = editor_rc_clone.borrow();
        let cur_preset = es.light_preset.clone();
        let cur_az = es.light_azimuth;
        let cur_el = es.light_elevation;
        drop(es);

        let (stored_preset, stored_az, stored_el) = {
            let s = state.borrow();
            (s.editor.light_preset.clone(), s.editor.light_azimuth, s.editor.light_elevation)
        };

        if cur_preset != stored_preset {
            if cur_preset == "custom" {
                {
                    let mut s = state.borrow_mut();
                    s.editor.light_azimuth = cur_az;
                    s.editor.light_elevation = cur_el;
                }
                update_light_direction(engine, &state);
                state.borrow_mut().editor.light_preset = "custom".into();
            } else {
                apply_light_preset(engine, &state, &cur_preset);
            }
        } else if cur_preset == "custom"
            && ((cur_az - stored_az).abs() > 0.1 || (cur_el - stored_el).abs() > 0.1)
        {
            {
                let mut s = state.borrow_mut();
                s.editor.light_azimuth = cur_az;
                s.editor.light_elevation = cur_el;
            }
            update_light_direction(engine, &state);
        }

        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(con_e) {
            tf.position = glam::Vec3::new(x0, y0, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(pv_e) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy1, tf.position.z);
        }
        ui::UIManager::position_children_of(pv_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(pl_e) {
            tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy1, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(nx_e) {
            tf.position =
                glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy1, tf.position.z);
        }
        ui::UIManager::position_children_of(nx_e, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(az_l) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy2, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(az_m) {
            tf.position = glam::Vec3::new(
                x0 + widget_w - gap - small_btn * 2.0 - _button_gap,
                y0 + cy2,
                tf.position.z,
            );
        }
        ui::UIManager::position_children_of(az_m, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(az_p) {
            tf.position = glam::Vec3::new(x0 + widget_w - gap - small_btn, y0 + cy2, tf.position.z);
        }
        ui::UIManager::position_children_of(az_p, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(el_l) {
            tf.position = glam::Vec3::new(x0 + cx, y0 + cy3, tf.position.z);
        }
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(el_m) {
            tf.position = glam::Vec3::new(
                x0 + widget_w - gap - small_btn * 2.0 - _button_gap,
                y0 + cy3,
                tf.position.z,
            );
        }
        ui::UIManager::position_children_of(el_m, &mut engine.world);
        if let Ok(mut tf) = engine.world.get::<&mut classic_core::Transform>(el_p) {
            tf.position = glam::Vec3::new(x0 + widget_w - gap - small_btn, y0 + cy3, tf.position.z);
        }
        ui::UIManager::position_children_of(el_p, &mut engine.world);

        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(pl_e) {
            let name = match state.borrow().editor.light_preset.as_str() {
                "sunny" => "Sunny Day",
                "cloudy" => "Cloudy",
                "dawn" => "Dawn / Dusk",
                "night" => "Night",
                _ => "Custom",
            };
            sdf.text = name.into();
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(az_l) {
            sdf.text = format!("az: {}deg", state.borrow().editor.light_azimuth.round() as i32);
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(el_l) {
            sdf.text = format!("el: {}deg", state.borrow().editor.light_elevation.round() as i32);
        }
    });
}
