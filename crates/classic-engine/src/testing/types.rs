//! # Skill: `classic-testing`
//!
//! **Read `.claude/skills/classic-testing/SKILL.md` before working on this module.**
//!
//! Test types for the CLASSIC_TEST framework.

use serde::Deserialize;

/// Simulated user action in a test step.
#[derive(Clone, Deserialize)]
pub enum TestAction {
    #[serde(rename = "setEditor")]
    SetEditor {
        target: String,
        #[serde(rename = "heightDelta")]
        height_delta: i32,
        #[serde(rename = "heightMode")]
        height_mode: String,
        #[serde(rename = "tileId")]
        tile_id: u32,
    },
    #[serde(rename = "drag")]
    Drag {
        from: (f32, f32),
        to: (f32, f32),
        #[serde(rename = "holdFrames")]
        hold_frames: u64,
    },
    #[serde(rename = "openMenu")]
    OpenMenu,
    #[serde(rename = "enableTextDemo")]
    EnableTextDemo,
    #[serde(rename = "mouseMove")]
    MouseMove { x: f32, y: f32 },
    #[serde(rename = "mouseClick")]
    MouseClick { x: f32, y: f32, button: u32 },
    #[serde(rename = "keyPress")]
    KeyPress { key: String, pressed: bool },
    #[serde(rename = "wheel")]
    Wheel { amount: f32 },
    #[serde(rename = "wait")]
    Wait { frames: u64 },
}

/// Kinds of assertions the test runner supports.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum AssertKind {
    #[serde(rename = "height")]
    Height,
    #[serde(rename = "tile")]
    Tile,
    #[serde(rename = "uiTextCentered")]
    UiTextCentered,
    #[serde(rename = "uiEnabled")]
    UiEnabled,
    #[serde(rename = "cameraAt")]
    CameraAt,
    #[serde(rename = "entityVisible")]
    EntityVisible,
    #[serde(rename = "entityPos")]
    EntityPos,
}

/// A single assertion against a region of tile/height data or a UI property.
#[derive(Clone, Deserialize)]
pub struct TileAssertion {
    pub kind: AssertKind,
    pub region: (i32, i32, i32, i32),
    pub expected: f32,
    pub log: String,
}

/// A scheduled test step: at the given frame, execute actions then run assertions.
#[derive(Clone, Deserialize)]
pub struct TestStep {
    pub frame: u64,
    pub actions: Vec<TestAction>,
    pub assertions: Vec<TileAssertion>,
    pub log: String,
}
