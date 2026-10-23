//! 3D glTF `Model` entities: clip playback, pose sampling and draw prep.
//!
//! A `Model` is real geometry, not a billboard: each node's mesh is drawn with
//! `placement · gltf_to_world() · node_world[node]`, where `placement` puts the
//! glb origin on the terrain ground point of the entity's tile position.  The
//! pose is sampled once per frame by [`Engine::update_models`]; the render pass
//! consumes the cached `Model.node_world`, and `gather_lights` consumes the
//! cached `Model.frame_offset` (the animated rig-origin translation) so a
//! parented light tracks the model.  See the `classic-gfx` / `classic-iso`
//! skills ("3D models").

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use classic_core::components::{Disabled, Model, Tilemap};
use classic_core::math::iso_world_pos;
use classic_core::model::{gltf_to_world, ModelAsset, PoseKey, PoseScratch};
use classic_core::tilemap::sample_height_mesh;
use classic_core::Transform;
use glam::{Mat4, Vec3};

use crate::{DrawKind, Engine};

/// A model mesh instance's GPU resources: `slot` in [`ModelGpuStore`], mesh
/// index, and albedo image index (if textured).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MeshRef {
    pub(crate) slot: u32,
    pub(crate) mesh: u32,
    pub(crate) image: Option<u32>,
}

/// Per loaded glb: its GPU meshes and the `Gfx` texture names of its images
/// (`"{model}::image::{i}"`, per model even when the GL texture is shared by
/// content, so traces keep naming the model's image), looked up by
/// [`MeshRef`] instead of formatting a string key per instance per frame.
#[derive(Default)]
pub(crate) struct ModelGpuStore {
    slots: HashMap<String, u32>,
    models: Vec<ModelGpuSlot>,
}

struct ModelGpuSlot {
    meshes: Vec<classic_gfx::ModelMeshGpu>,
    /// The embedded images' names, then the bound atlases' (`classic_atlas`).
    images: Vec<String>,
    /// Per mesh: the index in `images` of its bound atlas, if any.
    mesh_atlas: Vec<Option<u32>>,
}

impl ModelGpuStore {
    /// Register (or replace) `name`'s meshes, its `images` embedded image
    /// names and, per mesh, the `Gfx.textures` name of a bound atlas
    /// (`classic_atlas`, resolved by `Engine::model_atlas_texture`).
    pub(crate) fn insert(
        &mut self,
        name: &str,
        meshes: Vec<classic_gfx::ModelMeshGpu>,
        images: usize,
        atlases: Vec<Option<String>>,
    ) {
        let mut images: Vec<String> = (0..images).map(|i| format!("{name}::image::{i}")).collect();
        let mesh_atlas = atlases
            .into_iter()
            .map(|a| {
                let a = a?;
                Some(match images.iter().position(|n| *n == a) {
                    Some(i) => i as u32,
                    None => {
                        images.push(a);
                        images.len() as u32 - 1
                    }
                })
            })
            .collect();
        let slot = ModelGpuSlot { meshes, images, mesh_atlas };
        match self.slots.get(name) {
            Some(&s) => self.models[s as usize] = slot,
            None => {
                self.slots.insert(name.to_string(), self.models.len() as u32);
                self.models.push(slot);
            }
        }
    }

    /// The slot of a loaded model.
    pub(crate) fn slot(&self, name: &str) -> Option<u32> {
        self.slots.get(name).copied()
    }

    /// The GPU mesh (`None` without a `Gfx`).
    pub(crate) fn mesh(&self, r: MeshRef) -> Option<&classic_gfx::ModelMeshGpu> {
        self.models.get(r.slot as usize)?.meshes.get(r.mesh as usize)
    }

    /// The albedo image index of mesh `mesh` of `slot`: its bound atlas,
    /// else its embedded image (`embedded`), else none.
    pub(crate) fn mesh_image(
        &self,
        slot: u32,
        mesh: usize,
        embedded: Option<usize>,
    ) -> Option<u32> {
        let atlas = self.models.get(slot as usize).and_then(|s| s.mesh_atlas.get(mesh).copied());
        atlas.flatten().or(embedded.map(|i| i as u32))
    }

    /// The albedo texture name, if textured.
    pub(crate) fn texture(&self, r: MeshRef) -> Option<&str> {
        let slot = self.models.get(r.slot as usize)?;
        r.image.and_then(|i| slot.images.get(i as usize)).map(String::as_str)
    }

    /// The legacy `"{model}::{mesh}"` key (tests).
    #[cfg(test)]
    pub(crate) fn key(&self, r: MeshRef) -> String {
        let name = self.slots.iter().find(|(_, &s)| s == r.slot).map(|(n, _)| n.as_str());
        format!("{}::{}", name.unwrap_or("?"), r.mesh)
    }
}
/// One mesh instance of a model draw.
pub(crate) struct ModelInstance {
    /// The node's full world transform (metres).
    pub(crate) model: Mat4,
    /// Its GPU mesh and albedo image in `Engine::model_gpu`.
    pub(crate) mesh: MeshRef,
    /// The team tint: `Model.tint` over the mesh's `team_rect`, set only when
    /// the model is tinted and the mesh is textured with a team region.
    pub(crate) team: Option<classic_gfx::TeamTint>,
    /// The mesh's self-lit rects, strengths scaled by `Model.emissive`
    /// (empty when it has none or the multiplier is `0`).
    pub(crate) emissive: Vec<classic_gfx::EmissiveRect>,
    /// Draw with the alpha-tested program (`ModelMesh::alpha_test`).
    pub(crate) alpha_test: bool,
}

/// The first upload of a model image's content (see `Engine::model_textures`).
pub(crate) struct SharedModelTexture {
    /// The `Gfx.textures` name it was uploaded under (`"{model}::image::{i}"`
    /// of the first model that carried it).
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Its cut texels (`None` = fully opaque), kept for the meshes of later
    /// glbs that alias it without decoding it.
    pub(crate) alpha: Option<classic_core::model::AlphaMask>,
}

/// Precomputed per-entity model draw (all node instances), shared by the
/// shadow casters and the main pass.
pub(crate) struct ModelDraw {
    pub(crate) order: f32,
    pub(crate) name: String,
    pub(crate) instances: Vec<ModelInstance>,
    /// World-space AABB of every instance (for the shadow-box fit).
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
}

impl ModelDraw {
    /// Whether any instance glows (gates the bloom pass).
    pub(crate) fn has_emissive(&self) -> bool {
        self.instances.iter().any(|i| i.emissive.iter().any(|e| e.strength > 0.0))
    }

    /// The AABB's bottom and top faces as unit-quad caster matrices, the form
    /// `shadow::fit_directional_light_matrix` expects for its `casters`, so the
    /// light box encloses the whole model.
    pub(crate) fn shadow_casters(&self) -> [Mat4; 2] {
        let size = Vec3::new(self.max.x - self.min.x, self.max.y - self.min.y, 1.0);
        [self.min.z, self.max.z].map(|z| {
            Mat4::from_translation(Vec3::new(self.min.x, self.min.y, z)) * Mat4::from_scale(size)
        })
    }
}

/// Sample `m`'s pose from `asset` at `m.time` (an out-of-range or missing clip
/// index is the rest pose) into `Model.node_world` + `Model.frame_offset`,
/// unless `m.pose_key` says the cached pose already is exactly that.
fn sample_pose(
    asset: &ModelAsset,
    generation: u64,
    clip: Option<usize>,
    m: &mut Model,
    tf: &Transform,
    scratch: &mut PoseScratch,
) {
    let fresh = m.pose_key.as_ref().is_some_and(|k| {
        k.generation == generation
            && k.clip == clip
            && k.time == m.time.to_bits()
            && k.scale == tf.scale.to_array().map(f32::to_bits)
            && k.model == m.model
    });
    if fresh {
        return;
    }
    let mut world = std::mem::take(&mut m.node_world);
    asset.node_world_transforms_into(clip.unwrap_or(usize::MAX), m.time, &mut world, scratch);
    m.node_world = world;
    m.frame_offset = tf.scale * asset.root_translation_from(&m.node_world);
    m.pose_key = Some(PoseKey {
        model: m.model.clone(),
        generation,
        clip,
        time: m.time.to_bits(),
        scale: tf.scale.to_array().map(f32::to_bits),
    });
}

/// [`Engine::node_indices`] results, per glb resource name then per node
/// name / prefix.  Cleared whenever the engine's model-load generation moves
/// (a re-loaded glb); a clip swap to another glb is simply another key.
#[derive(Default)]
pub(crate) struct NodeIndexCache(RefCell<NodeIndexes>);

#[derive(Default)]
struct NodeIndexes {
    generation: u64,
    exact: HashMap<String, HashMap<String, Rc<[usize]>>>,
    prefix: HashMap<String, HashMap<String, Rc<[usize]>>>,
}

/// The main-pass draw order of model instances keyed `(alpha_test, view
/// depth, _)`: every opaque instance before any alpha-tested one, each group
/// front to back (larger view depth is nearer the camera); a stable sort, so
/// equal keys keep render-list order.
pub(crate) fn sort_model_draw_order<T>(order: &mut [(bool, f32, T)]) {
    order.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
}

impl Engine {
    /// Restart a named `Model` entity on `model` (a resolved model resource
    /// name) from time zero, looping if `repeat`.  The model counterpart of
    /// [`Engine::start_anim`].
    ///
    /// Each `.glb` carries one clip named after the model, so the clip played
    /// is the model name's last `::` segment (the namespace is dropped).
    pub fn start_model_clip(&mut self, name: &str, model: &str, repeat: bool) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut m) = self.world.get::<&mut Model>(entity) else { return false };
        m.model = model.to_string();
        m.clip = Some(model.rsplit("::").next().unwrap_or(model).to_string());
        m.repeat = repeat;
        m.playing = true;
        m.time = 0.0;
        drop(m);
        // Re-pose now: the guest swaps clips after this frame's
        // `update_models` ran, so without this the draw, node lights/emitters
        // and the camera pin would see the old glb's pose for a frame.
        self.resample_model(entity);
        true
    }

    /// The `Gfx.textures` key a mesh's bound atlas (`classic_atlas`,
    /// `"<ns>::<texture>"`, a ROM catalog PNG) draws with: the key itself,
    /// else its bare `<texture>` (a single-ROM boot registers textures
    /// unqualified).  Resolved when the glb loads: a ROM's textures upload
    /// before its models.
    pub(crate) fn model_atlas_texture(&self, key: &str) -> String {
        let has = |k: &str| self.gfx.as_ref().is_some_and(|g| g.textures.contains_key(k));
        if !has(key) {
            if let Some((_, bare)) = key.rsplit_once("::") {
                if has(bare) {
                    return bare.to_string();
                }
            }
        }
        key.to_string()
    }

    /// Whether a named (namespace-resolved) model resource is loaded.
    pub fn has_model(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    /// Advance every `Model`'s clip by `delta` seconds and re-sample its pose
    /// into `Model.node_world` + `Model.frame_offset`.  A model with no clip
    /// (or an unknown one) holds its bind pose; a finished one-shot clip holds
    /// its last pose.  Run once per frame by the demo's model system.
    ///
    /// The pose is only re-sampled when it can have changed (a different
    /// glb, clip, clip time or scale; see [`PoseKey`]), and not at all for a
    /// hidden (`Disabled`) model nothing reads it from: it is not drawn and
    /// its parented lights are off, but an `Emitter` parented to it keeps
    /// following it while its particles fade, so those keep sampling.  A
    /// skipped model's clip still advances, and it re-samples once shown.
    pub fn update_models(&mut self, delta: f32) {
        let _perf = classic_gfx::perf::scope("update_models");
        let followed = self.emitter_followed_hidden_models();
        let models = &self.models;
        let generation = self.model_generation;
        let scratch = &mut self.pose_scratch;
        for (e, (m, tf, hidden)) in
            self.world.query_mut::<(&mut Model, &Transform, Option<&Disabled>)>()
        {
            let Some(asset) = models.get(&m.model) else { continue };
            let clip = m.clip.as_deref().and_then(|c| asset.clip_index(c));
            if let Some(ci) = clip {
                let duration = asset.clips[ci].duration;
                if m.playing {
                    m.time += delta * m.speed;
                }
                if duration > 0.0 && m.time >= duration {
                    if m.repeat {
                        m.time %= duration;
                    } else {
                        m.time = duration;
                        m.playing = false;
                    }
                }
            }
            if hidden.is_some() && !followed.contains(&e) {
                continue;
            }
            sample_pose(asset, generation, clip, m, tf, scratch);
        }
    }

    /// Hidden model entities an `Emitter` is parented to (empty, without
    /// scanning the emitters, when no model is hidden).
    fn emitter_followed_hidden_models(&self) -> Vec<hecs::Entity> {
        let mut hidden = self.world.query::<(&Model, &Disabled)>();
        if hidden.iter().next().is_none() {
            return Vec::new();
        }
        let mut emitters = self.world.query::<&classic_core::components::Emitter>();
        emitters
            .iter()
            .filter_map(|(_, em)| em.parent.as_deref().and_then(|p| self.names.get(p)).copied())
            .collect()
    }

    /// Re-sample one `Model` entity's pose at its current clip time without
    /// advancing it (`Model.node_world` + `Model.frame_offset`), so a pose
    /// changed outside [`Engine::update_models`] (a clip swap to a glb with a
    /// different node set) is consistent before the next draw.
    pub fn resample_model(&mut self, entity: hecs::Entity) {
        let _perf = classic_gfx::perf::scope("update_models");
        let models = &self.models;
        let (generation, scratch) = (self.model_generation, &mut self.pose_scratch);
        let Ok((m, tf)) = self.world.query_one_mut::<(&mut Model, &Transform)>(entity) else {
            return;
        };
        let Some(asset) = models.get(&m.model) else { return };
        let clip = m.clip.as_deref().and_then(|c| asset.clip_index(c));
        sample_pose(asset, generation, clip, m, tf, scratch);
    }

    /// The world placement of a model entity: its origin on the terrain ground
    /// point of its tile position (no ground offset — the exported glb carries
    /// its own clearance), scaled by its `Transform`, axis-fixed from glTF.
    pub(crate) fn model_placement(&self, tf: &Transform, tilemap_name: &str) -> Option<Mat4> {
        let &tm_entity = self.names.get(tilemap_name)?;
        let tilemap_tf = self.world.get::<&Transform>(tm_entity).ok()?;
        let tilemap = self.world.get::<&Tilemap>(tm_entity).ok()?;
        let h = sample_height_mesh(
            &tilemap.height_data,
            tilemap.size_x,
            tilemap.size_y,
            tf.position.x,
            tf.position.y,
        );
        let ground = iso_world_pos(tf.position.x, tf.position.y, h) + tilemap_tf.position;
        Some(Mat4::from_translation(ground) * Mat4::from_scale(tf.scale) * gltf_to_world())
    }

    /// The world transform (metres) of the named glTF node of a model entity
    /// (`placement · node_world[node]`), for `node`-attached lights and emitters.
    pub(crate) fn model_node_world(&self, tf: &Transform, m: &Model, node: &str) -> Option<Mat4> {
        let asset = self.models.get(&m.model)?;
        let idx = *self.node_indices(&m.model, asset, node, false).first()?;
        let pose = m.node_world.get(idx)?;
        Some(self.model_placement(tf, &m.tilemap)? * *pose)
    }

    /// The indices of `asset`'s (resource `model`'s) nodes named `pattern`
    /// (the first one; `prefix == false`) or whose name starts with it (all,
    /// in node order), cached per (glb, pattern) so a node-attached emitter
    /// or light does not scan every node name each frame.
    pub(crate) fn node_indices(
        &self,
        model: &str,
        asset: &ModelAsset,
        pattern: &str,
        prefix: bool,
    ) -> Rc<[usize]> {
        let mut cache = self.node_index.0.borrow_mut();
        if cache.generation != self.model_generation {
            *cache = NodeIndexes { generation: self.model_generation, ..Default::default() };
        }
        let map = if prefix { &mut cache.prefix } else { &mut cache.exact };
        if let Some(hit) = map.get(model).and_then(|m| m.get(pattern)) {
            return hit.clone();
        }
        let nodes = asset.nodes.iter().enumerate();
        let found: Rc<[usize]> = if prefix {
            nodes.filter(|(_, n)| n.name.starts_with(pattern)).map(|(i, _)| i).collect()
        } else {
            nodes.filter(|(_, n)| n.name == pattern).map(|(i, _)| i).take(1).collect()
        };
        map.entry(model.to_string()).or_default().insert(pattern.to_string(), found.clone());
        found
    }

    /// World transforms of every node of `m`'s **current** glb whose name
    /// starts with `prefix`, in glb node order (the multi-node `Emitter.node`
    /// pattern).  Resolved from the live asset each call, so a clip swap to a
    /// glb with a different node set re-resolves on the next frame.  Empty
    /// when nothing matches or the pose is not (yet) sized to this glb.
    pub(crate) fn model_nodes_world_with_prefix(
        &self,
        tf: &Transform,
        m: &Model,
        prefix: &str,
    ) -> Vec<Mat4> {
        let Some(asset) = self.models.get(&m.model) else { return Vec::new() };
        if m.node_world.len() != asset.nodes.len() {
            return Vec::new();
        }
        let Some(placement) = self.model_placement(tf, &m.tilemap) else { return Vec::new() };
        self.node_indices(&m.model, asset, prefix, true)
            .iter()
            .filter_map(|&i| m.node_world.get(i).map(|pose| placement * *pose))
            .collect()
    }

    /// Build the per-entity model draws for the `DrawKind::Model` render items.
    pub(crate) fn model_draws(
        &self,
        items: &[(f32, hecs::Entity, DrawKind)],
        name_by_entity: &HashMap<hecs::Entity, &str>,
    ) -> Vec<ModelDraw> {
        let mut draws = Vec::new();
        for (order, entity, kind) in items {
            if !matches!(kind, DrawKind::Model) {
                continue;
            }
            let (Ok(tf), Ok(model)) =
                (self.world.get::<&Transform>(*entity), self.world.get::<&Model>(*entity))
            else {
                continue;
            };
            let Some(asset) = self.models.get(&model.model) else { continue };
            let Some(slot) = self.model_gpu.slot(&model.model) else { continue };
            // A pose not sized to this glb (a model swapped without a
            // re-sample) falls back to the rest pose rather than vanishing.
            let rest;
            let node_world = if model.node_world.len() == asset.nodes.len() {
                &model.node_world
            } else {
                rest = asset.node_world_transforms(usize::MAX, 0.0);
                &rest
            };
            let Some(placement) = self.model_placement(&tf, &model.tilemap) else { continue };
            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut instances = Vec::new();
            for (ni, node) in asset.nodes.iter().enumerate() {
                let Some(mi) = node.mesh else { continue };
                let mesh = &asset.meshes[mi];
                let world = placement * node_world[ni];
                for x in [mesh.min.x, mesh.max.x] {
                    for y in [mesh.min.y, mesh.max.y] {
                        for z in [mesh.min.z, mesh.max.z] {
                            let p = world.transform_point3(Vec3::new(x, y, z));
                            min = min.min(p);
                            max = max.max(p);
                        }
                    }
                }
                let image = self.model_gpu.mesh_image(slot, mi, mesh.image);
                let textured = image.is_some();
                let team = match (model.tint, mesh.team_rect, textured) {
                    (Some(tint), Some(rect), true) => Some(classic_gfx::TeamTint { rect, tint }),
                    _ => None,
                };
                let gain = model.emissive.unwrap_or(1.0);
                let emissive = match textured {
                    true if gain > 0.0 => mesh
                        .emissive
                        .iter()
                        .map(|e| classic_gfx::EmissiveRect { strength: e.strength * gain, ..*e })
                        .collect(),
                    _ => Vec::new(),
                };
                instances.push(ModelInstance {
                    model: world,
                    mesh: MeshRef { slot, mesh: mi as u32, image },
                    team,
                    emissive,
                    alpha_test: mesh.alpha_test,
                });
            }
            if instances.is_empty() {
                continue;
            }
            let name = name_by_entity.get(entity).copied().unwrap_or("").to_string();
            draws.push(ModelDraw { order: *order, name, instances, min, max });
        }
        draws
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::{Light, LightKind, Role};
    use classic_core::RoleKind;

    use crate::ResourceKind;

    /// A minimal self-contained `.glb`: a `rig` root node with one child mesh
    /// node (a single triangle) and one LINEAR clip named `clip` translating
    /// the rig from glTF `(-4, 10, -4)` at t=0 to `(0, 2, 0)` at t=1 — world
    /// `(-4, 4, 10)` → `(0, 0, 2)` after the +90°-X axis fix.
    pub(crate) fn tiny_glb(clip: &str) -> Vec<u8> {
        tiny_glb_with_nodes(clip, "")
    }

    /// [`tiny_glb`] plus `extra_nodes` (raw glTF node JSON, each starting with
    /// a comma) appended after `body` as further children of `rig`, so two
    /// glbs can differ in node count.
    fn tiny_glb_with_nodes(clip: &str, extra_nodes: &str) -> Vec<u8> {
        let extra = extra_nodes.matches("\"name\"").count();
        let children: Vec<String> = (1..=1 + extra).map(|i| i.to_string()).collect();
        let children = children.join(",");
        let mut bin: Vec<u8> = Vec::new();
        let floats: [f32; 9 + 2 + 6] =
            [0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1., -4., 10., -4., 0., 2., 0.];
        for f in floats {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"rig","children":[{children}]}},{{"name":"body","mesh":0}}
                {extra_nodes}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],
            "buffers":[{{"byteLength":{len}}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},
                {{"buffer":0,"byteOffset":36,"byteLength":8}},
                {{"buffer":0,"byteOffset":44,"byteLength":24}}],
            "accessors":[
                {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
                  "min":[0,0,0],"max":[1,1,0]}},
                {{"bufferView":1,"componentType":5126,"count":2,"type":"SCALAR",
                  "min":[0],"max":[1]}},
                {{"bufferView":2,"componentType":5126,"count":2,"type":"VEC3"}}],
            "animations":[{{"name":"{clip}",
                "channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}],
                "samplers":[{{"input":1,"output":2,"interpolation":"LINEAR"}}]}}]}}"#,
            len = bin.len()
        );
        pack_glb(json, &bin)
    }

    /// Assemble a `.glb` from its JSON chunk and (4-byte padded) BIN chunk.
    fn pack_glb(json: String, bin: &[u8]) -> Vec<u8> {
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
        glb.extend_from_slice(bin);
        glb
    }

    /// A 2x2 RGBA PNG filled with `rgba`.
    fn png_2x2(rgba: [u8; 4]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// A one-triangle `.glb` textured with `png` (embedded), its material
    /// carrying `team_rect` as the `classic_team_rect` extra when given.
    fn textured_glb(png: &[u8], team_rect: Option<[f32; 4]>) -> Vec<u8> {
        let extras = team_rect
            .map(|r| format!(r#","extras":{{"classic_team_rect":{r:?}}}"#))
            .unwrap_or_default();
        textured_glb_extras(png, &extras)
    }

    /// [`textured_glb`] with a raw material `extras` suffix (`,"extras":{..}`).
    fn textured_glb_extras(png: &[u8], extras: &str) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for f in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        bin.extend_from_slice(png);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"rig","children":[1]}},{{"name":"body","mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},
                "material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}{extras}}}],
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
        pack_glb(json, &bin)
    }

    fn flat_tilemap(e: &mut Engine) {
        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            size_x: 8,
            size_y: 8,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 64],
            height_data: vec![0.0f32; 81],
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::splat(-1.0),
            selection_iso_end: Vec3::splat(-1.0),
            blend: None,
        };
        let tm = e.world.spawn((
            tilemap,
            Transform::new(Vec3::ZERO, Vec3::ONE),
            Role::new(RoleKind::Tilemap),
        ));
        e.names.insert("tilemap".into(), tm);
    }

    #[test]
    fn model_rom_hydrates_and_resolves_both_ref_passes() {
        // A namespaced ROM: `Model.model` must resolve through the resource-ref
        // pass and `Model.tilemap` through the cross-ref pass, or the draw path
        // skips the model (lesson: every new ref field goes in both passes).
        let manifest_json = r#"{"entrypoint": "lunar", "namespace": "lunar",
            "shaders": [], "textures": [], "animations": [],
            "models": [{"name": "landing", "src": "/models/landing.glb"}]}"#;
        let state = r#"{"entities": {
            "pad": {"components": []},
            "rocket": {"components": [{"type": "Model", "position": [2, 3, 0],
                "model": "landing", "tilemap": "pad", "clip": "landing"}]}}}"#;
        let mut resources = classic_rom::ResourceSet::default();
        resources.insert(classic_rom::ResourceKind::Model, "landing", tiny_glb("landing"));
        let rom = classic_rom::Rom {
            manifest: serde_json::from_str(manifest_json).unwrap(),
            manifest_json: manifest_json.into(),
            resources,
            state: state.into(),
        };
        let loaded = classic_rom::LoadedRoms {
            root: "lunar".into(),
            order: vec![classic_rom::LoadedRom {
                name: "lunar".into(),
                namespace: "lunar".into(),
                rom,
                sha256: None,
            }],
        };
        let mut e = Engine::new_for_test();
        let sink = classic_rom::VecBootSink::new();
        e.hydrate_roms(&loaded, &sink);

        assert!(e.models.contains_key("lunar::landing"));
        assert!(sink.events().iter().any(|ev| matches!(ev,
            classic_rom::BootEvent::ResourceDecoded { kind: classic_rom::ResourceKind::Model, name, .. }
                if name == "lunar::landing")));
        assert_eq!(
            e.resolve_resource("lunar", ResourceKind::Model, "landing").as_deref(),
            Some("lunar::landing")
        );
        let rocket = e.names["lunar::rocket"];
        {
            let m = e.world.get::<&Model>(rocket).unwrap();
            assert_eq!(m.model, "lunar::landing");
            assert_eq!(m.tilemap, "lunar::pad");
            // The spawner also adds the subsumed Transform.
            assert_eq!(e.world.get::<&Transform>(rocket).unwrap().position, Vec3::new(2., 3., 0.));
        }
        // The pose samples against the resolved asset (clip = rest until played).
        e.update_models(0.0);
        let m = e.world.get::<&Model>(rocket).unwrap();
        assert_eq!(m.node_world.len(), 2);
        assert!((m.frame_offset - Vec3::new(-4.0, 4.0, 10.0)).length() < 1e-4);
        drop(m);
        let dumped = e.dump_state();
        assert!(dumped.contains("\"Model\""), "dump includes Model: {dumped}");
        assert!(!dumped.contains("node_world"), "transient pose is not serialized");
    }

    #[test]
    fn model_clip_plays_and_holds_its_last_pose() {
        let mut e = Engine::new_for_test();
        e.load_model_glb("landing", &tiny_glb("landing")).unwrap();
        let rocket = e.world.spawn((
            Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE),
            Model::new("x", "tilemap"),
        ));
        e.names.insert("rocket".into(), rocket);
        assert!(e.start_model_clip("rocket", "landing", false));
        e.update_models(0.5);
        let at = |e: &Engine| e.world.get::<&Model>(rocket).unwrap().frame_offset;
        assert!((at(&e) - Vec3::new(-2.0, 2.0, 6.0)).length() < 1e-4, "{}", at(&e));
        e.update_models(10.0);
        assert!((at(&e) - Vec3::new(0.0, 0.0, 2.0)).length() < 1e-4, "{}", at(&e));
        assert!(!e.world.get::<&Model>(rocket).unwrap().playing, "one-shot stops");
        // A looping clip wraps instead.
        assert!(e.start_model_clip("rocket", "landing", true));
        e.update_models(1.25);
        assert!((at(&e) - Vec3::new(-3.0, 3.0, 8.0)).length() < 1e-3, "{}", at(&e));
    }

    #[test]
    fn unchanged_and_hidden_poses_are_not_resampled() {
        let mut e = Engine::new_for_test();
        e.load_model_glb("landing", &tiny_glb("landing")).unwrap();
        let rocket = e.world.spawn((
            Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE),
            Model::new("x", "tilemap"),
        ));
        e.names.insert("rocket".into(), rocket);
        assert!(e.start_model_clip("rocket", "landing", false));
        e.update_models(10.0);
        // Finished one-shot: a sentinel pose survives a frame (no re-sample).
        let sentinel = Vec3::splat(123.0);
        let pose = |e: &Engine| e.world.get::<&Model>(rocket).unwrap().frame_offset;
        e.world.get::<&mut Model>(rocket).unwrap().frame_offset = sentinel;
        e.update_models(0.016);
        assert_eq!(pose(&e), sentinel, "unchanged key skips the sample");
        // A new scale is a new key.
        e.world.get::<&mut Transform>(rocket).unwrap().scale = Vec3::splat(2.0);
        e.update_models(0.016);
        assert!((pose(&e) - Vec3::new(0.0, 0.0, 4.0)).length() < 1e-4, "{}", pose(&e));
        // Hidden and playing: the clip advances but the pose is not sampled...
        assert!(e.start_model_clip("rocket", "landing", true));
        e.world.insert_one(rocket, Disabled).unwrap();
        e.world.get::<&mut Model>(rocket).unwrap().frame_offset = sentinel;
        e.update_models(0.5);
        assert_eq!(pose(&e), sentinel, "hidden model skipped");
        assert_eq!(e.world.get::<&Model>(rocket).unwrap().time, 0.5);
        // ...unless an emitter follows it.
        e.world.spawn((classic_core::components::Emitter {
            parent: Some("rocket".into()),
            ..serde_json::from_str("{\"effect\": \"fx\"}").unwrap()
        },));
        e.update_models(0.0);
        assert!((pose(&e) - Vec3::new(-4.0, 4.0, 12.0)).length() < 1e-3, "{}", pose(&e));
        // Shown again: samples (the key moved on while hidden).
        let _ = e.world.remove_one::<Disabled>(rocket);
        e.world.get::<&mut Model>(rocket).unwrap().frame_offset = sentinel;
        e.update_models(0.25);
        assert_ne!(pose(&e), sentinel);
    }

    #[test]
    fn parented_light_tracks_model_rig_origin() {
        // Burn light parented to a Model: ground point of the model's tile +
        // the animated rig-origin translation + the light's own offset.
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        e.load_model_glb("landing", &tiny_glb("landing")).unwrap();
        let tile = Vec3::new(3.0, 2.0, 0.0);
        let rocket =
            e.world.spawn((Transform::new(tile, Vec3::ONE), Model::new("landing", "tilemap")));
        e.names.insert("rocket".into(), rocket);
        let offset = Vec3::new(0.0, 0.0, -2.0);
        e.world.spawn((Light {
            kind: LightKind::Point,
            position: offset,
            color: [1.0, 0.6, 0.2],
            intensity: 3.0,
            radius: 8.125,
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: Some("rocket".into()),
            node: None,
        },));
        assert!(e.start_model_clip("rocket", "landing", false));
        e.update_models(0.5);

        let lights = e.gather_lights();
        let ground = iso_world_pos(tile.x, tile.y, 0.0);
        let expected = ground + Vec3::new(-2.0, 2.0, 6.0) + offset;
        assert!((lights[0].position - expected).length() < 1e-4, "{}", lights[0].position);
        assert_eq!(lights[0].radius, 8.125, "radius is metres, uploaded verbatim");

        // The draw placement agrees with the light: the rig node's world
        // translation is the same ground + rig-origin point.
        let items = vec![(0.0, rocket, DrawKind::Model)];
        let names = HashMap::from([(rocket, "rocket")]);
        let draws = e.model_draws(&items, &names);
        assert_eq!(draws.len(), 1);
        let rocket_m = e.world.get::<&Model>(rocket).unwrap();
        let rig_world = Mat4::from_translation(ground) * gltf_to_world() * rocket_m.node_world[0];
        assert!((rig_world.w_axis.truncate() - (expected - offset)).length() < 1e-4);
        // The mesh instance sits under the rig and the AABB encloses it.
        let inst = &draws[0].instances[0];
        assert_eq!(e.model_gpu.key(inst.mesh), "landing::0");
        let origin = inst.model.w_axis.truncate();
        assert!(origin.cmpge(draws[0].min - 1e-4).all() && origin.cmple(draws[0].max + 1e-4).all());
    }

    #[test]
    fn node_indices_are_cached_per_glb_and_reload() {
        let mut e = Engine::new_for_test();
        let lamps = r#",{"name":"fx_lamp_0","translation":[1,0,0]},
            {"name":"fx_lamp_1","translation":[0,0,2]}"#;
        e.load_model_glb("work", &tiny_glb_with_nodes("work", lamps)).unwrap();
        let asset = e.models["work"].clone();
        let scan: Vec<usize> = (0..asset.nodes.len())
            .filter(|&i| asset.nodes[i].name.starts_with("fx_lamp_"))
            .collect();
        assert_eq!(&*e.node_indices("work", &asset, "fx_lamp_", true), scan.as_slice());
        assert_eq!(&*e.node_indices("work", &asset, "fx_lamp_1", false), &[scan[1]]);
        assert!(e.node_indices("work", &asset, "nope", false).is_empty());
        // Cached: the same allocation comes back.
        let a = e.node_indices("work", &asset, "fx_lamp_", true);
        assert!(Rc::ptr_eq(&a, &e.node_indices("work", &asset, "fx_lamp_", true)));
        // Re-loading `work` without lamps moves the generation: a miss.
        e.load_model_glb("work", &tiny_glb("work")).unwrap();
        let asset = e.models["work"].clone();
        assert!(e.node_indices("work", &asset, "fx_lamp_", true).is_empty());
    }

    #[test]
    fn clip_swap_to_a_different_node_set_is_posed_immediately() {
        // `start_model_clip` runs after this frame's `update_models` (the
        // guest hook comes later), so the swap must re-pose on its own: the
        // draw, node lights and node emitters all see the new glb at once.
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        e.load_model_glb("idle", &tiny_glb("idle")).unwrap();
        let lamps = r#",{"name":"fx_lamp_0","translation":[1,0,0]},
            {"name":"fx_lamp_1","translation":[0,0,2]}"#;
        e.load_model_glb("work", &tiny_glb_with_nodes("work", lamps)).unwrap();
        assert_eq!((e.models["idle"].nodes.len(), e.models["work"].nodes.len()), (2, 4));
        let tile = Vec3::new(3.0, 2.0, 0.0);
        let hab = e.world.spawn((Transform::new(tile, Vec3::ONE), Model::new("idle", "tilemap")));
        e.names.insert("hab".into(), hab);
        e.update_models(0.25);

        assert!(e.start_model_clip("hab", "work", true));
        // No `update_models` in between.
        let m = e.world.get::<&Model>(hab).unwrap().clone();
        assert_eq!(m.node_world.len(), 4, "pose sized to the new glb");
        assert_eq!(m.time, 0.0);
        let rest_rig = Vec3::new(-4.0, 4.0, 10.0);
        assert!((m.frame_offset - rest_rig).length() < 1e-4, "clip time 0: {}", m.frame_offset);
        let items = vec![(0.0, hab, DrawKind::Model)];
        let draws = e.model_draws(&items, &HashMap::new());
        assert_eq!(draws.len(), 1, "the model does not vanish for a frame");
        assert_eq!(e.model_gpu.key(draws[0].instances[0].mesh), "work::0");

        // Node attachments resolve to the new glb's nodes.
        let tf = e.world.get::<&Transform>(hab).unwrap().clone();
        let ground = iso_world_pos(tile.x, tile.y, 0.0);
        let lamp0 = e.model_node_world(&tf, &m, "fx_lamp_0").expect("light node");
        let rig = Mat4::from_translation(ground) * gltf_to_world() * m.node_world[0];
        let want = rig.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!((lamp0.w_axis.truncate() - want).length() < 1e-4);
        let sites = e.model_nodes_world_with_prefix(&tf, &m, "fx_lamp_");
        assert_eq!(sites.len(), 2, "emitter node pattern sees both lamps");
        let light = e.attachment_frame(Some("hab"), Some("fx_lamp_0"), Vec3::ZERO, Vec3::Z);
        assert!((light.unwrap().0 - want).length() < 1e-4);
    }

    #[test]
    fn unsized_pose_draws_the_rest_pose() {
        // Defensive: a model swapped without a re-sample draws its rest pose
        // instead of being skipped.
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        e.load_model_glb("idle", &tiny_glb("idle")).unwrap();
        let lamp = r#",{"name":"fx_lamp_0"}"#;
        e.load_model_glb("work", &tiny_glb_with_nodes("work", lamp)).unwrap();
        let hab = e.world.spawn((
            Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE),
            Model::new("idle", "tilemap"),
        ));
        e.update_models(0.0);
        e.world.get::<&mut Model>(hab).unwrap().model = "work".into();
        let draws = e.model_draws(&[(0.0, hab, DrawKind::Model)], &HashMap::new());
        assert_eq!(draws.len(), 1);
        assert_eq!(e.model_gpu.key(draws[0].instances[0].mesh), "work::0");
    }

    #[test]
    fn model_draw_order_is_opaque_front_to_back_then_alpha_tested() {
        let mut order = vec![
            (false, -3.0, "far"),
            (true, 9.0, "cut_near"),
            (false, 5.0, "near"),
            (true, -1.0, "cut_far"),
            (false, 5.0, "near_tie"),
            (false, 0.0, "mid"),
        ];
        sort_model_draw_order(&mut order);
        let names: Vec<_> = order.iter().map(|o| o.2).collect();
        assert_eq!(names, ["near", "near_tie", "mid", "far", "cut_near", "cut_far"]);
    }

    #[test]
    fn alpha_test_follows_cut_texels_through_shared_images() {
        // The first glb decodes an atlas with a cut texel its triangle
        // covers; a second glb aliasing the same bytes (not decoded again)
        // still gets the alpha-tested program from the shared mask.
        let cut = png_2x2([200, 200, 200, 0]);
        let mut e = Engine::new_for_test();
        e.load_model_glb("a", &textured_glb(&cut, None)).unwrap();
        e.load_model_glb("b", &textured_glb(&cut, None)).unwrap();
        assert!(e.models["b"].images[0].rgba8.is_empty(), "b aliased a's texture");
        assert!(e.models["a"].meshes[0].alpha_test && e.models["b"].meshes[0].alpha_test);
        e.load_model_glb("solid", &textured_glb(&png_2x2([200, 200, 200, 255]), None)).unwrap();
        assert!(!e.models["solid"].meshes[0].alpha_test, "opaque atlas: no discard");
        // The flag reaches the draw.
        flat_tilemap(&mut e);
        let ents: Vec<_> = ["a", "solid"]
            .map(|m| {
                let ent = e.world.spawn((
                    Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE),
                    Model::new(m, "tilemap"),
                ));
                (0.0, ent, DrawKind::Model)
            })
            .into();
        e.update_models(0.0);
        let draws = e.model_draws(&ents, &HashMap::new());
        assert_eq!(
            [draws[0].instances[0].alpha_test, draws[1].instances[0].alpha_test],
            [true, false]
        );
    }

    #[test]
    fn identical_model_images_share_one_texture() {
        // A kit building: one glb per clip, each embedding the same atlas.
        let atlas = png_2x2([200, 200, 200, 255]);
        let mut e = Engine::new_for_test();
        for clip in ["idle", "build", "work"] {
            let dims = e.load_model_glb(&format!("hab::{clip}"), &textured_glb(&atlas, None));
            assert_eq!(dims, Some((2, 2)), "every clip reports the atlas size");
        }
        // One content entry: one upload, the rest alias it.
        assert_eq!(e.model_textures.len(), 1, "one texture for three clips");
        assert_eq!(e.model_textures.values().next().unwrap().name, "hab::idle::image::0");
        // Only the first glb decoded the atlas.
        assert_eq!(e.models["hab::idle"].images[0].rgba8.len(), 16);
        for clip in ["hab::build", "hab::work"] {
            let img = &e.models[clip].images[0];
            assert!(img.rgba8.is_empty(), "{clip}: a shared image is not decoded");
            assert_eq!((img.width, img.height), (2, 2));
        }
        // Different bytes get their own texture.
        e.load_model_glb("depot::idle", &textured_glb(&png_2x2([9, 9, 9, 255]), None)).unwrap();
        assert_eq!(e.model_textures.len(), 2);
    }

    #[test]
    fn team_tint_reaches_only_meshes_with_a_team_rect() {
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        let atlas = png_2x2([230, 230, 230, 255]);
        let rect = [0.5, 0.0, 1.0, 0.25];
        e.load_model_glb("kit", &textured_glb(&atlas, Some(rect))).unwrap();
        e.load_model_glb("rocket", &textured_glb(&atlas, None)).unwrap();
        let red = [1.0, 0.1, 0.1];
        let mut spawn = |name: &str, model: &str, tint: Option<[f32; 3]>| {
            let mut m = Model::new(model, "tilemap");
            m.tint = tint;
            let ent = e.world.spawn((Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE), m));
            e.names.insert(name.into(), ent);
            ent
        };
        let tinted = spawn("tinted", "kit", Some(red));
        let plain = spawn("plain", "kit", None);
        let no_rect = spawn("no_rect", "rocket", Some(red));
        e.update_models(0.0);
        let items: Vec<_> = [tinted, plain, no_rect].map(|x| (0.0, x, DrawKind::Model)).into();
        let draws = e.model_draws(&items, &HashMap::new());
        let team: Vec<_> = draws.iter().map(|d| d.instances[0].team).collect();
        assert_eq!(team, [Some(classic_gfx::TeamTint { rect, tint: red }), None, None]);
        // The texture name stays per model even though the GL texture is shared.
        assert_eq!(e.model_gpu.texture(draws[2].instances[0].mesh), Some("rocket::image::0"));
    }

    #[test]
    fn a_bound_atlas_is_the_draw_texture() {
        // `classic_atlas` wins over an embedded image; the team / emissive
        // regions apply to the bound texture.
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        let extras = r#","extras":{"classic_atlas":"buildtest::kit_atlas","classic_team_rect":[0,0,1,0.25],
            "classic_emissive":[{"rect":[0.5,0.5,1,1],"strength":2}]}"#;
        e.load_model_glb("kit", &textured_glb_extras(&png_2x2([0, 0, 0, 0]), extras)).unwrap();
        let mut m = Model::new("kit", "tilemap");
        m.tint = Some([1.0, 0.0, 0.0]);
        let ent = e.world.spawn((Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE), m));
        e.update_models(0.0);
        let draws = e.model_draws(&[(0.0, ent, DrawKind::Model)], &HashMap::new());
        let inst = &draws[0].instances[0];
        // no Gfx in tests: the key is used as is
        assert_eq!(e.model_gpu.texture(inst.mesh), Some("buildtest::kit_atlas"));
        assert!(inst.team.is_some() && inst.emissive.len() == 1);
        assert!(!inst.alpha_test, "the (cut) embedded image is not what draws");
    }

    #[test]
    fn emissive_rects_reach_the_draw_scaled_by_the_model() {
        let mut e = Engine::new_for_test();
        flat_tilemap(&mut e);
        let atlas = png_2x2([230, 230, 230, 255]);
        let lit = r#","extras":{"classic_emissive":[{"rect":[0.5,0,1,0.5],"strength":2}]}"#;
        e.load_model_glb("lamp", &textured_glb_extras(&atlas, lit)).unwrap();
        e.load_model_glb("plain", &textured_glb(&atlas, None)).unwrap();
        let mut spawn = |model: &str, gain: Option<f32>| {
            let mut m = Model::new(model, "tilemap");
            m.emissive = gain;
            e.world.spawn((Transform::new(Vec3::new(2.0, 2.0, 0.0), Vec3::ONE), m))
        };
        let ents = [spawn("lamp", None), spawn("lamp", Some(0.5)), spawn("lamp", Some(0.0))];
        let plain = spawn("plain", None);
        e.update_models(0.0);
        let items: Vec<_> =
            ents.iter().chain([&plain]).map(|x| (0.0, *x, DrawKind::Model)).collect();
        let draws = e.model_draws(&items, &HashMap::new());
        let strengths: Vec<Vec<f32>> = draws
            .iter()
            .map(|d| d.instances[0].emissive.iter().map(|r| r.strength).collect())
            .collect();
        assert_eq!(strengths, [vec![2.0], vec![1.0], vec![], vec![]]);
        assert_eq!(draws[0].instances[0].emissive[0].rect, [0.5, 0.0, 1.0, 0.5]);
        let glow: Vec<bool> = draws.iter().map(|d| d.has_emissive()).collect();
        assert_eq!(glow, [true, true, false, false], "bloom gate: only glowing models");
        // The glow lands only inside the rect (the shader's CPU reference).
        let rects = &draws[0].instances[0].emissive;
        let (albedo, dark) = ([0.9, 0.9, 0.9], [0.0; 3]);
        assert_eq!(classic_gfx::emissive_shade(rects, [0.75, 0.25], albedo, dark), [1.8; 3]);
        assert_eq!(classic_gfx::emissive_shade(rects, [0.25, 0.25], albedo, dark), [0.0; 3]);
        assert_eq!(classic_gfx::emissive_shade(rects, [0.75, 0.5], albedo, dark), [0.0; 3]);
    }

    #[test]
    fn model_emissive_spawns_and_dumps_through_the_registry() {
        classic_core::register_all_components();
        let reg =
            classic_core::registry::ordered_regs().into_iter().find(|r| r.name == "Model").unwrap();
        let mut b = hecs::EntityBuilder::new();
        let v = serde_json::json!({"position": [1, 2, 0], "model": "kit", "tilemap": "t",
            "emissive": 0.25});
        (reg.spawn)(&mut b, v).unwrap();
        let mut world = hecs::World::new();
        let ent = world.spawn(b.build());
        assert_eq!(world.get::<&Model>(ent).unwrap().emissive, Some(0.25));
        assert_eq!(reg.dump_value(&world, ent).unwrap()["emissive"], serde_json::json!(0.25));
        world.get::<&mut Model>(ent).unwrap().emissive = None;
        assert!(reg.dump_value(&world, ent).unwrap().get("emissive").is_none(), "omitted");
    }

    #[test]
    fn model_tint_spawns_and_dumps_through_the_registry() {
        classic_core::register_all_components();
        let mut b = hecs::EntityBuilder::new();
        let reg =
            classic_core::registry::ordered_regs().into_iter().find(|r| r.name == "Model").unwrap();
        let v = serde_json::json!({"position": [1, 2, 0], "model": "kit", "tilemap": "t",
            "tint": [0.2, 0.4, 1.0]});
        (reg.spawn)(&mut b, v).unwrap();
        let mut world = hecs::World::new();
        let ent = world.spawn(b.build());
        assert_eq!(world.get::<&Model>(ent).unwrap().tint, Some([0.2, 0.4, 1.0]));
        let dumped = reg.dump_value(&world, ent).unwrap();
        assert_eq!(dumped["tint"], serde_json::json!([0.2f32, 0.4f32, 1.0f32]));
        // Untinted: the field is omitted, so existing dumps are unchanged.
        world.get::<&mut Model>(ent).unwrap().tint = None;
        assert!(reg.dump_value(&world, ent).unwrap().get("tint").is_none());
    }
}
