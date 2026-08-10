//! Test types for the CLASSIC_TEST framework.

/// Simulated user action in a test step.
#[derive(Clone)]
pub enum TestAction {
    SetEditor { target: String, height_delta: i32, height_mode: String, tile_id: u32 },
    Drag { from: (f32, f32), to: (f32, f32), hold_frames: u64 },
    OpenMenu,
    EnableTextDemo,
    MouseMove { x: f32, y: f32 },
    MouseClick { x: f32, y: f32, button: u32 },
    KeyPress { key: String, pressed: bool },
    Wheel { amount: f32 },
    Wait { frames: u64 },
}

/// Kinds of assertions the test runner supports.
#[derive(Clone, Copy)]
pub enum AssertKind {
    Height,
    Tile,
    UiTextCentered,
    UiEnabled,
    CameraAt,
    EntityVisible,
    EntityPos,
}

/// A single assertion against a region of tile/height data or a UI property.
pub struct TileAssertion {
    pub kind: AssertKind,
    pub region: (i32, i32, i32, i32),
    pub expected: f32,
    pub log: &'static str,
}

/// A scheduled test step: at the given frame, execute actions then run assertions.
pub struct TestStep {
    pub frame: u64,
    pub actions: Vec<TestAction>,
    pub assertions: Vec<TileAssertion>,
    pub log: &'static str,
}
