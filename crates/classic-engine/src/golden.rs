//! # Skill: `classic-testing`
//!
//! **Read `.claude/skills/classic-testing/SKILL.md` before working on this module.**
///
/// Render-trace golden harness.
///
/// Captures a deterministic, frame-by-frame record of every draw call
/// (model matrices, textures, sort order, etc.) for comparison against
/// committed reference files.  Does not require a GPU or pixel readback —
/// the trace is structurally deterministic and works on any platform.
///
/// ## Golden files
///
/// Reference traces live at `tests/golden/<scenario>/<tag>.trace.jsonl`
/// (one JSON object per line).  Mismatched actual traces are written to
/// `target/classic-test/<scenario>/<tag>.actual.trace.jsonl`.
///
/// ## Modes
///
/// - `CLASSIC_GOLDEN=check` (default under CLASSIC_TEST): compare, fail on diff.
/// - `CLASSIC_GOLDEN=update`: overwrite reference files with current output.
use std::collections::BTreeMap;

/// A single draw-call entry in the trace.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TraceItem {
    /// Sort order (z-depth or iso-order).
    pub order: f32,
    /// Human-readable render kind.
    pub kind: String,
    /// Debug name of the entity (from `DebugName` component or fallback).
    pub name: String,
    /// 16-element float model matrix (row-major for readability).
    pub model: [f32; 16],
    /// Whether the camera matrix was ignored for this draw.
    pub camera_ignored: bool,
    /// Texture name if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// Sprite frame index if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<f32>,
    /// Color if applicable (RGBA, 0-1 range).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 4]>,
    /// Depth-map texture name if the sprite renders with per-pixel depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    /// Depth range (isoDepth units) the depth map's grayscale spans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_range: Option<f32>,
}

/// A complete render trace for one capture point (tag + frame).
#[derive(Clone, Debug, serde::Serialize)]
pub struct RenderTrace {
    /// User-defined capture tag.
    pub tag: String,
    /// Frame number at capture time.
    pub frame: u64,
    /// Logical viewport dimensions.
    pub viewport: [f32; 2],
    /// Camera state.
    pub camera: CameraSnapshot,
    /// Per-kind draw-call count summary.
    pub counts: BTreeMap<String, usize>,
    /// Ordered draw-call items.
    pub items: Vec<TraceItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CameraSnapshot {
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub matrix: [f32; 16],
}

/// The trace collector — push entries during the render loop.
pub struct TraceCollector {
    pub tag: String,
    pub viewport: [f32; 2],
    pub camera_matrix: [f32; 16],
    pub camera_pos: [f32; 3],
    pub camera_scale: [f32; 3],
    items: Vec<TraceItem>,
}

/// Parameters for a single trace item push.
pub struct TraceItemParams<'a> {
    pub order: f32,
    pub kind: &'a str,
    pub name: &'a str,
    pub model: &'a glam::Mat4,
    pub camera_ignored: bool,
    pub texture: Option<&'a str>,
    pub frame: Option<f32>,
    pub color: Option<[f32; 4]>,
    pub depth: Option<&'a str>,
    pub depth_range: Option<f32>,
}

impl TraceCollector {
    pub fn new(
        tag: &str,
        viewport_w: f32,
        viewport_h: f32,
        camera_matrix: &glam::Mat4,
        camera_pos: glam::Vec3,
        camera_scale: glam::Vec3,
    ) -> Self {
        Self {
            tag: tag.to_string(),
            viewport: [viewport_w, viewport_h],
            camera_matrix: mat4_to_row(camera_matrix),
            camera_pos: camera_pos.to_array(),
            camera_scale: camera_scale.to_array(),
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, p: TraceItemParams<'_>) {
        self.items.push(TraceItem {
            order: p.order,
            kind: p.kind.to_string(),
            name: p.name.to_string(),
            model: mat4_to_row(p.model),
            camera_ignored: p.camera_ignored,
            texture: p.texture.map(|s| s.to_string()),
            frame: p.frame,
            color: p.color,
            depth: p.depth.map(|s| s.to_string()),
            depth_range: p.depth_range,
        });
    }

    pub fn finish(self) -> RenderTrace {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for item in &self.items {
            *counts.entry(item.kind.clone()).or_default() += 1;
        }
        RenderTrace {
            tag: self.tag,
            frame: classic_core::instrument::frame(),
            viewport: self.viewport,
            camera: CameraSnapshot {
                position: self.camera_pos,
                scale: self.camera_scale,
                matrix: self.camera_matrix,
            },
            counts,
            items: self.items,
        }
    }
}

/// Serialise a render trace as multi-line JSONL:
/// header line (tag, frame, viewport, camera, counts) +
/// one line per `TraceItem`.
pub fn serialize_trace(trace: &RenderTrace) -> String {
    let mut out = String::new();
    // Header: everything except items
    let header = serde_json::json!({
        "tag": trace.tag,
        "frame": trace.frame,
        "viewport": trace.viewport,
        "camera": trace.camera,
        "counts": trace.counts,
    });
    let header_str = serde_json::to_string(&header).expect("serialize header");
    out.push_str(&header_str);
    out.push('\n');

    for item in &trace.items {
        let line = serde_json::to_string(item).expect("serialize item");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Compare two traces line-wise.  Returns Ok(()) on match, Err(lines_diff) on mismatch.
pub fn compare_traces(actual: &str, expected: &str) -> Result<(), Vec<String>> {
    let actual_lines: Vec<&str> = actual.lines().collect();
    let expected_lines: Vec<&str> = expected.lines().collect();
    if actual_lines == expected_lines {
        return Ok(());
    }
    let mut diffs: Vec<String> = Vec::new();
    let max = actual_lines.len().max(expected_lines.len());
    for i in 0..max {
        let a = actual_lines.get(i).copied().unwrap_or("<missing>");
        let e = expected_lines.get(i).copied().unwrap_or("<missing>");
        if a != e {
            diffs.push(format!("- expected[{}]: {}", i, e));
            diffs.push(format!("+ actual[{}]:   {}", i, a));
            if diffs.len() >= 40 {
                diffs.push(format!("... {} more diffs omitted ...", max - i - 1));
                break;
            }
        }
    }
    Err(diffs)
}

fn mat4_to_row(m: &glam::Mat4) -> [f32; 16] {
    m.to_cols_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_traces_compare_equal() {
        let json = r#"{"tag":"a","frame":1,"viewport":[1,2]}"#;
        assert!(compare_traces(json, json).is_ok());
    }

    #[test]
    fn different_traces_compare_unequal() {
        assert!(compare_traces("{\"tag\":\"a\"}", "{\"tag\":\"b\"}").is_err());
    }
}
