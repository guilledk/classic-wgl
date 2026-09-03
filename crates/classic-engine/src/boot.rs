//! Boot plan: a precomputed, incrementally-consumable hydration pipeline.
//!
//! [`BootPlan`] is a `Vec<BootStep>` built once by [`crate::Engine::begin_boot`]
//! and drained one-or-many steps per frame by [`crate::Engine::boot_step`].  Each
//! texture is split into a CPU [`BootStep::Decode`] (owned [`DecodedTexture`],
//! `Send`) and a GL [`BootStep::Upload`], so decode can later move off the main
//! thread while upload stays on it.

use std::collections::HashMap;
use std::sync::Arc;

use classic_rom::{BootEvent, BootSink, LoadedRoms, ResourceKind};

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

/// A precomputed hydration plan, drained by [`crate::Engine::boot_step`].
pub struct BootPlan<'a> {
    pub(crate) loaded: &'a LoadedRoms,
    pub(crate) sink: &'a dyn BootSink,
    pub(crate) steps: Vec<BootStep>,
    /// Pending basis uploads, uploaded after the plan drains.
    pub(crate) basis_jobs: Vec<BasisTextureJob>,
    pub(crate) cursor: usize,
    /// Decoded textures awaiting upload (decode writes, upload reads).
    pub(crate) decoded: HashMap<String, DecodedTexture>,
}

impl<'a> BootPlan<'a> {
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

    /// The number of steps consumed so far (incremental booters persist this
    /// across frames when they rebuild the plan each chunk).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Set the step cursor (used by incremental booters to resume mid-plan).
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    /// Inject pre-decoded texture pixels (the off-thread decode results) so the
    /// matching [`BootStep::Upload`] steps skip their `Decode` work.
    pub fn set_decoded(&mut self, decoded: HashMap<String, DecodedTexture>) {
        self.decoded = decoded;
    }

    /// Take the decoded-texture map back out (to persist across a plan rebuild).
    pub fn take_decoded(&mut self) -> HashMap<String, DecodedTexture> {
        std::mem::take(&mut self.decoded)
    }
}

/// Decode every pending [`BootStep::Decode`] step in `plan` into owned, `Send`
/// [`DecodedTexture`]s keyed for the matching [`BootStep::Upload`], emitting a
/// [`BootEvent::ResourceDecoded`] per texture.
///
/// This is the off-main-thread half of boot: it touches only `image` decode
/// (no GL) and consumes each `Decode` step (replacing it with the default
/// [`BootStep::Noop`]) so the large pixel payloads are moved, never cloned.
/// Every non-decode step is left untouched for the GL thread to run.  The
/// returned map is `Send` and crosses the thread boundary as the decoded-assets
/// payload.
///
/// On native the individual decodes fan out across a
/// [`classic_worker::ThreadPool`] (sized by `CLASSIC_LOADER_THREADS`); on wasm
/// they run serially (the web path decodes inline via [`crate::Engine::boot_step`]
/// instead of through this function).
pub fn decode_plan(plan: &mut BootPlan<'_>) -> HashMap<String, DecodedTexture> {
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
        return HashMap::new();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        decode_jobs_parallel(jobs, plan.sink)
    }
    #[cfg(target_arch = "wasm32")]
    {
        decode_jobs_serial(jobs, plan.sink)
    }
}

/// A single moved-out `Decode` step, fully owned and `Send`.
struct DecodeJob {
    key: String,
    rom: String,
    kind: ResourceKind,
    format: TextureFormat,
    bytes: Arc<[u8]>,
}

/// Decode `jobs` serially (wasm fallback; the native path uses the pool).
#[cfg(target_arch = "wasm32")]
fn decode_jobs_serial(
    jobs: Vec<DecodeJob>,
    sink: &dyn BootSink,
) -> HashMap<String, DecodedTexture> {
    let mut decoded = HashMap::new();
    for job in jobs {
        let texture = decode_texture(job.format, &job.bytes);
        let dims = texture.dims();
        sink.on_event(BootEvent::ResourceDecoded {
            rom: job.rom,
            kind: job.kind,
            name: job.key.clone(),
            dims,
        });
        decoded.insert(job.key, texture);
    }
    decoded
}

/// A decoded texture plus its plan-order metadata, re-assembled after the
/// parallel decode fan-out so `ResourceDecoded` events stay in plan order.
#[cfg(not(target_arch = "wasm32"))]
struct DecodedResult {
    rom: String,
    kind: ResourceKind,
    key: String,
    dims: (u32, u32),
    texture: DecodedTexture,
}

/// Decode `jobs` in parallel on a [`classic_worker::ThreadPool`], emitting
/// `ResourceDecoded` events in the original plan order so the observable event
/// stream is identical to the serial path.
#[cfg(not(target_arch = "wasm32"))]
fn decode_jobs_parallel(
    jobs: Vec<DecodeJob>,
    sink: &dyn BootSink,
) -> HashMap<String, DecodedTexture> {
    use std::sync::mpsc;

    let threads = crate::env_config::EnvConfig::get().loader_threads;
    let pool = classic_worker::ThreadPool::new(threads);
    let total = jobs.len();
    let (tx, rx) = mpsc::channel();

    for (index, job) in jobs.into_iter().enumerate() {
        let tx = tx.clone();
        pool.spawn(move || {
            let texture = decode_texture(job.format, &job.bytes);
            let dims = texture.dims();
            let result =
                DecodedResult { rom: job.rom, kind: job.kind, key: job.key, dims, texture };
            let _ = tx.send((index, result));
        });
    }
    drop(tx);

    // Re-assemble in plan order so `ResourceDecoded` events stay deterministic
    // regardless of which pool thread finishes first.
    let mut ordered: Vec<Option<DecodedResult>> = (0..total).map(|_| None).collect();
    for _ in 0..total {
        let (index, result) = rx.recv().expect("decode worker panicked");
        ordered[index] = Some(result);
    }

    let mut decoded = HashMap::new();
    for result in ordered.into_iter().flatten() {
        sink.on_event(BootEvent::ResourceDecoded {
            rom: result.rom,
            kind: result.kind,
            name: result.key.clone(),
            dims: result.dims,
        });
        decoded.insert(result.key, result.texture);
    }
    decoded
}

/// Transcode every pending `.basis` job in parallel (CPU, native only),
/// returning the decoded payload keyed by job index.  `None` marks a job that
/// failed to transcode (its texture is treated as missing).  Mirrors
/// [`decode_plan`] but for GPU-compressed sheets: the `basis_universal` decode
/// fans out across the loader pool while the GL upload stays on the render
/// thread via `Engine::upload_basis_predecoded`.
///
/// Emits a [`BootEvent::ResourceDecoded`] per successfully-transcoded sheet, in
/// plan order (so the observable stream is deterministic regardless of which
/// pool thread finishes first).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode_basis_jobs(
    jobs: &[BasisTextureJob],
    caps: classic_gfx::Caps,
    sink: &dyn BootSink,
) -> Vec<Option<classic_gfx::DecodedBasis>> {
    let total = jobs.len();
    if total == 0 {
        return Vec::new();
    }
    use std::sync::mpsc;

    let threads = crate::env_config::EnvConfig::get().loader_threads;
    let pool = classic_worker::ThreadPool::new(threads);
    let (tx, rx) = mpsc::channel();

    for (index, job) in jobs.iter().enumerate() {
        let tx = tx.clone();
        let bytes = Arc::clone(&job.bytes);
        let format = job.format.clone();
        pool.spawn(move || {
            let decoded = classic_gfx::transcode_basis(&bytes, &format, caps);
            let _ = tx.send((index, decoded));
        });
    }
    drop(tx);

    let mut ordered: Vec<Option<classic_gfx::DecodedBasis>> = (0..total).map(|_| None).collect();
    for _ in 0..total {
        let (index, decoded) = rx.recv().expect("basis worker panicked");
        ordered[index] = decoded;
    }

    // Re-assemble in plan order so `ResourceDecoded` events stay deterministic.
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

/// Decode every pending texture *and* `.basis` sheet off-thread (native only),
/// returning the decoded payloads.  `basis` is indexed to match the plan's
/// `basis_jobs` order (upload one at a time with
/// [`crate::Engine::upload_basis_predecoded_at`]).  The heavy work fans out
/// across the loader pool; the caller uploads on the GL thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn decode_assets(
    loaded: &LoadedRoms,
    caps: classic_gfx::Caps,
    sink: &dyn BootSink,
) -> (HashMap<String, DecodedTexture>, Vec<Option<classic_gfx::DecodedBasis>>) {
    let engine = crate::Engine::new();
    let mut plan = engine.begin_boot(loaded, sink);
    let decoded = decode_plan(&mut plan);
    let basis = decode_basis_jobs(&plan.basis_jobs, caps, sink);
    (decoded, basis)
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
