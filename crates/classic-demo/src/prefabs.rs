//! Simple, state-independent prefabs: camera controls, cursor, ECS systems
//! (agent + animator), footprint colliders and debug keyboard toggles.

use std::rc::Rc;

use classic_core::collision::polygon_from_verts;
use classic_core::components::{
    AgentState, Animator, ColliderData, IsoAgent, IsoSprite, Tilemap, Transform,
};
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

/// Register the isometric agent update system: idle animation + path-following
/// state machine.  Only entities with `(IsoAgent, Animator, Transform)` are processed.
pub fn init_agent_system(engine: &mut Engine) {
    const ANIM_DIRS: [&str; 8] =
        ["East", "SouthEast", "South", "SouthWest", "West", "NorthWest", "North", "NorthEast"];

    engine.on_update(|engine| {
        let delta = engine.time.delta;
        let anim_names: std::collections::HashSet<String> =
            engine.animations.keys().cloned().collect();

        let mut z_updates: Vec<(hecs::Entity, f32)> = Vec::new();

        for (_e, (agent, anim, tf)) in
            engine.world.query::<(&mut IsoAgent, &mut Animator, &mut Transform)>().iter()
        {
            match agent.state {
                AgentState::Idle => {
                    let dir_name = ANIM_DIRS[agent.anim_index % 8];
                    let anim_name = format!("idle{dir_name}");
                    if anim_names.contains(&anim_name) {
                        anim.animation = Some(anim_name);
                        anim.playing = true;
                        anim.repeat = true;
                    }
                }
                AgentState::FollowPath => {
                    if agent.target_index >= agent.path.len()
                        || agent.target_index == 0
                        || agent.path.get(agent.target_index).is_none()
                    {
                        agent.state = AgentState::Idle;
                        continue;
                    }

                    if agent.delta >= 1.0 {
                        agent.delta = 0.0;
                        agent.target_index += 1;

                        if agent.target_index >= agent.path.len() {
                            agent.state = AgentState::Idle;
                            continue;
                        }

                        let from = &agent.path[agent.target_index - 1];
                        let to = &agent.path[agent.target_index];
                        agent.init_dist =
                            glam::Vec2::new(from.x - to.x, from.y - to.y).length().max(0.001);
                    }

                    let from = agent.path[agent.target_index - 1];
                    let to = agent.path[agent.target_index];

                    // Direction
                    let dx = to.x - from.x;
                    let dy = to.y - from.y;
                    let radians = dy.atan2(dx);
                    agent.direction = radians.to_degrees();
                    let mut ix = (agent.direction / 45.0).floor() as i32;
                    ix = ((ix % 8) + 8) % 8;
                    agent.anim_index = ix as usize;

                    let dir_name = ANIM_DIRS[agent.anim_index % 8];
                    let anim_name = format!("walk{dir_name}");
                    if anim_names.contains(&anim_name) {
                        anim.animation = Some(anim_name);
                        anim.playing = true;
                        anim.repeat = true;
                    }

                    // Lerp position
                    let start = glam::Vec3::new(from.x, from.y, tf.position.z);
                    let end = glam::Vec3::new(to.x, to.y, tf.position.z);
                    tf.position.x = start.x + (end.x - start.x) * agent.delta;
                    tf.position.y = start.y + (end.y - start.y) * agent.delta;

                    // Terrain height sampling
                    let tilemap_entity = engine.names.get(&agent.tilemap).copied();
                    if let Some(tm_e) = tilemap_entity {
                        if let Ok(tm) = engine.world.get::<&Tilemap>(tm_e) {
                            let px = tf.position.x;
                            let py = tf.position.y;
                            let ftx = px.floor() as i32;
                            let fty = py.floor() as i32;
                            let fx = px - ftx as f32;
                            let fy = py - fty as f32;

                            let at = |tx: i32, ty: i32| -> f32 {
                                let tx = tx.clamp(0, tm.size_x) as usize;
                                let ty = ty.clamp(0, tm.size_y) as usize;
                                tm.height_data
                                    .get(ty * (tm.size_x as usize + 1) + tx)
                                    .copied()
                                    .unwrap_or(0.0)
                            };

                            let h_nw = at(ftx, fty);
                            let h_ne = at(ftx + 1, fty);
                            let h_sw = at(ftx, fty + 1);
                            let h_se = at(ftx + 1, fty + 1);
                            let hi = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, px, py);
                            let target_z = hi * tm.height_scale;

                            // Speed factor from steepness
                            let dx_h = (h_ne - h_nw) * (1.0 - fy) + (h_se - h_sw) * fy;
                            let dy_h = (h_sw - h_nw) * (1.0 - fx) + (h_se - h_ne) * fx;
                            let steepness = (dx_h * dx_h + dy_h * dy_h).sqrt();
                            let speed_factor = 1.0 - (steepness.min(3.0) / 3.0) * 0.5;

                            agent.delta +=
                                (agent.speed * speed_factor * delta) / agent.init_dist.max(0.001);

                            z_updates.push((_e, target_z));
                        }
                    }

                    if tilemap_entity.is_none() {
                        agent.state = AgentState::Idle;
                    }
                }
            }
        }

        // Phase 2: smooth z interpolation
        for (e, target_z) in z_updates {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                let z_speed = (delta * 4.0).min(1.0);
                tf.position.z += (target_z - tf.position.z) * z_speed;
            }
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
