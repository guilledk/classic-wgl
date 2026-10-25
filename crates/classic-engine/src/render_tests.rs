//! Sprite-sheet v2 depth: the resolved frame's `depth_scale` and the boot-time
//! anchor-range fallback from vehicle defs.

use std::collections::HashMap;

use classic_core::math::{DEPTH_FAR, DEPTH_NEAR};
use classic_core::types::{FrameTable, VehicleDef};

use crate::Engine;

fn table(mode: &str, range: f32) -> FrameTable {
    let mut t: FrameTable = serde_json::from_value(serde_json::json!({
        "version": 2,
        "sheets": [{ "name": "atlas", "src": "/res/atlas.png", "size": [64, 64],
                     "depth": "/res/atlas_depth.png", "depth_mode": mode,
                     "depth_range_m": range }],
        "frames": { "tex_0": { "sheet": 0, "rect": [0, 0, 8, 8] } }
    }))
    .unwrap();
    t.precompute_companions();
    t
}

fn scale(tables: &HashMap<String, FrameTable>) -> f32 {
    Engine::resolve_frame(tables, "tex", "tex_0").expect("frame resolves").depth_scale
}

#[test]
fn resolved_frames_carry_the_sheet_depth_scale() {
    let mut tables = HashMap::new();
    tables.insert("tex".to_string(), table("global", 0.0));
    assert_eq!(scale(&tables), 1.0, "legacy sheets keep today's scale");
    tables.insert("tex".to_string(), table("anchor", 3.0));
    assert!((scale(&tables) - 6.0 / (DEPTH_NEAR - DEPTH_FAR)).abs() < 1e-9);
    // An anchor sheet with no range anywhere falls back to the legacy scale
    // (and boot logs the contract violation).
    tables.insert("tex".to_string(), table("anchor", 0.0));
    assert_eq!(scale(&tables), 1.0);
}

#[test]
fn vehicle_layers_fill_missing_anchor_ranges() {
    let def: VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "v", "directions": 16, "depth_range_m": 3.0,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tex" },
            { "name": "wheel", "kind": "wheel", "texture": "wheel", "depth_range_m": 0.5 },
            { "name": "own", "kind": "cargo", "texture": "own", "depth_range_m": 0.5 },
            { "name": "legacy", "kind": "cargo", "texture": "legacy", "depth_range_m": 0.5 }
        ]
    }))
    .unwrap();
    let mut tables = HashMap::new();
    tables.insert("tex".to_string(), table("anchor", 0.0));
    tables.insert("wheel".to_string(), table("anchor", 0.0));
    tables.insert("own".to_string(), table("anchor", 2.0));
    tables.insert("legacy".to_string(), table("global", 0.0));
    Engine::fill_anchor_depth_ranges(&mut tables, &def);
    let range = |t: &str| tables[t].sheets[0].depth_range_m;
    assert_eq!(range("tex"), 3.0, "the def's range is the hull's fallback");
    assert_eq!(range("wheel"), 0.5, "a layer's range wins over the def's");
    assert_eq!(range("own"), 2.0, "the sheet's own range wins");
    assert_eq!(range("legacy"), 0.0, "global sheets are untouched");
    let s = |t: &str| {
        let mut one = HashMap::new();
        one.insert("tex".to_string(), tables[t].clone());
        scale(&one)
    };
    assert!((s("wheel") - 1.0 / (DEPTH_NEAR - DEPTH_FAR)).abs() < 1e-9);
}
