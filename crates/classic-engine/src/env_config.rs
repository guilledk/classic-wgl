use std::sync::LazyLock;

/// Hoisted per-process env-var configuration, parsed once via `LazyLock`.
/// Replaces the per-frame `std::env::var()` calls spread throughout the engine.
pub struct EnvConfig {
    /// CLASSIC_TEST: `1`/`all` or scenario name (empty = disabled).
    pub test: String,
    /// CLASSIC_FRAMES frame limit (desktop binary exits after N frames).
    pub max_frames: Option<u64>,
    /// CLASSIC_FIXED_DT: override delta per frame. Auto-defaults to 1/60
    /// when CLASSIC_TEST is set.
    pub fixed_dt: Option<f32>,
    /// CLASSIC_WIDTH: forced logical viewport width.
    pub forced_width: Option<f32>,
    /// CLASSIC_HEIGHT: forced logical viewport height.
    pub forced_height: Option<f32>,
    /// CLASSIC_UI_DEBUG: dump UI entity positions each frame (first 120 frames).
    pub ui_debug: bool,
    /// CLASSIC_GOLDEN: `check` / `update` / `none`.
    pub golden_mode: String,
    /// CLASSIC_GOLDEN_PNG: enable pixel capture on golden tests.
    pub golden_png: bool,
    /// CLASSIC_GOLDEN_TOL: per-channel pixel difference tolerance (default 2).
    pub golden_tol: u8,
    /// CLASSIC_HEADLESS: use headless (surfaceless) EGL platform, no window.
    pub headless: bool,
    /// CLASSIC_OFFSCREEN: render to an offscreen FBO.
    pub offscreen: bool,
    /// CLASSIC_DUMP_DIR: directory for state-dump file output (default `./dump/`).
    pub dump_dir: String,
    /// CLASSIC_DUMP_ON_EXIT: auto-dump state on shutdown.
    pub dump_on_exit: bool,
    /// CLASSIC_TEST_FAILFAST: abort on first assertion failure.
    pub failfast: bool,
    /// CLASSIC_TEST_FILE: path to a JSON test-scenario file.
    pub test_file: String,
    /// CLASSIC_ROM: which ROM to boot (a `rom:` name, file path, or URL).
    pub rom: String,
    /// CLASSIC_GOLDEN_DIR: directory holding the golden trace + PNG baseline.
    /// Per-scene so a second scene can have its own reference output.
    pub golden_dir: String,
    /// CLASSIC_SHADOWS: `0` disables the directional shadow map (default on).
    pub shadows: bool,
    /// CLASSIC_NO_UI: skip the demo layer's editor/HUD/overlay prefabs so a
    /// capture shows only the lit scene.  Diagnostic + lighting-golden aid.
    pub no_ui: bool,
    /// CLASSIC_SHADOW_DEBUG: render the raw shadow visibility factor (white =
    /// lit, black = occluded) instead of shaded albedo.  Bring-up diagnostic:
    /// makes "no shadows" and "subtle shadows" unambiguously distinguishable.
    pub shadow_debug: bool,
    /// CLASSIC_SHADOW_DUMP: write the directional shadow depth map to
    /// `<dump_dir>/shadow_depth.png` on the golden-capture frame.
    pub shadow_dump: bool,
}

static CONFIG: LazyLock<EnvConfig> = LazyLock::new(|| {
    let test: String = read("CLASSIC_TEST");
    let test_active = !test.is_empty() && test != "0";
    EnvConfig {
        max_frames: read("CLASSIC_FRAMES").parse().ok(),
        fixed_dt: read("CLASSIC_FIXED_DT").parse().ok().or_else(|| {
            if test_active {
                Some(1.0 / 60.0)
            } else {
                None
            }
        }),
        forced_width: read("CLASSIC_WIDTH").parse().ok(),
        forced_height: read("CLASSIC_HEIGHT").parse().ok(),
        ui_debug: read_bool("CLASSIC_UI_DEBUG"),
        golden_mode: read("CLASSIC_GOLDEN"),
        golden_png: read_bool("CLASSIC_GOLDEN_PNG"),
        golden_tol: read("CLASSIC_GOLDEN_TOL").parse().ok().unwrap_or(2),
        headless: read_bool("CLASSIC_HEADLESS"),
        offscreen: read_bool("CLASSIC_OFFSCREEN"),
        dump_dir: {
            let d = read_string("CLASSIC_DUMP_DIR");
            if d.is_empty() {
                String::from("dump")
            } else {
                d
            }
        },
        dump_on_exit: read_bool("CLASSIC_DUMP_ON_EXIT"),
        failfast: read_bool("CLASSIC_TEST_FAILFAST"),
        test_file: read("CLASSIC_TEST_FILE"),
        golden_dir: {
            let d = read_string("CLASSIC_GOLDEN_DIR");
            if d.is_empty() {
                String::from("tests/golden/baseline")
            } else {
                d
            }
        },
        rom: read("CLASSIC_ROM"),
        shadows: read("CLASSIC_SHADOWS") != "0",
        no_ui: read_bool("CLASSIC_NO_UI"),
        shadow_debug: read_bool("CLASSIC_SHADOW_DEBUG"),
        shadow_dump: read_bool("CLASSIC_SHADOW_DUMP"),
        test,
    }
});

impl EnvConfig {
    pub fn get() -> &'static Self {
        &CONFIG
    }

    pub fn test_active(&self) -> bool {
        !self.test.is_empty() && self.test != "0"
    }

    /// Whether a golden-trace comparison is active (`CLASSIC_GOLDEN` is
    /// `check` or `update`).  Golden capture must be deterministic, so this
    /// also forces synchronous workers.
    pub fn golden_active(&self) -> bool {
        !self.golden_mode.is_empty()
    }
}

fn read(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn read_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn read_bool(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("1" | "true" | "yes"))
}
