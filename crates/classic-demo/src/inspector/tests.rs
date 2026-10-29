//! Inspector tests: the pure helpers (paging, namespace grouping, layout,
//! JSON lines) and the whole flow on a GL-free engine (toggle + pause, tree
//! clicks, world pick, transforms).

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::components::{Disabled, Role, Tilemap, Transform};
use classic_core::RoleKind;
use classic_engine::Engine;
use classic_rom::{LoadedRom, ResourceSet, Rom, RomManifest};
use glam::{Vec2, Vec3};
use serde_json::json;

use super::tree::{self, page_range, AssetKind, PAGE};
use super::{compute_layout, detail, max_scroll, Group, RowAction, Selection, ROW_H, TOGGLE_KEY};
use crate::state::{DemoState, DemoStateRef};

const VW: f32 = 1280.0;
const VH: f32 = 720.0;

#[test]
fn paging_splits_into_pages_of_fifty_and_clamps() {
    assert_eq!(page_range(0, 0), (0, 0, 0, 1), "an empty list is one empty page");
    assert_eq!(page_range(50, 0), (0, 50, 0, 1));
    assert_eq!(page_range(51, 0), (0, 50, 0, 2));
    assert_eq!(page_range(51, 1), (50, 51, 1, 2));
    assert_eq!(page_range(120, 9), (100, 120, 2, 3), "past the end clamps to the last page");
    assert_eq!(tree::page_of(0), 0);
    assert_eq!(tree::page_of(PAGE - 1), 0);
    assert_eq!(tree::page_of(PAGE), 1);
    assert_eq!(max_scroll(10, 30), 0);
    assert_eq!(max_scroll(40, 30), 10);
}

#[test]
fn entities_group_by_rom_namespace_with_strays_apart() {
    let names: Vec<String> =
        ["common::font", "demo::a", "demo::b", "plain", "zzz::lost"].map(String::from).to_vec();
    let (per, other) = tree::group_by_namespace(&names, &["common".into(), "demo".into()]);
    assert_eq!(
        per,
        vec![vec!["common::font".to_string()], vec!["demo::a".into(), "demo::b".into()]]
    );
    assert_eq!(other, vec!["plain".to_string(), "zzz::lost".into()]);
    // the global namespace (a single-ROM scene) claims the bare names
    let (per, other) = tree::group_by_namespace(&names, &[String::new()]);
    assert_eq!(per[0], vec!["plain".to_string()]);
    assert_eq!(other.len(), 4);
    assert_eq!(tree::qualified("", "x"), "x");
    assert_eq!(tree::qualified("ns", "x"), "ns::x");
    assert_eq!(tree::qualified("ns", "o::x"), "o::x");
    assert_eq!(tree::bare("a::b"), "b");
}

#[test]
fn panels_sit_left_and_right_under_the_top_bar_and_the_preview_pushes_the_rows() {
    let l = compute_layout(VW, VH, false);
    assert!(l.tree.panel.x >= 0.0 && l.tree.panel.y >= 68.0, "under the 68 px top bar");
    assert!(l.detail.panel.x + l.detail.panel.w <= VW);
    assert!(l.tree.panel.x + l.tree.panel.w < l.detail.panel.x, "a world gap between them");
    assert!(l.tree.panel.y + l.tree.panel.h <= VH);
    assert!(l.preview.is_none());
    let visible = l.tree.visible_rows();
    assert!(visible >= 30, "a 720 px viewport shows {visible} rows");
    // rows map back from the pointer
    let row3 = Vec2::new(l.tree.list.x + 4.0, l.tree.list.y + 3.5 * ROW_H);
    assert_eq!(l.tree.row_at(row3), Some(3));
    assert_eq!(l.tree.row_at(Vec2::new(l.tree.list.x, l.tree.list.y - 1.0)), None, "title");

    let p = compute_layout(VW, VH, true);
    let r = p.preview.expect("a preview box");
    assert!((r.y - l.detail.list.y).abs() < 1e-3, "the box takes the top of the list");
    assert!(p.detail.list.y >= r.y + r.h, "rows start below it");
    assert_eq!(p.tree, l.tree, "the tree does not move");
}

#[test]
fn json_lines_inline_short_values_and_elide_long_arrays() {
    assert_eq!(detail::inline_json(&json!(0.30000001192092896)).unwrap(), "0.3");
    assert_eq!(detail::inline_json(&json!([1.0, 2.5, -0.0])).unwrap(), "[1, 2.5, 0]");
    assert_eq!(detail::inline_json(&json!({"a": 1, "b": "x"})).unwrap(), "{a: 1, b: \"x\"}");
    let big: Vec<u32> = (0..160_000).collect();
    assert_eq!(
        detail::inline_json(&json!(big)).unwrap(),
        "[0, 1, 2, 3, ... 160000 items]",
        "a map-sized grid stays one line"
    );
    let mut out = Vec::new();
    detail::json_field(
        "nested",
        &json!({"pos": [1, 2, 3], "deep": {"k": [[1, 2], [3, 4]]}}),
        1,
        &mut out,
    );
    let text: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(text, ["  nested:", "    pos: [1, 2, 3]", "    deep:", "      k: [[1, 2], [3, 4]]"]);
    assert_eq!(detail::human_bytes(512), "512 B");
    assert_eq!(detail::human_bytes(1536), "1.5 KiB");
}

// ---- the whole flow, GL-free -------------------------------------------

fn rom(json_manifest: serde_json::Value) -> Rom {
    let manifest: RomManifest = serde_json::from_value(json_manifest).unwrap();
    let mut resources = ResourceSet::default();
    for t in &manifest.manifest.textures {
        resources.insert(classic_rom::ResourceKind::Texture, t.name.clone(), vec![0; 2048]);
    }
    Rom { manifest, manifest_json: String::new(), resources, state: String::new() }
}

fn flat_tilemap() -> Tilemap {
    Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::new(45.0, 45.0, 1.0),
        size_x: 8,
        size_y: 8,
        tile_set: "tileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0; 64],
        height_data: vec![0.5; 81],
        height_scale: 64.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
        selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        blend: None,
    }
}

/// Two ROMs (`common` → `demo`), a flat map, a probe entity on it with a
/// screen collider, and the inspector installed (closed).
fn fixture() -> (Engine, DemoStateRef) {
    let mut engine = Engine::new_for_test();
    engine.loaded_roms = vec![
        LoadedRom {
            name: "common".into(),
            namespace: "common".into(),
            rom: rom(json!({"textures": [{"name": "atlas", "src": "t/atlas.png"}]})),
            sha256: None,
        },
        LoadedRom {
            name: "demo".into(),
            namespace: String::new(),
            rom: rom(json!({"entrypoint": "demo", "deps": ["common"], "textures": []})),
            sha256: None,
        },
    ];
    let tm = engine.world.spawn((
        flat_tilemap(),
        Transform::new(Vec3::ZERO, Vec3::ONE),
        Role::new(RoleKind::Tilemap),
    ));
    engine.names.insert("demo::tilemap".into(), tm);
    engine.name_order.push("demo::tilemap".into());
    assert!(engine.spawn_named("demo::probe"));
    let probe = engine.names["demo::probe"];
    engine.world.insert_one(probe, Transform::new(Vec3::new(3.0, 4.0, 0.0), Vec3::ONE)).unwrap();
    // a world-ish entity under the middle of the screen
    assert!(engine.spawn_collider("demo::probe", 600.0, 340.0, 40.0, 40.0));
    for i in 0..(PAGE + 5) {
        assert!(engine.spawn_named(&format!("common::e{i:02}")));
    }
    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));
    super::install(&mut engine, &state);
    (engine, state)
}

fn frame(engine: &mut Engine) {
    let mut platform = Default::default();
    engine.frame(&mut platform, VW, VH, 1.0 / 60.0);
}

fn press_toggle(engine: &mut Engine) {
    engine.scripted_input.press(TOGGLE_KEY);
    engine.scripted_input.release(TOGGLE_KEY);
    frame(engine);
}

fn click(engine: &mut Engine, p: Vec2) {
    engine.scripted_input.click(p, 0);
    frame(engine); // pressed: the collider takes it, the pre-update hook acts
    frame(engine); // released
}

/// The screen centre of the tree row starting with `text`, scrolling the
/// tree (a frame) when it is out of view.
fn tree_row(engine: &mut Engine, state: &DemoStateRef, text: &str) -> Vec2 {
    let (i, visible) = {
        let s = state.borrow();
        let st = &s.inspector;
        let i = st
            .tree_rows()
            .iter()
            .position(|r| r.text.trim_start().starts_with(text))
            .unwrap_or_else(|| panic!("no row {text:?} in {:#?}", st.tree_rows()));
        (i, st.layout().tree.visible_rows())
    };
    let scroll = state.borrow().inspector.tree_scroll;
    if i < scroll || i >= scroll + visible {
        state.borrow_mut().inspector.tree_scroll = i;
        frame(engine);
    }
    let s = state.borrow();
    let row = i - s.inspector.tree_scroll;
    let list = s.inspector.layout().tree.list;
    Vec2::new(list.x + 20.0, list.y + (row as f32 + 0.5) * ROW_H)
}

#[test]
fn closed_by_default_with_everything_hidden() {
    let (mut engine, state) = fixture();
    frame(&mut engine);
    assert!(!state.borrow().inspector.open && !engine.sim_paused());
    let hidden = engine
        .world
        .query::<&super::UiNode>()
        .iter()
        .filter(|(e, _)| engine.is_disabled(*e))
        .count();
    assert!(hidden >= 2 * super::MAX_ROWS, "the pools are hidden ({hidden})");
    // a click in the world is not the inspector's
    click(&mut engine, Vec2::new(610.0, 350.0));
    assert_eq!(state.borrow().inspector.selection, None);
    assert_eq!(
        engine.pick_at(610.0, 350.0, "").as_deref(),
        Some("demo::probe"),
        "pick_at untouched"
    );
}

#[test]
fn backquote_pauses_and_opens_then_closes_and_resumes() {
    let (mut engine, state) = fixture();
    frame(&mut engine);
    press_toggle(&mut engine);
    assert!(state.borrow().inspector.open);
    assert!(engine.sim_paused(), "paused the same frame the key is seen");
    let panels =
        state.borrow().inspector.panels.as_ref().map(|p| (p.tree_bg, p.detail_bg)).unwrap();
    assert!(!engine.is_disabled(panels.0) && !engine.is_disabled(panels.1));
    let rows: Vec<String> =
        state.borrow().inspector.tree_rows().iter().map(|r| r.text.clone()).collect();
    assert!(rows[0].contains("common") && rows[1].contains("demo"), "deps first: {rows:?}");

    press_toggle(&mut engine);
    assert!(!state.borrow().inspector.open && !engine.sim_paused());
    assert!(engine.world.get::<&Disabled>(panels.0).is_ok());
}

#[test]
fn tree_clicks_expand_page_and_select() {
    let (mut engine, state) = fixture();
    frame(&mut engine);
    press_toggle(&mut engine);

    let at = tree_row(&mut engine, &state, "[+] common");
    click(&mut engine, at);
    assert!(state.borrow().inspector.expanded_roms.contains(&0));
    assert_eq!(state.borrow().inspector.selection, Some(Selection::Rom(0)));
    assert!(state.borrow().inspector.detail_rows().iter().any(|r| r.text.contains("Textures")));

    let at = tree_row(&mut engine, &state, "[+] Entities (55)");
    click(&mut engine, at);
    let shown = |state: &DemoStateRef, name: &str| {
        state
            .borrow()
            .inspector
            .tree_rows()
            .iter()
            .any(|r| r.action == Some(RowAction::Select(Selection::Entity(name.to_string()))))
    };
    assert!(shown(&state, "common::e00") && !shown(&state, "common::e52"), "page 1 of 2");
    let at = tree_row(&mut engine, &state, "next 5 >");
    click(&mut engine, at);
    assert!(shown(&state, "common::e52") && !shown(&state, "common::e00"), "page 2");
    assert_eq!(state.borrow().inspector.pages.get(&(0, Group::Entities)), Some(&1));

    let at = tree_row(&mut engine, &state, "[+] Assets (1)");
    click(&mut engine, at);
    let at = tree_row(&mut engine, &state, "[+] Textures (1)");
    click(&mut engine, at);
    let at = tree_row(&mut engine, &state, "atlas");
    click(&mut engine, at);
    let sel = state.borrow().inspector.selection.clone();
    assert!(
        matches!(sel, Some(Selection::Asset(ref a)) if a.kind == AssetKind::Texture && a.rom == 0)
    );
    let detail: Vec<String> =
        state.borrow().inspector.detail_rows().iter().map(|r| r.text.clone()).collect();
    assert!(detail.iter().any(|t| t.contains("common::atlas")), "{detail:?}");
    assert!(detail.iter().any(|t| t.contains("2.0 KiB")), "{detail:?}");
}

#[test]
fn a_world_click_picks_reveals_and_shows_transforms() {
    let (mut engine, state) = fixture();
    frame(&mut engine);
    press_toggle(&mut engine);
    click(&mut engine, Vec2::new(615.0, 355.0));
    {
        let s = state.borrow();
        let st = &s.inspector;
        assert_eq!(st.selected_entity(), Some("demo::probe"));
        assert!(st.expanded_roms.contains(&1) && st.expanded.contains(&(1, Group::Entities)));
        assert!(st.tree_rows().iter().any(|r| r.text.trim() == "probe"), "revealed in the tree");
        assert!(engine.selection().selected.is_empty(), "no RTS select underneath");
    }
    frame(&mut engine); // the detail refreshes on its own clock
    let detail: Vec<String> =
        state.borrow().inspector.detail_rows().iter().map(|r| r.text.clone()).collect();
    let tile = detail.iter().find(|t| t.contains("tile ")).expect("a tile row");
    assert!(tile.contains("(3.00, 4.00)") && tile.contains("ground 0.50 m"), "{tile}");
    let world = engine.iso_to_world(3.0, 4.0, 0.0).unwrap();
    let (sx, sy) = engine.world_to_screen(world);
    assert!(detail.iter().any(|t| t.contains(&format!("({sx:.0}, {sy:.0}) px"))), "{detail:?}");
    assert_eq!(super::pick::anchor_screen(&engine, "demo::probe"), Some(Vec2::new(sx, sy)));
    // the probe has no sprite: no preview box
    assert!(state.borrow().inspector.preview_rect().is_none());

    // a click on empty ground keeps the selection
    click(&mut engine, Vec2::new(500.0, 600.0));
    assert_eq!(state.borrow().inspector.selected_entity(), Some("demo::probe"));
}

#[test]
fn the_wheel_scrolls_the_panel_under_the_pointer_not_the_camera() {
    let (mut engine, state) = fixture();
    frame(&mut engine);
    press_toggle(&mut engine);
    let at = tree_row(&mut engine, &state, "[+] common");
    click(&mut engine, at);
    let at = tree_row(&mut engine, &state, "[+] Entities (55)");
    click(&mut engine, at);
    state.borrow_mut().inspector.tree_scroll = 0;
    frame(&mut engine);
    let over_tree = tree_row(&mut engine, &state, "[-] common");
    engine.scripted_input.move_to(over_tree);
    frame(&mut engine);
    let mut platform = engine.input.clone();
    platform.keys_pressed.clear();
    platform.mouse_pressed = [false; 3];
    platform.mouse_released = [false; 3];
    platform.mouse_wheel = -0.3; // one notch down
    engine.frame(&mut platform, VW, VH, 1.0 / 60.0);
    assert_eq!(state.borrow().inspector.tree_scroll, 3, "three rows down");
    assert_eq!(platform.mouse_wheel, 0.0, "consumed: the camera zoom never sees it");
}
