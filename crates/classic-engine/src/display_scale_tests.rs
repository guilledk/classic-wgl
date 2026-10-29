//! Display scale: the engine runs in logical points while GL draws physical
//! pixels.  A real GL context (headless EGL); skips when libEGL is absent.

use classic_platform::headless::HeadlessPlatform;
use classic_platform::{FrameSize, InputState, Platform};

use crate::boot::InterleavedBoot;
use crate::Engine;

/// A 640x360-pixel headless engine at `ratio`, its offscreen target sized
/// like the desktop's headless path does (physical); `None` without libEGL.
fn gl_engine(ratio: f32) -> Option<(HeadlessPlatform, Engine)> {
    let platform = match HeadlessPlatform::new(640, 360) {
        Ok(p) => p.with_pixel_ratio(ratio),
        Err(e) => {
            eprintln!("skipping: no headless EGL ({e})");
            return None;
        }
    };
    let mut engine = InterleavedBoot::new(None).engine(platform.gl());
    let manifest: classic_rom::RomManifest =
        serde_json::from_value(serde_json::json!({ "textures": [] })).expect("an empty manifest");
    engine.ensure_gfx(platform.gl(), &manifest, &classic_rom::NullBootSink);
    let (pw, ph) = platform.viewport().physical();
    engine.gfx.as_mut().unwrap().set_render_target(pw, ph);
    Some((platform, engine))
}

#[test]
fn the_view_is_logical_and_the_drawable_physical() {
    for (ratio, logical) in
        [(1.0, (640.0, 360.0)), (1.5, (640.0 / 1.5, 240.0)), (2.0, (320.0, 180.0))]
    {
        let Some((platform, mut engine)) = gl_engine(ratio) else { return };
        let view = platform.viewport();
        assert_eq!((view.width, view.height), logical, "{ratio}");
        let mut input = InputState::new();
        engine.frame_sized(&mut input, view, 1.0 / 60.0);

        assert_eq!(engine.pixel_ratio(), ratio);
        assert_eq!(engine.viewport_size(), logical, "{ratio}: viewport_size stays logical");
        let gfx = engine.gfx.as_ref().unwrap();
        assert_eq!((gfx.viewport_w, gfx.viewport_h), logical, "{ratio}: projection in points");
        assert_eq!((gfx.viewport_px_w, gfx.viewport_px_h), (640, 360), "{ratio}: GL in pixels");
        let rt = gfx.render_target.as_ref().unwrap();
        assert_eq!((rt.width, rt.height), (640, 360), "{ratio}: the target is physical");
        let ui = engine.ui.as_ref().map(|u| (u.viewport_w, u.viewport_h));
        if let Some(ui) = ui {
            assert_eq!(ui, logical, "{ratio}: UI lays out in points");
        }
    }
}

#[test]
fn a_ratio_change_at_the_same_logical_size_resizes_the_drawable() {
    let Some((_platform, mut engine)) = gl_engine(1.0) else { return };
    let mut input = InputState::new();
    engine.frame_sized(&mut input, FrameSize::logical(320.0, 180.0), 1.0 / 60.0);
    assert_eq!(engine.gfx.as_ref().unwrap().viewport_px_w, 320);
    // Dragged onto a 2x display: same points, twice the pixels.
    let at2 = FrameSize { width: 320.0, height: 180.0, pixel_ratio: 2.0 };
    engine.frame_sized(&mut input, at2, 1.0 / 60.0);
    let gfx = engine.gfx.as_ref().unwrap();
    assert_eq!((gfx.viewport_w, gfx.viewport_px_w, gfx.viewport_px_h), (320.0, 640, 360));
    assert_eq!(engine.viewport_size(), (320.0, 180.0));
}
