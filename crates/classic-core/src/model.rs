//! glTF (`.glb`) model parse + node-hierarchy animation, GL-free.
//!
//! This is the pure-CPU half of the 3D-model path: it parses a self-contained
//! glTF binary (`.glb`) into an [`ModelAsset`] (nodes, meshes, clips) and
//! samples the node hierarchy at a clip time, producing per-node world
//! transforms in the engine's Blender-canonical world space (+Z up, metres).
//!
//! The US Rocket export is **node-parented, not vertex-skinned**: the armature
//! bones export as a node hierarchy, each mesh is parented to a bone node, and
//! the clips animate those nodes' TRS.  There are no `JOINTS_0`/`WEIGHTS_0`
//! attributes on any primitive (verified against the real `.glb`), so the
//! skinning math here is *hierarchy* animation — no matrix palette — which
//! maps 1:1 onto the exporter's object-parented rig.

use anyhow::{anyhow, bail, Result};
use glam::{Mat4, Quat, Vec3, Vec4};
use sha2::{Digest, Sha256};

/// The +90°-about-X axis fix: maps glTF (+Y up, +Z forward) to the engine's
/// Blender-canonical world space (+Z up, +X forward, +ty → −Y).  Inverts the
/// glTF exporter's −90°-about-X conversion.
pub fn gltf_to_world() -> Mat4 {
    Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
}

/// Which TRS component an animation channel animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrsProperty {
    Translation,
    Rotation,
    Scale,
}

/// A node in the model hierarchy.  `mesh` is an index into [`ModelAsset::meshes`].
#[derive(Clone, Debug)]
pub struct ModelNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub mesh: Option<usize>,
}

/// A mesh (one or more merged primitives), in mesh-local space.
#[derive(Clone, Debug, Default)]
pub struct ModelMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices (u32; every primitive's indices are merged with a base
    /// vertex offset).
    pub indices: Vec<u32>,
    /// Index into [`ModelAsset::images`] for the mesh's albedo texture, or
    /// `None` for an untextured (solid colour) mesh.
    pub image: Option<usize>,
    /// The ROM catalog texture the material binds as its albedo instead of
    /// an embedded image ([`ATLAS_EXTRA`], `"<ns>::<texture>"`).  Wins over
    /// [`Self::image`] when both are present.
    pub atlas: Option<String>,
    /// The team-colour region of the albedo texture, from the textured
    /// material's [`TEAM_RECT_EXTRA`] extra: `[u0, v0, u1, v1]` in TEXCOORD_0
    /// space.  `None` when the material carries no team region (the mesh is
    /// never team-tinted).
    pub team_rect: Option<[f32; 4]>,
    /// The self-lit regions of the albedo texture, from the textured
    /// material's [`EMISSIVE_EXTRA`] extra (at most [`MAX_EMISSIVE_RECTS`]).
    /// Empty when the material carries none (nothing glows).
    pub emissive: Vec<EmissiveRect>,
    /// Whether the mesh's material asks for transparency (`alphaMode`
    /// `MASK` or `BLEND`).  Every material the kit exporter writes is
    /// `OPAQUE`.
    pub alpha_material: bool,
    /// Whether the mesh must draw with the alpha-tested `mesh.frag` variant
    /// (`discard` below [`ALPHA_CUT`]): an [`Self::alpha_material`], or one
    /// whose UVs can sample a cut texel ([`AlphaMask::mesh_hits`]).  The rest
    /// draw without `discard`, which keeps early depth testing on.  Set by
    /// [`mark_alpha_test`] (the engine knows the de-duplicated images).
    pub alpha_test: bool,
    /// Mesh-local position bounds (for shadow-box fitting).
    pub min: Vec3,
    pub max: Vec3,
}

/// The glTF material `extras` key carrying a model's team-colour region.
///
/// **Contract (written by the kit exporter, read by [`parse_model_glb`]):** a
/// material's `extras` object may carry
/// `"classic_team_rect": [u0, v0, u1, v1]`, four numbers in glTF
/// texture-coordinate space — the space of `TEXCOORD_0`, origin at the
/// **top-left** of the image, `u` right, `v` down, `0..1` over the image — with
/// `0 <= u0 < u1 <= 1` and `0 <= v0 < v1 <= 1`.  The rect names a region of
/// that material's `baseColorTexture`: every texel inside it belongs to a
/// team-coloured face and is baked **neutral** (light grey / white, times any
/// AO or wear), so the engine can multiply it by a team colour.  A fragment is
/// inside when `u0 <= u < u1 && v0 <= v < v1` (half-open, so a rect laid on
/// texel edges selects exactly its texels under `NEAREST` sampling).
///
/// The rect is read from the material that supplies the mesh's albedo
/// texture (the first textured primitive's: an image or an [`ATLAS_EXTRA`]
/// binding), like [`ModelMesh::image`]; an untextured material's rect is
/// ignored.  A present but malformed value fails
/// the parse.  Applied when a `Model` sets `tint` (see
/// `classic_core::components::Model::tint`), a multiplier in the same space
/// as the texture's stored values (sRGB-encoded colours for sRGB textures,
/// `0..1`), applied to the sampled texel before lighting.
pub const TEAM_RECT_EXTRA: &str = "classic_team_rect";

/// The glTF material `extras` key carrying a model's self-lit regions.
///
/// **Contract (written by the kit exporter, read by [`parse_model_glb`]):** a
/// material's `extras` object may carry
/// `"classic_emissive": [{"rect": [u0, v0, u1, v1], "strength": s}, ...]`,
/// at most [`MAX_EMISSIVE_RECTS`] entries.  Each `rect` follows exactly the
/// [`TEAM_RECT_EXTRA`] rules (TEXCOORD_0 space, top-left origin, `0 <= u0 <
/// u1 <= 1`, `0 <= v0 < v1 <= 1`, half-open on the far edges, so a rect on
/// texel edges selects exactly its texels) and names a region of that
/// material's `baseColorTexture` whose texels are lights (windows, lamps,
/// screens).  `strength` (finite, `>= 0`) is the glow multiplier: the engine
/// adds `albedo × strength` to the lit colour of every fragment inside the
/// rect, unlit (no sun, no shadow), in the texture's stored space like the
/// team tint.  The first matching rect wins where rects overlap.  Read from
/// the material that supplies the mesh's albedo texture; an untextured
/// material's entry is ignored.  A present but malformed value (or more
/// than [`MAX_EMISSIVE_RECTS`] entries) fails the parse.  A `Model` may scale
/// every strength with `emissive` (see
/// `classic_core::components::Model::emissive`).
pub const EMISSIVE_EXTRA: &str = "classic_emissive";

/// The glTF material `extras` key binding a ROM catalog texture as the
/// albedo.
///
/// **Contract (written by classic-roms' `encode_models`, read by
/// [`parse_model_glb`]):** a material's `extras` may carry
/// `"classic_atlas": "<ns>::<texture>"`, a non-empty string naming a texture
/// of the ROM catalog (`textures[]`, the PNG the ROM ships once) under
/// the engine's namespace-qualified texture key.  The material's albedo is
/// that texture, sampled with `TEXCOORD_0` exactly like a `baseColorTexture`
/// (so the material needs no image, texture or sampler of its own), and its
/// [`TEAM_RECT_EXTRA`] / [`EMISSIVE_EXTRA`] name regions of it.  Atlases are
/// opaque RGB: a bound mesh never takes the alpha-tested path.  A present but
/// malformed value fails the parse.
pub const ATLAS_EXTRA: &str = "classic_atlas";

/// Most [`EMISSIVE_EXTRA`] entries one material may carry (the `mesh.frag`
/// uniform array size).
pub const MAX_EMISSIVE_RECTS: usize = 8;

/// One self-lit texture region of a mesh (see [`EMISSIVE_EXTRA`]).
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct EmissiveRect {
    /// `[u0, v0, u1, v1]` in TEXCOORD_0 space, half-open on the far edges.
    pub rect: [f32; 4],
    /// Glow multiplier on the albedo (`>= 0`).
    pub strength: f32,
}

impl EmissiveRect {
    /// Whether `uv` is inside [`Self::rect`] (`u0 <= u < u1 && v0 <= v < v1`).
    pub fn contains(&self, uv: [f32; 2]) -> bool {
        let [u0, v0, u1, v1] = self.rect;
        u0 <= uv[0] && uv[0] < u1 && v0 <= uv[1] && uv[1] < v1
    }
}

/// `mesh.frag`'s alpha test cuts fragments whose sampled alpha is below
/// `0.01`: an 8-bit texel with alpha `<= ALPHA_CUT` (2/255 = 0.0078).
pub const ALPHA_CUT: u8 = 2;

/// The texels of an RGBA8 image that `mesh.frag` would cut (alpha `<=`
/// [`ALPHA_CUT`]), one bit each.  Built once per decoded image and kept with
/// the shared texture, so a de-duplicated image (not decoded again) can still
/// be checked.
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaMask {
    width: u32,
    height: u32,
    bits: Vec<u64>,
}

impl AlphaMask {
    /// The mask of `rgba8` (`width × height`, top-down rows); `None` when no
    /// texel is cut (the common, fully opaque atlas).
    pub fn from_rgba8(width: u32, height: u32, rgba8: &[u8]) -> Option<Self> {
        let n = (width as usize) * (height as usize);
        if rgba8.len() < n * 4 || !rgba8.as_chunks::<4>().0.iter().any(|p| p[3] <= ALPHA_CUT) {
            return None;
        }
        let mut bits = vec![0u64; n.div_ceil(64)];
        for (i, p) in rgba8.as_chunks::<4>().0.iter().take(n).enumerate() {
            if p[3] <= ALPHA_CUT {
                bits[i / 64] |= 1 << (i % 64);
            }
        }
        Some(Self { width, height, bits })
    }

    /// Whether texel `(x, y)` is cut.
    pub fn is_cut(&self, x: u32, y: u32) -> bool {
        let i = (y as usize) * (self.width as usize) + x as usize;
        self.bits[i / 64] >> (i % 64) & 1 == 1
    }

    /// The texel a `NEAREST`, `CLAMP_TO_EDGE` sample at `t` (`0..1` along an
    /// axis of `size` texels) reads.
    fn texel(t: f32, size: u32) -> u32 {
        let x = (t * size as f32).floor();
        if x.is_nan() || x < 0.0 {
            0
        } else {
            (x as u32).min(size - 1)
        }
    }

    /// Whether any triangle of a mesh (`uvs` per vertex, `indices` in
    /// threes) can sample a cut texel: conservatively, any cut texel inside
    /// a triangle's UV bounding box (clamped like the sampler).  A mesh with
    /// no UVs samples texel `(0, 0)`.
    pub fn mesh_hits(&self, uvs: &[[f32; 2]], indices: &[u32]) -> bool {
        if uvs.is_empty() {
            return self.is_cut(0, 0);
        }
        indices.as_chunks::<3>().0.iter().any(|tri| {
            let corner = |k: usize| uvs.get(tri[k] as usize).copied().unwrap_or([0.0; 2]);
            let (a, b, c) = (corner(0), corner(1), corner(2));
            let (x0, x1) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
            let (y0, y1) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
            let (tx0, tx1) = (Self::texel(x0, self.width), Self::texel(x1, self.width));
            let (ty0, ty1) = (Self::texel(y0, self.height), Self::texel(y1, self.height));
            (ty0..=ty1).any(|y| (tx0..=tx1).any(|x| self.is_cut(x, y)))
        })
    }
}

/// Decide every mesh's [`ModelMesh::alpha_test`]: `mask(image)` is the
/// [`AlphaMask`] of a mesh's albedo image (`None` = nothing cut).
pub fn mark_alpha_test<'a>(asset: &mut ModelAsset, mask: impl Fn(usize) -> Option<&'a AlphaMask>) {
    for mesh in &mut asset.meshes {
        // A bound atlas (opaque RGB) replaces the image: nothing to cut.
        let cut = mesh.atlas.is_none()
            && mesh.image.and_then(&mask).is_some_and(|m| m.mesh_hits(&mesh.uvs, &mesh.indices));
        mesh.alpha_test = mesh.alpha_material || cut;
    }
}

/// SHA-256 of a GLB-embedded image's **encoded** bytes (the PNG/JPEG
/// buffer-view slice): the key the engine de-duplicates model textures by.
pub type ImageHash = [u8; 32];

/// A decoded GLB-embedded texture image, converted to RGBA8 (top-down rows).
#[derive(Clone, Debug)]
pub struct ModelImage {
    pub width: u32,
    pub height: u32,
    /// Decoded pixels.  Empty when the caller of [`parse_model_glb_with`]
    /// already held an image with this [`Self::content_hash`] (decode skipped),
    /// or once the engine has uploaded them to GL.
    pub rgba8: Vec<u8>,
    /// SHA-256 of the encoded image bytes (see [`ImageHash`]).
    pub content_hash: ImageHash,
}

/// A sampler's interpolation mode (glTF `animation.sampler.interpolation`).
///
/// The export contract is dense `Linear` keys; `Step` and `CubicSpline` are
/// sampled per the glTF spec so a spec-valid file never fails to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

/// One animated channel: a keyed TRS track for a single node.
#[derive(Clone, Debug)]
pub struct ModelChannel {
    pub node: usize,
    pub property: TrsProperty,
    pub interpolation: Interpolation,
    /// Sampler input times (seconds), ascending.
    pub times: Vec<f32>,
    /// Output values as `xyzw` (translation/scale leave `w = 0`).  One per key,
    /// or three per key (`in-tangent, value, out-tangent`) for `CubicSpline`.
    pub values: Vec<Vec4>,
}

impl ModelChannel {
    /// The channel's last time (its duration contribution).
    fn end_time(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }

    /// The keyed value at key `k` (skipping the cubic-spline tangents).
    fn key(&self, k: usize) -> Vec4 {
        match self.interpolation {
            Interpolation::CubicSpline => self.values[3 * k + 1],
            _ => self.values[k],
        }
    }

    /// Sample the raw `xyzw` value at `time`, clamping to the end keys.
    fn sample(&self, time: f32) -> Vec4 {
        let times = &self.times;
        let n = times.len();
        if n == 0 {
            return Vec4::ZERO;
        }
        if n == 1 || time <= times[0] {
            return self.key(0);
        }
        if time >= times[n - 1] {
            return self.key(n - 1);
        }
        // Largest `lo` with `times[lo] <= time` (`times` is ascending).
        let lo = times.partition_point(|&t| t <= time) - 1;
        let span = times[lo + 1] - times[lo];
        let t = if span > f32::EPSILON { (time - times[lo]) / span } else { 0.0 };
        let (a, b) = (self.key(lo), self.key(lo + 1));
        match self.interpolation {
            Interpolation::Step => a,
            Interpolation::Linear if self.property == TrsProperty::Rotation => {
                let q = Quat::from_vec4(a).slerp(Quat::from_vec4(b), t);
                Vec4::from(q)
            }
            Interpolation::Linear => a.lerp(b, t),
            Interpolation::CubicSpline => {
                // Hermite basis over the key span; tangents are per-second.
                let m0 = self.values[3 * lo + 2] * span;
                let m1 = self.values[3 * (lo + 1)] * span;
                let (t2, t3) = (t * t, t * t * t);
                let v = a * (2.0 * t3 - 3.0 * t2 + 1.0)
                    + m0 * (t3 - 2.0 * t2 + t)
                    + b * (-2.0 * t3 + 3.0 * t2)
                    + m1 * (t3 - t2);
                if self.property == TrsProperty::Rotation {
                    v.normalize_or_zero()
                } else {
                    v
                }
            }
        }
    }

    /// Sample a translation/scale channel at `time`.
    fn sample_vec3(&self, time: f32) -> Vec3 {
        self.sample(time).truncate()
    }

    /// Sample a rotation channel at `time`.
    fn sample_rotation(&self, time: f32) -> Quat {
        Quat::from_vec4(self.sample(time)).normalize()
    }
}

/// One named animation clip (landing / launch).
#[derive(Clone, Debug)]
pub struct ModelClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<ModelChannel>,
    /// Per node, the index into `channels` of its translation, rotation and
    /// scale channel (the first one, as a scan would find), built by
    /// [`ModelClip::index_channels`] (the parser calls it).  Empty = not
    /// indexed: lookups fall back to scanning `channels`.
    pub node_channels: Vec<[Option<u32>; 3]>,
}

impl ModelClip {
    /// Build [`Self::node_channels`] for a model of `nodes` nodes, so a pose
    /// sample finds a node's channels in O(1) instead of scanning every
    /// channel three times per node.
    pub fn index_channels(&mut self, nodes: usize) {
        let len = self.channels.iter().map(|c| c.node + 1).fold(nodes, usize::max);
        let mut index = vec![[None; 3]; len];
        for (i, c) in self.channels.iter().enumerate() {
            let slot = &mut index[c.node][c.property as usize];
            if slot.is_none() {
                *slot = Some(i as u32);
            }
        }
        self.node_channels = index;
    }

    fn channel_for(&self, node: usize, property: TrsProperty) -> Option<&ModelChannel> {
        if self.node_channels.is_empty() {
            return self.channels.iter().find(|c| c.node == node && c.property == property);
        }
        let i = self.node_channels.get(node)?[property as usize]?;
        Some(&self.channels[i as usize])
    }
}

/// What a model entity's cached pose was sampled from.  Two samples with
/// equal keys produce bit-identical poses, so an unchanged key skips the
/// re-sample (a finished one-shot clip, a paused or bind-pose model).
#[derive(Clone, Debug, PartialEq)]
pub struct PoseKey {
    /// The model resource name.
    pub model: String,
    /// The engine's model-load generation (bumped whenever a model loads, so
    /// a re-loaded resource under the same name re-samples).
    pub generation: u64,
    /// The clip index (`None` = bind pose).
    pub clip: Option<usize>,
    /// The clip time's bits.
    pub time: u32,
    /// The entity's `Transform.scale` bits (it scales `frame_offset`).
    pub scale: [u32; 3],
}

/// Reusable buffers for [`ModelAsset::node_world_transforms_into`], so a
/// per-frame pose sample allocates nothing once warm.
#[derive(Clone, Debug, Default)]
pub struct PoseScratch {
    local: Vec<Mat4>,
    stack: Vec<usize>,
}

/// A parsed glTF model: node hierarchy + meshes + clips.
#[derive(Clone, Debug, Default)]
pub struct ModelAsset {
    pub nodes: Vec<ModelNode>,
    pub meshes: Vec<ModelMesh>,
    pub clips: Vec<ModelClip>,
    /// GLB-embedded texture images, in mesh `image` index order.
    pub images: Vec<ModelImage>,
    /// The node index of the rig origin (the scene's root node).  Its animated
    /// translation is the model's world-metre drift + altitude.
    pub root: Option<usize>,
}

impl ModelAsset {
    /// Compute every node's world transform at `clip` time `time`, in **glTF
    /// space** (+Y up, metres).  Apply [`gltf_to_world`] to the result for
    /// engine world space.
    ///
    /// When `clip` is out of range every node keeps its rest TRS (so a model
    /// with no clips still renders its bind pose).
    pub fn node_world_transforms(&self, clip: usize, time: f32) -> Vec<Mat4> {
        let mut world = Vec::new();
        self.node_world_transforms_into(clip, time, &mut world, &mut PoseScratch::default());
        world
    }

    /// [`Self::node_world_transforms`] into `world` (resized to the node
    /// count), reusing `scratch`: the per-frame form, which allocates only
    /// when a buffer has to grow.
    pub fn node_world_transforms_into(
        &self,
        clip: usize,
        time: f32,
        world: &mut Vec<Mat4>,
        scratch: &mut PoseScratch,
    ) {
        let local = &mut scratch.local;
        local.clear();
        local.resize(self.nodes.len(), Mat4::IDENTITY);
        let clip_ref = self.clips.get(clip);
        for (i, node) in self.nodes.iter().enumerate() {
            let mut t = node.translation;
            let mut r = node.rotation;
            let mut s = node.scale;
            if let Some(clip) = clip_ref {
                if let Some(ch) = clip.channel_for(i, TrsProperty::Translation) {
                    t = ch.sample_vec3(time);
                }
                if let Some(ch) = clip.channel_for(i, TrsProperty::Rotation) {
                    r = ch.sample_rotation(time);
                }
                if let Some(ch) = clip.channel_for(i, TrsProperty::Scale) {
                    s = ch.sample_vec3(time);
                }
            }
            local[i] = Mat4::from_scale_rotation_translation(s, r, t);
        }

        // Top-down: process parents before children via a DFS from every root.
        world.clear();
        world.resize(self.nodes.len(), Mat4::IDENTITY);
        let stack = &mut scratch.stack;
        stack.clear();
        stack.extend(
            self.nodes.iter().enumerate().filter(|(_, n)| n.parent.is_none()).map(|(i, _)| i),
        );
        while let Some(i) = stack.pop() {
            let parent_world = self.nodes[i].parent.map(|p| world[p]).unwrap_or(Mat4::IDENTITY);
            world[i] = parent_world * local[i];
            stack.extend(self.nodes[i].children.iter().copied());
        }
    }

    /// The model's world-metre descent/drift at `clip` time `time`: the root
    /// node's world translation, axis-fixed into engine world space.  This is
    /// what `gather_lights` folds into a parented light so it tracks the model's
    /// animated motion.
    pub fn root_translation_world(&self, clip: usize, time: f32) -> Vec3 {
        self.root_translation_from(&self.node_world_transforms(clip, time))
    }

    /// [`Self::root_translation_world`] from an already-sampled pose (the
    /// output of [`Self::node_world_transforms`]), so a caller that needs both
    /// the pose and the root translation samples the hierarchy once.
    pub fn root_translation_from(&self, node_world: &[Mat4]) -> Vec3 {
        match self.root.and_then(|r| node_world.get(r)) {
            Some(m) => gltf_to_world().transform_point3(m.w_axis.truncate()),
            None => Vec3::ZERO,
        }
    }

    /// Find a clip by name.
    pub fn clip_index(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }
}

/// Parse a self-contained glTF binary (`.glb`) into an [`ModelAsset`],
/// decoding every embedded image.
pub fn parse_model_glb(bytes: &[u8]) -> Result<ModelAsset> {
    parse_model_glb_with(bytes, |_| None)
}

/// [`parse_model_glb`], skipping the decode of images the caller already
/// holds: `known(hash)` returns the `(width, height)` of a previously decoded
/// image whose encoded bytes hash to `hash`, and that image comes back with
/// empty [`ModelImage::rgba8`] and those dimensions.  A kit building ships one
/// glb per clip, each embedding the same atlas, so only the first is decoded.
pub fn parse_model_glb_with(
    bytes: &[u8],
    known: impl Fn(&ImageHash) -> Option<(u32, u32)>,
) -> Result<ModelAsset> {
    let gltf::Gltf { document, blob } =
        gltf::Gltf::from_slice(bytes).map_err(|e| anyhow!("failed to parse glb: {e}"))?;
    let buffers = gltf::import_buffers(&document, None, blob)
        .map_err(|e| anyhow!("failed to parse glb: {e}"))?;

    // --- nodes (hierarchy + rest TRS) ---
    let gltf_nodes: Vec<_> = document.nodes().collect();
    let mut nodes: Vec<ModelNode> = Vec::with_capacity(gltf_nodes.len());
    // Map gltf node index -> our index (identical; nodes are indexed 0..N).
    let mut parent: Vec<Option<usize>> = vec![None; gltf_nodes.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); gltf_nodes.len()];
    for n in &gltf_nodes {
        for c in n.children() {
            children[n.index()].push(c.index());
            parent[c.index()] = Some(n.index());
        }
    }
    for n in &gltf_nodes {
        let (translation, rotation, scale) = n.transform().decomposed();
        nodes.push(ModelNode {
            name: n.name().unwrap_or_default().to_string(),
            parent: parent[n.index()],
            children: children[n.index()].clone(),
            translation: Vec3::from_array(translation),
            rotation: Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
            scale: Vec3::from_array(scale),
            mesh: n.mesh().map(|m| m.index()),
        });
    }

    // --- meshes (merge primitives into one interleaved VBO payload) ---
    let mut meshes: Vec<ModelMesh> = Vec::new();
    for mesh in document.meshes() {
        let mut merged =
            ModelMesh { name: mesh.name().unwrap_or_default().to_string(), ..Default::default() };
        for prim in mesh.primitives() {
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let base = merged.positions.len() as u32;
            let positions = reader
                .read_positions()
                .ok_or_else(|| anyhow!("mesh {:?}: no positions", mesh.name()))?;
            let normals = reader.read_normals();
            let uvs = reader.read_tex_coords(0).map(|r| r.into_f32());
            // First textured primitive's material supplies the mesh's albedo
            // texture (an image index, or a bound ROM atlas; the rocket is a
            // single shared material across meshes) and the team region
            // inside that texture.
            if merged.image.is_none() && merged.atlas.is_none() {
                let material = prim.material();
                merged.image = material
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .map(|t| t.texture().source().index());
                merged.atlas = material_atlas(&material)?;
                merged.alpha_material |= material.alpha_mode() != gltf::material::AlphaMode::Opaque;
                if merged.image.is_some() || merged.atlas.is_some() {
                    merged.team_rect = material_team_rect(&material)?;
                    merged.emissive = material_emissive(&material)?;
                }
            }
            merged.positions.extend(positions);
            if let Some(ns) = normals {
                merged.normals.extend(ns);
            }
            if let Some(uv) = uvs {
                for u in uv {
                    merged.uvs.push(u);
                }
            }
            if let Some(indices) = reader.read_indices() {
                for i in indices.into_u32() {
                    merged.indices.push(i + base);
                }
            } else {
                // Non-indexed triangle list: emit sequential indices.
                let count = merged.positions.len() as u32 - base;
                merged.indices.extend(base..(base + count));
            }
        }
        if let Some(first) = merged.positions.first() {
            let (mut lo, mut hi) = (Vec3::from_array(*first), Vec3::from_array(*first));
            for p in &merged.positions {
                lo = lo.min(Vec3::from_array(*p));
                hi = hi.max(Vec3::from_array(*p));
            }
            (merged.min, merged.max) = (lo, hi);
        }
        meshes.push(merged);
    }

    // --- images (hash the encoded bytes; decode unknown ones to RGBA8) ---
    let mut model_images = Vec::new();
    for image in document.images() {
        let source = image.source();
        // A slice import resolves no URIs (external or data), exactly as
        // `gltf::import_slice` refused them: every image is a buffer view.
        let gltf::image::Source::View { ref view, .. } = source else {
            bail!("failed to parse glb: image {} is not embedded in the binary", image.index());
        };
        let start = view.offset();
        let encoded = buffers[view.buffer().index()]
            .get(start..start + view.length())
            .ok_or_else(|| anyhow!("image {}: buffer view out of range", image.index()))?;
        let content_hash: ImageHash = Sha256::digest(encoded).into();
        let img = match known(&content_hash) {
            Some((width, height)) => ModelImage { width, height, rgba8: Vec::new(), content_hash },
            None => {
                let data = gltf::image::Data::from_source(source, None, &buffers)
                    .map_err(|e| anyhow!("failed to parse glb: {e}"))?;
                let rgba8 = image_to_rgba8(&data.pixels, data.format, data.width, data.height)?;
                ModelImage { width: data.width, height: data.height, rgba8, content_hash }
            }
        };
        model_images.push(img);
    }

    // --- clips (node TRS channels) ---
    let mut clips: Vec<ModelClip> = Vec::new();
    for anim in document.animations() {
        let mut channels: Vec<ModelChannel> = Vec::new();
        for channel in anim.channels() {
            let reader = channel.reader(|b| Some(&buffers[b.index()]));
            let target = channel.target();
            let node = target.node().index();
            let property = match target.property() {
                gltf::animation::Property::Translation => TrsProperty::Translation,
                gltf::animation::Property::Rotation => TrsProperty::Rotation,
                gltf::animation::Property::Scale => TrsProperty::Scale,
                gltf::animation::Property::MorphTargetWeights => {
                    bail!("morph-target weights are not supported (clip {:?})", anim.name())
                }
            };
            let interpolation = match channel.sampler().interpolation() {
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let times: Vec<f32> = reader
                .read_inputs()
                .ok_or_else(|| anyhow!("clip {:?}: channel has no inputs", anim.name()))?
                .collect();
            let outputs = reader
                .read_outputs()
                .ok_or_else(|| anyhow!("clip {:?}: channel has no outputs", anim.name()))?;
            use gltf::animation::util::ReadOutputs;
            let values: Vec<Vec4> = match (property, outputs) {
                (TrsProperty::Translation, ReadOutputs::Translations(it))
                | (TrsProperty::Scale, ReadOutputs::Scales(it)) => {
                    it.map(|v| Vec3::from_array(v).extend(0.0)).collect()
                }
                (TrsProperty::Rotation, ReadOutputs::Rotations(rot)) => {
                    rot.into_f32().map(Vec4::from_array).collect()
                }
                _ => bail!("clip {:?}: channel/output property mismatch", anim.name()),
            };
            let per_key = if interpolation == Interpolation::CubicSpline { 3 } else { 1 };
            if values.len() != times.len() * per_key {
                bail!(
                    "clip {:?}: {} outputs for {} keys ({interpolation:?})",
                    anim.name(),
                    values.len(),
                    times.len()
                );
            }
            channels.push(ModelChannel { node, property, interpolation, times, values });
        }
        let duration = channels.iter().map(|c| c.end_time()).fold(0.0f32, f32::max);
        let mut clip = ModelClip {
            name: anim.name().unwrap_or_default().to_string(),
            duration,
            channels,
            node_channels: Vec::new(),
        };
        clip.index_channels(nodes.len());
        clips.push(clip);
    }

    // The rig origin: the default scene's first root node (the exporter emits a
    // single `rig` root), falling back to the first parentless node.
    let root = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .and_then(|s| s.nodes().next().map(|n| n.index()))
        .or_else(|| nodes.iter().position(|n| n.parent.is_none()));
    Ok(ModelAsset { nodes, meshes, clips, images: model_images, root })
}

/// The [`ATLAS_EXTRA`] of a material: `None` without it; an error when it
/// is present but not a non-empty string.
fn material_atlas(material: &gltf::Material<'_>) -> Result<Option<String>> {
    let Some(raw) = material.extras() else { return Ok(None) };
    let extras: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| anyhow!("material {:?}: bad extras: {e}", material.name()))?;
    match extras.get(ATLAS_EXTRA) {
        None => Ok(None),
        Some(serde_json::Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
        Some(v) => bail!(
            "material {:?}: `{ATLAS_EXTRA}` must be a non-empty \"<ns>::<texture>\" string: {v}",
            material.name()
        ),
    }
}

/// The [`TEAM_RECT_EXTRA`] of a material, validated against the contract:
/// `None` without `extras` or without the key; an error when the key is
/// present but not four finite, ordered numbers inside `0..=1`.
fn material_team_rect(material: &gltf::Material<'_>) -> Result<Option<[f32; 4]>> {
    let Some(raw) = material.extras() else { return Ok(None) };
    let extras: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| anyhow!("material {:?}: bad extras: {e}", material.name()))?;
    let Some(value) = extras.get(TEAM_RECT_EXTRA) else { return Ok(None) };
    let invalid = |why: &str| {
        anyhow!(
            "material {:?}: `{TEAM_RECT_EXTRA}` must be [u0, v0, u1, v1] with \
             0 <= u0 < u1 <= 1 and 0 <= v0 < v1 <= 1 ({why}): {value}",
            material.name()
        )
    };
    let rect: [f32; 4] =
        serde_json::from_value(value.clone()).map_err(|e| invalid(&e.to_string()))?;
    rect_ok(&rect).map_err(invalid)?;
    Ok(Some(rect))
}

/// Validate a `[u0, v0, u1, v1]` rect against the shared rect contract.
fn rect_ok(rect: &[f32; 4]) -> std::result::Result<(), &'static str> {
    let [u0, v0, u1, v1] = *rect;
    if !rect.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)) {
        return Err("out of 0..=1");
    }
    if u0 >= u1 || v0 >= v1 {
        return Err("empty or inverted");
    }
    Ok(())
}

/// The [`EMISSIVE_EXTRA`] of a material, validated against the contract:
/// empty without `extras` or without the key; an error when the key is
/// present but malformed.
fn material_emissive(material: &gltf::Material<'_>) -> Result<Vec<EmissiveRect>> {
    let Some(raw) = material.extras() else { return Ok(Vec::new()) };
    let extras: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| anyhow!("material {:?}: bad extras: {e}", material.name()))?;
    let Some(value) = extras.get(EMISSIVE_EXTRA) else { return Ok(Vec::new()) };
    let invalid = |why: &str| {
        anyhow!(
            "material {:?}: `{EMISSIVE_EXTRA}` must be at most {MAX_EMISSIVE_RECTS} \
             {{\"rect\": [u0, v0, u1, v1], \"strength\": s}} with 0 <= u0 < u1 <= 1, \
             0 <= v0 < v1 <= 1 and a finite s >= 0 ({why}): {value}",
            material.name()
        )
    };
    let rects: Vec<EmissiveRect> =
        serde_json::from_value(value.clone()).map_err(|e| invalid(&e.to_string()))?;
    if rects.len() > MAX_EMISSIVE_RECTS {
        return Err(invalid("too many rects"));
    }
    for r in &rects {
        rect_ok(&r.rect).map_err(invalid)?;
        if !(r.strength.is_finite() && r.strength >= 0.0) {
            return Err(invalid("bad strength"));
        }
    }
    Ok(rects)
}

/// Convert glTF image pixel data (as decoded by [`gltf::image::Data`]) to
/// tightly-packed RGBA8.  Handles the 8-bit formats the Blender exporter emits;
/// anything else is rejected (no 16-bit/float textures in this path).
fn image_to_rgba8(
    pixels: &[u8],
    format: gltf::image::Format,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    use gltf::image::Format;
    let count = (width * height) as usize;
    let rgba = match format {
        Format::R8G8B8A8 => pixels.to_vec(),
        Format::R8G8B8 => {
            let mut out = Vec::with_capacity(count * 4);
            for &[r, g, b] in pixels.as_chunks::<3>().0 {
                out.extend_from_slice(&[r, g, b, 255]);
            }
            out
        }
        other => bail!("unsupported GLB image format {other:?} (expected R8G8B8A8/R8G8B8)"),
    };
    if rgba.len() != count * 4 {
        bail!("GLB image size mismatch: {} pixels -> {} bytes", count, rgba.len());
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_fix_maps_gltf_up_to_world_up() {
        let m = gltf_to_world();
        assert!((m.transform_point3(Vec3::Y) - Vec3::Z).length() < 1e-5);
        assert!((m.transform_point3(Vec3::Z) - Vec3::NEG_Y).length() < 1e-5);
        assert!((m.transform_point3(Vec3::X) - Vec3::X).length() < 1e-5);
    }

    fn z_channel(interpolation: Interpolation, zs: &[f32]) -> ModelChannel {
        ModelChannel {
            node: 0,
            property: TrsProperty::Translation,
            interpolation,
            times: vec![0.0, 10.0, 20.0],
            values: zs.iter().map(|&z| Vec4::new(0.0, 0.0, z, 0.0)).collect(),
        }
    }

    #[test]
    fn linear_channel_interpolates_and_clamps() {
        let ch = z_channel(Interpolation::Linear, &[50.0, 25.0, 0.0]);
        let at = |t| ch.sample_vec3(t).z;
        assert_eq!(at(-1.0), 50.0);
        assert_eq!(at(5.0), 37.5);
        assert_eq!(at(10.0), 25.0);
        assert_eq!(at(15.0), 12.5);
        assert_eq!(at(99.0), 0.0);
    }

    #[test]
    fn step_channel_holds_the_previous_key() {
        let ch = z_channel(Interpolation::Step, &[50.0, 25.0, 0.0]);
        assert_eq!(ch.sample_vec3(9.9).z, 50.0);
        assert_eq!(ch.sample_vec3(10.0).z, 25.0);
    }

    #[test]
    fn cubic_spline_channel_follows_hermite_tangents() {
        // Keys (in-tangent, value, out-tangent): a line z = 50 - 2.5 t sampled
        // with matching tangents must reproduce the line exactly.
        let v = |z: f32| [-2.5, z, -2.5];
        let zs: Vec<f32> = [v(50.0), v(25.0), v(0.0)].concat();
        let ch = z_channel(Interpolation::CubicSpline, &zs);
        for t in [0.0, 3.0, 10.0, 17.5, 20.0] {
            assert!((ch.sample_vec3(t).z - (50.0 - 2.5 * t)).abs() < 1e-4, "t={t}");
        }
        // Zero tangents ease in/out: the midpoint is still halfway, the quarter
        // point is closer to the start than a line would be.
        let zs: Vec<f32> = [[0.0, 50.0, 0.0], [0.0, 25.0, 0.0], [0.0, 0.0, 0.0]].concat();
        let ch = z_channel(Interpolation::CubicSpline, &zs);
        assert!((ch.sample_vec3(5.0).z - 37.5).abs() < 1e-4);
        assert!(ch.sample_vec3(2.5).z > 50.0 - 2.5 * 2.5);
    }

    #[test]
    fn hierarchy_composes_parent_first() {
        // root -> child.  child.rest = translate (0, 10, 0); root.rest = (0, 5, 0).
        let asset = ModelAsset {
            nodes: vec![
                ModelNode {
                    name: "root".into(),
                    parent: None,
                    children: vec![1],
                    translation: Vec3::new(0.0, 5.0, 0.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                    mesh: None,
                },
                ModelNode {
                    name: "child".into(),
                    parent: Some(0),
                    children: vec![],
                    translation: Vec3::new(0.0, 10.0, 0.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                    mesh: Some(0),
                },
            ],
            meshes: vec![ModelMesh::default()],
            clips: vec![],
            images: vec![],
            root: Some(0),
        };
        let world = asset.node_world_transforms(usize::MAX, 0.0);
        // child world translation = root(5) + child(10) = 15 in glTF +Y.
        assert!((world[1].w_axis.truncate() - Vec3::new(0.0, 15.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn indexed_buffered_pose_matches_the_scan_bit_for_bit() {
        // root -> {a, b}; a has T/R/S channels plus a duplicate T channel
        // (the first one wins, as a scan finds it), b only rotation.
        let node = |name: &str, parent, children| ModelNode {
            name: name.into(),
            parent,
            children,
            translation: Vec3::new(0.5, 1.0, -0.25),
            rotation: Quat::from_rotation_y(0.3),
            scale: Vec3::splat(1.5),
            mesh: None,
        };
        let ch = |node, property, values: Vec<Vec4>| ModelChannel {
            node,
            property,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values,
        };
        let q = |a: f32| Vec4::from(Quat::from_rotation_z(a));
        let clip = ModelClip {
            name: "c".into(),
            duration: 1.0,
            channels: vec![
                ch(2, TrsProperty::Rotation, vec![q(0.0), q(1.0)]),
                ch(1, TrsProperty::Translation, vec![Vec4::ZERO, Vec4::new(1.0, 2.0, 3.0, 0.0)]),
                ch(1, TrsProperty::Translation, vec![Vec4::ONE, Vec4::ONE]),
                ch(1, TrsProperty::Rotation, vec![q(0.2), q(-0.7)]),
                ch(1, TrsProperty::Scale, vec![Vec4::ONE, Vec4::new(2.0, 0.5, 1.0, 0.0)]),
            ],
            node_channels: Vec::new(),
        };
        let mut indexed = clip.clone();
        indexed.index_channels(3);
        assert_eq!(indexed.node_channels[1], [Some(1), Some(3), Some(4)]);
        assert_eq!(indexed.node_channels[0], [None; 3]);
        let asset = |clip: ModelClip| ModelAsset {
            nodes: vec![
                node("root", None, vec![1, 2]),
                node("a", Some(0), vec![]),
                node("b", Some(0), vec![]),
            ],
            clips: vec![clip],
            root: Some(0),
            ..Default::default()
        };
        let (scan, fast) = (asset(clip), asset(indexed));
        let mut world = Vec::new();
        let mut scratch = PoseScratch::default();
        for t in [0.0, 0.25, 0.6, 1.0, 3.0] {
            for c in [0, usize::MAX] {
                fast.node_world_transforms_into(c, t, &mut world, &mut scratch);
                let want = scan.node_world_transforms(c, t);
                let bits = |m: &[Mat4]| {
                    m.iter().flat_map(|m| m.to_cols_array()).map(f32::to_bits).collect::<Vec<_>>()
                };
                assert_eq!(bits(&world), bits(&want), "clip {c} t {t}");
            }
        }
    }

    /// A 2x1 RGBA PNG: left texel `left`, right texel `right`.
    fn png_2x1(left: [u8; 4], right: [u8; 4]) -> Vec<u8> {
        let mut px = left.to_vec();
        px.extend_from_slice(&right);
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_raw(2, 1, px)
            .unwrap()
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// A one-triangle textured `.glb` embedding `png`, whose material carries
    /// `material_extras` (a JSON object literal) and, if `textured`, a
    /// `baseColorTexture`.
    fn textured_glb(png: &[u8], material_extras: Option<&str>, textured: bool) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for f in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        bin.extend_from_slice(png);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let extras = material_extras.map(|e| format!(r#","extras":{e}"#)).unwrap_or_default();
        let tex = if textured { r#""baseColorTexture":{"index":0}"# } else { "" };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"body","mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},
                "material":0}}]}}],
            "materials":[{{"name":"kit","pbrMetallicRoughness":{{{tex}}}{extras}}}],
            "textures":[{{"source":0}}],
            "images":[{{"bufferView":2,"mimeType":"image/png"}}],
            "buffers":[{{"byteLength":{len}}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},
                {{"buffer":0,"byteOffset":36,"byteLength":24}},
                {{"buffer":0,"byteOffset":60,"byteLength":{png_len}}}],
            "accessors":[
                {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
                  "min":[0,0,0],"max":[1,1,0]}},
                {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}}]}}"#,
            len = bin.len(),
            png_len = png.len()
        );
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    #[test]
    fn team_rect_extra_parses_from_the_textured_material() {
        let png = png_2x1([200, 200, 200, 255], [10, 20, 30, 255]);
        let extras = r#"{"classic_team_rect":[0.5,0,1,0.25],"other":"kept"}"#;
        let asset = parse_model_glb(&textured_glb(&png, Some(extras), true)).unwrap();
        let mesh = &asset.meshes[0];
        assert_eq!(mesh.image, Some(0));
        assert_eq!(mesh.team_rect, Some([0.5, 0.0, 1.0, 0.25]));
        assert_eq!(mesh.uvs.len(), 3);
        let img = &asset.images[0];
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.rgba8, [200, 200, 200, 255, 10, 20, 30, 255]);
        // The content key is the SHA-256 of the *encoded* image bytes.
        let expected: ImageHash = Sha256::digest(&png).into();
        assert_eq!(img.content_hash, expected);
    }

    #[test]
    fn team_rect_is_absent_without_the_extra_or_a_texture() {
        let png = png_2x1([255; 4], [0, 0, 0, 255]);
        for extras in [None, Some(r#"{"unrelated":1}"#)] {
            let asset = parse_model_glb(&textured_glb(&png, extras, true)).unwrap();
            assert_eq!(asset.meshes[0].team_rect, None, "{extras:?}");
        }
        // A rect on an untextured material names no texture region: ignored.
        let rect = Some(r#"{"classic_team_rect":[0,0,1,1]}"#);
        let asset = parse_model_glb(&textured_glb(&png, rect, false)).unwrap();
        assert_eq!((asset.meshes[0].image, asset.meshes[0].team_rect), (None, None));
    }

    #[test]
    fn malformed_team_rect_fails_the_parse_loudly() {
        let png = png_2x1([255; 4], [0, 0, 0, 255]);
        for bad in
            ["[0,0,1]", "[0.5,0,0.25,1]", "[0,0.5,1,0.5]", "[0,0,1.5,1]", "[-0.1,0,1,1]", "\"x\""]
        {
            let extras = format!(r#"{{"classic_team_rect":{bad}}}"#);
            let err = parse_model_glb(&textured_glb(&png, Some(&extras), true)).unwrap_err();
            assert!(err.to_string().contains(TEAM_RECT_EXTRA), "{bad}: {err}");
        }
    }

    #[test]
    fn emissive_extra_parses_from_the_textured_material() {
        let png = png_2x1([200, 200, 200, 255], [10, 20, 30, 255]);
        let extras = r#"{"classic_team_rect":[0,0,0.5,1],
            "classic_emissive":[{"rect":[0.5,0,1,0.25],"strength":2.5},
                                {"rect":[0.5,0.5,0.75,1],"strength":0}]}"#;
        let asset = parse_model_glb(&textured_glb(&png, Some(extras), true)).unwrap();
        let mesh = &asset.meshes[0];
        assert_eq!(mesh.team_rect, Some([0.0, 0.0, 0.5, 1.0]), "both extras coexist");
        assert_eq!(
            mesh.emissive,
            [
                EmissiveRect { rect: [0.5, 0.0, 1.0, 0.25], strength: 2.5 },
                EmissiveRect { rect: [0.5, 0.5, 0.75, 1.0], strength: 0.0 },
            ]
        );
        assert!(mesh.emissive[0].contains([0.5, 0.0]), "near edges inside");
        assert!(!mesh.emissive[0].contains([1.0, 0.1]) && !mesh.emissive[0].contains([0.6, 0.25]));
    }

    #[test]
    fn emissive_is_empty_without_the_extra_or_a_texture() {
        let png = png_2x1([255; 4], [0, 0, 0, 255]);
        for extras in [None, Some(r#"{"classic_team_rect":[0,0,1,1]}"#), Some(r#"{"x":1}"#)] {
            let asset = parse_model_glb(&textured_glb(&png, extras, true)).unwrap();
            assert!(asset.meshes[0].emissive.is_empty(), "{extras:?}");
        }
        let lit = Some(r#"{"classic_emissive":[{"rect":[0,0,1,1],"strength":1}]}"#);
        let asset = parse_model_glb(&textured_glb(&png, lit, false)).unwrap();
        assert!(asset.meshes[0].emissive.is_empty(), "untextured: ignored");
        // An empty list is valid and means nothing glows.
        let none = Some(r#"{"classic_emissive":[]}"#);
        assert!(parse_model_glb(&textured_glb(&png, none, true)).unwrap().meshes[0]
            .emissive
            .is_empty());
    }

    #[test]
    fn malformed_emissive_fails_the_parse_loudly() {
        let png = png_2x1([255; 4], [0, 0, 0, 255]);
        let nine = format!("[{}]", [r#"{"rect":[0,0,1,1],"strength":1}"#; 9].join(","));
        for bad in [
            r#"{"rect":[0,0,1,1],"strength":1}"#,
            r#"[{"rect":[0,0,1],"strength":1}]"#,
            r#"[{"rect":[0.5,0,0.25,1],"strength":1}]"#,
            r#"[{"rect":[0,0,1.5,1],"strength":1}]"#,
            r#"[{"rect":[0,0,1,1],"strength":-1}]"#,
            r#"[{"rect":[0,0,1,1]}]"#,
            r#"[{"strength":1}]"#,
            nine.as_str(),
        ] {
            let extras = format!(r#"{{"classic_emissive":{bad}}}"#);
            let err = parse_model_glb(&textured_glb(&png, Some(&extras), true)).unwrap_err();
            assert!(err.to_string().contains(EMISSIVE_EXTRA), "{bad}: {err}");
        }
    }

    /// A `w × h` RGBA image, opaque except `cut` texels (alpha 0).
    fn rgba_with_cuts(w: u32, h: u32, cut: &[(u32, u32)]) -> Vec<u8> {
        let mut px = vec![255u8; (w * h * 4) as usize];
        for &(x, y) in cut {
            px[((y * w + x) * 4 + 3) as usize] = 0;
        }
        px
    }

    #[test]
    fn alpha_mask_is_none_for_opaque_images_and_marks_cut_texels() {
        assert_eq!(AlphaMask::from_rgba8(4, 4, &rgba_with_cuts(4, 4, &[])), None);
        // Alpha 3 survives the 0.01 test; alpha 2 does not.
        let mut px = rgba_with_cuts(4, 4, &[]);
        px[3] = 3;
        assert_eq!(AlphaMask::from_rgba8(4, 4, &px), None);
        px[3] = ALPHA_CUT;
        let m = AlphaMask::from_rgba8(4, 4, &px).unwrap();
        assert!(m.is_cut(0, 0) && !m.is_cut(1, 0));
        // Bits past a 64-texel word boundary.
        let m = AlphaMask::from_rgba8(16, 8, &rgba_with_cuts(16, 8, &[(5, 6)])).unwrap();
        assert!(m.is_cut(5, 6) && !m.is_cut(6, 5) && !m.is_cut(5, 5));
    }

    #[test]
    fn mesh_hits_only_when_a_triangle_can_sample_a_cut_texel() {
        // 8x8 atlas, one cut texel at (6, 1): u in [0.75, 0.875), v in [0.125, 0.25).
        let m = AlphaMask::from_rgba8(8, 8, &rgba_with_cuts(8, 8, &[(6, 1)])).unwrap();
        let tri = |uvs: [[f32; 2]; 3]| m.mesh_hits(&uvs, &[0, 1, 2]);
        assert!(!tri([[0.0, 0.5], [0.5, 0.5], [0.0, 1.0]]), "far from the cut");
        assert!(tri([[0.7, 0.1], [0.9, 0.1], [0.7, 0.3]]), "covers it");
        // Conservative: the bbox reaches it even if the triangle might not.
        assert!(tri([[0.5, 0.0], [0.9, 0.0], [0.5, 0.2]]));
        // The far edge of the cut texel's neighbour does not sample it.
        assert!(!tri([[0.5, 0.25], [0.74, 0.25], [0.5, 0.5]]));
        // Out-of-range UVs clamp to the edge texels, like the sampler.
        assert!(!tri([[-1.0, -1.0], [0.1, -1.0], [-1.0, 0.1]]));
        let edge = AlphaMask::from_rgba8(8, 8, &rgba_with_cuts(8, 8, &[(7, 7)])).unwrap();
        assert!(edge.mesh_hits(&[[1.5, 1.5], [2.0, 1.5], [1.5, 2.0]], &[0, 1, 2]));
        // No UVs: samples (0, 0).
        let origin = AlphaMask::from_rgba8(8, 8, &rgba_with_cuts(8, 8, &[(0, 0)])).unwrap();
        assert!(origin.mesh_hits(&[], &[0, 1, 2]) && !m.mesh_hits(&[], &[0, 1, 2]));
    }

    #[test]
    fn mark_alpha_test_flags_cut_meshes_and_non_opaque_materials() {
        let png = png_2x1([200, 200, 200, 255], [0, 0, 0, 0]);
        // The test triangle's UVs are (0, 0), (1, 0), (0, 1): it spans both texels.
        let mut asset = parse_model_glb(&textured_glb(&png, None, true)).unwrap();
        assert!(!asset.meshes[0].alpha_material && !asset.meshes[0].alpha_test);
        let mask = AlphaMask::from_rgba8(2, 1, &asset.images[0].rgba8).unwrap();
        mark_alpha_test(&mut asset, |_| Some(&mask));
        assert!(asset.meshes[0].alpha_test, "its UVs reach the cut texel");
        mark_alpha_test(&mut asset, |_| None);
        assert!(!asset.meshes[0].alpha_test, "nothing cut");
    }

    /// Assemble a glb from a JSON document and its BIN chunk.
    fn glb_from(json: &serde_json::Value, bin: &[u8]) -> Vec<u8> {
        let mut js = serde_json::to_vec(json).unwrap();
        while !js.len().is_multiple_of(4) {
            js.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + js.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(js.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&js);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    #[test]
    fn classic_atlas_binds_a_rom_texture_without_an_image() {
        let mut bin = Vec::new();
        for f in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let json = serde_json::json!({
            "asset": { "version": "2.0" }, "scene": 0, "scenes": [{ "nodes": [0] }],
            "nodes": [{ "name": "body", "mesh": 0 }],
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "material": 0 }] }],
            "materials": [{ "name": "kit", "pbrMetallicRoughness": {},
                "extras": { "classic_atlas": "buildtest::kit_atlas",
                            "classic_team_rect": [0.0, 0.0, 1.0, 0.25],
                            "classic_emissive": [{ "rect": [0.5, 0.5, 1.0, 1.0], "strength": 2.0 }] } }],
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                            { "buffer": 0, "byteOffset": 36, "byteLength": 24 }],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                  "min": [0, 0, 0], "max": [1, 1, 0] },
                { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" }]
        });
        let mut asset = parse_model_glb(&glb_from(&json, &bin)).unwrap();
        let mesh = &asset.meshes[0];
        assert_eq!((mesh.image, mesh.atlas.as_deref()), (None, Some("buildtest::kit_atlas")));
        assert_eq!(mesh.team_rect, Some([0.0, 0.0, 1.0, 0.25]), "regions of the bound atlas");
        assert_eq!(mesh.emissive.len(), 1);
        assert!(asset.images.is_empty());
        mark_alpha_test(&mut asset, |_| None);
        assert!(!asset.meshes[0].alpha_test, "a bound atlas is opaque");
        // a malformed binding fails loudly
        let mut bad = json.clone();
        bad["materials"][0]["extras"]["classic_atlas"] = serde_json::json!(3);
        let err = parse_model_glb(&glb_from(&bad, &bin)).unwrap_err();
        assert!(err.to_string().contains(ATLAS_EXTRA), "{err}");
    }

    #[test]
    fn known_images_skip_the_decode() {
        let png = png_2x1([1, 2, 3, 4], [5, 6, 7, 8]);
        let glb = textured_glb(&png, None, true);
        let hash = parse_model_glb(&glb).unwrap().images[0].content_hash;
        let asset = parse_model_glb_with(&glb, |h| (*h == hash).then_some((2048, 2048))).unwrap();
        let img = &asset.images[0];
        assert!(img.rgba8.is_empty(), "a known image is not decoded");
        assert_eq!((img.width, img.height, img.content_hash), (2048, 2048, hash));
        // Different bytes are not "known".
        let other = textured_glb(&png_2x1([9; 4], [9; 4]), None, true);
        let asset = parse_model_glb_with(&other, |h| (*h == hash).then_some((1, 1))).unwrap();
        assert_eq!(asset.images[0].rgba8.len(), 8);
    }
}
