//! # Skill: `classic-graph`
//!
//! **Read `.agents/skills/classic-graph/SKILL.md` before working on this module.**
//!
//! GPU half of the graph element: a series' pyramid levels
//! (`classic_core::graph::SeriesBuf`) mirrored into VBOs, and the two
//! instanced draws that read them in place.
//!
//! - **Level 0** (raw `[x, y]` samples) draws as an antialiased polyline
//!   (`graphLine`): one instance per segment, its four attributes the same
//!   VBO at consecutive offsets (`p0..p3` = previous, start, end, next
//!   sample), nine vertices (quad + bevel triangle), miter joins with a
//!   limit and a bevel fallback.
//! - **Levels >= 1** ([`Bucket`]s) draw as a min/max envelope
//!   (`graphEnvelope`): one instance per pair of consecutive buckets, a
//!   four-vertex trapezoid between their centres.
//!
//! Both are `O(entries in view)` vertices (the level is chosen so that is
//! about the plot width) and at most two draws per series (the ring may
//! wrap).  Each level VBO is the ring's slots with **seam pads**: one slot
//! before (a copy of the last slot) and two after (copies of the first
//! two), so an instance's neighbours across the wrap seam read continuous
//! memory and the line has no gap there.  Levels upload lazily on first
//! use, then only their dirty slots (`SeriesBuf::take_dirty`).  Never
//! `gl.LINES`.

use classic_core::graph::{Bucket, GraphMode, Selection, SeriesBuf};
use glow::HasContext;

use crate::Gfx;

/// `graphLine` attribute layout (location = index).
pub const LINE_ATTRS: [&str; 4] = ["a_p0", "a_p1", "a_p2", "a_p3"];
/// `graphEnvelope` attribute layout (location = index).
pub const ENVELOPE_ATTRS: [&str; 2] = ["a_b0", "a_b1"];

/// Seam pad slots before the ring in a level VBO (a copy of the last slot).
pub const PAD_LEAD: usize = 1;
/// Seam pad slots after the ring (copies of slots `0` and `1`).
pub const PAD_TRAIL: usize = 2;

/// Vertices per polyline segment instance (quad + bevel triangle).
pub const LINE_VERTS_PER_INSTANCE: u64 = 9;
/// Vertices per envelope bucket-pair instance.
pub const ENVELOPE_VERTS_PER_INSTANCE: u64 = 4;

/// One instanced draw of a selection: `instances` consecutive instances from
/// ring slot `start_slot` (instance `i` starts at entry `start_slot + i`).
/// `first`: instance 0 starts the polyline (no previous sample).  `last`:
/// the instance ending the polyline (no next sample), `-1` for none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeDraw {
    pub start_slot: usize,
    pub instances: usize,
    pub first: bool,
    pub last: i32,
}

/// Split a selection of `level` into at most two instanced draws.  An
/// instance joins entry `j` to `j + 1`, so `count` entries make `count - 1`
/// instances; across a wrap the first draw's last instance reaches into the
/// trailing seam pad.  Pure.
pub fn plan_range_draws(sel: &Selection) -> Vec<RangeDraw> {
    let (s1, n1) = sel.ranges[0];
    let n2 = sel.ranges[1].1;
    let mut out = Vec::new();
    if n1 + n2 < 2 {
        return out;
    }
    if n2 == 0 {
        out.push(RangeDraw { start_slot: s1, instances: n1 - 1, first: true, last: n1 as i32 - 2 });
        return out;
    }
    // The first draw's last instance crosses the seam into slot 0.
    let last1 = if n2 == 1 { n1 as i32 - 1 } else { -1 };
    out.push(RangeDraw { start_slot: s1, instances: n1, first: true, last: last1 });
    if n2 >= 2 {
        out.push(RangeDraw { start_slot: 0, instances: n2 - 1, first: false, last: n2 as i32 - 2 });
    }
    out
}

/// Vertices a selection draws (the `graph_verts` perf counter).
pub fn selection_vertex_count(sel: &Selection) -> u64 {
    let per = match sel.mode {
        GraphMode::Polyline => LINE_VERTS_PER_INSTANCE,
        GraphMode::Envelope => ENVELOPE_VERTS_PER_INSTANCE,
    };
    plan_range_draws(sel).iter().map(|d| d.instances as u64 * per).sum()
}

/// `level`'s slots with the seam pads: `[last][slots..][slot 0][slot 1]`.
pub fn padded_level_bytes(buf: &SeriesBuf, level: u32) -> Vec<u8> {
    let stride = SeriesBuf::slot_stride(level);
    let src = buf.level_bytes(level);
    let slots = buf.slots(level);
    let mut out = Vec::with_capacity((slots + PAD_LEAD + PAD_TRAIL) * stride);
    out.extend_from_slice(&src[(slots - 1) * stride..]);
    out.extend_from_slice(src);
    out.extend_from_slice(&src[..PAD_TRAIL * stride]);
    out
}

struct LevelVbo {
    buffer: glow::Buffer,
    slots: usize,
}

/// A series' GPU mirror: one lazily uploaded VBO per pyramid level and the
/// VAO its draws configure (instance divisors live there, off the shared
/// VAO).
pub struct SeriesGpu {
    vao: glow::VertexArray,
    levels: Vec<Option<LevelVbo>>,
    capacity: usize,
}

impl SeriesGpu {
    /// An empty mirror for `buf` (no level resident yet).  Clears `buf`'s
    /// dirty tracking: a level uploads whole on first use.
    pub fn new(gl: &glow::Context, buf: &mut SeriesBuf) -> Self {
        let vao = unsafe { gl.create_vertex_array() }.expect("create graph VAO");
        unsafe {
            gl.bind_vertex_array(Some(vao));
            for i in 0..LINE_ATTRS.len() as u32 {
                gl.vertex_attrib_divisor(i, 1);
            }
            gl.bind_vertex_array(None);
        }
        buf.take_dirty();
        Self {
            vao,
            levels: (0..=buf.max_level()).map(|_| None).collect(),
            capacity: buf.capacity(),
        }
    }

    /// The ring capacity this mirror was built for; a series whose capacity
    /// changed needs a new mirror.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Bytes resident on the GPU.
    pub fn gpu_bytes(&self) -> usize {
        self.levels
            .iter()
            .enumerate()
            .filter_map(|(k, l)| {
                l.as_ref()
                    .map(|l| (l.slots + PAD_LEAD + PAD_TRAIL) * SeriesBuf::slot_stride(k as u32))
            })
            .sum()
    }

    /// Re-upload the slots that changed since the last sync, for every
    /// resident level (`buffer_sub_data` per dirty range, plus the seam pads
    /// of a level that changed).
    pub fn sync(&mut self, gl: &glow::Context, buf: &mut SeriesBuf) {
        assert_eq!(buf.capacity(), self.capacity, "SeriesGpu::sync: the series was resized");
        for dirty in buf.take_dirty() {
            let Some(vbo) = &self.levels[dirty.level as usize] else { continue };
            let stride = SeriesBuf::slot_stride(dirty.level);
            let src = buf.level_bytes(dirty.level);
            let slots = vbo.slots;
            unsafe {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo.buffer));
                for &(first, count) in &dirty.ranges {
                    gl.buffer_sub_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        ((PAD_LEAD + first) * stride) as i32,
                        &src[first * stride..(first + count) * stride],
                    );
                }
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, &src[(slots - 1) * stride..]);
                gl.buffer_sub_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    ((PAD_LEAD + slots) * stride) as i32,
                    &src[..PAD_TRAIL * stride],
                );
            }
        }
    }

    fn ensure_level(&mut self, gl: &glow::Context, buf: &SeriesBuf, level: u32) -> glow::Buffer {
        if let Some(vbo) = &self.levels[level as usize] {
            return vbo.buffer;
        }
        let bytes = padded_level_bytes(buf, level);
        let buffer = unsafe { gl.create_buffer() }.expect("create graph VBO");
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::DYNAMIC_DRAW);
        }
        self.levels[level as usize] = Some(LevelVbo { buffer, slots: buf.slots(level) });
        buffer
    }

    /// Free the VAO and every level VBO (GL defers it past pending draws).
    pub fn delete(self, gl: &glow::Context) {
        unsafe {
            for l in self.levels.into_iter().flatten() {
                gl.delete_buffer(l.buffer);
            }
            gl.delete_vertex_array(self.vao);
        }
    }
}

/// How to draw one series into a plot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphSeriesParams {
    /// The plot rect `(x, y, w, h)` in logical points (top-left origin).
    pub rect: glam::Vec4,
    /// The view `[x_min, x_max, y_min, y_max]` in data units.
    pub view: [f64; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    /// Line width in logical points.
    pub line_width: f32,
    /// Alpha multiplier of a wide min/max band.
    pub envelope_alpha: f32,
}

/// The data → physical-pixel mapping uniforms: `(origin, scale, offset)`
/// with `px = offset + (p - origin) * scale` (y up in data, down in pixels).
pub fn plot_transform(
    rect: glam::Vec4,
    view: [f64; 4],
    ratio: f32,
) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let [x0, x1, y0, y1] = view;
    let r = ratio as f64;
    let span = |a: f64, b: f64| if (b - a).abs() > f64::MIN_POSITIVE { b - a } else { 1.0 };
    let sx = rect.z as f64 * r / span(x0, x1);
    let sy = -(rect.w as f64) * r / span(y0, y1);
    let offset = [(rect.x as f64 * r) as f32, ((rect.y + rect.w) as f64 * r) as f32];
    ([x0 as f32, y0 as f32], [sx as f32, sy as f32], offset)
}

impl Gfx {
    /// Draw `sel` of one series (see the module docs); returns the vertex
    /// count.  The caller scissors to the plot rect
    /// ([`Gfx::scissor_logical`]) and runs in the UI phase (depth test off,
    /// the default `SRC_ALPHA` blend).
    pub fn draw_graph_series(
        &self,
        gpu: &mut SeriesGpu,
        buf: &SeriesBuf,
        sel: &Selection,
        p: &GraphSeriesParams,
    ) -> u64 {
        let draws = plan_range_draws(sel);
        if draws.is_empty() {
            return 0;
        }
        let gl = &self.gl;
        let vbo = gpu.ensure_level(gl, buf, sel.level);
        let ratio = self.pixel_ratio;
        let (origin, scale, offset) = plot_transform(p.rect, p.view, ratio);
        let target = [self.viewport_px_w.max(1) as f32, self.viewport_px_h.max(1) as f32];
        let width_px = p.line_width.max(0.0) * ratio;
        // Lines thinner than a pixel draw one pixel wide, fainter.
        let half = (0.5 * width_px).max(0.5);
        let thin = width_px.min(1.0);
        let envelope = sel.mode == GraphMode::Envelope;
        let s = self.shader(if envelope { "graphEnvelope" } else { "graphLine" });
        s.bind(gl);
        s.uniform_vec2(gl, "u_origin", &origin);
        s.uniform_vec2(gl, "u_scale", &scale);
        s.uniform_vec2(gl, "u_offset", &offset);
        s.uniform_vec2(gl, "u_target", &target);
        s.uniform_1f(gl, "u_half_width", half);
        let [r, g, b, a] = p.color;
        s.uniform_vec4(gl, "u_color", &[r, g, b, a * thin]);
        s.uniform_1f(gl, "u_envelope_alpha", p.envelope_alpha);
        let stride = SeriesBuf::slot_stride(sel.level);
        let mut verts = 0u64;
        unsafe {
            gl.bind_vertex_array(Some(gpu.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let attrs: u32 = if envelope { 2 } else { 4 };
            for i in 0..LINE_ATTRS.len() as u32 {
                if i < attrs {
                    gl.enable_vertex_attrib_array(i);
                } else {
                    gl.disable_vertex_attrib_array(i);
                }
            }
            for d in &draws {
                if d.instances == 0 {
                    continue;
                }
                if envelope {
                    let base = (PAD_LEAD + d.start_slot) * stride;
                    for i in 0..2u32 {
                        let off = base + i as usize * stride;
                        gl.vertex_attrib_pointer_f32(
                            i,
                            4,
                            glow::FLOAT,
                            false,
                            stride as i32,
                            off as i32,
                        );
                    }
                    gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, d.instances as i32);
                    verts += d.instances as u64 * ENVELOPE_VERTS_PER_INSTANCE;
                } else {
                    // a_p0 is the slot before the segment (the lead pad at slot 0).
                    let base = (PAD_LEAD + d.start_slot - 1) * stride;
                    for i in 0..4u32 {
                        let off = base + i as usize * stride;
                        gl.vertex_attrib_pointer_f32(
                            i,
                            2,
                            glow::FLOAT,
                            false,
                            stride as i32,
                            off as i32,
                        );
                    }
                    s.uniform_1i(gl, "u_first", d.first as i32);
                    s.uniform_1i(gl, "u_last", d.last);
                    gl.draw_arrays_instanced(
                        glow::TRIANGLES,
                        0,
                        LINE_VERTS_PER_INSTANCE as i32,
                        d.instances as i32,
                    );
                    verts += d.instances as u64 * LINE_VERTS_PER_INSTANCE;
                }
            }
            gl.bind_vertex_array(Some(self.vao));
        }
        crate::perf::count("graph_verts", verts);
        verts
    }
}

// Keep the bucket layout the envelope shader reads in lockstep with core.
const _: () = assert!(std::mem::size_of::<Bucket>() == 16);

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(level: u32, mode: GraphMode, ranges: [(usize, usize); 2]) -> Selection {
        let count = ranges[0].1 + ranges[1].1;
        Selection { level, mode, start: 0, count, ranges }
    }

    #[test]
    fn an_unwrapped_selection_is_one_draw_of_count_minus_one_instances() {
        let d = plan_range_draws(&sel(0, GraphMode::Polyline, [(5, 10), (0, 0)]));
        assert_eq!(d, vec![RangeDraw { start_slot: 5, instances: 9, first: true, last: 8 }]);
        assert!(plan_range_draws(&sel(0, GraphMode::Polyline, [(5, 1), (0, 0)])).is_empty());
    }

    #[test]
    fn a_wrapped_selection_bridges_the_seam() {
        // Entries in slots 6, 7 | 0, 1, 2: segments 6-7, 7-0 (seam), 0-1, 1-2.
        let d = plan_range_draws(&sel(0, GraphMode::Polyline, [(6, 2), (0, 3)]));
        assert_eq!(
            d,
            vec![
                RangeDraw { start_slot: 6, instances: 2, first: true, last: -1 },
                RangeDraw { start_slot: 0, instances: 2, first: false, last: 1 },
            ]
        );
        let total: usize = d.iter().map(|r| r.instances).sum();
        assert_eq!(total, 4, "count - 1 segments, none lost at the seam");
        // One entry past the seam: the seam segment ends the line.
        let d = plan_range_draws(&sel(0, GraphMode::Polyline, [(6, 2), (0, 1)]));
        assert_eq!(d, vec![RangeDraw { start_slot: 6, instances: 2, first: true, last: 1 }]);
    }

    #[test]
    fn vertex_counts_follow_the_mode() {
        let line = sel(0, GraphMode::Polyline, [(0, 101), (0, 0)]);
        assert_eq!(selection_vertex_count(&line), 100 * LINE_VERTS_PER_INSTANCE);
        let env = sel(3, GraphMode::Envelope, [(10, 5), (0, 6)]);
        assert_eq!(selection_vertex_count(&env), 10 * ENVELOPE_VERTS_PER_INSTANCE);
    }

    #[test]
    fn seam_pads_copy_the_ring_ends() {
        let x: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let y: Vec<f32> = x.iter().map(|v| v * 10.0).collect();
        let buf = SeriesBuf::from_points(&x, &y);
        let bytes = padded_level_bytes(&buf, 0);
        let pts: &[[f32; 2]] = bytemuck::cast_slice(&bytes);
        assert_eq!(pts.len(), 8 + PAD_LEAD + PAD_TRAIL);
        assert_eq!(pts[0], [7.0, 70.0], "lead pad = last slot");
        assert_eq!(&pts[1..9], buf.points());
        assert_eq!(pts[9..], [[0.0, 0.0], [1.0, 10.0]], "trail pads = slots 0, 1");
        let b = padded_level_bytes(&buf, 1);
        let buckets: &[Bucket] = bytemuck::cast_slice(&b);
        assert_eq!(buckets.len(), buf.slots(1) + PAD_LEAD + PAD_TRAIL);
        assert_eq!(buckets[1], buf.level(1)[0]);
    }

    #[test]
    fn the_plot_transform_maps_the_view_onto_the_rect_in_pixels() {
        let rect = glam::Vec4::new(10.0, 20.0, 100.0, 50.0);
        for ratio in [1.0f32, 2.0] {
            let (o, s, off) = plot_transform(rect, [0.0, 10.0, -1.0, 1.0], ratio);
            let px = |x: f32, y: f32| [off[0] + (x - o[0]) * s[0], off[1] + (y - o[1]) * s[1]];
            assert_eq!(px(0.0, -1.0), [10.0 * ratio, 70.0 * ratio], "bottom-left");
            assert_eq!(px(10.0, 1.0), [110.0 * ratio, 20.0 * ratio], "top-right");
        }
    }
}
