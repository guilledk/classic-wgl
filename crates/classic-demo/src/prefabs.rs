//! Simple, state-independent prefabs: camera controls, cursor, the animator
//! system, footprint colliders and debug keyboard toggles.

use std::rc::Rc;

use classic_core::collision::polygon_from_verts;
use classic_core::components::{
    Animator, ColliderData, Emitter, IsoAgent, IsoSprite, Light, Tilemap, Transform,
};
use classic_core::math::{iso_camera_px, iso_world_pos};
use classic_core::tilemap::bilinear_height;
use classic_core::types::AnimationData;
use classic_engine::Engine;

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
    // The last keyframe at or before `counter` (keyframes are sorted by
    // frame), below the final one: what a front-to-back scan stops on.
    let lo = kf[1..].partition_point(|k| (k.frame as f32) <= counter);
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
    const NAMES: [&str; 6] = [
        "light.position",
        "light.color",
        "light.intensity",
        "light.radius",
        "light.dir",
        "light.cone",
    ];
    for_each_named_channel(data, counter, &NAMES, |slot, v| {
        let vec3 = || (v.len >= 3).then(|| glam::Vec3::new(v.v[0], v.v[1], v.v[2]));
        let scalar = || (v.len >= 1).then_some(v.v[0]);
        match slot {
            0 => l.position = vec3().unwrap_or(l.position),
            1 => l.color = vec3().map_or(l.color, |c| c.to_array()),
            2 => l.intensity = scalar().unwrap_or(l.intensity),
            3 => l.radius = scalar().unwrap_or(l.radius),
            4 => l.dir = vec3().unwrap_or(l.dir),
            _ => l.cone_angle = scalar().unwrap_or(l.cone_angle),
        }
    });
}

/// Sample every channel of `data` named in `names` at `counter` and hand it
/// to `apply` with its index in `names`: one pass over the channels, no
/// allocation.  Like a by-name lookup, only the first channel of a name
/// counts (an empty one hides any later namesake).
fn for_each_named_channel(
    data: &AnimationData,
    counter: f32,
    names: &[&str],
    mut apply: impl FnMut(usize, classic_core::types::ChannelValue),
) {
    let mut seen = 0u32;
    for ch in &data.channels {
        let Some(slot) = names.iter().position(|n| *n == ch.name) else { continue };
        if seen & (1 << slot) != 0 {
            continue;
        }
        seen |= 1 << slot;
        if let Some(v) = ch.sample_fixed(counter) {
            apply(slot, v);
        }
    }
}

/// Apply the `emitter.*` animation channels to an `Emitter` (the particle
/// counterpart of [`apply_light_channels`]).  `emitter.burst` is a cumulative
/// step channel: the particle system fires one burst per integer it crosses.
pub fn apply_emitter_channels(data: &AnimationData, counter: f32, em: &mut Emitter) {
    const NAMES: [&str; 9] = [
        "emitter.rate",
        "emitter.speed",
        "emitter.scale",
        "emitter.intensity",
        "emitter.burst",
        "emitter.enabled",
        "emitter.color",
        "emitter.position",
        "emitter.dir",
    ];
    for_each_named_channel(data, counter, &NAMES, |slot, v| {
        let vec3 = || (v.len >= 3).then(|| glam::Vec3::new(v.v[0], v.v[1], v.v[2]));
        let Some(s) = (v.len >= 1).then_some(v.v[0]) else { return };
        match slot {
            0 => em.rate = s,
            1 => em.speed = s,
            2 => em.scale = s,
            3 => em.intensity = s,
            4 => em.burst = s,
            5 => em.enabled = s > 0.5,
            6 => em.color = vec3().map_or(em.color, |c| c.to_array()),
            7 => em.position = vec3().unwrap_or(em.position),
            _ => em.dir = vec3().unwrap_or(em.dir),
        }
    });
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
/// Simulation (`on_sim_update`): a paused engine holds every animation.
///
/// Allocation-light: animations are looked up in place (no per-frame map of
/// every animation), a write borrows its `AnimationData`, and a sprite's
/// `frame_name` is only replaced when it actually changes.
pub fn init_animator_system(engine: &mut Engine) {
    /// The component an `Animator.target` (`"<entity>.<Component>"`) drives.
    #[derive(Clone, Copy)]
    enum Target {
        IsoAgent,
        IsoSprite,
        Light,
        Emitter,
    }

    struct FrameWrite<'a> {
        entity: hecs::Entity,
        target: Target,
        frame: f32,
        offset: glam::Vec3,
        counter: f32,
        data: &'a AnimationData,
    }

    let mut name_buf = String::new();
    engine.on_sim_update(move |engine| {
        let _perf = classic_gfx::perf::scope("animator");
        let delta = engine.time.delta;
        let Engine { world, animations, names, frame_tables, .. } = engine;

        let mut frame_writes: Vec<FrameWrite> = Vec::new();
        for (_e, anim) in world.query::<&mut Animator>().iter() {
            if !anim.playing && !anim.repeat {
                continue;
            }
            let Some(ref anim_name) = anim.animation else {
                continue;
            };
            let Some(data) = animations.get(anim_name.as_str()) else {
                continue;
            };
            let seq_len = data.sequence.len();

            anim.counter += delta * data.rate * anim.speed;

            if seq_len == 0 {
                anim.counter = 0.0;
                anim.frame = 0.0;
                anim.playing = false;
                continue;
            }

            let frame_idx = anim.counter.floor() as usize;
            let frame_offset = interpolate_offset(data, anim.counter);
            anim.offset = frame_offset;
            if frame_idx >= seq_len {
                anim.frame = rest_frame(&data.sequence, anim.repeat);
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
            } else if let Some(&frame) = data.sequence.get(frame_idx) {
                anim.frame = frame as f32;
            }

            let Some((target_name, component)) = anim.target.split_once('.') else {
                continue;
            };
            let target = match component {
                "IsoAgent" => Target::IsoAgent,
                "IsoSprite" => Target::IsoSprite,
                "Light" => Target::Light,
                "Emitter" => Target::Emitter,
                _ => continue,
            };
            if let Some(&entity) = names.get(target_name) {
                frame_writes.push(FrameWrite {
                    entity,
                    target,
                    frame: anim.frame,
                    offset: frame_offset,
                    counter: anim.counter,
                    data,
                });
            }
        }

        // `frame_name` = `"<texture>_<frame>"` when the texture has a frame
        // table, else `None`; built in a reused buffer, stored only on change.
        let mut set_frame_name = |slot: &mut Option<String>, texture: &str, frame: f32| {
            if !frame_tables.contains_key(texture) {
                *slot = None;
                return;
            }
            use std::fmt::Write as _;
            name_buf.clear();
            let _ = write!(name_buf, "{texture}_{}", frame as u32);
            if slot.as_deref() != Some(name_buf.as_str()) {
                *slot = Some(name_buf.clone());
            }
        };
        for w in &frame_writes {
            let texture = &w.data.src;
            match w.target {
                Target::IsoAgent | Target::IsoSprite => {
                    if matches!(w.target, Target::IsoAgent) {
                        if let Ok(mut a) = world.get::<&mut IsoAgent>(w.entity) {
                            if &a.texture != texture {
                                a.texture.clone_from(texture);
                            }
                            a.frame = w.frame;
                            a.frame_offset = w.offset;
                            let a = &mut *a;
                            set_frame_name(&mut a.frame_name, &a.texture, w.frame);
                        }
                    }
                    if let Ok(mut s) = world.get::<&mut IsoSprite>(w.entity) {
                        if &s.texture != texture {
                            s.texture.clone_from(texture);
                        }
                        s.frame = w.frame;
                        s.frame_offset = w.offset;
                        let s = &mut *s;
                        set_frame_name(&mut s.frame_name, &s.texture, w.frame);
                    }
                }
                Target::Light => {
                    if let Ok(mut l) = world.get::<&mut Light>(w.entity) {
                        apply_light_channels(w.data, w.counter, &mut l);
                    }
                }
                Target::Emitter => {
                    if let Ok(mut em) = world.get::<&mut Emitter>(w.entity) {
                        apply_emitter_channels(w.data, w.counter, &mut em);
                    }
                }
            }
        }
    });
}

/// Register the model system: advance every `Model` clip and re-sample its pose
/// once per frame, so the draw path and `gather_lights` (a light parented to a
/// model tracks its animated rig origin) see this frame's pose.  Registered
/// right after the animator system so a model clip and a light animation
/// started together by the guest stay in lockstep (and pause together: both
/// are `on_sim_update` closures).
pub fn init_model_system(engine: &mut Engine) {
    engine.on_sim_update(|engine| {
        let delta = engine.time.delta;
        engine.update_models(delta);
    });
}

/// Attach footprint Polygon colliders to all static (non-agent) IsoSprite
/// entities.
pub fn init_footprint_colliders(engine: &mut Engine) {
    // Look up the tilemap entity once.
    let tm_entity =
        engine.entity_by_role(classic_core::RoleKind::Tilemap).expect("Tilemap-role entity");

    let (isosprite_entities, _tilemap_name, tilemap_pos) = {
        let tilemap = engine.world.get::<&Tilemap>(tm_entity).unwrap();
        let tilemap_tf = engine.world.get::<&Transform>(tm_entity).unwrap();
        let isosprite_entities: Vec<hecs::Entity> =
            engine.world.query::<&IsoSprite>().iter().map(|(e, _)| e).collect();

        (isosprite_entities, tilemap.tile_set.clone(), tilemap_tf.position)
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

            let mut world_verts: Vec<glam::Vec3> = Vec::with_capacity(footprint.len());
            for pt in &footprint {
                let px = sprite_iso_pos.x + pt.x;
                let py = sprite_iso_pos.y + pt.y;

                let h = bilinear_height(hd, sx, sy, px, py);

                let world = iso_world_pos(px, py, h) + tilemap_pos;
                world_verts.push(iso_camera_px(world));
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
/// paths, Shift+L = light markers, F9 = dump, F10 = save ROM archive).
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
        // Shift+L: a bare L is a common guest binding (unittest v2 toggles
        // night with it), and flipping the light markers with it drew a yellow
        // cross at every light in the guest's night captures.
        let shift = engine.input.is_key_down("ShiftLeft") || engine.input.is_key_down("ShiftRight");
        if shift && engine.input.was_key_pressed("KeyL") {
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
            node: None,
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

#[cfg(test)]
mod scan_equivalence {
    //! The binary-searched, allocation-free animator paths against the
    //! front-to-back scans they replaced, bit for bit.
    use super::{apply_emitter_channels, apply_light_channels, interpolate_offset};
    use classic_core::components::{Emitter, Light};
    use classic_core::types::{AnimChannel, AnimationData, OffsetKeyframe};

    /// The old `interpolate_offset` keyframe scan.
    fn scan_offset(kf: &[OffsetKeyframe], counter: f32) -> glam::Vec3 {
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
        let (a, b) = (&kf[lo], &kf[lo + 1]);
        let span = b.frame as f32 - a.frame as f32;
        let t = if span > f32::EPSILON { (counter - a.frame as f32) / span } else { 0.0 };
        glam::Vec3::from_array(a.offset).lerp(glam::Vec3::from_array(b.offset), t)
    }

    /// The old `channel_sample` key scan.
    fn scan_sample(ch: &AnimChannel, counter: f32) -> Option<Vec<f32>> {
        let first = ch.keys.first()?;
        if counter <= first.0 as f32 {
            return Some(first.1.clone());
        }
        let last = &ch.keys[ch.keys.len() - 1];
        if counter >= last.0 as f32 {
            return Some(last.1.clone());
        }
        for i in 0..ch.keys.len() - 1 {
            let ((f0, v0), (f1, v1)) = (&ch.keys[i], &ch.keys[i + 1]);
            let (f0, f1) = (*f0 as f32, *f1 as f32);
            if counter >= f0 && counter <= f1 {
                let t = if f1 > f0 { (counter - f0) / (f1 - f0) } else { 0.0 };
                return Some(v0.iter().zip(v1).map(|(a, b)| a + (b - a) * t).collect());
            }
        }
        Some(last.1.clone())
    }

    fn counters() -> Vec<f32> {
        let mut c: Vec<f32> = (-8..=160).map(|i| i as f32 * 0.25).collect();
        c.extend([f32::NAN, 3.0, 7.0, 7.0001, 19.999, 1e9, -1e9]);
        c
    }

    /// Sorted keys with ties (two keys on frame 7) and uneven spans.
    const FRAMES: [u32; 7] = [0, 3, 7, 7, 12, 20, 33];

    #[test]
    fn offsets_and_channels_match_the_scan() {
        let kf: Vec<OffsetKeyframe> = FRAMES
            .iter()
            .enumerate()
            .map(|(i, &frame)| OffsetKeyframe { frame, offset: [i as f32, -(i as f32) * 0.5, 2.0] })
            .collect();
        let mut data: AnimationData =
            serde_json::from_str(r#"{"name":"a","src":"a","rate":24.0,"sequence":[0]}"#).unwrap();
        data.offset_keyframes = kf.clone();
        let ch = AnimChannel {
            name: "light.color".into(),
            component: 3,
            keys: FRAMES
                .iter()
                .map(|&f| (f, vec![f as f32 * 0.1, 1.0 - f as f32 * 0.03, 0.5]))
                .collect(),
        };
        data.channels = vec![ch.clone()];
        for c in counters() {
            let bits = |v: glam::Vec3| v.to_array().map(f32::to_bits);
            assert_eq!(bits(interpolate_offset(&data, c)), bits(scan_offset(&kf, c)), "offset {c}");
            let want = scan_sample(&ch, c).unwrap();
            let got = ch.sample_fixed(c).unwrap();
            assert_eq!(got.len, want.len());
            let fb: Vec<u32> = got.v[..got.len].iter().map(|v| v.to_bits()).collect();
            let wb: Vec<u32> = want.iter().map(|v| v.to_bits()).collect();
            assert_eq!(fb, wb, "channel {c}");
            assert_eq!(
                data.channel_sample("light.color", c)
                    .map(|v| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>()),
                Some(wb),
                "channel_sample {c}"
            );
        }
    }

    #[test]
    fn first_channel_of_a_name_wins() {
        let mut data: AnimationData =
            serde_json::from_str(r#"{"name":"a","src":"a","rate":24.0,"sequence":[0]}"#).unwrap();
        let ch = |name: &str, keys: Vec<(u32, Vec<f32>)>| AnimChannel {
            name: name.into(),
            component: keys.first().map_or(1, |k| k.1.len() as u8),
            keys,
        };
        data.channels = vec![
            ch("light.intensity", vec![]),
            ch("light.intensity", vec![(0, vec![9.0])]),
            ch("light.radius", vec![(0, vec![2.0]), (10, vec![4.0])]),
            ch("light.radius", vec![(0, vec![7.0])]),
            ch("emitter.color", vec![(0, vec![1.0, 2.0])]),
            ch("emitter.rate", vec![(0, vec![5.0]), (10, vec![15.0])]),
        ];
        let mut l = Light {
            kind: classic_core::components::LightKind::Point,
            position: glam::Vec3::ZERO,
            color: [0.0; 3],
            intensity: 1.0,
            radius: 0.0,
            dir: glam::Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
            node: None,
        };
        apply_light_channels(&data, 5.0, &mut l);
        assert_eq!(l.intensity, 1.0, "an empty first channel hides its namesake");
        assert_eq!(l.radius, 3.0);
        let mut em: Emitter = serde_json::from_str(r#"{"effect":"fx"}"#).unwrap();
        let color = em.color;
        apply_emitter_channels(&data, 5.0, &mut em);
        assert_eq!(em.color, color, "a 2-float colour is ignored");
        assert_eq!(em.rate, 10.0);
    }
}
