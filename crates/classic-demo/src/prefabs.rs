//! Simple, state-independent prefabs: camera controls, cursor, the animator
//! system, footprint colliders and debug keyboard toggles.

use std::rc::Rc;

use classic_core::collision::polygon_from_verts;
use classic_core::components::{Animator, ColliderData, IsoAgent, IsoSprite, Tilemap, Transform};
use classic_core::math::iso_to_cartesian_4;
use classic_core::tilemap::bilinear_height;
use classic_engine::Engine;
use glam::Mat4;

use crate::state::DemoStateRef;

/// WASD camera pan + mouse-wheel zoom.
pub fn init_camera_wasd(engine: &mut Engine) {
    engine.on_update(|engine| {
        let speed = engine.scroll_speed * engine.time.delta;
        let inp = &engine.input;
        if inp.is_key_down("KeyW") {
            engine.camera.position.y -= speed;
        }
        if inp.is_key_down("KeyS") {
            engine.camera.position.y += speed;
        }
        if inp.is_key_down("KeyA") {
            engine.camera.position.x -= speed;
        }
        if inp.is_key_down("KeyD") {
            engine.camera.position.x += speed;
        }
        if engine.input.mouse_wheel.abs() > 0.01 {
            let dz = engine.input.mouse_wheel * engine.time.delta;
            engine.camera.scale.x += dz;
            engine.camera.scale.y += dz;
            let min = glam::Vec3::new(0.1, 0.1, 1.0);
            engine.camera.scale = engine.camera.scale.max(min);
        }
    });
}

/// Move the mouse cursor sprite to follow the pointer.
pub fn init_cursor(engine: &mut Engine) {
    let cursor_entity = engine.entity_by_role(classic_core::RoleKind::Cursor);
    engine.on_update(move |engine| {
        let Some(cursor_e) = cursor_entity else { return };
        let mp = engine.input.mouse_pos;
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(cursor_e) {
            tf.position.x = mp.x;
            tf.position.y = mp.y;
        }
    });
}

/// Register the animator system: advances all `Animator` counters and
/// pushes frame changes to their target IsoSprite / IsoAgent components.
pub fn init_animator_system(engine: &mut Engine) {
    engine.on_update(|engine| {
        let delta = engine.time.delta;

        let anim_rates: std::collections::HashMap<String, (f32, usize)> = engine
            .animations
            .iter()
            .map(|(n, a)| (n.clone(), (a.rate, a.sequence.len())))
            .collect();

        let mut frame_writes: Vec<(hecs::Entity, String, f32)> = Vec::new();
        for (_e, anim) in engine.world.query::<&mut Animator>().iter() {
            if !anim.playing && !anim.repeat {
                continue;
            }
            let Some(ref anim_name) = anim.animation else {
                continue;
            };
            let Some(&(rate, seq_len)) = anim_rates.get(anim_name.as_str()) else {
                continue;
            };

            anim.counter += delta * rate * anim.speed;

            if seq_len == 0 {
                anim.counter = 0.0;
                anim.frame = 0.0;
                anim.playing = false;
                continue;
            }

            let frame_idx = anim.counter.floor() as usize;
            if frame_idx >= seq_len {
                anim.counter = 0.0;
                anim.frame = engine
                    .animations
                    .get(anim_name.as_str())
                    .and_then(|a| a.sequence.first().copied())
                    .unwrap_or(0) as f32;
                if !anim.repeat {
                    anim.playing = false;
                }
            } else if let Some(&frame) =
                engine.animations.get(anim_name.as_str()).and_then(|a| a.sequence.get(frame_idx))
            {
                anim.frame = frame as f32;
            }

            let parts: Vec<&str> = anim.target.splitn(2, '.').collect();
            if parts.len() == 2 {
                if let Some(&target_e) = engine.names.get(parts[0]) {
                    frame_writes.push((target_e, parts[1].to_string(), anim.frame));
                }
            }
        }

        for (target_e, comp_type, frame) in &frame_writes {
            match comp_type.as_str() {
                "IsoAgent" => {
                    if let Ok(mut a) = engine.world.get::<&mut IsoAgent>(*target_e) {
                        a.frame = *frame;
                    }
                    if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(*target_e) {
                        s.frame = *frame;
                    }
                }
                "IsoSprite" => {
                    if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(*target_e) {
                        s.frame = *frame;
                    }
                }
                _ => {}
            }
        }
    });
}

/// Attach footprint Polygon colliders to all static (non-agent) IsoSprite
/// entities.  Port of `initFootprintColliders` from `prefabs.ts`.
pub fn init_footprint_colliders(engine: &mut Engine) {
    // Look up the tilemap entity once.
    let tm_entity =
        engine.entity_by_role(classic_core::RoleKind::Tilemap).expect("Tilemap-role entity");

    let (isosprite_entities, _tilemap_name, iso_to_cart_world, tilemap_pos) = {
        let tilemap = engine.world.get::<&Tilemap>(tm_entity).unwrap();
        let tilemap_tf = engine.world.get::<&Transform>(tm_entity).unwrap();
        let isosprite_entities: Vec<hecs::Entity> =
            engine.world.query::<&IsoSprite>().iter().map(|(e, _)| e).collect();

        let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);

        (isosprite_entities, tilemap.tile_set.clone(), iso_to_cart_world, tilemap_tf.position)
    };

    for entity in isosprite_entities {
        // Skip agents — they get their own collider handling elsewhere.
        if engine.world.get::<&IsoAgent>(entity).is_ok() {
            continue;
        }

        let (sprite_iso_pos, footprint) = {
            let s = engine.world.get::<&IsoSprite>(entity).unwrap();
            (s.position, s.footprint.clone())
        };

        // Per-vertex bilinear height lookup.
        let tm = engine.world.get::<&Tilemap>(tm_entity).unwrap();
        let hd = &tm.height_data;
        let sx = tm.size_x;
        let sy = tm.size_y;
        let hs = tm.height_scale;

        let mut world_verts: Vec<glam::Vec3> = Vec::with_capacity(footprint.len());
        for pt in &footprint {
            let px = sprite_iso_pos.x + pt.x;
            let py = sprite_iso_pos.y + pt.y;

            let h = bilinear_height(hd, sx, sy, px, py);

            let mut v = glam::Vec3::new(px, py, 0.0);
            v = iso_to_cart_world.transform_point3(v);
            v += tilemap_pos;
            v.y -= h * hs;
            world_verts.push(v);
        }

        if world_verts.is_empty() {
            continue;
        }

        let shape = polygon_from_verts(world_verts);
        let pid = engine.physics.register_collider(ColliderData::new(shape));
        log::debug!("registered footprint collider pid={pid} for sprite");

        // Set sprite z-offset from terrain height (matches TS prefabs.ts:367).
        let px = sprite_iso_pos.x;
        let py = sprite_iso_pos.y;
        let terrain_z = bilinear_height(hd, sx, sy, px, py) * hs;

        if let Ok(mut tf) = engine.world.get::<&mut Transform>(entity) {
            tf.position.z = terrain_z;
        }
    }
}

/// Register keyboard toggles for debug overlays (F = footprints, F9 = dump).
pub fn init_debug_toggles(engine: &mut Engine, state: &DemoStateRef) {
    let state = Rc::clone(state);
    engine.on_update(move |engine| {
        if engine.input.was_key_pressed("KeyF") {
            {
                let mut s = state.borrow_mut();
                s.editor.debug_footprints = !s.editor.debug_footprints;
                engine.show_grid = s.editor.debug_footprints;
            }
        }
        // F9: dump state.json (tile/nav/height data is inlined).
        if engine.input.was_key_pressed("F9") {
            let state_json = engine.dump_state();
            engine.save_file("state.json", &state_json);
        }
    });
}
