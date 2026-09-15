//! ROM boot: the [`BootPipeline`] sequencer over a precomputed [`BootPlan`].
//!
//! [`BootPlan`] is a `Vec<BootStep>` built once from the resolved ROM DAG.  Each
//! texture is split into a CPU [`BootStep::Decode`] (owned [`DecodedTexture`],
//! `Send`) and a GL [`BootStep::Upload`], so decode can move off the GL thread
//! while upload stays on it.  [`BootPipeline`] owns the plan and walks every
//! boot through the same stages — synchronously, time-budgeted per frame, or
//! with the CPU half prepared off-thread (see [`pipeline`]).

use std::collections::HashMap;
use std::sync::Arc;

use classic_rom::ResourceKind;
#[cfg(not(target_arch = "wasm32"))]
use classic_rom::{BootEvent, BootSink};

pub mod pipeline;

pub use pipeline::{BootFinish, BootPipeline, BootPoll, BootStage};

/// Owned, decoded texture pixels (Send), ready for GL upload.
#[derive(Clone, Debug)]
pub enum DecodedTexture {
    Rgba8 { width: u32, height: u32, pixels: Vec<u8> },
    Luma8 { width: u32, height: u32, pixels: Vec<u8> },
    Rgb8 { width: u32, height: u32, pixels: Vec<u8> },
}

impl DecodedTexture {
    pub fn dims(&self) -> (u32, u32) {
        match self {
            DecodedTexture::Rgba8 { width, height, .. }
            | DecodedTexture::Luma8 { width, height, .. }
            | DecodedTexture::Rgb8 { width, height, .. } => (*width, *height),
        }
    }
}

/// The GL channel layout a [`DecodedTexture`] uploads as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    Luma8,
    Rgb8,
}

/// A pending GPU-compressed (`.basis`) texture upload: one unique `src` sheet
/// plus every manifest entry key that aliases it.  Collected by
/// [`crate::Engine::begin_boot`] and uploaded after the plan drains
/// (synchronously on native, awaited through the web transcoder worker on wasm).
#[derive(Clone, Debug)]
pub(crate) struct BasisTextureJob {
    pub(crate) keys: Vec<String>,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) format: String,
    /// The ROM this sheet belongs to (for `ResourceDecoded` events).
    pub(crate) rom: String,
    /// The resource kind (Texture / Normal / Depth), derived from the sheet's
    /// manifest name + compressed target.
    pub(crate) kind: ResourceKind,
}

/// One unit of boot work.
#[derive(Clone, Debug)]
pub enum BootStep {
    /// Decode a texture to owned pixels (CPU; off-thread-able).
    Decode { key: String, rom: String, kind: ResourceKind, format: TextureFormat, bytes: Arc<[u8]> },
    /// Upload a previously-decoded texture (looked up by `key` in the plan).
    Upload { key: String },
    /// Alias one texture key to another already-uploaded key (shared `src`).
    AliasTexture { key: String, from_key: String },
    /// Register one ROM's non-GL metadata (texture names, depth/normal
    /// bookkeeping, animations, frame tables, animation channels, vehicles,
    /// data artifacts).
    RegisterMetadata { ns: String, entry: usize },
    /// Load one SDF font (decode atlas + upload + register metrics).
    LoadSdfFont { key: String, metrics_json: String, atlas_png: Arc<[u8]> },
    /// Hydrate one ROM's entity state + grids.
    HydrateEntry { ns: String, entry: usize },
    /// Shared tail: DAG bookkeeping, item catalog, vehicle overrides.
    Finish,
    /// An empty placeholder left in a consumed slot (never executed).
    Noop,
}

/// `Noop` is the placeholder left behind when a step is moved out of the plan
/// via `std::mem::take`, so it is never a meaningful step to execute.
impl Default for BootStep {
    fn default() -> Self {
        BootStep::Noop
    }
}

/// A precomputed hydration plan for one resolved ROM DAG, drained by a
/// [`BootPipeline`].
pub struct BootPlan {
    pub(crate) steps: Vec<BootStep>,
    /// Pending basis uploads, uploaded after the plan drains.
    pub(crate) basis_jobs: Vec<BasisTextureJob>,
    pub(crate) cursor: usize,
    /// Decoded textures awaiting upload (decode writes, upload reads).
    pub(crate) decoded: HashMap<String, DecodedTexture>,
}

impl BootPlan {
    /// The number of steps not yet consumed.
    pub fn remaining(&self) -> usize {
        self.steps.len().saturating_sub(self.cursor)
    }

    /// True when every step has been consumed.
    pub fn is_done(&self) -> bool {
        self.cursor >= self.steps.len()
    }

    /// The total number of steps in the plan.
    pub fn total_steps(&self) -> usize {
        self.steps.len()
    }

    /// The number of steps consumed so far.
    pub fn cursor(&self) -> usize {
        self.cursor
    }
}

/// Decode every pending [`BootStep::Decode`] step in `plan` into owned, `Send`
/// [`DecodedTexture`]s, stored in `plan.decoded` for the matching
/// [`BootStep::Upload`], emitting a [`BootEvent::ResourceDecoded`] per texture.
///
/// This is the off-GL-thread half of texture boot: it touches only `image`
/// decode (no GL) and consumes each `Decode` step (replacing it with the
/// default [`BootStep::Noop`]) so the large pixel payloads are moved, never
/// cloned.  Every non-decode step is left untouched for the GL thread to run.
///
/// The individual decodes fan out across the loader thread pool.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode_plan(plan: &mut BootPlan, sink: &dyn BootSink) {
    // Move every Decode step out of the plan, leaving Noop placeholders.
    let mut jobs = Vec::new();
    for step in &mut plan.steps {
        let taken = std::mem::take(step);
        match taken {
            BootStep::Decode { key, rom, kind, format, bytes } => {
                jobs.push(DecodeJob { key, rom, kind, format, bytes });
            }
            other => *step = other,
        }
    }
    if jobs.is_empty() {
        return;
    }

    // `run_all` re-assembles in plan order, so `ResourceDecoded` events stay
    // deterministic regardless of which loader thread finishes first.
    for result in loader_queue("classic-decode").run_all(jobs) {
        sink.on_event(BootEvent::ResourceDecoded {
            rom: result.rom,
            kind: result.kind,
            name: result.key.clone(),
            dims: result.texture.dims(),
        });
        plan.decoded.insert(result.key, result.texture);
    }
}

/// The pool boot-time decode fans out on: `CLASSIC_LOADER_THREADS` named threads.
#[cfg(not(target_arch = "wasm32"))]
fn loader_queue<J: classic_worker::Job<State = ()>>(name: &str) -> classic_worker::JobQueue<J> {
    let threads = crate::env_config::EnvConfig::get().loader_threads;
    classic_worker::JobQueue::pooled(name, threads).expect("failed to spawn loader threads")
}

/// A single moved-out `Decode` step, fully owned and `Send`.
#[cfg(not(target_arch = "wasm32"))]
struct DecodeJob {
    key: String,
    rom: String,
    kind: ResourceKind,
    format: TextureFormat,
    bytes: Arc<[u8]>,
}

/// A decoded texture plus the metadata of the step it came from.
#[cfg(not(target_arch = "wasm32"))]
struct DecodedResult {
    rom: String,
    kind: ResourceKind,
    key: String,
    texture: DecodedTexture,
}

#[cfg(not(target_arch = "wasm32"))]
impl classic_worker::Job for DecodeJob {
    type State = ();
    type Output = DecodedResult;

    fn run(self, _: &mut ()) -> DecodedResult {
        let texture = decode_texture(self.format, &self.bytes);
        DecodedResult { rom: self.rom, kind: self.kind, key: self.key, texture }
    }
}

/// One `.basis` sheet to transcode for the given GL capabilities.
#[cfg(not(target_arch = "wasm32"))]
struct BasisDecodeJob {
    bytes: Arc<[u8]>,
    format: String,
    caps: classic_gfx::Caps,
}

#[cfg(not(target_arch = "wasm32"))]
impl classic_worker::Job for BasisDecodeJob {
    type State = ();
    type Output = Option<classic_gfx::DecodedBasis>;

    fn run(self, _: &mut ()) -> Self::Output {
        classic_gfx::transcode_basis(&self.bytes, &self.format, self.caps)
    }
}

/// Transcode every pending `.basis` job in parallel (CPU, native only),
/// returning the decoded payload keyed by job index.  `None` marks a job that
/// failed to transcode (its texture is treated as missing).  Mirrors
/// [`decode_plan`] but for GPU-compressed sheets: the `basis_universal` decode
/// fans out across the loader queue while the GL upload stays on the render
/// thread via `Engine::upload_basis_predecoded`.
///
/// Emits a [`BootEvent::ResourceDecoded`] per successfully-transcoded sheet, in
/// plan order (so the observable stream is deterministic regardless of which
/// loader thread finishes first).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode_basis_jobs(
    jobs: &[BasisTextureJob],
    caps: classic_gfx::Caps,
    sink: &dyn BootSink,
) -> Vec<Option<classic_gfx::DecodedBasis>> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let ordered = loader_queue("classic-basis").run_all(jobs.iter().map(|job| BasisDecodeJob {
        bytes: Arc::clone(&job.bytes),
        format: job.format.clone(),
        caps,
    }));

    for (job, decoded) in jobs.iter().zip(&ordered) {
        if let Some(payload) = decoded {
            let dims = match payload {
                classic_gfx::DecodedBasis::Compressed { width, height, .. }
                | classic_gfx::DecodedBasis::Rgba8 { width, height, .. } => (*width, *height),
            };
            sink.on_event(BootEvent::ResourceDecoded {
                rom: job.rom.clone(),
                kind: job.kind,
                name: job.keys[0].clone(),
                dims,
            });
        }
    }
    ordered
}

/// Decode a PNG into owned pixels of the given channel layout.
pub fn decode_texture(format: TextureFormat, bytes: &[u8]) -> DecodedTexture {
    let img = image::load_from_memory(bytes).expect("decode PNG");
    match format {
        TextureFormat::Rgba8 => {
            let rgba = img.to_rgba8();
            DecodedTexture::Rgba8 {
                width: rgba.width(),
                height: rgba.height(),
                pixels: rgba.into_raw(),
            }
        }
        TextureFormat::Luma8 => {
            let luma = img.to_luma8();
            DecodedTexture::Luma8 {
                width: luma.width(),
                height: luma.height(),
                pixels: luma.into_raw(),
            }
        }
        TextureFormat::Rgb8 => {
            let rgb = img.to_rgb8();
            DecodedTexture::Rgb8 {
                width: rgb.width(),
                height: rgb.height(),
                pixels: rgb.into_raw(),
            }
        }
    }
}
