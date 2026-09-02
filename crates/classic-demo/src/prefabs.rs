//! Simple, state-independent prefabs: camera controls, cursor, the animator
//! system, footprint colliders and debug keyboard toggles.

use std::rc::Rc;

use classic_core::collision::polygon_from_verts;
use classic_core::components::{
    Animator, ColliderData, IsoAgent, IsoSprite, Light, Tilemap, Transform,
};
use classic_core::math::iso_to_cartesian_4;
use classic_core::tilemap::bilinear_height;
use classic_core::types::AnimationData;
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
            let factor = 1.0 + engine.input.mouse_wheel * engine.time.delta;
            engine.camera.scale.x *= factor;
            engine.camera.scale.y *= factor;
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

/// Resolve the visual offset at a fractional animation time.
///
/// Sparse keyframes (the versioned rocket blob) are linearly interpolated
/// between the surrounding keyframes, matching the Blender `location` fcurves;
/// the legacy dense format is indexed directly by the floored frame.
fn interpolate_offset(animation: &AnimationData, counter: f32) -> glam::Vec3 {
    let kf = &animation.offset_keyframes;
    if kf.is_empty() {
        let frame_idx = counter.floor() as usize;
        return animation
            .offsets
            .get(frame_idx)
            .map(|v| glam::Vec3::from_array(*v))
            .unwrap_or(glam::Vec3::ZERO);
    }
    if counter <= kf[0].frame as f32 {
        return glam::Vec3::from_array(kf[0].offset);
    }
    if counter >= kf[kf.len() - 1].frame as f32 {
        return glam::Vec3::from_array(kf[kf.len() - 1].offset);
    }
    let mut lo = 0usize;
    while lo + 1 < kf.len() && (kf[lo + 1].frame as f32) <= counter {
        lo += 1;
    }
    let lo_kf = &kf[lo];
    let hi_kf = &kf[lo + 1];
    let span = hi_kf.frame as f32 - lo_kf.frame as f32;
    let t = if span > f32::EPSILON { (counter - lo_kf.frame as f32) / span } else { 0.0 };
    glam::Vec3::from_array(lo_kf.offset).lerp(glam::Vec3::from_array(hi_kf.offset), t)
}

/// Apply the `light.*` animation channels to a `Light` component at a
/// fractional timeline position.  Channels absent from the animation leave
/// the light's field unchanged, so a clip with no light channels (e.g. the
/// rocket's launch) simply holds the light's last state.
fn apply_light_channels(data: &AnimationData, counter: f32, l: &mut Light) {
    if let Some(p) = data.channel_sample("light.position", counter) {
        if p.len() >= 3 {
            l.position = glam::Vec3::new(p[0], p[1], p[2]);
        }
    }
    if let Some(c) = data.channel_sample("light.color", counter) {
        if c.len() >= 3 {
            l.color = [c[0], c[1], c[2]];
        }
    }
    if let Some(i) = data.channel_sample("light.intensity", counter) {
        if let Some(&v) = i.first() {
            l.intensity = v;
        }
    }
    if let Some(r) = data.channel_sample("light.radius", counter) {
        if let Some(&v) = r.first() {
            l.radius = v;
        }
    }
    if let Some(d) = data.channel_sample("light.dir", counter) {
        if d.len() >= 3 {
            l.dir = glam::Vec3::new(d[0], d[1], d[2]);
        }
    }
    if let Some(k) = data.channel_sample("light.cone", counter) {
        if let Some(&v) = k.first() {
            l.cone_angle = v;
        }
    }
}

/// The frame a finished animation rests on.  A looping animation returns to its
/// first frame; a one-shot holds its last frame so the sprite stays in its end
/// pose (e.g. the landing rocket keeps its legs deployed on the pad) instead of
/// snapping back to the folded frame-zero pose.
fn rest_frame(sequence: &[u32], repeat: bool) -> f32 {
    if repeat {
        sequence.first().copied().unwrap_or(0) as f32
    } else {
        sequence.last().copied().unwrap_or(0) as f32
    }
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

        struct FrameWrite {
            entity: hecs::Entity,
            comp_type: String,
            frame: f32,
            offset: glam::Vec3,
            counter: f32,
            texture: Option<String>,
            anim_name: Option<String>,
        }

        let mut frame_writes: Vec<FrameWrite> = Vec::new();
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
            let animation_data = engine.animations.get(anim_name.as_str());
            let frame_offset = animation_data
                .map(|a| interpolate_offset(a, anim.counter))
                .unwrap_or(glam::Vec3::ZERO);
            anim.offset = frame_offset;
            if frame_idx >= seq_len {
                anim.frame =
                    animation_data.map(|a| rest_frame(&a.sequence, anim.repeat)).unwrap_or(0.0);
                if anim.repeat {
                    anim.counter = 0.0;
                    anim.offset = glam::Vec3::ZERO;
                } else {
                    // One-shot finished: hold the final frame + offset so the
                    // sprite rests in its end pose (e.g. the rocket's legs stay
                    // deployed on the pad) instead of snapping back to frame 0.
                    anim.counter = seq_len as f32;
                    anim.playing = false;
                }
            } else if let Some(&frame) = animation_data.and_then(|a| a.sequence.get(frame_idx)) {
                anim.frame = frame as f32;
            }

            let texture = animation_data.map(|a| a.src.clone());
            let parts: Vec<&str> = anim.target.splitn(2, '.').collect();
            if parts.len() == 2 {
                if let Some(&target_e) = engine.names.get(parts[0]) {
                    frame_writes.push(FrameWrite {
                        entity: target_e,
                        comp_type: parts[1].to_string(),
                        frame: anim.frame,
                        offset: frame_offset,
                        counter: anim.counter,
                        texture,
                        anim_name: Some(anim_name.clone()),
                    });
                }
            }
        }

        for w in &frame_writes {
            match w.comp_type.as_str() {
                "IsoAgent" => {
                    if let Ok(mut a) = engine.world.get::<&mut IsoAgent>(w.entity) {
                        if let Some(t) = &w.texture {
                            if &a.texture != t {
                                a.texture.clone_from(t);
                            }
                        }
                        a.frame = w.frame;
                        a.frame_offset = w.offset;
                        a.frame_name = engine
                            .frame_tables
                            .contains_key(&a.texture)
                            .then(|| format!("{}_{}", a.texture, w.frame as u32));
                    }
                    if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(w.entity) {
                        if let Some(t) = &w.texture {
                            if &s.texture != t {
                                s.texture.clone_from(t);
                            }
                        }
                        s.frame = w.frame;
                        s.frame_offset = w.offset;
                        s.frame_name = engine
                            .frame_tables
                            .contains_key(&s.texture)
                            .then(|| format!("{}_{}", s.texture, w.frame as u32));
                    }
                }
                "IsoSprite" => {
                    if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(w.entity) {
                        if let Some(t) = &w.texture {
                            if &s.texture != t {
                                s.texture.clone_from(t);
                            }
                        }
                        s.frame = w.frame;
                        s.frame_offset = w.offset;
                        s.frame_name = engine
                            .frame_tables
                            .contains_key(&s.texture)
                            .then(|| format!("{}_{}", s.texture, w.frame as u32));
                    }
                }
                "Light" => {
                    if let Ok(mut l) = engine.world.get::<&mut Light>(w.entity) {
                        let Some(data) =
                            w.anim_name.as_deref().and_then(|n| engine.animations.get(n))
                        else {
                            continue;
                        };
                        apply_light_channels(data, w.counter, &mut l);
                    }
                }
                _ => {}
            }
        }
    });
}

/// Attach footprint Polygon colliders to all static (non-agent) IsoSprite
/// entities.
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

        // Per-vertex bilinear height lookup, shape build, and z-offset — all
        // done under a single immutable world borrow, released before the
        // collider registration below (which needs `&mut self`).
        let result = {
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
                None
            } else {
                let shape = polygon_from_verts(world_verts);
                // Set sprite z-offset from terrain height (world metres — the
                // same unit the depth formula and `height_at` now consume).
                let terrain_z = bilinear_height(hd, sx, sy, sprite_iso_pos.x, sprite_iso_pos.y);
                Some((engine.debug_name(entity), shape, terrain_z))
            }
        };

        let Some((name, shape, terrain_z)) = result else { continue };
        engine.register_named_collider(&name, ColliderData::world(shape));
        log::debug!("registered footprint collider for sprite '{name}'");

        if let Ok(mut tf) = engine.world.get::<&mut Transform>(entity) {
            tf.position.z = terrain_z;
        }
    }
}

/// Register keyboard toggles for debug overlays (F = footprints, V = vehicle
/// paths, L = lights, F9 = dump, F10 = save ROM archive).
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
        if engine.input.was_key_pressed("KeyL") {
            let mut s = state.borrow_mut();
            s.debug_lights = !s.debug_lights;
        }
        // F9: dump state.json (tile/nav/height data is inlined).
        if engine.input.was_key_pressed("F9") {
            let state_json = engine.dump_state();
            engine.save_file("state.json", &state_json);
        }
        // F10: save the current world as a packed ROM archive.
        if engine.input.was_key_pressed("F10") {
            engine.save_rom();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{apply_light_channels, interpolate_offset, rest_frame};
    use classic_core::components::{Light, LightKind};
    use classic_core::types::{AnimChannel, AnimationData, OffsetKeyframe};

    fn anim(offsets: Vec<[f32; 3]>, keyframes: Vec<OffsetKeyframe>) -> AnimationData {
        AnimationData {
            name: "a".into(),
            src: "a".into(),
            rate: 24.0,
            sequence: vec![],
            offsets,
            offset_keyframes: keyframes,
            channels: vec![],
            metadata: None,
        }
    }

    #[test]
    fn interpolate_offset_lerps_between_keyframes() {
        let a = anim(
            vec![],
            vec![
                OffsetKeyframe { frame: 0, offset: [0.0, -3200.0, 0.0] },
                OffsetKeyframe { frame: 240, offset: [0.0, 0.0, 0.0] },
            ],
        );
        // Clamp before the first keyframe.
        assert!((interpolate_offset(&a, -10.0).y - (-3200.0)).abs() < 0.001);
        // Halfway between keyframes → half the descent.
        assert!((interpolate_offset(&a, 120.0).y - (-1600.0)).abs() < 0.001);
        // At a keyframe exactly.
        assert!(interpolate_offset(&a, 240.0).y.abs() < 0.001);
        // Clamp after the last keyframe.
        assert!(interpolate_offset(&a, 250.0).y.abs() < 0.001);
    }

    #[test]
    fn interpolate_offset_falls_back_to_dense() {
        let a = anim(vec![[0.0, -100.0, 0.0], [0.0, -50.0, 0.0]], vec![]);
        // Floor(1.9) = 1 → the second dense entry.
        assert!((interpolate_offset(&a, 1.9).y - (-50.0)).abs() < 0.001);
    }

    #[test]
    fn rest_frame_holds_last_for_one_shot() {
        // A one-shot rests on its final frame (legs deployed), not frame 0.
        let seq = vec![0, 0, 0, 1, 1, 2];
        assert_eq!(rest_frame(&seq, false), 2.0);
    }

    #[test]
    fn rest_frame_loops_back_to_first() {
        let seq = vec![5, 6, 7, 8];
        assert_eq!(rest_frame(&seq, true), 5.0);
    }

    #[test]
    fn rest_frame_empty_sequence_is_zero() {
        assert_eq!(rest_frame(&[], false), 0.0);
        assert_eq!(rest_frame(&[], true), 0.0);
    }

    #[test]
    fn apply_light_channels_drives_each_light_field() {
        // The burn envelope: intensity 0 → 3 → 0 across the burn window, with a
        // position/color/radius that stay constant (as the rocket exporter
        // emits them).  Sampling at the burn peak must light the Light, and an
        // absent channel must leave the field untouched.
        let mut data = anim(vec![], vec![]);
        data.channels = vec![
            AnimChannel {
                name: "light.intensity".into(),
                component: 1,
                keys: vec![(0, vec![0.0]), (192, vec![0.0]), (216, vec![3.0]), (246, vec![0.0])],
            },
            AnimChannel {
                name: "light.position".into(),
                component: 3,
                keys: vec![(0, vec![10.0, 20.0, 30.0]), (288, vec![10.0, 20.0, 30.0])],
            },
            AnimChannel {
                name: "light.color".into(),
                component: 3,
                keys: vec![(0, vec![1.0, 0.55, 0.15]), (288, vec![1.0, 0.55, 0.15])],
            },
        ];

        let mut light = Light {
            kind: LightKind::Point,
            position: glam::Vec3::ZERO,
            color: [0.0, 0.0, 0.0],
            intensity: 0.0,
            radius: 200.0,
            dir: glam::Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
        };
        apply_light_channels(&data, 216.0, &mut light);

        assert_eq!(light.intensity, 3.0);
        assert_eq!(light.position, glam::Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(light.color, [1.0, 0.55, 0.15]);
        // `radius` has no channel → unchanged.
        assert_eq!(light.radius, 200.0);

        // Before ignition the light is off.
        apply_light_channels(&data, 100.0, &mut light);
        assert_eq!(light.intensity, 0.0);
    }
}
