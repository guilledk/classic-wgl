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
    /// The team-colour region of the albedo texture, from the textured
    /// material's [`TEAM_RECT_EXTRA`] extra: `[u0, v0, u1, v1]` in TEXCOORD_0
    /// space.  `None` when the material carries no team region (the mesh is
    /// never team-tinted).
    pub team_rect: Option<[f32; 4]>,
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
/// texture (the first textured primitive's), like [`ModelMesh::image`]; an
/// untextured material's rect is ignored.  A present but malformed value fails
/// the parse.  Applied when a `Model` sets `tint` (see
/// `classic_core::components::Model::tint`), a multiplier in the same space
/// as the texture's stored values (sRGB-encoded colours for sRGB textures,
/// `0..1`), applied to the sampled texel before lighting.
pub const TEAM_RECT_EXTRA: &str = "classic_team_rect";

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
}

impl ModelClip {
    fn channel_for(&self, node: usize, property: TrsProperty) -> Option<&ModelChannel> {
        self.channels.iter().find(|c| c.node == node && c.property == property)
    }
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
        let mut local = vec![Mat4::IDENTITY; self.nodes.len()];
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
        let mut world = vec![Mat4::IDENTITY; self.nodes.len()];
        let mut stack: Vec<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent.is_none())
            .map(|(i, _)| i)
            .collect();
        while let Some(i) = stack.pop() {
            let parent_world = self.nodes[i].parent.map(|p| world[p]).unwrap_or(Mat4::IDENTITY);
            world[i] = parent_world * local[i];
            stack.extend(self.nodes[i].children.iter().copied());
        }
        world
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
            // texture image index (the rocket is a single shared material
            // across meshes) and the team region inside that texture.
            if merged.image.is_none() {
                let material = prim.material();
                merged.image = material
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .map(|t| t.texture().source().index());
                if merged.image.is_some() {
                    merged.team_rect = material_team_rect(&material)?;
                }
            }
            for p in positions {
                merged.positions.push(p);
            }
            if let Some(ns) = normals {
                for n in ns {
                    merged.normals.push(n);
                }
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
        clips.push(ModelClip {
            name: anim.name().unwrap_or_default().to_string(),
            duration,
            channels,
        });
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
    let [u0, v0, u1, v1] = rect;
    if !rect.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)) {
        return Err(invalid("out of 0..=1"));
    }
    if u0 >= u1 || v0 >= v1 {
        return Err(invalid("empty or inverted"));
    }
    Ok(Some(rect))
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
