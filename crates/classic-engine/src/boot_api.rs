//! ROM boot and hydration: GL/shader bring-up, the incremental boot plan,
//! namespacing, resource loading, and ROM dump/save round-trips.

use std::collections::HashMap;
use std::rc::Rc;

use classic_core::components::{
    Animator, IsoAgent, IsoSprite, IsoVehicle, NavMesh, SdfTextRender, Tilemap,
};
use classic_core::instrument::Chan;
use classic_core::math::{iso_basis, iso_camera_ray, Ray, DEPTH_FAR, DEPTH_NEAR};
use classic_core::tilemap::{build_mesh, build_tile_texture, raycast_terrain, PPM_TARGET, TILE_M};
use classic_core::types::AnimChannel;
use classic_core::types::FrameTable;
use classic_core::types::OffsetKeyframe;
use classic_core::types::SdfFontMetrics;
use classic_core::{RoleKind, SpriteRender, Transform};
use classic_gfx::{Gfx, GlBuffer};
use glam::Vec3;

use crate::{boot, BasisTextureJob, Engine, ResourceKind, TextureDepth, TilemapGpu};

impl Engine {
    /// Compile the engine shader catalog (built-ins + any manifest `shaders[]`
    /// overrides) into a fresh [`Gfx`], exactly once.  Idempotent across a
    /// multi-ROM load: the first call creates the GL layer, later calls no-op.
    /// Emits a [`classic_rom::BootEvent::ShaderCompiled`] per compiled program.
    pub(crate) fn ensure_gfx(
        &mut self,
        gl: Rc<glow::Context>,
        manifest: &classic_rom::RomManifest,
        sink: &dyn classic_rom::BootSink,
    ) {
        if self.gfx.is_some() && self.gfx_full {
            return;
        }
        if self.gfx.is_none() {
            self.gfx = Some(Gfx::new(gl));
        }
        let registry = classic_gfx::ShaderSourceRegistry::builtin();
        // The engine owns the shader declarations: compile the full builtin
        // catalog by default, letting the ROM override any shader by *name*
        // (a manifest `shaders[]` entry with a matching name swaps its
        // vertex/fragment filenames + layout).
        let overrides: HashMap<&str, &classic_core::types::ShaderInfo> =
            manifest.manifest.shaders.iter().map(|info| (info.name.as_str(), info)).collect();
        for builtin in classic_gfx::builtin_shaders() {
            let (vs_name, fs_name, attr, unif): (&str, &str, Vec<&str>, Vec<&str>) = match overrides
                .get(builtin.name)
            {
                Some(info) => (
                    info.vertex.as_str(),
                    info.fragment.as_str(),
                    info.attr.iter().map(String::as_str).collect(),
                    info.unif.iter().map(String::as_str).collect(),
                ),
                None => {
                    (builtin.vertex, builtin.fragment, builtin.attr.to_vec(), builtin.unif.to_vec())
                }
            };
            let vs = registry.resolve_vertex(vs_name);
            let fs = registry.resolve_fragment(fs_name);
            self.gfx
                .as_mut()
                .unwrap()
                .add_shader(builtin.name, &vs, &fs, &attr, &unif)
                .expect("compile shader");
            sink.on_event(classic_rom::BootEvent::ShaderCompiled {
                name: builtin.name.to_string(),
            });
        }

        // The embedded DejaVu Sans SDF font, so `draw_sdf` works from frame 0
        // (the boot loading screen renders before any ROM font is hydrated).
        self.load_embedded_font();
        self.gfx_full = true;
    }

    /// Set up the GL layer eagerly so the boot loading screen can draw from
    /// frame 0, *before* any ROM manifest is available.  Only the two shaders
    /// the loader needs are compiled (`solid` for rects/lines, `sdf` for text)
    /// plus the embedded font; the real boot compiles the full catalog (with
    /// any manifest overrides) via [`Engine::ensure_gfx`].  Idempotent.
    ///
    /// Used by the windowed desktop path, whose background thread resolves +
    /// decodes + compiles while this thread renders the loader.
    pub fn init_gfx(&mut self, gl: Rc<glow::Context>) {
        if self.gfx.is_some() {
            return;
        }
        self.gfx = Some(Gfx::new(gl));
        let registry = classic_gfx::ShaderSourceRegistry::builtin();
        for builtin in classic_gfx::builtin_shaders() {
            if builtin.name != "solid" && builtin.name != "sdf" {
                continue;
            }
            let vs = registry.resolve_vertex(builtin.vertex);
            let fs = registry.resolve_fragment(builtin.fragment);
            self.gfx
                .as_mut()
                .unwrap()
                .add_shader(builtin.name, &vs, &fs, builtin.attr, builtin.unif)
                .expect("compile shader");
        }
        self.load_embedded_font();
    }

    /// Load the embedded DejaVu Sans SDF atlas + metrics into
    /// [`Engine::sdf_fonts`] and upload the atlas texture under
    /// [`classic_core::components::DEFAULT_SDF_FONT`] so the boot loading
    /// screen (and any text drawn before the ROM font hydrates) renders from
    /// frame 0.  Byte-identical to the atlas the `common` ROM ships.
    fn load_embedded_font(&mut self) {
        // TODO(mono-font): swap to a DejaVu Sans Mono atlas for a cleaner
        // monospaced log scroller.  Same generator + Bitstream Vera/PD license
        // (already in use via `common::dejavusans`), so no new license surface;
        // needs `DejaVuSansMono.ttf` vendored into `classic-assets/fonts/`.
        const METRICS_JSON: &str = include_str!("../assets/dejavusans-sdf.json");
        const ATLAS_PNG: &[u8] = include_bytes!("../assets/dejavusans-sdf.png");

        let font_name = classic_core::components::DEFAULT_SDF_FONT;
        let metrics: SdfFontMetrics =
            serde_json::from_str(METRICS_JSON).expect("embedded SDF font metrics JSON");
        self.sdf_fonts.insert(font_name.to_string(), metrics);

        let atlas_name = format!("{font_name}-sdf");
        let img = image::load_from_memory(ATLAS_PNG).expect("embedded SDF atlas PNG");
        let luma = img.to_luma8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_r8(&atlas_name, &luma, luma.width(), luma.height());
            if let Some(tex) = gfx.textures.get(&atlas_name) {
                tex.set_linear(&gfx.gl);
            }
        }
    }

    /// Build the hydration [`boot::BootPlan`] for a resolved multi-ROM DAG.
    ///
    /// Precomputes the sequence of [`boot::BootStep`]s — one texture decode +
    /// upload per unique sheet, one SDF font, one metadata registration, one
    /// entity-hydration batch, and the shared finish tail — plus the pending
    /// basis jobs.  Needs no engine; consumed by a [`boot::BootPipeline`].
    pub(crate) fn begin_boot(loaded: &classic_rom::LoadedRoms) -> boot::BootPlan {
        let multi = loaded.order.len() > 1;
        let mut steps: Vec<boot::BootStep> = Vec::new();
        let mut basis_jobs: Vec<BasisTextureJob> = Vec::new();

        for (entry_idx, entry) in loaded.order.iter().enumerate() {
            let ns = Self::effective_namespace(entry, multi);
            let manifest = &entry.rom.manifest;
            let resources = &entry.rom.resources;
            let rom = if manifest.entrypoint.is_empty() {
                "root".to_string()
            } else {
                manifest.entrypoint.clone()
            };

            // Cheap non-GL registrations (texture names, depth/normal
            // bookkeeping, animations, frame tables, animation channels,
            // vehicles, data) happen as one step before the per-texture work.
            steps.push(boot::BootStep::RegisterMetadata { ns: ns.clone(), entry: entry_idx });

            // SDF atlas textures are skipped here — the font path uploads them.
            let atlas_names: std::collections::HashSet<String> =
                resources.fonts().keys().map(|f| Self::qualify(&ns, &format!("{f}-sdf"))).collect();

            // Several manifest entries share one `src` (every frame-table texture
            // points at its shared colour sheet), so decode + upload each unique
            // `src` once and alias the rest to the same GL texture.
            let mut uploaded_by_src: HashMap<String, String> = HashMap::new();
            let mut basis_by_src: HashMap<String, usize> = HashMap::new();

            for entry in &manifest.manifest.textures {
                let key = Self::qualify(&ns, &entry.name);
                if atlas_names.contains(&key) {
                    continue;
                }
                let Some(bytes) = resources.textures().get(&entry.name) else {
                    continue;
                };
                if let Some(from_key) = uploaded_by_src.get(&entry.src) {
                    steps.push(boot::BootStep::AliasTexture { key, from_key: from_key.clone() });
                    continue;
                }
                if let Some(job_idx) = basis_by_src.get(&entry.src) {
                    basis_jobs[*job_idx].keys.push(key);
                    continue;
                }
                // GPU-compressed textures (Phase 1) transcode + upload via the
                // `format`-declared target; uncompressed textures use the native
                // channel count (RGB8 normal / R8 depth / RGBA8 albedo).
                if let Some(format) = &entry.format {
                    let kind = if entry.name.ends_with("-depth") {
                        classic_rom::ResourceKind::Depth
                    } else if entry.name.ends_with("-normal") {
                        classic_rom::ResourceKind::Normal
                    } else {
                        classic_rom::ResourceKind::Texture
                    };
                    let idx = basis_jobs.len();
                    basis_jobs.push(BasisTextureJob {
                        keys: vec![key],
                        bytes: bytes.clone(),
                        format: format.clone(),
                        rom: rom.clone(),
                        kind,
                    });
                    basis_by_src.insert(entry.src.clone(), idx);
                    continue;
                }
                let kind = if entry.name.ends_with("-depth") {
                    classic_rom::ResourceKind::Depth
                } else if entry.name.ends_with("-normal") {
                    classic_rom::ResourceKind::Normal
                } else {
                    classic_rom::ResourceKind::Texture
                };
                let format = match kind {
                    classic_rom::ResourceKind::Depth => boot::TextureFormat::Luma8,
                    classic_rom::ResourceKind::Normal => boot::TextureFormat::Rgb8,
                    _ => boot::TextureFormat::Rgba8,
                };
                steps.push(boot::BootStep::Decode {
                    key: key.clone(),
                    rom: rom.clone(),
                    kind,
                    format,
                    bytes: bytes.clone(),
                });
                steps.push(boot::BootStep::Upload { key: key.clone() });
                uploaded_by_src.insert(entry.src.clone(), key);
            }

            // Per-texture depth maps (grayscale `gl_FragDepth` masks), uploaded
            // as sibling `"{name}-depth"` textures.
            for entry in &manifest.manifest.textures {
                if entry.depth.is_some() {
                    if let Some(bytes) = resources.depths().get(&entry.name) {
                        let key = Self::qualify(&ns, &entry.name);
                        let depth_tex = format!("{key}-depth");
                        steps.push(boot::BootStep::Decode {
                            key: depth_tex.clone(),
                            rom: rom.clone(),
                            kind: classic_rom::ResourceKind::Depth,
                            format: boot::TextureFormat::Luma8,
                            bytes: bytes.clone(),
                        });
                        steps.push(boot::BootStep::Upload { key: depth_tex });
                    }
                }
            }

            // Per-texture normal maps (RGB world-space normals), uploaded as
            // sibling `"{name}-normal"` textures.
            for entry in &manifest.manifest.textures {
                if entry.normal.is_some() {
                    if let Some(bytes) = resources.normals().get(&entry.name) {
                        let key = Self::qualify(&ns, &entry.name);
                        let normal_tex = format!("{key}-normal");
                        steps.push(boot::BootStep::Decode {
                            key: normal_tex.clone(),
                            rom: rom.clone(),
                            kind: classic_rom::ResourceKind::Normal,
                            format: boot::TextureFormat::Rgb8,
                            bytes: bytes.clone(),
                        });
                        steps.push(boot::BootStep::Upload { key: normal_tex });
                    }
                }
            }

            // SDF fonts: metrics JSON + atlas PNG (font name + "-sdf"), keyed by
            // the namespace-qualified font name.
            for (font_name, metrics_bytes) in resources.fonts() {
                let key = Self::qualify(&ns, font_name);
                let atlas_src = format!("{font_name}-sdf");
                let metrics_json =
                    std::str::from_utf8(metrics_bytes).expect("SDF metrics UTF-8").to_string();
                if let Some(atlas_png) = resources.textures().get(&atlas_src) {
                    steps.push(boot::BootStep::LoadSdfFont {
                        key,
                        metrics_json,
                        atlas_png: atlas_png.clone(),
                    });
                }
            }

            // Entity graph + grids.
            steps.push(boot::BootStep::HydrateEntry { ns, entry: entry_idx });
        }

        steps.push(boot::BootStep::Finish);

        boot::BootPlan { steps, basis_jobs, cursor: 0, decoded: HashMap::new() }
    }

    /// Register a ROM's non-GL metadata: texture names, depth/normal companion
    /// bookkeeping, animations, frame tables, animation channels, vehicles, and
    /// data artifacts.  Called by the [`boot::BootStep::RegisterMetadata`] step
    /// with `self.namespace` already set to the entry's effective namespace.
    fn register_manifest_metadata(
        &mut self,
        manifest: &classic_rom::RomManifest,
        resources: &classic_rom::ResourceSet,
    ) {
        // Every manifest texture name (including the SDF atlases) is registered
        // so `has_texture`/`resolve_resource` see it under its qualified key.
        for entry in &manifest.manifest.textures {
            self.texture_names.insert(self.entity_key(&entry.name));
        }

        // Depth/normal companion bookkeeping (the pixel uploads happen in the
        // decode/upload steps; here we record the sibling texture names).
        for entry in &manifest.manifest.textures {
            if entry.depth.is_some() {
                let key = self.entity_key(&entry.name);
                self.texture_depths
                    .insert(key.clone(), TextureDepth { depth_tex: format!("{key}-depth") });
            }
        }
        for entry in &manifest.manifest.textures {
            if entry.normal.is_some() {
                let key = self.entity_key(&entry.name);
                self.texture_normals.insert(key.clone(), format!("{key}-normal"));
            }
        }

        for anim in &manifest.manifest.animations {
            let mut anim = anim.clone();
            anim.src = self.entity_key(&anim.src);
            self.animations.insert(self.entity_key(&anim.name), anim);
        }

        // Packed-atlas frame tables (`frames.json`) keyed by texture name,
        // loaded from the ROM's `frames` resources.  Companion sheets are
        // uploaded as ordinary textures (declared in the manifest) and are
        // referenced by name from each frame's `sheet` index.  The table's
        // sheet + frame keys are qualified so the `{texture}_{frame}` frame-name
        // convention stays consistent under namespacing.
        for (name, bytes) in resources.frames() {
            match serde_json::from_slice::<FrameTable>(bytes) {
                Ok(mut table) => {
                    for sheet in &mut table.sheets {
                        sheet.name = self.entity_key(&sheet.name);
                    }
                    table.frames = table
                        .frames
                        .into_iter()
                        .map(|(fname, fr)| (self.entity_key(&fname), fr))
                        .collect();
                    table.precompute_companions();
                    self.frame_tables.insert(self.entity_key(name), table);
                }
                Err(e) => {
                    classic_core::cl_error!(Chan::Guest, "frame table '{name}' parse failed: {e}");
                }
            }
        }

        // Per-animation renderer metadata (frame offsets) declared in the
        // manifest is loaded from the ROM's `animations/` resources and folded
        // into the registered `AnimationData`.
        for (name, metadata_bytes) in resources.animations() {
            self.load_animation_channels(&self.entity_key(name), metadata_bytes);
        }

        // Wheeled-vehicle definitions (JSON sidecars) from the `vehicles`
        // resources, keyed by the manifest-declared name.  Each part's texture
        // is qualified so `spawn_vehicle` binds the namespaced sheet.
        for (name, bytes) in resources.vehicles() {
            match serde_json::from_slice::<classic_core::types::VehicleDef>(bytes) {
                Ok(mut def) => {
                    for part in def.parts.iter_mut().chain(def.tires.iter_mut()) {
                        part.texture = self.entity_key(&part.texture);
                    }
                    // The anchors data-artifact name is namespace-qualified the
                    // same way `vehicle_anchors` keys are, so `spawn_vehicle`
                    // resolves the right artifact under the ROM's namespace.
                    def.anchors = self.entity_key(&def.anchors);
                    self.vehicles.insert(self.entity_key(name), def);
                }
                Err(e) => {
                    classic_core::cl_error!(Chan::Guest, "vehicle '{name}' parse failed: {e}");
                }
            }
        }

        // Blender-exported data artifacts (vehicle anchors, …) from the `data`
        // resources, keyed by name and referenced by authored defs' `anchors`.
        for (name, bytes) in resources.data() {
            match serde_json::from_slice::<classic_core::types::VehicleAnchors>(bytes) {
                Ok(anchors) => {
                    self.vehicle_anchors.insert(self.entity_key(name), anchors);
                }
                Err(e) => {
                    classic_core::cl_error!(
                        Chan::Guest,
                        "data artifact '{name}' parse failed: {e}"
                    );
                }
            }
        }
    }

    /// Execute up to `n` [`boot::BootStep`]s from `plan` (built for `loaded`),
    /// returning the number consumed.  `n == usize::MAX` drains the whole plan.
    /// A `Decode` step already run off-thread (see [`boot::BootPipeline::prepare`])
    /// is a `Noop` by now, with its pixels waiting in `plan.decoded`.
    pub(crate) fn boot_step(
        &mut self,
        plan: &mut boot::BootPlan,
        loaded: &classic_rom::LoadedRoms,
        sink: &dyn classic_rom::BootSink,
        n: usize,
    ) -> usize {
        let mut ran = 0;
        while ran < n && plan.cursor < plan.steps.len() {
            let step = std::mem::take(&mut plan.steps[plan.cursor]);
            plan.cursor += 1;
            ran += 1;
            match step {
                boot::BootStep::Decode { key, rom, kind, format, bytes } => {
                    let decoded = boot::decode_texture(format, &bytes);
                    let dims = decoded.dims();
                    plan.decoded.insert(key.clone(), decoded);
                    sink.on_event(classic_rom::BootEvent::ResourceDecoded {
                        rom,
                        kind,
                        name: key,
                        dims,
                    });
                }
                boot::BootStep::Upload { key } => {
                    if let Some(decoded) = plan.decoded.remove(&key) {
                        self.upload_decoded(&key, &decoded);
                    }
                    sink.on_event(classic_rom::BootEvent::TextureUploaded { name: key });
                }
                boot::BootStep::AliasTexture { key, from_key } => {
                    let tex = self.gfx.as_ref().and_then(|g| g.textures.get(&from_key)).cloned();
                    if let (Some(tex), Some(gfx)) = (tex, self.gfx.as_mut()) {
                        gfx.textures.insert(key, tex);
                    }
                }
                boot::BootStep::RegisterMetadata { ns, entry } => {
                    self.namespace = ns;
                    let entry = &loaded.order[entry];
                    self.register_manifest_metadata(&entry.rom.manifest, &entry.rom.resources);
                }
                boot::BootStep::LoadSdfFont { key, metrics_json, atlas_png } => {
                    self.load_sdf_font(&key, &metrics_json, &atlas_png);
                }
                boot::BootStep::HydrateEntry { ns, entry } => {
                    self.namespace = ns.clone();
                    let entry = &loaded.order[entry];
                    self.hydrate_rom_entry(&ns, entry, sink);
                }
                boot::BootStep::Finish => {
                    self.finish_hydrate_roms(loaded);
                }
                boot::BootStep::Noop => {}
            }
        }
        ran
    }

    /// Transcode + upload one pending `.basis` texture inline, then alias its
    /// remaining keys.  Emits [`classic_rom::BootEvent::TextureUploaded`].
    pub(crate) fn upload_basis(&mut self, job: &BasisTextureJob, sink: &dyn classic_rom::BootSink) {
        self.load_texture_basis(&job.keys[0], &job.bytes, &job.format);
        sink.on_event(classic_rom::BootEvent::TextureUploaded { name: job.keys[0].clone() });
        self.alias_basis_keys(job);
    }

    /// Upload one already-transcoded `.basis` job (`payload`; `None` = it failed
    /// to transcode), then alias its remaining keys.  Emits
    /// [`classic_rom::BootEvent::TextureUploaded`] (the `ResourceDecoded` event
    /// was already emitted off-thread by [`boot::decode_basis_jobs`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn upload_basis_decoded(
        &mut self,
        job: &BasisTextureJob,
        payload: Option<&classic_gfx::DecodedBasis>,
        sink: &dyn classic_rom::BootSink,
    ) {
        if let Some(p) = payload {
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.upload_decoded_basis(&job.keys[0], p);
            }
        }
        sink.on_event(classic_rom::BootEvent::TextureUploaded { name: job.keys[0].clone() });
        self.alias_basis_keys(job);
    }

    /// Upload `.basis` textures through the web transcoder worker (awaited),
    /// aliasing each job's remaining keys.  Emits a
    /// [`classic_rom::BootEvent::ResourceDecoded`] per sheet as each worker
    /// transcode finishes (in plan order), then its `TextureUploaded`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn upload_basis_async(
        &mut self,
        jobs: &[BasisTextureJob],
        sink: &dyn classic_rom::BootSink,
    ) {
        for job in jobs {
            let dims = self.load_texture_basis_async(&job.keys[0], &job.bytes, &job.format).await;
            if let Some(dims) = dims {
                sink.on_event(classic_rom::BootEvent::ResourceDecoded {
                    rom: job.rom.clone(),
                    kind: job.kind,
                    name: job.keys[0].clone(),
                    dims,
                });
            }
            sink.on_event(classic_rom::BootEvent::TextureUploaded { name: job.keys[0].clone() });
            self.alias_basis_keys(job);
        }
    }

    /// Point every key after a `.basis` job's first at the first key's texture.
    fn alias_basis_keys(&mut self, job: &BasisTextureJob) {
        let tex = self.gfx.as_ref().and_then(|g| g.textures.get(&job.keys[0])).cloned();
        if let (Some(tex), Some(gfx)) = (tex, self.gfx.as_mut()) {
            for alias in &job.keys[1..] {
                gfx.textures.insert(alias.clone(), tex.clone());
            }
        }
    }

    /// Hydrate the engine from a single ROM (the legacy path).  Wraps the ROM
    /// in a one-entry [`classic_rom::LoadedRoms`] (its declared namespace, no
    /// deps) and delegates to [`Engine::load_roms`].
    pub fn load_rom(&mut self, gl: Rc<glow::Context>, rom: &classic_rom::Rom) {
        let name = if rom.manifest.entrypoint.is_empty() {
            "root".to_string()
        } else {
            rom.manifest.entrypoint.clone()
        };
        let loaded = classic_rom::LoadedRoms {
            root: name.clone(),
            order: vec![classic_rom::LoadedRom {
                name,
                namespace: rom.manifest.namespace.clone(),
                rom: rom.clone(),
                sha256: None,
            }],
        };
        self.load_roms(gl, &loaded, &classic_rom::NullBootSink);
    }

    /// Hydrate the engine from a resolved multi-ROM dependency DAG.
    ///
    /// Shaders are compiled once (honoring the root ROM's `shaders[]`
    /// overrides); each ROM's resources, entity graph, and grids are then
    /// hydrated in topological order (deps before dependents).  Entity names
    /// are namespace-qualified via [`Engine::entity_key`] while a ROM's
    /// namespace is non-empty.  The full DAG is recorded in
    /// [`Engine::loaded_roms`] for [`Engine::dump_roms`]; the root ROM's
    /// manifest/resources are also mirrored into the single-ROM fields for
    /// backward compatibility (`dump_rom`, `has_texture`, the F10 save path).
    ///
    /// A synchronous [`boot::BootPipeline`] without a finish hook; boot progress
    /// is streamed to `sink`.
    pub fn load_roms(
        &mut self,
        gl: Rc<glow::Context>,
        loaded: &classic_rom::LoadedRoms,
        sink: &dyn classic_rom::BootSink,
    ) {
        boot::BootPipeline::new(loaded.clone(), None).poll(self, &gl, sink, None);
    }

    /// The GL-free core of [`Engine::load_roms`] (no shader compile, and uploads
    /// find no GL layer), so the multi-ROM logic is unit-testable without a GL
    /// context.
    #[cfg(test)]
    pub(crate) fn hydrate_roms(
        &mut self,
        loaded: &classic_rom::LoadedRoms,
        sink: &dyn classic_rom::BootSink,
    ) {
        boot::BootPipeline::new(loaded.clone(), None).poll_with(self, None, sink, None);
    }

    /// Hydrate one ROM entry's state + grids (after its resources are loaded).
    fn hydrate_rom_entry(
        &mut self,
        ns: &str,
        entry: &classic_rom::LoadedRom,
        sink: &dyn classic_rom::BootSink,
    ) {
        let keys = self.load_state_in(ns, &entry.rom.state).expect("load ROM state");
        self.rewrite_cross_refs(ns, &keys);
        self.rewrite_resource_refs(ns, &keys);
        self.load_grids(&entry.rom.resources);
        sink.on_event(classic_rom::BootEvent::StateSpawned {
            rom: if entry.rom.manifest.entrypoint.is_empty() {
                entry.name.clone()
            } else {
                entry.rom.manifest.entrypoint.clone()
            },
            entities: keys.len(),
        });
    }

    /// The plan's `Finish` step:
    /// DAG bookkeeping, the item catalog, and per-scene vehicle overrides.
    fn finish_hydrate_roms(&mut self, loaded: &classic_rom::LoadedRoms) {
        self.loaded_roms = loaded.order.clone();

        // Item catalog: the root ROM's items/inventory_types (per-ROM item
        // merging across the dep closure is deferred to the multi-guest work).
        if let Some(root) = loaded.root_rom() {
            self.items = classic_core::inventory::ItemRegistry::build(
                &root.manifest.items,
                &root.manifest.inventory_types,
            );
            self.rom_manifest_json = Some(root.manifest_json.clone());
            self.rom_manifest = Some(root.manifest.clone());
            self.rom_resources = Some(root.resources.clone());
        }

        // Per-scene vehicle tuning: the shared vehicle def now lives in a dep
        // ROM (`lunar-common`), so the root scene's `vehicle_overrides` (emitted
        // into the manifest by `classic-roms`, keys already namespace-qualified)
        // are merged onto the hydrated `self.vehicles` *after* the whole dep
        // closure has loaded.
        if let Some(root) = loaded.root_rom() {
            self.apply_vehicle_overrides(&root.manifest.vehicle_overrides);
        }
    }

    /// Merge the root manifest's `vehicle_overrides` (qualified vehicle name →
    /// typed `VehicleOverrides`) into the hydrated vehicle registry by direct
    /// field assignment — no JSON round-trip.  A vehicle not present is skipped.
    pub(crate) fn apply_vehicle_overrides(
        &mut self,
        overrides: &std::collections::HashMap<String, classic_core::types::VehicleOverrides>,
    ) {
        for (name, ov) in overrides {
            if let Some(def) = self.vehicles.get_mut(name) {
                ov.apply_to(def);
            }
        }
    }

    /// Qualify an entity name with the active namespace (a no-op when the
    /// namespace is empty).  The single point where multi-ROM namespacing will
    /// be applied: `names`/`name_order` and every name lookup route through this
    /// once several ROMs can load concurrently.
    pub fn entity_key(&self, name: &str) -> String {
        self.entity_key_ns(&self.namespace, name)
    }

    /// Qualify a name under an explicit namespace (a no-op for the global/empty
    /// namespace or an already-qualified `ns::name`).  The per-guest counterpart
    /// of [`Engine::entity_key`]: guest SDK name routing uses this so a ROM's
    /// entities are keyed by its own namespace rather than the last ROM loaded.
    pub fn entity_key_ns(&self, ns: &str, name: &str) -> String {
        Self::qualify(ns, name)
    }

    /// [`Engine::entity_key_ns`] without an engine (boot planning).
    fn qualify(ns: &str, name: &str) -> String {
        if ns.is_empty() || name.contains("::") {
            name.to_string()
        } else {
            format!("{ns}::{name}")
        }
    }

    /// The effective namespace a ROM was hydrated under: its declared
    /// `namespace`, or its entrypoint/name when it participates in a multi-ROM
    /// DAG (empty = global).  The demo layer uses this to scope a ROM's guest.
    pub fn rom_namespace(&self, name: &str) -> String {
        let multi = self.loaded_roms.len() > 1;
        if let Some(entry) = self.loaded_roms.iter().find(|e| e.name == name) {
            return Self::effective_namespace(entry, multi);
        }
        String::new()
    }

    /// Derive a ROM's effective namespace for the multi-ROM model: its declared
    /// `namespace` when set, else its entrypoint (falling back to the resolver
    /// name) when it participates in a multi-ROM DAG.  The legacy single-ROM
    /// boot (one ROM, empty namespace) stays on the global (`""`) namespace so
    /// shipped scenes and their golden traces are unchanged.
    fn effective_namespace(entry: &classic_rom::LoadedRom, multi: bool) -> String {
        if !entry.namespace.is_empty() {
            return entry.namespace.clone();
        }
        if multi {
            if entry.rom.manifest.entrypoint.is_empty() {
                entry.name.clone()
            } else {
                entry.rom.manifest.entrypoint.clone()
            }
        } else {
            String::new()
        }
    }

    /// Rewrite the cross-entity references stored on a ROM's components into
    /// namespace-qualified keys, using the ROM's namespace as the referring
    /// namespace.  Covered: `NavMesh.map_entity`, `IsoSprite.tilemap` /
    /// `IsoAgent.tilemap`, `Animator.target` (the `entity.component` entity
    /// segment), and `IsoVehicle.tilemap` / `wheel_entities` / `tire_entities`.
    /// A bare reference resolves in the referring namespace first, then the
    /// global namespace (see [`Engine::resolve_entity_name`]); a dangling
    /// reference is left untouched (the draw path reports it as missing).
    fn rewrite_cross_refs(&mut self, ns: &str, keys: &[String]) {
        for key in keys {
            let Some(&entity) = self.names.get(key) else { continue };

            if let Some(map_entity) =
                self.world.get::<&NavMesh>(entity).ok().map(|n| n.map_entity.clone())
            {
                if !map_entity.is_empty() {
                    if let Some(resolved) = self.resolve_entity_name(ns, &map_entity) {
                        if let Ok(mut n) = self.world.get::<&mut NavMesh>(entity) {
                            n.map_entity = resolved;
                        }
                    }
                }
            }

            if let Some(tilemap) =
                self.world.get::<&IsoSprite>(entity).ok().map(|s| s.tilemap.clone())
            {
                if !tilemap.is_empty() {
                    if let Some(resolved) = self.resolve_entity_name(ns, &tilemap) {
                        if let Ok(mut s) = self.world.get::<&mut IsoSprite>(entity) {
                            s.tilemap = resolved;
                        }
                    }
                }
            }

            if let Some(tilemap) =
                self.world.get::<&IsoAgent>(entity).ok().map(|a| a.tilemap.clone())
            {
                if !tilemap.is_empty() {
                    if let Some(resolved) = self.resolve_entity_name(ns, &tilemap) {
                        if let Ok(mut a) = self.world.get::<&mut IsoAgent>(entity) {
                            a.tilemap = resolved;
                        }
                    }
                }
            }

            if let Some(target) = self.world.get::<&Animator>(entity).ok().map(|a| a.target.clone())
            {
                let parts: Vec<&str> = target.splitn(2, '.').collect();
                if let Some(entity_name) = parts.first() {
                    if !entity_name.is_empty() {
                        if let Some(resolved) = self.resolve_entity_name(ns, entity_name) {
                            let new_target = if parts.len() == 2 {
                                format!("{resolved}.{}", parts[1])
                            } else {
                                resolved
                            };
                            if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
                                a.target = new_target;
                            }
                        }
                    }
                }
            }

            if let Some((tilemap, wheels, tires)) = self
                .world
                .get::<&IsoVehicle>(entity)
                .ok()
                .map(|v| (v.tilemap.clone(), v.wheel_entities.clone(), v.tire_entities.clone()))
            {
                if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(entity) {
                    if !tilemap.is_empty() {
                        if let Some(resolved) = self.resolve_entity_name(ns, &tilemap) {
                            v.tilemap = resolved;
                        }
                    }
                    for w in wheels.iter().zip(v.wheel_entities.iter_mut()) {
                        if !w.0.is_empty() {
                            if let Some(resolved) = self.resolve_entity_name(ns, w.0) {
                                *w.1 = resolved;
                            }
                        }
                    }
                    for t in tires.iter().zip(v.tire_entities.iter_mut()) {
                        if !t.0.is_empty() {
                            if let Some(resolved) = self.resolve_entity_name(ns, t.0) {
                                *t.1 = resolved;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rewrite the resource references stored on a ROM's components into
    /// namespace-qualified keys, using the ROM's namespace as the referring
    /// namespace.  Covered: `Tilemap.tile_set`, `NavMesh.tile_set`,
    /// `IsoSprite.texture`, `IsoAgent.texture`, `SpriteRender.texture` (all
    /// textures), `SdfTextRender.atlas_name` (font), and `Animator.animation`.
    /// A bare reference resolves in the referring namespace first, then the
    /// global namespace (see [`Engine::resolve_resource`]); a dangling
    /// reference is left untouched (the draw path reports it as missing).
    pub(crate) fn rewrite_resource_refs(&mut self, ns: &str, keys: &[String]) {
        for key in keys {
            let Some(&entity) = self.names.get(key) else { continue };

            if let Some(tile_set) =
                self.world.get::<&Tilemap>(entity).ok().map(|t| t.tile_set.clone())
            {
                if !tile_set.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Texture, &tile_set)
                    {
                        if let Ok(mut t) = self.world.get::<&mut Tilemap>(entity) {
                            t.tile_set = resolved;
                        }
                    }
                }
            }

            if let Some(tile_set) =
                self.world.get::<&NavMesh>(entity).ok().map(|n| n.tile_set.clone())
            {
                if !tile_set.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Texture, &tile_set)
                    {
                        if let Ok(mut n) = self.world.get::<&mut NavMesh>(entity) {
                            n.tile_set = resolved;
                        }
                    }
                }
            }

            if let Some(texture) =
                self.world.get::<&IsoSprite>(entity).ok().map(|s| s.texture.clone())
            {
                if !texture.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Texture, &texture)
                    {
                        if let Ok(mut s) = self.world.get::<&mut IsoSprite>(entity) {
                            s.texture = resolved;
                        }
                    }
                }
            }

            if let Some(texture) =
                self.world.get::<&IsoAgent>(entity).ok().map(|a| a.texture.clone())
            {
                if !texture.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Texture, &texture)
                    {
                        if let Ok(mut a) = self.world.get::<&mut IsoAgent>(entity) {
                            a.texture = resolved;
                        }
                    }
                }
            }

            if let Some(texture) =
                self.world.get::<&SpriteRender>(entity).ok().map(|s| s.texture.clone())
            {
                if !texture.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Texture, &texture)
                    {
                        if let Ok(mut s) = self.world.get::<&mut SpriteRender>(entity) {
                            s.texture = resolved;
                        }
                    }
                }
            }

            if let Some(atlas_name) =
                self.world.get::<&SdfTextRender>(entity).ok().map(|s| s.atlas_name.clone())
            {
                if !atlas_name.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Font, &atlas_name)
                    {
                        if let Ok(mut s) = self.world.get::<&mut SdfTextRender>(entity) {
                            s.atlas_name = resolved;
                        }
                    }
                }
            }

            if let Some(anim_name) =
                self.world.get::<&Animator>(entity).ok().and_then(|a| a.animation.clone())
            {
                if !anim_name.is_empty() {
                    if let Some(resolved) =
                        self.resolve_resource(ns, ResourceKind::Animation, &anim_name)
                    {
                        if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
                            a.animation = Some(resolved);
                        }
                    }
                }
            }
        }
    }

    /// Hydrate the tile/nav/height grids referenced by the entity state from
    /// the ROM's grid resources (raw little-endian numbers keyed by name).
    fn load_grids(&mut self, resources: &classic_rom::ResourceSet) {
        let grids = resources.grids();

        if let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) {
            let (tiles_grid, heights_grid) = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(tm) => (tm.tiles_grid.clone(), tm.heights_grid.clone()),
                Err(_) => (None, None),
            };
            if let Some(name) = tiles_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_tiles_bulk(&decode_u32(bytes));
                }
            }
            if let Some(name) = heights_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_heights_bulk(&decode_f32(bytes));
                }
            }
        }

        if let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) {
            let data_grid = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(nav) => nav.data_grid.clone(),
                Err(_) => None,
            };
            if let Some(name) = data_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_nav_bulk(&decode_u32(bytes));
                }
            }
        }
    }

    /// Reconstruct a [`classic_rom::Rom`] from the loaded manifest + resources
    /// and the current world state.  Returns `None` if no ROM was loaded.
    pub fn dump_rom(&self) -> Option<classic_rom::Rom> {
        let mut resources = self.rom_resources.clone()?;
        self.refresh_grids(&mut resources);
        Some(classic_rom::Rom {
            manifest: self.rom_manifest.clone()?,
            manifest_json: self.rom_manifest_json.clone()?,
            resources,
            state: self.dump_state(),
        })
    }

    /// Refresh the tile/nav/height grid resources in `resources` from the
    /// current world state (the re-hydration `dump_rom`/`dump_roms` do before
    /// serializing).
    fn refresh_grids(&self, resources: &mut classic_rom::ResourceSet) {
        if let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) {
            if let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) {
                if let Some(name) = &tm.tiles_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_u32(&tm.data),
                    );
                }
                if let Some(name) = &tm.heights_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_f32(&tm.height_data),
                    );
                }
            }
        }
        if let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) {
            if let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) {
                if let Some(name) = &nav.data_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_u32(&nav.data),
                    );
                }
            }
        }
    }

    /// Reconstruct the full multi-ROM dependency DAG, mirroring
    /// [`Engine::dump_rom`]: the root ROM's resources are re-hydrated with the
    /// current tile/nav/height grids and its `state` refreshed from the current
    /// world; dependency ROMs keep their boot-time resources.  Returns `None`
    /// when no ROMs are loaded.
    pub fn dump_roms(&self) -> Option<classic_rom::LoadedRoms> {
        if self.loaded_roms.is_empty() {
            return None;
        }
        let mut order = self.loaded_roms.clone();
        if let Some(root) = order.last_mut() {
            self.refresh_grids(&mut root.rom.resources);
            root.rom.state = self.dump_state();
        }
        Some(classic_rom::LoadedRoms {
            root: self.loaded_roms.last().map(|e| e.name.clone()).unwrap_or_default(),
            order,
        })
    }

    /// Load the unified typed-channel animation blob (`animation.bin`),
    /// falling back to the legacy sparse-offset (`KAOS`) and dense formats.
    ///
    /// The current format (magic `b"KACH"`) carries named, typed, sparse
    /// keyframe channels — `offset` (sprite motion) plus `light.*` channels
    /// (position/color/intensity/radius/dir/cone) — all in the engine's final
    /// units by the exporter, so the animator interpolates them verbatim.  The
    /// `offset` channel is folded into `offset_keyframes` so the sprite
    /// animator's existing interpolation path keeps working unchanged.
    pub fn load_animation_channels(&mut self, animation_name: &str, bytes: &[u8]) {
        const MAGIC: &[u8; 4] = b"KACH";

        if bytes.len() < 4 || &bytes[0..4] != MAGIC {
            self.load_animation_offsets(animation_name, bytes);
            return;
        }

        let Some(animation) = self.animations.get_mut(animation_name) else {
            return;
        };

        let version = bytes[4];
        if version != 1 || bytes.len() < 13 {
            return;
        }
        let channel_count =
            u32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) as usize;
        let mut o = 13usize;
        let mut channels = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            if o + 1 > bytes.len() {
                break;
            }
            let name_len = bytes[o] as usize;
            o += 1;
            if o + name_len > bytes.len() {
                break;
            }
            let name = String::from_utf8_lossy(&bytes[o..o + name_len]).into_owned();
            o += name_len;
            if o + 1 > bytes.len() {
                break;
            }
            let component = bytes[o];
            o += 1;
            if o + 4 > bytes.len() {
                break;
            }
            let key_count =
                u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize;
            o += 4;
            let mut keys = Vec::with_capacity(key_count);
            for _ in 0..key_count {
                if o + 4 > bytes.len() {
                    break;
                }
                let frame =
                    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                o += 4;
                let floats = component as usize;
                if o + floats * 4 > bytes.len() {
                    break;
                }
                let mut v = Vec::with_capacity(floats);
                for _ in 0..floats {
                    v.push(f32::from_le_bytes([
                        bytes[o],
                        bytes[o + 1],
                        bytes[o + 2],
                        bytes[o + 3],
                    ]));
                    o += 4;
                }
                keys.push((frame, v));
            }
            channels.push(AnimChannel { name, component, keys });
        }
        animation.channels = channels;

        // Fold the `offset` channel into `offset_keyframes` so the sprite
        // animator's existing interpolation path is unchanged.
        if let Some(ch) = animation.channels.iter().find(|c| c.name == "offset") {
            animation.offset_keyframes = ch
                .keys
                .iter()
                .map(|(frame, v)| OffsetKeyframe {
                    frame: *frame,
                    offset: [
                        v.first().copied().unwrap_or(0.0),
                        v.get(1).copied().unwrap_or(0.0),
                        v.get(2).copied().unwrap_or(0.0),
                    ],
                })
                .collect();
        }
    }

    /// Resolve a bare or `ns::name` entity reference against the loaded ROMs'
    /// namespaces.  A qualified name resolves exactly; a bare name resolves in
    /// the referring namespace first, then the global (empty) namespace, then
    /// fails (`None`).  The single resolution point for the multi-ROM namespace
    /// model; entity lookups route through it as multi-ROM scenes land.
    pub fn resolve_entity_name(&self, referring_ns: &str, name: &str) -> Option<String> {
        if name.contains("::") {
            return self.names.contains_key(name).then(|| name.to_string());
        }
        if !referring_ns.is_empty() {
            let qualified = format!("{referring_ns}::{name}");
            if self.names.contains_key(&qualified) {
                return Some(qualified);
            }
        }
        self.names.contains_key(name).then(|| name.to_string())
    }

    /// The namespace prefix of a qualified `ns::name` key (the empty string for
    /// a bare name).  The inverse of [`Engine::entity_key_ns`]'s qualification.
    pub fn namespace_of(name: &str) -> String {
        name.split_once("::").map(|(ns, _)| ns.to_string()).unwrap_or_default()
    }

    /// Whether a (qualified) name is registered under the given resource kind.
    fn resource_exists(&self, kind: ResourceKind, name: &str) -> bool {
        match kind {
            ResourceKind::Texture => {
                self.texture_names.contains(name)
                    || self.gfx.as_ref().map(|g| g.textures.contains_key(name)).unwrap_or(false)
            }
            ResourceKind::Font => self.sdf_fonts.contains_key(name),
            ResourceKind::Animation => self.animations.contains_key(name),
            ResourceKind::FrameTable => self.frame_tables.contains_key(name),
            ResourceKind::Vehicle => self.vehicles.contains_key(name),
        }
    }

    /// Resolve a bare or `ns::name` resource reference against the loaded ROMs'
    /// namespaces, mirroring [`Engine::resolve_entity_name`].  A qualified name
    /// resolves exactly; a bare name resolves in the referring namespace first,
    /// then the global (empty) namespace, then fails (`None`).
    pub fn resolve_resource(
        &self,
        referring_ns: &str,
        kind: ResourceKind,
        name: &str,
    ) -> Option<String> {
        if name.contains("::") {
            return self.resource_exists(kind, name).then(|| name.to_string());
        }
        if !referring_ns.is_empty() {
            let qualified = format!("{referring_ns}::{name}");
            if self.resource_exists(kind, &qualified) {
                return Some(qualified);
            }
        }
        self.resource_exists(kind, name).then(|| name.to_string())
    }

    /// Load per-frame visual offsets emitted by the animation renderer.
    ///
    /// Two encodings are accepted, distinguished by a 4-byte magic prefix:
    ///
    /// - **Sparse keyframes** (current): `b"KAOS"`, `u8` version (= 1), `u32`
    ///   keyframe_count, `f32` `pixels_per_meter`, then keyframe_count ×
    ///   `(u32 frame_idx, f32 x, f32 y, f32 z)` `rig_location` triplets.  The
    ///   animator linearly interpolates between keyframes.
    /// - **Legacy dense**: `u32` frame_count, `f32` `pixels_per_meter`, then
    ///   frame_count × `[f32 x, f32 y, f32 z]` triplets (one per frame).
    ///
    /// `rig_location` is Blender world `(x = drift, y = drift, z = altitude)`
    /// in metres.  It is stored verbatim as `(drift_x_m, drift_y_m, altitude_m)`
    /// — the `pixels_per_meter` field is now ignored (the sprite model consumes
    /// world metres directly, see `compute_iso_sprite_model`).
    pub fn load_animation_offsets(&mut self, animation_name: &str, bytes: &[u8]) {
        const MAGIC: &[u8; 4] = b"KAOS";

        let Some(animation) = self.animations.get_mut(animation_name) else {
            return;
        };

        if bytes.len() >= 13 && &bytes[0..4] == MAGIC {
            let version = bytes[4];
            if version != 1 {
                return;
            }
            let count = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
            let mut keyframes = Vec::with_capacity(count);
            let mut o = 13;
            for _ in 0..count {
                if o + 16 > bytes.len() {
                    break;
                }
                let frame =
                    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                let x =
                    f32::from_le_bytes([bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7]]);
                let y =
                    f32::from_le_bytes([bytes[o + 8], bytes[o + 9], bytes[o + 10], bytes[o + 11]]);
                let z = f32::from_le_bytes([
                    bytes[o + 12],
                    bytes[o + 13],
                    bytes[o + 14],
                    bytes[o + 15],
                ]);
                o += 16;
                keyframes.push(OffsetKeyframe { frame, offset: Vec3::new(x, y, z).to_array() });
            }
            animation.offset_keyframes = keyframes;
            return;
        }

        if bytes.len() < 8 {
            return;
        }
        let frame_count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;

        let mut offsets = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let o = 8 + i * 12;
            if o + 12 > bytes.len() {
                break;
            }
            let x = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            let y = f32::from_le_bytes([bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7]]);
            let z = f32::from_le_bytes([bytes[o + 8], bytes[o + 9], bytes[o + 10], bytes[o + 11]]);
            offsets.push(Vec3::new(x, y, z).to_array());
        }
        animation.offsets = offsets;
    }

    /// Upload a PNG texture from raw bytes, returning the decoded `(w, h)`.
    pub fn load_texture_png(&mut self, name: &str, png_bytes: &[u8]) -> (u32, u32) {
        let decoded = boot::decode_texture(boot::TextureFormat::Rgba8, png_bytes);
        let dims = decoded.dims();
        self.upload_decoded(name, &decoded);
        dims
    }

    /// Upload a grayscale PNG as an R8 texture (depth maps, SDF atlases),
    /// returning the decoded `(w, h)`.
    pub fn load_texture_luma8(&mut self, name: &str, png_bytes: &[u8]) -> (u32, u32) {
        let decoded = boot::decode_texture(boot::TextureFormat::Luma8, png_bytes);
        let dims = decoded.dims();
        self.upload_decoded(name, &decoded);
        dims
    }

    /// Upload an RGB PNG as an RGB8 texture (world-space normal maps),
    /// returning the decoded `(w, h)`.
    pub fn load_texture_rgb8(&mut self, name: &str, png_bytes: &[u8]) -> (u32, u32) {
        let decoded = boot::decode_texture(boot::TextureFormat::Rgb8, png_bytes);
        let dims = decoded.dims();
        self.upload_decoded(name, &decoded);
        dims
    }

    /// Upload a decoded texture to the GL layer (a no-op when `gfx == None`).
    pub fn upload_decoded(&mut self, name: &str, texture: &boot::DecodedTexture) {
        if let Some(gfx) = self.gfx.as_mut() {
            match texture {
                boot::DecodedTexture::Rgba8 { width, height, pixels } => {
                    gfx.add_texture_rgba8(name, pixels, *width, *height);
                }
                boot::DecodedTexture::Luma8 { width, height, pixels } => {
                    gfx.add_texture_r8(name, pixels, *width, *height);
                }
                boot::DecodedTexture::Rgb8 { width, height, pixels } => {
                    gfx.add_texture_rgb8(name, pixels, *width, *height);
                }
            }
        }
    }

    /// Upload a Basis Universal `.basis` payload as a GPU-compressed texture
    /// (transcoding to the `format`-declared target, or raw RGBA8 as fallback).
    /// A payload that cannot be transcoded is dropped (logged), leaving the
    /// texture unuploaded.
    pub fn load_texture_basis(&mut self, name: &str, basis_bytes: &[u8], format: &str) {
        if let Some(gfx) = self.gfx.as_mut() {
            if !gfx.add_texture_basis(name, basis_bytes, format) {
                log::warn!("texture {name}: basis transcode unavailable (format {format})");
            }
        }
    }

    /// Web-only async counterpart to [`Engine::load_texture_basis`]: transcode
    /// off the main thread (worker) and upload, awaited by the caller.  Returns
    /// the decoded texture dimensions so the caller can emit a per-sheet
    /// [`classic_rom::BootEvent::ResourceDecoded`] (the worker→main `decoded`
    /// message) at the moment the transcode finishes.
    #[cfg(target_arch = "wasm32")]
    pub async fn load_texture_basis_async(
        &mut self,
        name: &str,
        basis_bytes: &[u8],
        format: &str,
    ) -> Option<(u32, u32)> {
        if let Some(gfx) = self.gfx.as_mut() {
            let dims = gfx.add_texture_basis_async(name, basis_bytes, format).await;
            if dims.is_none() {
                log::warn!("texture {name}: basis transcode unavailable (format {format})");
            }
            return dims;
        }
        None
    }

    /// Load an SDF font from its metrics JSON and atlas PNG, keyed by the
    /// (namespace-qualified) font name; the atlas texture is uploaded under
    /// `"{font_name}-sdf"` with LINEAR filtering.
    pub fn load_sdf_font(&mut self, font_name: &str, metrics_json: &str, atlas_png: &[u8]) {
        let atlas_name = format!("{font_name}-sdf");
        let metrics: SdfFontMetrics =
            serde_json::from_str(metrics_json).expect("parse SDF font metrics JSON");
        self.sdf_fonts.insert(font_name.to_string(), metrics);

        let img = image::load_from_memory(atlas_png).expect("decode SDF atlas PNG");
        let luma = img.to_luma8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_r8(&atlas_name, &luma, luma.width(), luma.height());
            if let Some(tex) = gfx.textures.get(&atlas_name) {
                tex.set_linear(&gfx.gl);
            }
        }
    }

    /// Shared tail of the `commit_terrain` path: build the mesh and tile-data
    /// texture, upload both, write the data back onto the component, and
    /// register the mouse-to-iso parallax solve.
    ///
    /// The parallax closure must be registered exactly once per tilemap, which
    /// is the main reason this is factored out rather than duplicated.
    pub(crate) fn finish_tilemap_init(
        &mut self,
        entity: hecs::Entity,
        tiles: Vec<u32>,
        heights: Vec<f32>,
        height_scale: Option<f32>,
    ) {
        let (tile_set_name, size_x, size_y, tile_pixel_size) = {
            let tm = self.world.get::<&Tilemap>(entity).expect("Tilemap component");
            (tm.tile_set.clone(), tm.size_x, tm.size_y, tm.tile_pixel_size)
        };

        let height_scale = height_scale.unwrap_or(tile_pixel_size[0] as f32);
        self.base_height_scale = height_scale;
        let (mesh_data, vcount) = build_mesh(size_x, size_y, &tiles, &heights);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &tiles);

        let gfx = self.gfx.as_mut().expect("gfx not initialized");

        // Upload mesh.
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::STATIC_DRAW);

        // Upload tile data texture.
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        // Store tile data on the component.
        if let Ok(mut tm) = self.world.get::<&mut Tilemap>(entity) {
            tm.data = tiles;
            tm.height_data = heights;
            tm.height_scale = height_scale;
            let img_h = gfx.textures.get(&tile_set_name).map(|t| t.size.1).unwrap_or(0);
            let img_w = gfx.textures.get(&tile_set_name).map(|t| t.size.0).unwrap_or(0);
            if img_w > 0 {
                let px = tile_pixel_size[0].max(1);
                tm.tile_set_pixel_size = [img_w, img_h];
                tm.tiles_per_row = img_w / px;
            }
        }

        let name = self.debug_name(entity);
        self.tilemap_gpu.insert(name, TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Register updateMousePos: convert screen coords → iso tile coords by
        // casting the cursor through the world-metre ground plane and
        // intersecting the ray with the terrain height field.
        let tm_entity = entity;
        self.on_update(move |engine| {
            let (height_data, size_x, size_y, tilemap_pos) = {
                let tm = engine.world.get::<&Tilemap>(tm_entity).unwrap();
                let tf = engine.world.get::<&Transform>(tm_entity).unwrap();
                (tm.height_data.clone(), tm.size_x, tm.size_y, tf.position)
            };

            // Un-project the mouse to the world-metre ground plane (`z = 0`)
            // through the orthographic camera: undo pan/zoom, then intersect the
            // camera view ray with the ground plane.
            let mp = engine.input.mouse_pos;
            let mut screen = Vec3::new(mp.x, mp.y, 0.0);
            screen += engine.camera.fix();
            screen /= engine.camera.scale;
            let (right, up, back) = iso_basis();
            let view_x = screen.x / PPM_TARGET;
            let view_y = -screen.y / PPM_TARGET;
            // world.z = up.z·view_y + back.z·view_z == 0  =>  view_z = -up.z·view_y/back.z.
            let view_z = -up.z * view_y / back.z;
            let ground = right * view_x + up * view_y + back * view_z;

            // Cast the orthographic camera ray from the near depth plane into
            // the scene and intersect it with the terrain height field.  The
            // first surface the ray hits is the one the camera actually sees —
            // a slope in front occludes terrain behind it.  Colliders can later
            // stop the same ray.  The height field is authored in the tilemap's
            // local frame, so the ray is shifted by the tilemap's world offset.
            let ray = iso_camera_ray(screen.truncate());
            let local_ray = Ray::new(ray.origin - tilemap_pos, ray.dir);
            let hit =
                raycast_terrain(local_ray, &height_data, size_x, size_y, DEPTH_NEAR - DEPTH_FAR)
                    .unwrap_or(ground - tilemap_pos);

            // `mouse_iso_pos` becomes a full (x, y, z) where (x, y) are tile
            // coordinates and `z` the terrain height (metres) under the cursor.
            let tx = hit.x / TILE_M;
            let ty = -hit.y / TILE_M;
            let z = hit.z;

            if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(tm_entity) {
                tm.mouse_iso_pos = Vec3::new(tx, ty, z);
            }
        });
    }
}

/// Decode a little-endian `u32` grid byte blob.
fn decode_u32(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// Decode a little-endian `f32` grid byte blob.
fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// Encode a `u32` grid to little-endian bytes.
fn encode_u32(vals: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Encode an `f32` grid to little-endian bytes.
fn encode_f32(vals: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
