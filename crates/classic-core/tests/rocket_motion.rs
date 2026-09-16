//! US Rocket motion gates, measured through the engine's own glTF path
//! (`classic_core::model`) — the same parse + node sampling the renderer uses.
//!
//! The `.glb`s are classic-assets build output (gitignored, not in any repo),
//! so the test reads them from `$CLASSIC_ROCKET_GLB_DIR` (containing
//! `landing.glb` + `launch.glb`) and **skips** when the variable is unset or
//! the files are missing (CI has no assets checkout):
//!
//! ```text
//! CLASSIC_ROCKET_GLB_DIR=<classic-assets>/vehicles/us-rocket \
//!     cargo test -p classic-core --test rocket_motion -- --nocapture
//! ```
//!
//! Gates (mirroring classic-assets `vehicles/us-rocket/motion_report.py`):
//!
//! * max |Δv| of the rig origin between consecutive key steps ≤ 0.5 m/s;
//! * landing touchdown speed < 0.5 m/s;
//! * lowest mesh vertex z ≈ 0 (|z| ≤ 1 mm) at touchdown and at rest, and never
//!   below −2 cm in either clip;
//! * the landing's last pose equals the launch's first pose (every mesh's
//!   world bbox within 1 mm);
//! * the landing drifts from Blender world (−4, 4) onto the pad (0, 0).
//!
//! A per-frame `frame,t,x,y,z,vx,vy,vz,speed,dv_key,tilt_deg,feet_z` CSV is
//! written per clip (to `$CLASSIC_ROCKET_MOTION_CSV_DIR`, default the cargo
//! test tmpdir) for review.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use classic_core::model::{gltf_to_world, parse_model_glb, Interpolation, ModelAsset};
use glam::{Mat4, Vec3};

const FPS: f32 = 24.0;
/// Frames per exported key (keys every 2 frames = 12 Hz).
const KEY_STEP: usize = 2;
/// 289 frames (0..=288) = 12 s.
const FRAMES: usize = 289;

const MAX_DV: f32 = 0.5;
const MAX_TOUCHDOWN: f32 = 0.5;
const GROUND_EPS: f32 = 0.02;
const CONTACT_EPS: f32 = 1e-3;
const POSE_EPS: f32 = 1e-3;

struct Row {
    frame: usize,
    root: Vec3,
    tilt_deg: f32,
    feet_z: f32,
}

struct Measured {
    rows: Vec<Row>,
    /// Per mesh node: world bbox `(min, max)` at the first and last frame.
    first: Vec<(usize, Vec3, Vec3)>,
    last: Vec<(usize, Vec3, Vec3)>,
}

fn glb_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("CLASSIC_ROCKET_GLB_DIR")?);
    ["landing.glb", "launch.glb"].iter().all(|f| dir.join(f).is_file()).then_some(dir)
}

fn load(dir: &Path, name: &str) -> (ModelAsset, usize) {
    let bytes = std::fs::read(dir.join(format!("{name}.glb"))).expect("read glb");
    let asset = parse_model_glb(&bytes).expect("parse glb");
    // Contract: one clip per glb, named after the model (= file stem).
    assert_eq!(asset.clips.len(), 1, "{name}: one clip per glb");
    let clip = asset.clip_index(name).unwrap_or_else(|| panic!("{name}: clip named {name}"));
    let duration = asset.clips[clip].duration;
    let expected = (FRAMES - 1) as f32 / FPS;
    assert!((duration - expected).abs() < 1e-3, "{name}: duration {duration} != {expected}");
    assert!(asset.root.is_some(), "{name}: rig root node");
    for mesh in &asset.meshes {
        assert!(!mesh.positions.is_empty() && !mesh.indices.is_empty(), "{name}: empty mesh");
    }
    // Contract: dense LINEAR keys.  Report (don't fail) other interpolation —
    // the engine samples STEP/CUBICSPLINE per spec, the motion gates below
    // measure whatever the file actually encodes.
    let off: Vec<String> = asset.clips[clip]
        .channels
        .iter()
        .filter(|c| c.interpolation != Interpolation::Linear)
        .map(|c| format!("{}.{:?}={:?}", asset.nodes[c.node].name, c.property, c.interpolation))
        .collect();
    if !off.is_empty() {
        eprintln!("WARN {name}: {} non-LINEAR channels (contract is LINEAR): {off:?}", off.len());
    }
    (asset, clip)
}

fn bboxes(asset: &ModelAsset, node_world: &[Mat4]) -> Vec<(usize, Vec3, Vec3)> {
    let axis = gltf_to_world();
    asset
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(ni, node)| {
            let mesh = &asset.meshes[node.mesh?];
            let m = axis * node_world[ni];
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &mesh.positions {
                let w = m.transform_point3(Vec3::from_array(*p));
                lo = lo.min(w);
                hi = hi.max(w);
            }
            Some((ni, lo, hi))
        })
        .collect()
}

fn measure(asset: &ModelAsset, clip: usize) -> Measured {
    let root = asset.root.unwrap();
    let axis = gltf_to_world();
    let mut rows = Vec::with_capacity(FRAMES);
    let (mut first, mut last) = (Vec::new(), Vec::new());
    for frame in 0..FRAMES {
        let world = asset.node_world_transforms(clip, frame as f32 / FPS);
        let rig = axis * world[root];
        // The node matrix acts on glTF-space vectors: the rig's local up (Blender
        // +Z) is glTF +Y, carried into world space by `axis · node_world`.
        let up = rig.transform_vector3(Vec3::Y).normalize();
        let boxes = bboxes(asset, &world);
        let feet_z = boxes.iter().map(|b| b.1.z).fold(f32::MAX, f32::min);
        rows.push(Row {
            frame,
            root: asset.root_translation_from(&world),
            tilt_deg: up.z.clamp(-1.0, 1.0).acos().to_degrees(),
            feet_z,
        });
        if frame == 0 {
            first = boxes;
        } else if frame == FRAMES - 1 {
            last = boxes;
        }
    }
    Measured { rows, first, last }
}

struct Gates {
    max_dv: f32,
    max_dv_frame: usize,
    min_feet_z: f32,
    max_tilt: f32,
    /// Per-frame |Δv| at key frames (for the CSV).
    dv_key: Vec<Option<f32>>,
}

fn gates(rows: &[Row]) -> Gates {
    let dt = KEY_STEP as f32 / FPS;
    let keys: Vec<&Row> = rows.iter().step_by(KEY_STEP).collect();
    let vel: Vec<Vec3> = keys.windows(2).map(|k| (k[1].root - k[0].root) / dt).collect();
    let dv: Vec<f32> = vel.windows(2).map(|v| (v[1] - v[0]).length()).collect();
    // First maximum (ties resolve to the earliest key, like `np.argmax`).
    let (i, &max_dv) = dv
        .iter()
        .enumerate()
        .fold(None, |best: Option<(usize, &f32)>, (i, d)| match best {
            Some((_, b)) if b >= d => best,
            _ => Some((i, d)),
        })
        .expect("at least 3 keys");
    let mut dv_key = vec![None; rows.len()];
    for (n, d) in dv.iter().enumerate() {
        dv_key[keys[n + 1].frame] = Some(*d);
    }
    Gates {
        max_dv,
        max_dv_frame: keys[i + 1].frame,
        min_feet_z: rows.iter().map(|r| r.feet_z).fold(f32::MAX, f32::min),
        max_tilt: rows.iter().map(|r| r.tilt_deg).fold(0.0, f32::max),
        dv_key,
    }
}

fn write_csv(name: &str, rows: &[Row], g: &Gates) -> PathBuf {
    let dir = std::env::var_os("CLASSIC_ROCKET_MOTION_CSV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rocket_motion"));
    std::fs::create_dir_all(&dir).expect("create CSV dir");
    let dt = 1.0 / FPS;
    let mut out = String::from("frame,t,x,y,z,vx,vy,vz,speed,dv_key,tilt_deg,feet_z\n");
    for (i, r) in rows.iter().enumerate() {
        // Forward difference, like motion_report.py (0 on the last frame).
        let v = rows.get(i + 1).map_or(Vec3::ZERO, |n| (n.root - r.root) / dt);
        let dv = g.dv_key[i].map(|d| format!("{d:.4}")).unwrap_or_default();
        writeln!(
            out,
            "{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{:.4},{:.4}",
            r.frame,
            r.frame as f32 / FPS,
            r.root.x,
            r.root.y,
            r.root.z,
            v.x,
            v.y,
            v.z,
            v.length(),
            dv,
            r.tilt_deg,
            r.feet_z
        )
        .unwrap();
    }
    let path = dir.join(format!("{name}_motion.csv"));
    std::fs::write(&path, out).expect("write CSV");
    path
}

#[test]
fn rocket_clips_pass_the_motion_gates() {
    let Some(dir) = glb_dir() else {
        eprintln!("skipping: set CLASSIC_ROCKET_GLB_DIR to a dir with landing.glb + launch.glb");
        return;
    };
    let mut failures = Vec::new();
    let mut check = |label: String, ok: bool| {
        eprintln!("  [{}] {label}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures.push(label);
        }
    };

    let mut measured = Vec::new();
    for name in ["landing", "launch"] {
        let (asset, clip) = load(&dir, name);
        let m = measure(&asset, clip);
        let g = gates(&m.rows);
        let csv = write_csv(name, &m.rows, &g);
        eprintln!(
            "== {name} ({} nodes, {} meshes) -> {}",
            asset.nodes.len(),
            asset.meshes.len(),
            csv.display()
        );
        check(
            format!(
                "{name} max |Δv| per key step {:.3} m/s at f{} (limit {MAX_DV})",
                g.max_dv, g.max_dv_frame
            ),
            g.max_dv <= MAX_DV,
        );
        check(
            format!("{name} never below ground: min feet z {:.4} m", g.min_feet_z),
            g.min_feet_z >= -GROUND_EPS,
        );
        eprintln!("       {name} max tilt {:.2}°", g.max_tilt);
        if name == "landing" {
            let td = m.rows.iter().find(|r| r.feet_z <= CONTACT_EPS).expect("landing touches down");
            let prev = &m.rows[td.frame.saturating_sub(KEY_STEP)];
            let speed = (td.root.z - prev.root.z).abs() / ((td.frame - prev.frame) as f32 / FPS);
            check(
                format!("landing touchdown speed {speed:.3} m/s at f{}", td.frame),
                speed < MAX_TOUCHDOWN,
            );
            let rest = m.rows.last().unwrap().feet_z;
            check(
                format!("landing feet z≈0: touchdown {:.4} m, rest {rest:.4} m", td.feet_z),
                td.feet_z.abs() <= CONTACT_EPS && rest.abs() <= CONTACT_EPS,
            );
            // Drift axis: Blender world (−4, 4) settling onto the pad.
            let (start, end) = (m.rows[0].root, m.rows.last().unwrap().root);
            eprintln!("       landing rig start {start:?} end {end:?}");
            check(
                format!(
                    "landing drift (−4, 4) → (0, 0): start xy {:?}, end xy {:?}",
                    start.truncate(),
                    end.truncate()
                ),
                (start.truncate() - glam::Vec2::new(-4.0, 4.0)).length() < 0.05
                    && end.truncate().length() < 0.01,
            );
        }
        measured.push((name, asset, m));
    }

    let (landing, launch) = (&measured[0], &measured[1]);
    let names = |a: &ModelAsset, b: &[(usize, Vec3, Vec3)]| -> Vec<String> {
        b.iter().map(|(ni, _, _)| a.nodes[*ni].name.clone()).collect()
    };
    let same_meshes = names(&landing.1, &landing.2.last) == names(&launch.1, &launch.2.first);
    let err = landing
        .2
        .last
        .iter()
        .zip(&launch.2.first)
        .map(|(a, b)| (a.1 - b.1).abs().max_element().max((a.2 - b.2).abs().max_element()))
        .fold(0.0f32, f32::max);
    check(
        format!(
            "landing end == launch start: max bbox Δ {:.5} m over {} meshes",
            err,
            landing.2.last.len()
        ),
        same_meshes && err <= POSE_EPS,
    );

    assert!(failures.is_empty(), "motion gates failed: {failures:#?}");
}
