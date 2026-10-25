//! Vehicle definition v2 / sprite sheet v2 contract tests: both versions
//! parse, v1 maps onto layers without changing a frame index, and the pure
//! decoding functions pin the wire meaning.

use classic_core::math::{DEPTH_FAR, DEPTH_NEAR};
use classic_core::types::{DepthMode, FrameTable, VehicleDef, FRAME_TABLE_VERSION};
use classic_core::vehicle_def::{
    anchor_depth_gray, anchor_depth_offset_m, depth_scale, heading_neighbours, heading_to_dir_n,
    team_mask_alpha, team_mask_weight, AngleAxis, FrameAxis, FrameSelect, GearSide, GhostGroupMode,
    LayerKind, VehicleLayerDef,
};
use std::f32::consts::{FRAC_PI_4, TAU};

/// The mule's `scene.json` `vehicles[]` entry (classic-roms `unittest`), a v1
/// def as authored today.
fn mule_v1() -> serde_json::Value {
    serde_json::json!({
        "anchors": "mule_anchors",
        "cell": [529, 529],
        "columns": 4,
        "directions": 8,
        "name": "mule",
        "parts": [
            { "name": "body", "texture": "muleBody" },
            { "name": "wheel_fl", "texture": "muleWheelFl" },
            { "name": "wheel_fr", "texture": "muleWheelFr" },
            { "name": "wheel_rl", "texture": "muleWheelRl" },
            { "name": "wheel_rr", "texture": "muleWheelRr" }
        ],
        "pitch_levels": 5,
        "pitch_max_deg": 20.0,
        "reverse_speed": 0.9,
        "roll_levels": 3,
        "roll_max_deg": 20.0,
        "rows": 2,
        "safe_fall_m": 0.6,
        "steer_levels": 5,
        "steer_max_deg": 30.0,
        "tires": [
            { "name": "tire_fl", "texture": "muleTireFl" },
            { "name": "tire_fr", "texture": "muleTireFr" }
        ]
    })
}

/// A v2 tank-like def exercising every layer kind.
fn tank_v2() -> serde_json::Value {
    serde_json::json!({
        "version": 2,
        "name": "tank",
        "directions": 16,
        "px_per_m": 64.0,
        "anchors": "tank_anchors",
        "depth_range_m": 3.0,
        "pivot_turn": true,
        "ghost_group": "instance",
        "turn_rate_deg_per_sec": 60.0,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tankHull",
              "pitch": { "levels": 5, "max_deg": 20.0 },
              "roll": { "levels": 3, "max_deg": 15.0 } },
            { "name": "track_l", "kind": "gear_side", "texture": "tankTrackL",
              "side": "left", "offset": [0.0, 1.1, 0.0], "spin_frames": 4, "radius_m": 0.35 },
            { "name": "track_r", "kind": "gear_side", "texture": "tankTrackR",
              "side": "right", "offset": [0.0, -1.1, 0.0], "spin_frames": 4, "radius_m": 0.35 },
            { "name": "turret", "kind": "turret", "texture": "tankTurret",
              "directions": 24, "offset": [-0.2, 0.0, 1.4],
              "pitch": { "levels": 4, "min_deg": -5.0, "max_deg": 25.0 },
              "fire_frames": 3, "yaw_rate_deg_per_sec": 90.0 },
            { "name": "crane", "kind": "tool", "texture": "tankCrane", "parent": "turret",
              "clips": { "stow": { "start": 0, "frames": 1 },
                         "swing": { "start": 1, "frames": 6, "fps": 8.0 } },
              "ghost": false },
            { "name": "crate", "kind": "cargo", "texture": "tankCrate", "visible": false,
              "depth_range_m": 2.0 }
        ],
        "sockets": {
            "muzzle": { "layer": "turret", "offset": [2.4, 0.0, 0.3] },
            "lamp_l": { "layer": "hull", "offset": [1.8, 0.7, 0.9] }
        }
    })
}

#[test]
fn v1_def_still_parses_and_validates() {
    let def: VehicleDef = serde_json::from_value(mule_v1()).expect("v1 parses");
    assert_eq!(def.version, 1);
    assert!(!def.is_v2());
    assert!(def.layers.is_empty() && def.sockets.is_empty());
    assert_eq!(def.px_per_m(), classic_core::tilemap::PPM_TARGET);
    assert_eq!(def.ghost_group, GhostGroupMode::Instance);
    assert!(!def.pivot_turn);
    def.validate().expect("the mule is a valid v1 def");

    // The new keys stay out of a v1 def's serialized form.
    let out = serde_json::to_value(&def).unwrap();
    for key in ["version", "px_per_m", "layers", "sockets", "ghost_group", "depth_range_m"] {
        assert!(out.get(key).is_none(), "{key} leaked into a v1 dump");
    }
}

#[test]
fn v1_maps_onto_layers_with_identical_frame_indices() {
    let def: VehicleDef = serde_json::from_value(mule_v1()).unwrap();
    let layers = def.effective_layers();
    let names: Vec<_> = layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        ["body", "wheel_fl", "wheel_fr", "wheel_rl", "wheel_rr", "tire_fl", "tire_fr"]
    );
    assert_eq!(layers[0].kind, LayerKind::Hull);
    assert!(layers[1..].iter().all(|l| l.kind == LayerKind::Wheel));
    assert_eq!(layers[5].parent.as_deref(), Some("wheel_fl"));
    assert_eq!(layers[6].parent.as_deref(), Some("wheel_fr"));

    // The v1 body formula: `(pitch · roll_levels + roll) · 8 + dir`.
    let hull = &layers[0];
    assert_eq!(hull.frame_count(8), 8 * 5 * 3);
    for pitch in 0..5 {
        for roll in 0..3 {
            for dir in 0..8 {
                let sel = FrameSelect { dir, pitch, roll, ..Default::default() };
                assert_eq!(hull.frame_index(8, &sel), (pitch * 3 + roll) * 8 + dir);
            }
        }
    }
    // The v1 tire formula: `steer · 8 + dir`.
    let tire = &layers[5];
    for steer in 0..5 {
        for dir in 0..8 {
            let sel = FrameSelect { dir, steer, ..Default::default() };
            assert_eq!(tire.frame_index(8, &sel), steer * 8 + dir);
        }
    }
    // A v1 wheel is direction-only.
    assert_eq!(layers[1].frame_count(8), 8);
}

#[test]
fn v2_def_parses_every_layer_kind() {
    let def: VehicleDef = serde_json::from_value(tank_v2()).expect("v2 parses");
    def.validate().expect("valid v2");
    assert!(def.is_v2());
    assert_eq!(def.directions, 16);
    assert_eq!(def.px_per_m(), 64.0);
    assert!(def.pivot_turn);
    assert_eq!(def.depth_range_m, Some(3.0));
    assert_eq!(def.effective_layers(), def.layers);

    let l = |n: &str| def.layers.iter().find(|l| l.name == n).unwrap();
    assert_eq!(l("track_l").side, Some(GearSide::Left));
    assert_eq!(l("track_r").offset, Some([0.0, -1.1, 0.0]));
    assert_eq!(l("turret").dirs(def.directions), 24);
    assert_eq!(l("hull").dirs(def.directions), 16);
    assert!(!l("crane").ghost && l("crane").visible);
    assert!(!l("crate").visible && l("crate").ghost);
    assert_eq!(l("crane").clips["swing"].fps, 8.0);
    assert_eq!(l("crane").clips["stow"].fps, 12.0, "fps defaults to 12");
    assert_eq!(def.sockets["muzzle"].layer, "turret");
    assert_eq!(def.sockets["lamp_l"].offset, [1.8, 0.7, 0.9]);

    // Axes and frame counts per kind.
    assert_eq!(
        l("hull").axes(16),
        [FrameAxis::Dir, FrameAxis::Roll, FrameAxis::Pitch],
        "hull: dir x pitch x roll"
    );
    assert_eq!(l("hull").frame_count(16), 16 * 5 * 3);
    assert_eq!(l("track_l").axes(16), [FrameAxis::Dir, FrameAxis::Spin]);
    assert_eq!(l("turret").axes(16), [FrameAxis::Dir, FrameAxis::Fire, FrameAxis::Pitch]);
    assert_eq!(l("turret").frame_count(16), 24 * 3 * 4);
    assert_eq!(l("crane").axes(16), [FrameAxis::Dir, FrameAxis::Clip]);
    assert_eq!(l("crane").frame_count(16), 16 * 7);

    // Turret index: `(pitch · fire + fire_i) · 24 + dir`.
    let sel = FrameSelect { dir: 5, fire: 2, pitch: 3, ..Default::default() };
    assert_eq!(l("turret").frame_index(16, &sel), (3 * 3 + 2) * 24 + 5);
    // Clip index: `clip_frame · dirs + dir`.
    let sel = FrameSelect { dir: 3, clip: 4, ..Default::default() };
    assert_eq!(l("crane").frame_index(16, &sel), 4 * 16 + 3);
    // Wrapping axes wrap, clamped axes clamp.
    let sel = FrameSelect { dir: 17, spin: 5, ..Default::default() };
    assert_eq!(l("track_l").frame_index(16, &sel), 16 + 1);
    let sel = FrameSelect { dir: 0, pitch: 99, ..Default::default() };
    assert_eq!(l("turret").frame_index(16, &sel), 3 * 3 * 24);

    // A v2 def round-trips.
    let back: VehicleDef = serde_json::from_value(serde_json::to_value(&def).unwrap()).unwrap();
    assert_eq!(back.layers, def.layers);
    assert_eq!(back.sockets, def.sockets);
    assert_eq!(back.version, 2);
}

#[test]
fn validation_reports_contract_violations() {
    // v1: wrong part count and 16 directions.
    let mut v1 = mule_v1();
    v1["directions"] = 16.into();
    v1["parts"].as_array_mut().unwrap().pop();
    let def: VehicleDef = serde_json::from_value(v1).unwrap();
    let errs = def.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("exactly 5 parts")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("8 directions")), "{errs:?}");

    // v2: every rule at once.
    let def: VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "bad", "directions": 12,
        "parts": [ { "name": "body", "texture": "b" } ],
        "layers": [
            { "name": "a", "kind": "wheel", "texture": "t", "pitch": { "levels": 3, "max_deg": 10 } },
            { "name": "a", "kind": "gear_side", "texture": "t", "spin_frames": 4 },
            { "name": "c", "kind": "tool", "texture": "", "parent": "nope",
              "clips": { "x": { "start": 0, "frames": 0, "fps": 0 } } },
            { "name": "d", "kind": "turret", "texture": "t", "directions": 10, "parent": "e" },
            { "name": "e", "kind": "cargo", "texture": "t", "parent": "d" }
        ],
        "sockets": { "s": { "layer": "zz", "offset": [0, 0, 0] } }
    }))
    .unwrap();
    let errs = def.validate().unwrap_err().join("\n");
    for needle in [
        "directions 12",
        "not `parts`",
        "exactly one hull",
        "duplicate name",
        "cannot carry a pitch axis",
        "needs a side",
        "spin_frames needs radius_m",
        "empty texture",
        "unknown parent 'nope'",
        "has no frames",
        "fps must be > 0",
        "directions 10 not supported",
        "parent cycle",
        "unknown layer 'zz'",
    ] {
        assert!(errs.contains(needle), "missing `{needle}` in:\n{errs}");
    }
}

#[test]
fn angle_axis_quantizes_like_v1() {
    // Symmetric: index 0 = -max, centre = level.
    let ax = AngleAxis::symmetric(5, 20.0);
    assert_eq!(ax.index(0.0), 2);
    assert_eq!(ax.index((-20f32).to_radians()), 0);
    assert_eq!(ax.index(20f32.to_radians()), 4);
    assert_eq!(ax.index(1.0), 4, "clamps past max");
    assert_eq!(ax.index(f32::NAN), 2, "non-finite -> centre");
    assert!((ax.angle(1) - (-10f32).to_radians()).abs() < 1e-6);
    // Asymmetric gun pitch.
    let gun: AngleAxis =
        serde_json::from_value(serde_json::json!({"levels": 4, "min_deg": -5, "max_deg": 25}))
            .unwrap();
    assert_eq!(gun.index((-5f32).to_radians()), 0);
    assert_eq!(gun.index(25f32.to_radians()), 3);
    assert_eq!(gun.index(5f32.to_radians()), 1);
    assert!((gun.angle(2) - 15f32.to_radians()).abs() < 1e-6);
}

#[test]
fn headings_quantize_and_straddle_for_n_directions() {
    assert_eq!(heading_to_dir_n(0.0, 8), 0);
    assert_eq!(heading_to_dir_n(FRAC_PI_4, 8), 1);
    assert_eq!(heading_to_dir_n(-FRAC_PI_4, 8), 7);
    assert_eq!(heading_to_dir_n(TAU / 16.0 * 3.0, 16), 3);
    assert_eq!(heading_to_dir_n(TAU - 0.01, 24), 0);
    let (a, b, t) = heading_neighbours(TAU / 16.0 * 2.25, 16);
    assert_eq!((a, b), (2, 3));
    assert!((t - 0.25).abs() < 1e-4);
    let (a, b, _) = heading_neighbours(-0.01, 16);
    assert_eq!((a, b), (15, 0), "wraps across 0");
}

#[test]
fn sheet_v1_and_v2_parse() {
    // A v1 table (today's mule body) keeps every new field at its default.
    let v1: FrameTable = serde_json::from_value(serde_json::json!({
        "version": 1,
        "sheets": [ { "name": "unittest_atlas", "src": "/res/unittest_atlas.png",
                      "size": [4096, 4096], "normal": "/res/unittest_atlas_normal.png",
                      "depth": "/res/unittest_atlas_depth.png" } ],
        "frames": { "muleBody_0": { "sheet": 0, "rect": [662, 1989, 337, 153],
                    "source_size": [529, 529], "trim_offset": [90, 166], "anchor": null } }
    }))
    .unwrap();
    assert!(!v1.dir_interp);
    let s = &v1.sheets[0];
    assert_eq!(s.depth_mode, DepthMode::Global);
    assert!(!s.team_mask);
    assert_eq!(s.anchor_depth_range(Some(3.0)), None, "global sheets ignore fallbacks");
    assert_eq!(s.depth_scale(Some(3.0)), 1.0, "global scale is today's behaviour");
    v1.validate(None).unwrap();
    // ...and serializes byte-for-byte like before (no new keys).
    let out = serde_json::to_value(&v1).unwrap();
    assert!(out.get("dir_interp").is_none());
    for key in ["depth_mode", "depth_range_m", "team_mask"] {
        assert!(out["sheets"][0].get(key).is_none(), "{key} leaked");
    }

    let v2: FrameTable = serde_json::from_value(serde_json::json!({
        "version": 2,
        "dir_interp": true,
        "sheets": [ { "name": "tank_atlas", "src": "/res/tank_atlas.png", "size": [2048, 2048],
                      "normal": "/res/tank_atlas_normal.png", "depth": "/res/tank_atlas_depth.png",
                      "depth_mode": "anchor", "depth_range_m": 3.0, "team_mask": true } ],
        "frames": {}
    }))
    .unwrap();
    assert_eq!(v2.version, FRAME_TABLE_VERSION);
    assert!(v2.dir_interp);
    let s = &v2.sheets[0];
    assert_eq!(s.depth_mode, DepthMode::Anchor);
    assert!(s.team_mask);
    assert_eq!(s.anchor_depth_range(Some(9.0)), Some(3.0), "the sheet's range wins");
    assert!((s.depth_scale(None) - 6.0 / (DEPTH_NEAR - DEPTH_FAR)).abs() < 1e-9);
    v2.validate(None).unwrap();
}

#[test]
fn sheet_v2_validation() {
    let table = |interp: bool, mode: &str, range: f32, depth: bool| -> FrameTable {
        let mut sheet = serde_json::json!({ "name": "s", "src": "/s.png", "size": [4, 4],
                                            "depth_mode": mode, "depth_range_m": range });
        if depth {
            sheet["depth"] = "/s_depth.png".into();
        }
        serde_json::from_value(serde_json::json!({
            "version": 2, "dir_interp": interp, "sheets": [sheet], "frames": {}
        }))
        .unwrap()
    };
    // Anchor mode with no range anywhere.
    let errs = table(false, "anchor", 0.0, true).validate(None).unwrap_err();
    assert!(errs[0].contains("needs depth_range_m"), "{errs:?}");
    // ...but the vehicle's / layer's range fills it.
    table(false, "anchor", 0.0, true).validate(Some(3.0)).unwrap();
    assert_eq!(
        table(false, "anchor", 0.0, true).sheets[0].anchor_depth_range(Some(3.0)),
        Some(3.0)
    );
    // Interpolation needs an anchor depth companion.
    let errs = table(true, "global", 0.0, true).validate(None).unwrap_err();
    assert!(errs[0].contains("dir_interp"), "{errs:?}");
    let errs = table(true, "anchor", 3.0, false).validate(None).unwrap_err();
    assert!(errs[0].contains("dir_interp"), "{errs:?}");
    table(true, "anchor", 3.0, true).validate(None).unwrap();
    // A future table version is flagged.
    let mut t = table(false, "global", 0.0, false);
    t.version = FRAME_TABLE_VERSION + 1;
    assert!(t.validate(None).unwrap_err()[0].contains("newer"));
}

#[test]
fn depth_decoding_pins_the_wire_meaning() {
    // Legacy: gray spans the whole window, unchanged.
    assert_eq!(depth_scale(None), 1.0);
    // Anchor-relative over ±3 m: one 8-bit step is 6/255 m ≈ 2.4 cm.
    let r = 3.0;
    assert!((anchor_depth_offset_m(1.0, r) - 3.0).abs() < 1e-6);
    assert!((anchor_depth_offset_m(0.0, r) + 3.0).abs() < 1e-6);
    assert_eq!(anchor_depth_offset_m(0.5, r), 0.0);
    let step = anchor_depth_offset_m(129.0 / 255.0, r) - anchor_depth_offset_m(128.0 / 255.0, r);
    assert!((step - 6.0 / 255.0).abs() < 1e-6);
    // The window offset the shader adds equals the metric offset over the
    // window's span (positive gray = farther = larger window depth).
    let g = 0.8;
    let window = (g - 0.5) * depth_scale(Some(r));
    assert!((window - anchor_depth_offset_m(g, r) / (DEPTH_NEAR - DEPTH_FAR)).abs() < 1e-9);
    assert!(window > 0.0);
    // The bake's inverse.
    for m in [-2.9f32, -0.06, 0.0, 0.5, 2.99] {
        assert!((anchor_depth_offset_m(anchor_depth_gray(m, r), r) - m).abs() < 1e-5);
    }
    assert_eq!(anchor_depth_gray(10.0, r), 1.0, "clamps out of range");
}

#[test]
fn team_mask_round_trips() {
    assert_eq!(team_mask_weight(1.0), 0.0, "opaque normal = no team colour");
    assert_eq!(team_mask_weight(0.5), 1.0);
    assert_eq!(team_mask_weight(0.0), 1.0, "saturates");
    assert!((team_mask_weight(0.75) - 0.5).abs() < 1e-6);
    for w in [0.0f32, 0.25, 0.5, 1.0] {
        assert!((team_mask_weight(team_mask_alpha(w)) - w).abs() < 1e-6);
        assert!(team_mask_alpha(w) >= 0.5, "never encodes alpha below 0.5");
    }
}

#[test]
fn layer_builder_defaults() {
    let l = VehicleLayerDef::new("hull", LayerKind::Hull, "tex");
    assert!(l.ghost && l.visible);
    assert_eq!(l.frame_count(16), 16);
    assert_eq!(l.axes(24), [FrameAxis::Dir]);
}
