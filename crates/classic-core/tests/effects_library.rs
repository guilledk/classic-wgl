//! The reference effect library (`fixtures/effects.json`): one effect per
//! look the particle system ships (dust, smoke, the rocket blast and plume,
//! sparks, an explosion, a flipbook, arcs, lightning and a point-spawned
//! ember).  It pins the data model end to end: every layer kind, spawn
//! shape, ground mode, render mode, particle shape, blend and ribbon mode
//! must keep parsing from the same JSON a ROM's `data[]` effect library
//! carries.

use std::collections::BTreeSet;

use classic_core::effects::{
    Blend, EffectLibrary, GroundMode, LayerDef, ParticleShape, RenderMode, RibbonMode, SpawnShape,
};

const LIBRARY: &str = include_str!("fixtures/effects.json");

fn library() -> EffectLibrary {
    EffectLibrary::from_json(LIBRARY.as_bytes()).expect("reference library parses")
}

#[test]
fn reference_library_parses_and_resolves() {
    assert!(EffectLibrary::sniff(LIBRARY.as_bytes()));
    let lib = library();
    let names: BTreeSet<&str> = lib.effects.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), lib.effects.len(), "duplicate effect names");
    for fx in [
        "dust_puff",
        "rocket_blast",
        "nozzle_plume",
        "sparks",
        "explosion",
        "smoke_column",
        "smoke_flipbook",
        "arc",
        "lightning",
    ] {
        assert!(names.contains(fx), "missing effect {fx}");
    }
    // Sub-effects resolve inside the library.
    for fx in &lib.effects {
        for l in &fx.layers {
            if let LayerDef::Particles(p) = l {
                if let Some(sub) = &p.ground.sub_effect {
                    assert!(names.contains(sub.as_str()), "{}: dangling {sub}", fx.name);
                }
            }
        }
    }
}

#[test]
fn reference_library_covers_every_variant() {
    let lib = library();
    let (mut kinds, mut spawn, mut ground) = (BTreeSet::new(), Vec::new(), Vec::new());
    let (mut mode, mut shape, mut blend, mut ribbon) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut flipbook, mut swirl, mut lit, mut emissive) = (false, false, false, false);
    for fx in &lib.effects {
        for l in &fx.layers {
            match l {
                LayerDef::Particles(p) => {
                    kinds.insert("particles");
                    spawn.push(p.spawn.shape);
                    ground.push(p.ground.mode);
                    mode.push(p.render.mode);
                    shape.push(p.render.shape);
                    blend.push(p.render.blend);
                    flipbook |= p.render.flipbook.is_some() && p.render.texture.is_some();
                    swirl |= p.motion.swirl.is_some();
                    lit |= p.render.lit;
                    emissive |= p.render.emissive > 0.0;
                }
                LayerDef::Ribbon(r) => {
                    kinds.insert("ribbon");
                    ribbon.push(r.mode);
                }
                LayerDef::Light(_) => {
                    kinds.insert("light");
                }
            }
        }
    }
    assert_eq!(kinds, BTreeSet::from(["light", "particles", "ribbon"]));
    for s in [SpawnShape::Point, SpawnShape::Sphere, SpawnShape::Disc, SpawnShape::Ring] {
        assert!(spawn.contains(&s), "no {s:?} spawn shape");
    }
    for g in [GroundMode::None, GroundMode::Clamp, GroundMode::Bounce, GroundMode::Die] {
        assert!(ground.contains(&g), "no {g:?} ground mode");
    }
    for m in [RenderMode::Billboard, RenderMode::Streak] {
        assert!(mode.contains(&m), "no {m:?} render mode");
    }
    for s in [ParticleShape::Soft, ParticleShape::Smoke, ParticleShape::Glow] {
        assert!(shape.contains(&s), "no {s:?} particle shape");
    }
    for b in [Blend::Alpha, Blend::Additive] {
        assert!(blend.contains(&b), "no {b:?} blend");
    }
    for r in [RibbonMode::Arc, RibbonMode::Lightning] {
        assert!(ribbon.contains(&r), "no {r:?} ribbon");
    }
    assert!(flipbook && swirl && lit && emissive, "{flipbook} {swirl} {lit} {emissive}");
}
