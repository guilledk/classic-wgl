//! Analytic GPU particles: immutable spawn records in ring buffers, evaluated
//! in closed form by the instanced `particle.vert`.
//!
//! The CPU writes each particle **once**, at spawn, into a [`ParticleRing`]
//! (a fixed-capacity ring of 48-byte [`ParticleRecord`]s).  Every frame only the
//! slots written since the last upload are sent (`buffer_sub_data` of at most
//! two contiguous ranges — the run before and after the wrap).  The draw is one
//! `drawArraysInstanced(TRIANGLE_STRIP, 0, 4, capacity)`: the quad corner comes
//! from `gl_VertexID`, the record from three per-instance `vec4` attributes,
//! and dead slots (`t < 0 || t > life`) collapse to a degenerate vertex.
//!
//! Each ring owns its own VAO so the instance divisors never leak into the
//! renderer's shared VAO; [`Gfx::draw_particles`] restores the shared VAO.

use glam::{Mat4, Vec3};
use glow::HasContext;

use crate::{Gfx, GlTexture, RenderSettings};

/// One particle's immutable spawn record (3 × `vec4`, 48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleRecord {
    /// Spawn world position (metres).
    pub p0: [f32; 3],
    /// Birth time (seconds, on the particle clock's current epoch).
    pub t_birth: f32,
    /// Initial velocity (m/s).
    pub v0: [f32; 3],
    /// Lifetime (s); `0` marks an unused slot.
    pub life: f32,
    /// Per-particle random in `[0, 1)`; the shader hashes it for more.
    pub seed: f32,
    /// Base size (metres, quad diameter).
    pub size: f32,
    /// Initial rotation (radians).
    pub rot: f32,
    /// Ground height under the spawn point (world z, metres).
    pub ground_z: f32,
}

/// Byte size of one record (the instance stride).
pub const PARTICLE_RECORD_BYTES: usize = std::mem::size_of::<ParticleRecord>();

impl ParticleRecord {
    /// Whether the particle is alive at clock time `t`.
    pub fn alive_at(&self, t: f32) -> bool {
        self.life > 0.0 && t >= self.t_birth && t <= self.t_birth + self.life
    }
}

/// A fixed-capacity CPU ring of records plus the dirty span written since the
/// last upload.  GL-free (unit tested); [`ParticleGpu`] mirrors it on the GPU.
#[derive(Clone, Debug)]
pub struct ParticleRing {
    records: Vec<ParticleRecord>,
    head: usize,
    /// Slots written since the last [`ParticleRing::take_dirty`], from
    /// `dirty_start` (wrapping), saturating at the capacity.
    dirty_start: usize,
    dirty_len: usize,
    /// Latest `t_birth + life` written (an O(1) "anything alive?" check).
    max_death: f32,
}

impl ParticleRing {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            records: vec![ParticleRecord::default(); capacity],
            head: 0,
            dirty_start: 0,
            dirty_len: 0,
            max_death: f32::NEG_INFINITY,
        }
    }

    pub fn capacity(&self) -> usize {
        self.records.len()
    }

    pub fn records(&self) -> &[ParticleRecord] {
        &self.records
    }

    /// Write one record at the head (overwriting the oldest slot).
    pub fn push(&mut self, r: ParticleRecord) {
        if self.dirty_len == 0 {
            self.dirty_start = self.head;
        }
        self.records[self.head] = r;
        if r.life > 0.0 {
            self.max_death = self.max_death.max(r.t_birth + r.life);
        }
        self.head = (self.head + 1) % self.records.len();
        self.dirty_len = (self.dirty_len + 1).min(self.records.len());
    }

    /// Apply `f` to every record and mark the whole ring dirty (clock rebase).
    pub fn rewrite(&mut self, mut f: impl FnMut(&mut ParticleRecord)) {
        self.records.iter_mut().for_each(&mut f);
        self.max_death = self.recompute_last_death();
        self.dirty_start = 0;
        self.dirty_len = self.records.len();
    }

    /// The contiguous slot ranges `[start, end)` to upload — at most two (the
    /// run up to the end of the buffer and the wrapped run from 0) — and clear
    /// the dirty span.
    pub fn take_dirty(&mut self) -> Vec<(usize, usize)> {
        let cap = self.records.len();
        let len = std::mem::take(&mut self.dirty_len);
        if len == 0 {
            return Vec::new();
        }
        if len >= cap {
            return vec![(0, cap)];
        }
        let end = self.dirty_start + len;
        if end <= cap {
            vec![(self.dirty_start, end)]
        } else {
            vec![(self.dirty_start, cap), (0, end - cap)]
        }
    }

    /// Drop every record (all slots unused) and mark the whole ring dirty,
    /// so the GPU mirror is zeroed on its next upload.
    pub fn clear(&mut self) {
        self.records.fill(ParticleRecord::default());
        self.head = 0;
        self.max_death = f32::NEG_INFINITY;
        self.dirty_start = 0;
        self.dirty_len = self.records.len();
    }

    /// Live particles at clock time `t`.
    pub fn live_count(&self, t: f32) -> usize {
        self.records.iter().filter(|r| r.alive_at(t)).count()
    }

    /// Latest death time of any record written (`-inf` when empty); O(1).
    pub fn last_death(&self) -> f32 {
        self.max_death
    }

    fn recompute_last_death(&self) -> f32 {
        self.records
            .iter()
            .filter(|r| r.life > 0.0)
            .map(|r| r.t_birth + r.life)
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

/// GPU mirror of a [`ParticleRing`]: a VBO of `capacity` records and a VAO
/// with the three per-instance attributes bound at locations 0..=2.
pub struct ParticleGpu {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    capacity: usize,
}

impl ParticleGpu {
    pub fn new(gl: &glow::Context, capacity: usize) -> Self {
        unsafe {
            let vao = gl.create_vertex_array().expect("create particle VAO");
            let vbo = gl.create_buffer().expect("create particle VBO");
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                (capacity * PARTICLE_RECORD_BYTES) as i32,
                glow::DYNAMIC_DRAW,
            );
            for i in 0..3u32 {
                gl.enable_vertex_attrib_array(i);
                gl.vertex_attrib_pointer_f32(
                    i,
                    4,
                    glow::FLOAT,
                    false,
                    PARTICLE_RECORD_BYTES as i32,
                    (i * 16) as i32,
                );
                gl.vertex_attrib_divisor(i, 1);
            }
            gl.bind_vertex_array(None);
            Self { vao, vbo, capacity }
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Upload the ring's dirty ranges.
    pub fn upload(&self, gl: &glow::Context, ring: &mut ParticleRing) {
        let ranges = ring.take_dirty();
        if ranges.is_empty() {
            return;
        }
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            for (a, b) in ranges {
                let bytes: &[u8] = bytemuck::cast_slice(&ring.records()[a..b]);
                gl.buffer_sub_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    (a * PARTICLE_RECORD_BYTES) as i32,
                    bytes,
                );
            }
        }
    }

    /// Release the GL objects (instances are recycled when an effect dies).
    pub fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
    }
}

/// Per-draw layer parameters: the closed-form motion and the look, uploaded
/// as uniforms (see `particle.vert` for the exact meaning of each `vec4`).
#[derive(Clone, Debug)]
pub struct ParticleLayerUniforms {
    /// Particle clock time (s) on the records' epoch.
    pub time: f32,
    /// `gravity.xyz, drag`.
    pub motion: [f32; 4],
    /// `amp, freq, ramp, spin_max`.
    pub turbulence: [f32; 4],
    /// `slots (0 = off), kick_radius, omega, decay`.
    pub swirl0: [f32; 4],
    /// `lift, lift_time, growth, drift`.
    pub swirl1: [f32; 4],
    /// `arms, core, pinch_time, kick_jitter`.
    pub swirl2: [f32; 4],
    /// `mode (0 none, 1 clamp, 2 bounce, 3 die), restitution, offset, -`.
    pub ground: [f32; 4],
    pub size_t: [f32; 4],
    pub size_v: [f32; 4],
    pub color_t: [f32; 4],
    pub color_v: [[f32; 4]; 4],
    /// `mode (0 billboard, 1 streak), streak_time, depth_bias, -`.
    pub render: [f32; 4],
    /// `shape (0 soft, 1 smoke, 2 glow), softness, additive, alpha_mul`.
    pub shape: [f32; 4],
    /// `lit, emissive, -, -`.
    pub shade: [f32; 4],
    /// Emitter tint.
    pub tint: [f32; 3],
    /// `cols, rows, frames, fps`; `cols == 0` = no flipbook.
    pub flipbook: [f32; 4],
    /// Texture name (sampled when present, else the procedural shape).
    pub texture: Option<String>,
}

impl Default for ParticleLayerUniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            motion: [0.0; 4],
            turbulence: [0.0; 4],
            swirl0: [0.0; 4],
            swirl1: [0.0; 4],
            swirl2: [0.0; 4],
            ground: [0.0; 4],
            size_t: [0.0, 1.0, 2.0, 3.0],
            size_v: [1.0; 4],
            color_t: [0.0, 1.0, 2.0, 3.0],
            color_v: [[1.0; 4]; 4],
            render: [0.0, 0.05, 0.5, 0.0],
            shape: [0.0, 1.0, 0.0, 1.0],
            shade: [0.0; 4],
            tint: [1.0; 3],
            flipbook: [0.0; 4],
            texture: None,
        }
    }
}

/// Texture unit the particle sampler uses (clear of the sprite maps 0..2, the
/// shadow map 3 and the model composite 4..5).
const PARTICLE_TEX_UNIT: u32 = 6;
/// Texture unit of the baked smoke noise.
const PARTICLE_NOISE_UNIT: u32 = 9;

impl Gfx {
    /// Draw one particle ring (all `capacity` instances; dead slots collapse).
    ///
    /// Depth test `LEQUAL` against the scene, no depth write, premultiplied
    /// blending (`ONE, ONE_MINUS_SRC_ALPHA`) so one blend state serves alpha
    /// (`alpha = a`), additive (`alpha = 0`) and hybrid layers.  Restores the
    /// shared VAO, blend func and depth state.
    pub fn draw_particles(
        &self,
        gpu: &ParticleGpu,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
        u: &ParticleLayerUniforms,
    ) {
        let gl = &self.gl;
        let s = self.shader("particle");
        s.bind(gl);
        s.uniform_mat4(gl, "projection_matrix", &self.projection());
        s.uniform_mat4(gl, "camera_matrix", camera);
        s.uniform_mat4(gl, "world_matrix", world_matrix);
        s.uniform_vec2(gl, "depth_span", &settings.depth_span);
        s.uniform_1f(gl, "ppm", settings.ppm);
        s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
        s.uniform_1f(gl, "u_time", u.time);
        s.uniform_vec4(gl, "u_motion", &u.motion);
        s.uniform_vec4(gl, "u_turb", &u.turbulence);
        s.uniform_vec4(gl, "u_swirl0", &u.swirl0);
        s.uniform_vec4(gl, "u_swirl1", &u.swirl1);
        s.uniform_vec4(gl, "u_swirl2", &u.swirl2);
        s.uniform_vec4(gl, "u_ground", &u.ground);
        s.uniform_vec4(gl, "u_size_t", &u.size_t);
        s.uniform_vec4(gl, "u_size_v", &u.size_v);
        s.uniform_vec4(gl, "u_color_t", &u.color_t);
        s.uniform_vec4_array(gl, "u_color_v", &u.color_v);
        s.uniform_vec4(gl, "u_render", &u.render);
        s.uniform_vec4(gl, "u_shape", &u.shape);
        s.uniform_vec4(gl, "u_shade", &u.shade);
        s.uniform_vec3(gl, "u_tint", Vec3::from_array(u.tint));
        s.uniform_vec4(gl, "u_flip", &u.flipbook);
        match u.texture.as_deref().and_then(|t| self.textures.get(t)) {
            Some(t) => {
                t.bind(gl, PARTICLE_TEX_UNIT);
                // ROM textures load NEAREST for the pixel-art sprites; a
                // sampler object overrides that on this unit only, so the same
                // texture still samples NEAREST wherever a sprite draws it.
                unsafe { gl.bind_sampler(PARTICLE_TEX_UNIT, Some(self.particle_sampler)) };
                s.uniform_1i(gl, "u_tex", PARTICLE_TEX_UNIT as i32);
                s.uniform_1f(gl, "u_use_tex", 1.0);
            }
            None => {
                s.uniform_1i(gl, "u_tex", PARTICLE_TEX_UNIT as i32);
                s.uniform_1f(gl, "u_use_tex", 0.0);
            }
        }
        self.particle_noise.bind(gl, PARTICLE_NOISE_UNIT);
        s.uniform_1i(gl, "u_noise", PARTICLE_NOISE_UNIT as i32);
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(gpu.vao));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, gpu.capacity as i32);
            gl.bind_sampler(PARTICLE_TEX_UNIT, None);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(true);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(self.vao));
        }
    }
}

/// Edge length (texels) of the baked particle noise and its lattice size.
pub const NOISE_SIZE: usize = 64;
const NOISE_CELLS: usize = 8;

/// Tileable 3-octave value noise baked for the smoke shape: `R` = billow
/// value, `G`/`B` = its x/y slope per lattice cell encoded `0.5 + s / 4`.
/// Replaces 9 per-pixel noise evaluations (value + two gradient taps x 3
/// octaves, 4 `sin` hashes each) with one bilinear fetch.  Octaves use 1x,
/// 2x and 4x lattices so the tile wraps.  Pure (unit tested).
pub fn bake_noise() -> Vec<u8> {
    let hash = |x: usize, y: usize, s: u32| -> f32 {
        let mut v = (x as u32).wrapping_mul(374_761_393)
            ^ (y as u32).wrapping_mul(668_265_263)
            ^ s.wrapping_mul(1_274_126_177);
        v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
        ((v ^ (v >> 16)) & 0xffff) as f32 / 65535.0
    };
    let octave = |u: f32, v: f32, cells: usize, s: u32| -> f32 {
        let (x, y) = (u * cells as f32, v * cells as f32);
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let w = |i: usize| i % cells;
        let a = hash(w(ix), w(iy), s);
        let b = hash(w(ix + 1), w(iy), s);
        let c = hash(w(ix), w(iy + 1), s);
        let d = hash(w(ix + 1), w(iy + 1), s);
        let top = a + (b - a) * sx;
        top + (c + (d - c) * sx - top) * sy
    };
    let billow = |u: f32, v: f32| {
        0.68 * octave(u, v, NOISE_CELLS, 1)
            + 0.25 * octave(u, v, NOISE_CELLS * 2, 2)
            + 0.07 * octave(u, v, NOISE_CELLS * 4, 3)
    };
    let n = NOISE_SIZE;
    let val: Vec<f32> = (0..n * n)
        .map(|i| billow(((i % n) as f32 + 0.5) / n as f32, ((i / n) as f32 + 0.5) / n as f32))
        .collect();
    // Central differences, in billow units per lattice cell.
    let step = 2.0 * NOISE_CELLS as f32 / n as f32;
    let at = |x: usize, y: usize| val[(y % n) * n + x % n];
    let enc = |s: f32| ((0.5 + s * 0.25).clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let sx = (at(x + 1, y) - at(x + n - 1, y)) / step;
            let sy = (at(x, y + 1) - at(x, y + n - 1)) / step;
            let i = (y * n + x) * 4;
            out[i] = (at(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i + 1] = enc(sx);
            out[i + 2] = enc(sy);
            out[i + 3] = 255;
        }
    }
    out
}

/// The sampler particle textures draw through: LINEAR (no mips: ROM textures
/// have none) with clamped edges, whatever the texture's own filter.
pub(crate) fn linear_sampler(gl: &glow::Context) -> glow::Sampler {
    unsafe {
        let s = gl.create_sampler().expect("create sampler");
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.sampler_parameter_i32(s, name, value as i32);
        }
        s
    }
}

/// Upload [`bake_noise`] as a LINEAR, REPEAT texture.
pub(crate) fn noise_texture(gl: &glow::Context) -> GlTexture {
    let t = GlTexture::from_rgba8(gl, &bake_noise(), NOISE_SIZE as u32, NOISE_SIZE as u32);
    t.set_linear(gl);
    t.set_repeat(gl);
    t
}

/// Floats per ribbon vertex (see `ribbon.vert`).
pub const RIBBON_VERTEX_FLOATS: usize = 9;

impl Gfx {
    /// Draw `vertex_count` CPU-expanded ribbon vertices (triangles) from `vb`:
    /// additive, depth-tested `LEQUAL` without depth write.
    pub fn draw_ribbons(&self, vb: &crate::DynamicVb, vertex_count: i32, camera: &Mat4) {
        if vertex_count <= 0 {
            return;
        }
        let gl = &self.gl;
        let s = self.shader("ribbon");
        s.bind(gl);
        s.uniform_mat4(gl, "projection_matrix", &self.projection());
        s.uniform_mat4(gl, "camera_matrix", camera);
        let stride = (RIBBON_VERTEX_FLOATS * 4) as i32;
        vb.bind(gl);
        unsafe {
            for (name, off) in [("a_pos", 0), ("a_shape", 12), ("a_color", 24)] {
                let loc = s.attr(name);
                gl.vertex_attrib_pointer_f32(loc, 3, glow::FLOAT, false, stride, off);
                gl.enable_vertex_attrib_array(loc);
            }
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(true);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: f32) -> ParticleRecord {
        ParticleRecord { t_birth: t, life: 1.0, ..Default::default() }
    }

    #[test]
    fn baked_noise_tiles_and_spans_its_range() {
        let px = bake_noise();
        let n = NOISE_SIZE;
        assert_eq!(px.len(), n * n * 4);
        let r: Vec<u8> = px.chunks(4).map(|c| c[0]).collect();
        let (lo, hi) = (*r.iter().min().unwrap(), *r.iter().max().unwrap());
        assert!(lo < 90 && hi > 165, "billow range {lo}..{hi}");
        // Wraps: the last column continues into the first (no seam jump).
        for y in 0..n {
            let d = (r[y * n] as i32 - r[y * n + n - 1] as i32).abs();
            assert!(d < 40, "seam at row {y}: {d}");
        }
        // Slopes are centred on 0.5 (128) on average.
        let gx: f32 = px.chunks(4).map(|c| c[1] as f32).sum::<f32>() / (n * n) as f32;
        assert!((gx - 127.5).abs() < 6.0, "mean x slope {gx}");
    }

    #[test]
    fn record_is_three_vec4s() {
        assert_eq!(PARTICLE_RECORD_BYTES, 48);
    }

    #[test]
    fn dirty_ranges_split_at_the_wrap() {
        let mut ring = ParticleRing::new(8);
        for i in 0..6 {
            ring.push(rec(i as f32));
        }
        assert_eq!(ring.take_dirty(), vec![(0, 6)]);
        assert!(ring.take_dirty().is_empty(), "upload clears the span");
        for i in 0..4 {
            ring.push(rec(10.0 + i as f32));
        }
        // Slots 6, 7, then wrap to 0, 1.
        assert_eq!(ring.take_dirty(), vec![(6, 8), (0, 2)]);
        assert_eq!(ring.records()[1].t_birth, 13.0);
    }

    #[test]
    fn overfilling_uploads_the_whole_ring_once() {
        let mut ring = ParticleRing::new(4);
        for i in 0..11 {
            ring.push(rec(i as f32));
        }
        assert_eq!(ring.take_dirty(), vec![(0, 4)]);
        // The newest four survive.
        let mut births: Vec<f32> = ring.records().iter().map(|r| r.t_birth).collect();
        births.sort_by(f32::total_cmp);
        assert_eq!(births, vec![7.0, 8.0, 9.0, 10.0]);
    }

    #[test]
    fn rewrite_marks_everything_dirty_and_live_count_tracks_time() {
        let mut ring = ParticleRing::new(4);
        ring.push(rec(0.0));
        ring.push(rec(0.5));
        ring.take_dirty();
        assert_eq!(ring.live_count(0.75), 2);
        assert_eq!(ring.live_count(1.25), 1);
        assert_eq!(ring.live_count(2.0), 0);
        assert_eq!(ring.last_death(), 1.5);
        ring.rewrite(|r| r.t_birth -= 0.5);
        assert_eq!(ring.take_dirty(), vec![(0, 4)]);
        assert_eq!(ring.live_count(0.75), 1);
    }

    #[test]
    fn clear_drops_everything_and_reuploads_the_whole_ring() {
        let mut ring = ParticleRing::new(4);
        ring.push(rec(0.0));
        ring.push(rec(0.5));
        ring.take_dirty();
        ring.clear();
        assert_eq!(ring.live_count(0.75), 0);
        assert_eq!(ring.last_death(), f32::NEG_INFINITY);
        // A later push still uploads the zeroed slots with it.
        ring.push(rec(2.0));
        assert_eq!(ring.take_dirty(), vec![(0, 4)]);
        assert_eq!(ring.live_count(2.5), 1);
    }
}
