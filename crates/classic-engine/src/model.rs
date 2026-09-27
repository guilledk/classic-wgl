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

use std::collections::HashMap;

use classic_core::components::{Model, Tilemap};
use classic_core::math::iso_world_pos;
use classic_core::model::gltf_to_world;
use classic_core::tilemap::sample_height_mesh;
use classic_core::Transform;
use glam::{Mat4, Vec3};

use crate::{DrawKind, Engine};

/// One mesh instance of a model draw.
pub(crate) struct ModelInstance {
    /// The node's full world transform (metres).
    pub(crate) model: Mat4,
    /// Key into `Engine::model_gpu` (`"{model}::{mesh}"`).
    pub(crate) mesh_key: String,
    /// The albedo texture name (`"{model}::image::{i}"`), if textured.
    pub(crate) texture: Option<String>,
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
        true
    }

    /// Whether a named (namespace-resolved) model resource is loaded.
    pub fn has_model(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    /// Advance every `Model`'s clip by `delta` seconds and re-sample its pose
    /// into `Model.node_world` + `Model.frame_offset`.  A model with no clip
    /// (or an unknown one) holds its bind pose; a finished one-shot clip holds
    /// its last pose.  Run once per frame by the demo's model system.
    pub fn update_models(&mut self, delta: f32) {
        let models = &self.models;
        for (_e, (m, tf)) in self.world.query_mut::<(&mut Model, &Transform)>() {
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
            // Out-of-range clip index = rest pose.
            m.node_world = asset.node_world_transforms(clip.unwrap_or(usize::MAX), m.time);
            m.frame_offset = tf.scale * asset.root_translation_from(&m.node_world);
        }
    }

    /// The world placement of a model entity: its origin on the terrain ground
    /// point of its tile position (no ground offset — the exported glb carries
    /// its own clearance), scaled by its `Transform`, axis-fixed from glTF.
    fn model_placement(&self, tf: &Transform, tilemap_name: &str) -> Option<Mat4> {
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
            if model.node_world.len() != asset.nodes.len() {
                continue;
            }
            let Some(placement) = self.model_placement(&tf, &model.tilemap) else { continue };
            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut instances = Vec::new();
            for (ni, node) in asset.nodes.iter().enumerate() {
                let Some(mi) = node.mesh else { continue };
                let mesh = &asset.meshes[mi];
                let world = placement * model.node_world[ni];
                for x in [mesh.min.x, mesh.max.x] {
                    for y in [mesh.min.y, mesh.max.y] {
                        for z in [mesh.min.z, mesh.max.z] {
                            let p = world.transform_point3(Vec3::new(x, y, z));
                            min = min.min(p);
                            max = max.max(p);
                        }
                    }
                }
                instances.push(ModelInstance {
                    model: world,
                    mesh_key: format!("{}::{mi}", model.model),
                    texture: mesh.image.map(|ii| format!("{}::image::{ii}", model.model)),
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
        let mut bin: Vec<u8> = Vec::new();
        let floats: [f32; 9 + 2 + 6] =
            [0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1., -4., 10., -4., 0., 2., 0.];
        for f in floats {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"rig","children":[1]}},{{"name":"body","mesh":0}}],
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
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
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
        assert_eq!(inst.mesh_key, "landing::0");
        let origin = inst.model.w_axis.truncate();
        assert!(origin.cmpge(draws[0].min - 1e-4).all() && origin.cmple(draws[0].max + 1e-4).all());
    }
}
