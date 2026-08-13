//! Lunar scene: installs a procedurally generated lunar surface onto the
//! demo's reused entity names, plus a dev widget to re-roll it at runtime.
//!
//! `classic-core::terrain` does all the generation; this module installs the
//! result on the ECS entities and GPU, and owns the runtime state needed to
//! regenerate it.  The scene reuses the demo entity names (`tilemap`,
//! `tilemapNavigation`, `navAgent`, ...) so the whole editor toolchain works
//! on a generated map with no further changes.

use classic_core::components::{IsoAgent, NavMesh, Tilemap, Transform};
use classic_core::instrument::Chan;
use classic_core::terrain::lunar::{generate_lunar, LunarParams, LunarTerrain};
use classic_core::terrain::tileset::{build_lunar_tileset, DEFAULT_COLS, DEFAULT_ROWS};
use classic_core::{cl_info, cl_warn};
use classic_engine::Engine;

use crate::state::DemoStateRef;

/// Runtime state for the lunar scene, kept so the dev widget can re-roll the
/// map without re-running the whole bootstrap.
#[derive(Clone, Debug)]
pub struct LunarScene {
    pub params: LunarParams,
    pub terrain: LunarTerrain,
    /// Vertical exaggeration applied to the height field, overriding the
    /// tilemap default of `tile_pixel_size[0]`.
    pub height_scale: f32,
}

/// Monotonic milliseconds, or `None` where no clock is available.
///
/// `std::time::Instant::now()` panics on `wasm32-unknown-unknown`, and this
/// module runs during startup on the web target, so timing has to be optional
/// rather than merely inaccurate.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> Option<f64> {
    use std::sync::LazyLock;
    static START: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    Some(START.elapsed().as_secs_f64() * 1000.0)
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> Option<f64> {
    None
}

/// Human-readable elapsed time since `start`, for log lines.
fn since(start: Option<f64>) -> String {
    match (start, now_ms()) {
        (Some(a), Some(b)) => format!("{:.1} ms", b - a),
        _ => String::from("n/a"),
    }
}

/// Vertical scale for generated terrain.
///
/// The flat demo map uses `tile_pixel_size[0]` (32) with heights of exactly
/// 1.0.  Generated terrain spans roughly 7 height units, so the same scale
/// would give ~225px of relief against 45px tiles — dramatic, but it also
/// stretches the 3-pass mouse-picking parallax solve to ~7 tiles of
/// correction, where it converges poorly.  14 keeps both in a sane range.
pub const LUNAR_HEIGHT_SCALE: f32 = 14.0;

/// Generate a lunar surface and install it on the `tilemap` /
/// `tilemapNavigation` entities, including a procedurally painted tileset.
///
/// Must be called after `load_state`, and before `init_navigation` /
/// `init_nav_mesh_render` (which read the data this installs).
pub fn init_lunar_terrain(engine: &mut Engine, state: &DemoStateRef, params: LunarParams) {
    let t0 = now_ms();
    let terrain = generate_lunar(&params);
    let gen_ms = since(t0);

    cl_info!(
        Chan::Terrain,
        "lunar '{}' {}x{}: {} craters, relief {:.2}, {:.0}% walkable, {:.0}% buildable, \
         {} corridors, {} relax iters ({})",
        params.seed,
        terrain.size_x,
        terrain.size_y,
        terrain.stats.craters,
        terrain.stats.max_height - terrain.stats.min_height,
        terrain.stats.walkable_fraction * 100.0,
        terrain.stats.buildable_fraction * 100.0,
        terrain.stats.corridors_carved,
        terrain.stats.relax_iterations_used,
        gen_ms
    );

    let (tileset, tw, th) =
        build_lunar_tileset(&format!("{}:tileset", params.seed), 32, DEFAULT_COLS, DEFAULT_ROWS);

    engine.init_tilemap_generated(&terrain, &tileset, tw, th, Some(LUNAR_HEIGHT_SCALE));

    // Match the editor's walkability rule to the one the generator used,
    // so a height edit does not reclassify the whole map on the next
    // `sync_nav_heights`.
    engine.nav_slope_threshold = params.nav_max_slope;

    place_agent_at_spawn(engine, &terrain);

    state.borrow_mut().lunar =
        Some(LunarScene { params, terrain, height_scale: LUNAR_HEIGHT_SCALE });
}

/// Re-roll the terrain in place and re-upload every derived GPU resource.
///
/// Cheap enough (well under 200 ms for 200x200) to drive from a button.
pub fn regenerate_lunar_terrain(engine: &mut Engine, state: &DemoStateRef, params: LunarParams) {
    let t0 = now_ms();
    let terrain = generate_lunar(&params);

    let Some(tm_entity) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else {
        cl_warn!(Chan::Terrain, "regenerate: no Tilemap-role entity");
        return;
    };

    if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(tm_entity) {
        tm.data = terrain.tiles.clone();
        tm.height_data = terrain.heights.clone();
        tm.height_scale = LUNAR_HEIGHT_SCALE;
    }
    engine.rebuild_tilemap_mesh();

    if let Some(nav_entity) = engine.entity_by_role(classic_core::RoleKind::NavMesh) {
        if let Ok(mut nav) = engine.world.get::<&mut NavMesh>(nav_entity) {
            nav.data = terrain.nav.clone();
        }
    }
    engine.rebuild_nav_gpu();

    engine.nav_slope_threshold = params.nav_max_slope;
    place_agent_at_spawn(engine, &terrain);

    cl_info!(
        Chan::Terrain,
        "lunar regenerated seed '{}': {} craters, {:.0}% walkable ({})",
        params.seed,
        terrain.stats.craters,
        terrain.stats.walkable_fraction * 100.0,
        since(t0)
    );

    state.borrow_mut().lunar =
        Some(LunarScene { params, terrain, height_scale: LUNAR_HEIGHT_SCALE });
}

/// Drop the nav agent onto the first spawn point and clear any path it was
/// following (which would refer to terrain that no longer exists).
fn place_agent_at_spawn(engine: &mut Engine, terrain: &LunarTerrain) {
    let Some(agent) = engine.entity_by_role(classic_core::RoleKind::Agent) else { return };
    let Some(&(sx, sy)) = terrain.spawn_points.first() else { return };

    let h = terrain.height_at(sx, sy) * LUNAR_HEIGHT_SCALE;
    let pos = glam::Vec3::new(sx as f32 + 0.5, sy as f32 + 0.5, h);

    if let Ok(mut tf) = engine.world.get::<&mut Transform>(agent) {
        tf.position = pos;
    }
    if let Ok(mut a) = engine.world.get::<&mut IsoAgent>(agent) {
        a.position = pos;
        a.path.clear();
        a.target_index = 0;
        a.delta = 0.0;
        a.state = classic_core::components::AgentState::Idle;
    }
}

/// Dev widget: re-roll the map, and nudge the two parameters that most
/// change its character.
///
/// Generation is well under a fifth of a second for 200x200, so tuning by
/// button is genuinely interactive — which is the fastest way to find
/// values that both look right and play right.
pub fn init_lunar_widget(engine: &mut Engine, state: &DemoStateRef) {
    use classic_core::components::{SdfTextRender, TextJustify, UiAnchor};
    use std::cell::Cell;
    use std::rc::Rc;

    // Only meaningful once a lunar scene exists.
    if state.borrow().lunar.is_none() {
        return;
    }

    let init_density =
        state.borrow().lunar.as_ref().map(|s| s.params.crater_density).unwrap_or(14.0);
    let init_mare = state.borrow().lunar.as_ref().map(|s| s.params.mare_threshold).unwrap_or(-0.02);

    let btn: f32 = 28.0;
    let label_w: f32 = 132.0;
    let gap: f32 = 4.0;
    let widget_w: f32 = gap * 4.0 + btn * 2.0 + label_w;
    let widget_h: f32 = btn * 4.0 + gap * 5.0;

    // Shared with the update closure; `perform_calls` runs before the
    // update loop, so a click is visible in the same frame.
    let seed_n = Rc::new(Cell::new(0u32));
    let density = Rc::new(Cell::new(init_density));
    let mare = Rc::new(Cell::new(init_mare));
    let dirty = Rc::new(Cell::new(false));

    let mut mk_pair = |label: &str,
                       dec: Box<dyn FnMut() -> bool>,
                       inc: Box<dyn FnMut() -> bool>|
     -> (hecs::Entity, hecs::Entity, hecs::Entity) {
        let ui = engine.ui.as_mut().expect("ui");
        let minus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn,
            btn,
            [0.6, 0.1, 0.1, 1.0],
            classic_engine::ui::ButtonOptions {
                text: Some("-".into()),
                text_scale: 0.5,
                sdf_text: true,
                hover: true,
                click_action: Some(dec),
                ..Default::default()
            },
        );
        let lbl = ui.spawn_sdf_text(
            &mut engine.world,
            label,
            0.4,
            220.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let plus = ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            btn,
            btn,
            [0.1, 0.6, 0.1, 1.0],
            classic_engine::ui::ButtonOptions {
                text: Some("+".into()),
                text_scale: 0.5,
                sdf_text: true,
                hover: true,
                click_action: Some(inc),
                ..Default::default()
            },
        );
        (minus, lbl, plus)
    };

    let (d_minus, d_label, d_plus) = {
        let (a, b) = (density.clone(), density.clone());
        let (da, db) = (dirty.clone(), dirty.clone());
        mk_pair(
            "craters",
            Box::new(move || {
                a.set((a.get() - 2.0).max(0.0));
                da.set(true);
                true
            }),
            Box::new(move || {
                b.set((b.get() + 2.0).min(60.0));
                db.set(true);
                true
            }),
        )
    };

    let (m_minus, m_label, m_plus) = {
        let (a, b) = (mare.clone(), mare.clone());
        let (da, db) = (dirty.clone(), dirty.clone());
        mk_pair(
            "mare",
            Box::new(move || {
                a.set((a.get() - 0.06).max(-0.9));
                da.set(true);
                true
            }),
            Box::new(move || {
                b.set((b.get() + 0.06).min(0.9));
                db.set(true);
                true
            }),
        )
    };

    let container = {
        let ui = engine.ui.as_mut().expect("ui");
        ui.spawn_container(&mut engine.world, widget_w, widget_h, [0.0, 0.0, 0.0, 0.45])
    };

    let seed_label = {
        let ui = engine.ui.as_mut().expect("ui");
        ui.spawn_sdf_text(
            &mut engine.world,
            "seed",
            0.34,
            220.0,
            [0.8, 0.85, 1.0, 1.0],
            TextJustify::Center,
        )
    };

    let regen = {
        let s = seed_n.clone();
        let d = dirty.clone();
        let ui = engine.ui.as_mut().expect("ui");
        ui.spawn_button(
            &mut engine.world,
            &mut engine.physics,
            widget_w - gap * 2.0,
            btn,
            [0.2, 0.3, 0.5, 1.0],
            classic_engine::ui::ButtonOptions {
                text: Some("Regenerate".into()),
                text_scale: 0.36,
                sdf_text: true,
                hover: true,
                click_action: Some(Box::new(move || {
                    s.set(s.get() + 1);
                    d.set(true);
                    true
                })),
                ..Default::default()
            },
        )
    };

    {
        let ui = engine.ui.as_mut().expect("ui");
        ui.add_children(
            &mut engine.world,
            container,
            &[d_minus, d_label, d_plus, m_minus, m_label, m_plus, seed_label, regen],
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
    }

    state.borrow_mut().lunar_widget_e = Some(container);
    engine.set_enabled(container, false);

    let state = Rc::clone(state);
    engine.on_update(move |engine| {
        // Bottom-right, the same slot the height and light widgets use —
        // only one tool panel is visible at a time.
        let Some(ref ui) = engine.ui else { return };
        let x0 = ui.viewport_w - widget_w;
        let y0 = ui.viewport_h - widget_h;
        let rows = [gap, btn + gap * 2.0, btn * 2.0 + gap * 3.0, btn * 3.0 + gap * 4.0];

        let place = |engine: &mut Engine, e: hecs::Entity, x: f32, y: f32| {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                tf.position = glam::Vec3::new(x, y, tf.position.z);
            }
            classic_engine::ui::UIManager::position_children_of(e, &mut engine.world);
        };

        place(engine, container, x0, y0);
        let lx = x0 + gap + btn + gap;
        place(engine, d_minus, x0 + gap, y0 + rows[0]);
        place(engine, d_label, lx, y0 + rows[0]);
        place(engine, d_plus, lx + label_w, y0 + rows[0]);
        place(engine, m_minus, x0 + gap, y0 + rows[1]);
        place(engine, m_label, lx, y0 + rows[1]);
        place(engine, m_plus, lx + label_w, y0 + rows[1]);
        place(engine, seed_label, lx, y0 + rows[2]);
        place(engine, regen, x0 + gap, y0 + rows[3]);

        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(d_label) {
            sdf.text = format!("craters {:.0}", density.get());
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(m_label) {
            sdf.text = format!("mare {:.2}", mare.get());
        }
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(seed_label) {
            let walk = state
                .borrow()
                .lunar
                .as_ref()
                .map(|s| s.terrain.stats.walkable_fraction * 100.0)
                .unwrap_or(0.0);
            sdf.text = format!("seed #{}  {walk:.0}% open", seed_n.get());
        }

        if !dirty.replace(false) {
            return;
        }
        let Some(base) = state.borrow().lunar.as_ref().map(|s| s.params.clone()) else { return };
        let root = base.seed.split('#').next().unwrap_or("apollo").to_string();
        let params = LunarParams {
            seed: format!("{root}#{}", seed_n.get()),
            crater_density: density.get(),
            mare_threshold: mare.get(),
            ..base
        };
        regenerate_lunar_terrain(engine, &state, params);
    });
}

/// Centre the camera on the first landing zone.
pub fn focus_camera_on_spawn(engine: &mut Engine, state: &DemoStateRef) {
    let Some(scene) = state.borrow().lunar.clone() else { return };
    let Some(&(sx, sy)) = scene.terrain.spawn_points.first() else { return };
    let Some(tm_entity) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return };
    let scale = engine
        .world
        .get::<&Tilemap>(tm_entity)
        .map(|tm| tm.scale)
        .unwrap_or(glam::Vec3::new(45.0, 45.0, 1.0));

    let iso = glam::Mat4::from_scale(scale) * classic_core::math::cartesian_to_iso_4().inverse();
    let origin = iso.transform_point3(glam::Vec3::new(sx as f32, sy as f32, 0.0));
    engine.camera.position.x = origin.x;
    engine.camera.position.y = origin.y;
}

/// Install the generated navigation grid and wire click-to-move.
///
/// The generator already derived walkability from real terrain slope and
/// guaranteed every spawn is mutually reachable; re-deriving it from the
/// coarse height rule here would undo that.
pub fn hydrate_nav(engine: &mut Engine, state: &DemoStateRef) {
    let nav = state.borrow().lunar.as_ref().map(|s| s.terrain.nav.clone()).unwrap_or_default();
    engine.init_navigation_data(nav);
}

/// Zoom out for the generated terrain, centre on the first spawn, and apply
/// the airless lunar light preset.
pub fn setup_view(engine: &mut Engine, state: &DemoStateRef) {
    // Zoom out: at scale 1.0 a 45px tile fills the view with ~28 tiles, which
    // shows none of the terrain the generator produces.
    engine.camera.scale = glam::Vec3::new(0.32, 0.32, 1.0);
    focus_camera_on_spawn(engine, state);
    // Airless lighting: near-zero ambient and a hard low sun, which is what
    // makes the crater relief legible.
    crate::lighting::apply_light_preset(engine, state, "lunar");
    // The editor grid overlay fights the natural surface.
    engine.show_grid = false;
}
