//! Boot plan: a precomputed, incrementally-consumable hydration pipeline.
//!
//! [`BootPlan`] is a `Vec<BootStep>` built once by [`crate::Engine::begin_boot`]
//! and drained one-or-many steps per frame by [`crate::Engine::boot_step`].  Each
//! texture is split into a CPU [`BootStep::Decode`] (owned [`DecodedTexture`],
//! `Send`) and a GL [`BootStep::Upload`], so decode can later move off the main
//! thread while upload stays on it.

use std::collections::HashMap;

use classic_rom::{BootSink, LoadedRoms, ResourceKind};

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
    pub(crate) bytes: Vec<u8>,
    pub(crate) format: String,
}

/// One unit of boot work.
#[derive(Clone, Debug)]
pub enum BootStep {
    /// Decode a texture to owned pixels (CPU; off-thread-able).
    Decode { key: String, rom: String, kind: ResourceKind, format: TextureFormat, bytes: Vec<u8> },
    /// Upload a previously-decoded texture (looked up by `key` in the plan).
    Upload { key: String },
    /// Alias one texture key to another already-uploaded key (shared `src`).
    AliasTexture { key: String, from_key: String },
    /// Register one ROM's non-GL metadata (texture names, depth/normal
    /// bookkeeping, animations, frame tables, animation channels, vehicles,
    /// data artifacts).
    RegisterMetadata { ns: String, entry: usize },
    /// Load one SDF font (decode atlas + upload + register metrics).
    LoadSdfFont { key: String, metrics_json: String, atlas_png: Vec<u8> },
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
