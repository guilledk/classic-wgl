//! # Skill: `classic-testing`
//!
//! **Read `.agents/skills/classic-testing/SKILL.md` before working on this module.**
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
    /// Normal-map texture name if the sprite shades with runtime lighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<String>,
    /// Screen-space bounding rect `[x, y, w, h]` (top-left origin, pixels) of
    /// the draw's unit quad after the camera matrix.  Deterministic and GPU-free;
    /// emitted only into the sibling layout map — never into the JSONL golden
    /// trace (hence `#[serde(skip)]`).
    #[serde(skip)]
    pub screen: Option<[f32; 4]>,
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
    pub normal: Option<&'a str>,
    pub screen: Option<[f32; 4]>,
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
            normal: p.normal.map(|s| s.to_string()),
            screen: p.screen,
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

/// Serialise a render trace as a fixed-width layout map: one line per draw item
/// (sorted by `order`) with its screen-space rect.  This is the text-only frame
/// artifact for non-vision models — deterministic, GPU-free, and emitted next to
/// the golden trace, but never part of the line-by-line golden comparison.
pub fn serialize_layout(trace: &RenderTrace) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let [vw, vh] = trace.viewport;
    let [cx, cy, cz] = trace.camera.position;
    let [sx, sy, sz] = trace.camera.scale;
    let _ = writeln!(
        out,
        "# frame {} viewport {:.0}x{:.0} camera_pos ({:.1},{:.1},{:.1}) scale ({:.3},{:.3},{:.3})",
        trace.frame, vw, vh, cx, cy, cz, sx, sy, sz
    );
    let _ = writeln!(
        out,
        "# {:<16} {:<10} {:>10} {:>9} {:>9} {:>9} {:>9}  {:<16} color",
        "name", "kind", "order", "x", "y", "w", "h", "texture"
    );

    let mut items: Vec<&TraceItem> = trace.items.iter().collect();
    items.sort_by(|a, b| a.order.total_cmp(&b.order));
    for it in items {
        let rect = match it.screen {
            Some([x, y, w, h]) => format!("{x:>9.1} {y:>9.1} {w:>9.1} {h:>9.1}"),
            None => format!("{:>9} {:>9} {:>9} {:>9}", "-", "-", "-", "-"),
        };
        let color = it
            .color
            .map(|c| format!("[{:.2},{:.2},{:.2},{:.2}]", c[0], c[1], c[2], c[3]))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<16} {:<10} {:>10.2} {}  {:<16} {}",
            it.name,
            it.kind,
            it.order,
            rect,
            it.texture.clone().unwrap_or_default(),
            color
        );
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

/// Project a model matrix's unit quad `(0,0)..(1,1)` to a screen-space bounding
/// rect `[x, y, w, h]` (top-left origin, pixels).  `cam` is the camera view
/// matrix (`Camera::matrix`), which already maps world → screen pixels;
/// `camera_ignored` substitutes the identity so screen-aligned draws project
/// their model coords directly.  Pure and GL-free — this is what makes the
/// layout map deterministic and CI-safe.
pub fn project_rect(cam: &glam::Mat4, model: &glam::Mat4, camera_ignored: bool) -> [f32; 4] {
    let cam = if camera_ignored { glam::Mat4::IDENTITY } else { *cam };
    let corners = [
        glam::Vec3::new(0.0, 0.0, 0.0),
        glam::Vec3::new(1.0, 0.0, 0.0),
        glam::Vec3::new(0.0, 1.0, 0.0),
        glam::Vec3::new(1.0, 1.0, 0.0),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for c in corners {
        let p = cam.transform_point3(model.transform_point3(c));
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    [min_x, min_y, max_x - min_x, max_y - min_y]
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

    #[test]
    fn project_rect_translates_and_scales_unit_quad() {
        let model = glam::Mat4::from_translation(glam::Vec3::new(10.0, 20.0, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(30.0, 40.0, 1.0));
        assert_eq!(project_rect(&glam::Mat4::IDENTITY, &model, false), [10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn project_rect_camera_ignored_uses_identity() {
        let model = glam::Mat4::from_translation(glam::Vec3::new(5.0, 6.0, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(7.0, 8.0, 1.0));
        // A camera that would otherwise shove the quad far off-screen.
        let cam = glam::Mat4::from_translation(glam::Vec3::new(-1000.0, -1000.0, 0.0));
        assert_eq!(project_rect(&cam, &model, true), [5.0, 6.0, 7.0, 8.0]);
    }

    #[test]
    fn screen_field_is_not_serialized() {
        let item = TraceItem {
            order: 0.0,
            kind: "Sprite".into(),
            name: "n".into(),
            model: [0.0; 16],
            camera_ignored: false,
            texture: None,
            frame: None,
            color: None,
            depth: None,
            depth_range: None,
            normal: None,
            screen: Some([1.0, 2.0, 3.0, 4.0]),
        };
        let json = serde_json::to_string(&item).expect("serialize");
        assert!(!json.contains("screen"), "screen must not leak into the JSONL: {json}");
    }

    #[test]
    fn serialize_layout_renders_items_with_rects() {
        let item = TraceItem {
            order: 2.0,
            kind: "Sprite".into(),
            name: "rocket".into(),
            model: [0.0; 16],
            camera_ignored: false,
            texture: Some("demo_atlas".into()),
            frame: None,
            color: None,
            depth: None,
            depth_range: None,
            normal: None,
            screen: Some([10.0, 20.0, 30.0, 40.0]),
        };
        let trace = RenderTrace {
            tag: "baseline".into(),
            frame: 55,
            viewport: [1280.0, 720.0],
            camera: CameraSnapshot {
                position: [1.0, 2.0, 3.0],
                scale: [1.0, 1.0, 1.0],
                matrix: [0.0; 16],
            },
            counts: BTreeMap::new(),
            items: vec![item],
        };
        let map = serialize_layout(&trace);
        assert!(map.contains("frame 55"));
        assert!(map.contains("rocket"));
        assert!(map.contains("Sprite"));
        assert!(map.contains("demo_atlas"));
    }
}
