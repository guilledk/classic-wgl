//! Graph UI element components.  Light and serializable: a series is only
//! **named** here ([`GraphSeriesRef::data`]); the heavy sample data
//! (`classic_core::graph::SeriesBuf`) lives in the engine, keyed by that
//! name, so a dump never carries series data.
use serde::{Deserialize, Serialize};

/// A graph plot: one or more named series drawn over a shared view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphRender {
    pub series: Vec<GraphSeriesRef>,
    pub view: GraphView,
    pub axes: GraphAxes,
}

/// One series of a [`GraphRender`]: the engine-side data name and its style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphSeriesRef {
    /// Name of the engine-owned series buffer.
    pub data: String,
    #[serde(default)]
    pub style: GraphStyle,
}

/// How a series is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphStyle {
    /// Linear RGBA line colour.
    pub color: [f32; 4],
    /// Polyline width in **logical points** (scaled by the display pixel
    /// ratio at draw time).
    pub line_width: f32,
    /// Alpha multiplier of the min/max envelope fill drawn when zoomed out.
    pub envelope_alpha: f32,
}

impl Default for GraphStyle {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0, 1.0], line_width: 1.0, envelope_alpha: 0.35 }
    }
}

/// The visible data window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphView {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    /// Fit `y_min..y_max` to the data in the x window each frame.
    pub auto_y: bool,
    /// Keep the x window's right edge on the newest sample (live series).
    pub follow_tail: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        Self { x_min: 0.0, x_max: 1.0, y_min: 0.0, y_max: 1.0, auto_y: true, follow_tail: false }
    }
}

/// Axes, grid and title.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphAxes {
    pub show: bool,
    /// Linear RGBA grid-line colour.
    pub grid_color: [f32; 4],
    /// Target tick count per axis (see `classic_core::graph::nice_ticks`).
    pub ticks: u32,
    pub title: String,
}

impl Default for GraphAxes {
    fn default() -> Self {
        Self { show: true, grid_color: [1.0, 1.0, 1.0, 0.15], ticks: 5, title: String::new() }
    }
}
