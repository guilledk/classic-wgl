//! A drawable whose sheet is not uploaded yet: the boot spawns a scene's
//! entities before its `.basis` sheets land (an awaited Worker transcode on
//! the web, a budgeted stage natively), and the visual loader drew frames in
//! between, so `Gfx::texture` panicked (`texture
//! 'lunar-common::lunar-common_atlas_2' not found`, in
//! `draw_shadow_sprite_batches`).  These run on a real GL context (headless
//! EGL) and skip when libEGL is absent.

use std::sync::Arc;

use classic_core::components::{IsoSprite, Role, Tilemap};
use classic_core::{RoleKind, SpriteRender, Transform};
use classic_platform::headless::HeadlessPlatform;
use classic_platform::InputState;
use glam::Vec3;

use crate::boot::InterleavedBoot;
use crate::boot_loader::VisualBootSink;
use crate::Engine;

const W: u32 = 320;
const H: u32 = 180;
/// The sprite's texture: a packed multi-sheet atlas (like
/// `lunar-common::lunar-common_atlas`) whose frame table is metadata, hydrated
/// before the sheets themselves.
const ATLAS: &str = "test::atlas";
/// The sheet the sprite's frame lives on (like `lunar-common_atlas_2`): the
/// one that is late.
const SHEET: &str = "test::atlas_2";

/// An engine on headless EGL built the way a windowed / web boot builds it
/// (`InterleavedBoot::engine`: GL layer, embedded font, the visual loader
/// installed), plus the full shader catalog a ROM brings; `None` (skip)
/// without libEGL.  The platform owns the context, so it is returned too.
fn gl_engine(loader: &Arc<VisualBootSink>) -> Option<(HeadlessPlatform, Engine)> {
    let platform = match HeadlessPlatform::new(W, H) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping: no headless EGL ({e})");
            return None;
        }
    };
    let mut engine = InterleavedBoot::new(Some(loader.clone())).engine(platform.gl());
    let manifest: classic_rom::RomManifest =
        serde_json::from_value(serde_json::json!({ "textures": [] })).expect("an empty manifest");
    engine.ensure_gfx(platform.gl(), &manifest, &classic_rom::NullBootSink);
    engine.gfx.as_mut().unwrap().set_render_target(W, H);
    Some((platform, engine))
}

/// An iso sprite and a world sprite on the not-yet-uploaded sheet: the
/// sprite-batch normal / ghost / shadow paths and the per-draw sprite path.
fn spawn_scene(engine: &mut Engine) {
    // A primary tilemap, so the sun shadow pass (`draw_shadow_sprite_batches`,
    // where the live site panicked) runs.  Its tileset is missing too.
    let tilemap = Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::new(45.0, 45.0, 1.0),
        size_x: 4,
        size_y: 4,
        tile_set: "test::late_tiles".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0u32; 16],
        height_data: vec![0.0f32; 25],
        height_scale: 64.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
        selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        blend: None,
    };
    let tm = engine.world.spawn((
        tilemap,
        Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0)),
        Role::new(RoleKind::Tilemap),
    ));
    engine.names.insert("tilemap".into(), tm);
    let iso: IsoSprite = serde_json::from_value(serde_json::json!({
        "position": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0], "texture": ATLAS,
        "tilemap": "tilemap", "frame": 0.0, "frame_name": "tex_0", "tile_set_size": [1.0, 1.0], "anchor": [0.5, 0.98],
    }))
    .expect("iso sprite");
    // The frame table resolves `tex_0` to the late sheet.  The uniform-grid
    // path already skipped a sprite without a resident texture; this packed
    // path did not, which is how the boot loader's frames reached
    // `Gfx::texture` with a sheet still in the Worker.
    let mut table: classic_core::types::FrameTable = serde_json::from_value(serde_json::json!({
        "version": 2,
        "sheets": [
            { "name": ATLAS, "src": "/res/atlas.png", "size": [8, 8] },
            { "name": SHEET, "src": "/res/atlas_2.png", "size": [8, 8] }
        ],
        "frames": { "tex_0": { "sheet": 1, "rect": [0, 0, 8, 8] } }
    }))
    .expect("frame table");
    table.precompute_companions();
    engine.frame_tables.insert(ATLAS.into(), table);
    let tf = Transform::new(Vec3::ZERO, Vec3::ONE);
    let e = engine.world.spawn((tf.clone(), iso));
    engine.names.insert("lateIso".into(), e);
    let sprite: SpriteRender = serde_json::from_value(serde_json::json!({
        "position": [0.0, 0.0, 0.0], "scale": [16.0, 16.0, 1.0], "texture": SHEET,
        "ignore_cam": false, "frame": 0.0, "tile_set_size": [1.0, 1.0], "anchor": [0.5, 0.5],
    }))
    .expect("sprite");
    engine.world.spawn((tf, sprite));
}

fn frames(engine: &mut Engine, n: usize) {
    let mut input = InputState::new();
    for _ in 0..n {
        engine.frame(&mut input, W as f32, H as f32, 1.0 / 60.0);
    }
}

fn skips(engine: &Engine) -> u64 {
    engine.gfx.as_ref().unwrap().missing_texture_skips()
}

fn sheet_was_missing(engine: &Engine) -> bool {
    engine.gfx.as_ref().unwrap().texture_was_missing(SHEET)
}

#[test]
fn the_loader_hides_a_scene_whose_sheet_is_late_and_nothing_panics() {
    let loader = Arc::new(VisualBootSink::new());
    let Some((_platform, mut engine)) = gl_engine(&loader) else { return };
    spawn_scene(&mut engine);
    assert!(engine.world_hidden, "an installed loader hides the scene");

    // Booting: the loader draws, the scene does not even try its sheet.
    frames(&mut engine, 3);
    assert!(!sheet_was_missing(&engine), "no world draw ran while the loader was up");

    // The safety net: the loader is gone but the sheet is still not
    // resident.  Every draw that needs it is skipped, none panics.
    loader.uninstall(&mut engine);
    assert!(!engine.world_hidden);
    let before = skips(&engine);
    frames(&mut engine, 2);
    let skipped = skips(&engine);
    assert!(
        sheet_was_missing(&engine) && skipped > before,
        "the scene's draws reached the missing sheet and skipped it"
    );

    // The sheet lands: the scene draws it and nothing is skipped anymore.
    let rgba = vec![255u8; 8 * 8 * 4];
    engine.gfx.as_mut().unwrap().add_texture_rgba8(SHEET, &rgba, 8, 8);
    frames(&mut engine, 2);
    assert_eq!(skips(&engine), skipped, "a resident sheet is never skipped");
}
