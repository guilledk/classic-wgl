//! The inspector's right panel: the selected ROM, asset or entity.
//!
//! An entity shows its transforms (iso tile + terrain height, world metres,
//! screen pixels), the assets and entities it references (clickable), and
//! its registry-dumped components as compact JSON.  The component block is
//! cached per selection (a `Tilemap` dump carries the whole map); the rest is
//! cheap and rebuilt every few frames so the screen position tracks the
//! camera.

use classic_core::components::{
    Animator, IsoAgent, IsoSprite, IsoVehicle, Model, NavMesh, SdfTextRender, Tilemap, UiNode,
};
use classic_core::{SpriteRender, Transform};
use classic_engine::{Engine, ResourceKind};
use serde_json::Value;

use super::tree::{self, AssetKind, DIM, GROUP, LINK, WHITE};
use super::{AssetRef, Row, RowAction, Selection};

/// Component lines kept before the dump is cut short.
const MAX_COMPONENT_LINES: usize = 600;
/// Arrays longer than this are elided to a count.
const MAX_INLINE_ITEMS: usize = 12;
/// Inline JSON is used when it fits in this many characters.
const INLINE_WIDTH: usize = 56;

fn info(text: impl Into<String>) -> Row {
    Row { text: text.into(), color: WHITE, action: None }
}

fn heading(text: impl Into<String>) -> Row {
    Row { text: text.into(), color: GROUP, action: None }
}

fn dim(text: impl Into<String>) -> Row {
    Row { text: text.into(), color: DIM, action: None }
}

/// `1536` → `1.5 KiB`.
pub fn human_bytes(n: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// A short title for the selection (the detail panel's header).
pub fn title(engine: &Engine, sel: Option<&Selection>) -> String {
    match sel {
        None => "Nothing selected: click a row or the world".into(),
        Some(Selection::Rom(i)) => {
            format!("ROM {}", engine.loaded_roms.get(*i).map_or("?", |r| r.name.as_str()))
        }
        Some(Selection::Asset(a)) => format!("{} {}", a.kind.label(), a.name),
        Some(Selection::Entity(name)) => format!("Entity {name}"),
    }
}

/// Whether the selection gets a sprite preview box (filled by the sprite
/// preview layer): an entity drawn from a sprite sheet.
pub fn has_preview(engine: &Engine, sel: Option<&Selection>) -> bool {
    let Some(Selection::Entity(name)) = sel else { return false };
    let Some(&e) = engine.names.get(name) else { return false };
    engine.world.get::<&IsoSprite>(e).is_ok()
        || engine.world.get::<&IsoAgent>(e).is_ok()
        || engine.world.get::<&IsoVehicle>(e).is_ok()
        || engine.world.get::<&SpriteRender>(e).is_ok()
}

/// The detail rows above the component dump.
pub fn build(engine: &Engine, sel: &Selection, entity_count: usize) -> Vec<Row> {
    match sel {
        Selection::Rom(i) => rom_rows(engine, *i, entity_count),
        Selection::Asset(a) => asset_rows(engine, a),
        Selection::Entity(name) => entity_rows(engine, name),
    }
}

fn rom_rows(engine: &Engine, i: usize, entity_count: usize) -> Vec<Row> {
    let Some(entry) = engine.loaded_roms.get(i) else { return vec![dim("(ROM gone)")] };
    let m = &entry.rom.manifest;
    let res = &entry.rom.resources;
    let mut rows = vec![
        info(format!("name         {}", entry.name)),
        info(format!(
            "namespace    {}{}",
            display_ns(&engine.rom_namespace(&entry.name)),
            if m.namespace.is_empty() { "  (derived)".to_string() } else { String::new() }
        )),
        info(format!("entrypoint   {}", m.entrypoint)),
        info(format!(
            "deps         {}",
            if m.deps.is_empty() { "(none)".to_string() } else { m.deps.join(", ") }
        )),
        info(format!(
            "format       v{}{}",
            m.format_version,
            m.version.as_deref().map(|v| format!("   version {v}")).unwrap_or_default()
        )),
        info(format!("trusted      {}   host_features {}", m.trusted, m.host_features)),
        info(format!("state        {}   entities {entity_count}", m.state)),
        info(format!(
            "sha256       {}",
            entry.sha256.as_deref().map_or("(unknown)", |s| &s[..s.len().min(16)])
        )),
        heading("Resources"),
    ];
    let sum = |map: &std::collections::BTreeMap<String, std::sync::Arc<[u8]>>| {
        map.values().map(|v| v.len()).sum::<usize>()
    };
    for kind in AssetKind::ALL {
        let n = tree::rom_assets(&entry.rom, kind).len();
        if n == 0 {
            continue;
        }
        let bytes: usize = tree::rom_assets(&entry.rom, kind)
            .iter()
            .filter_map(|a| res.get(kind.resource_kind(), &a.name).map(<[u8]>::len))
            .sum();
        rows.push(info(format!("  {:<11} {n:>4}   {}", kind.plural(), human_bytes(bytes))));
    }
    for (label, map) in [
        ("depth maps", res.depths()),
        ("normal maps", res.normals()),
        ("frame tables", res.frames()),
    ] {
        if !map.is_empty() {
            rows.push(info(format!("  {label:<11} {:>4}   {}", map.len(), human_bytes(sum(map)))));
        }
    }
    rows.push(info(format!("  total       {:>4} blobs", res.len())));
    rows
}

fn display_ns(ns: &str) -> &str {
    if ns.is_empty() {
        "(global)"
    } else {
        ns
    }
}

fn asset_rows(engine: &Engine, a: &AssetRef) -> Vec<Row> {
    let Some(entry) = engine.loaded_roms.get(a.rom) else { return vec![dim("(ROM gone)")] };
    let ns = engine.rom_namespace(&entry.name);
    let q = tree::qualified(&ns, &a.name);
    let src = tree::rom_assets(&entry.rom, a.kind)
        .into_iter()
        .find(|e| e.name == a.name)
        .map(|e| e.src)
        .unwrap_or_default();
    let bytes = entry.rom.resources.get(a.kind.resource_kind(), &a.name).map(<[u8]>::len);
    let mut rows = vec![
        info(format!("kind         {}", a.kind.label())),
        info(format!("name         {q}")),
        Row {
            text: format!("rom          {}", entry.name),
            color: LINK,
            action: Some(RowAction::Select(Selection::Rom(a.rom))),
        },
        info(format!("src          {src}")),
        info(format!("bytes        {}", bytes.map_or("(not bundled)".into(), human_bytes))),
    ];
    match a.kind {
        AssetKind::Texture => {
            let tex = entry.rom.manifest.manifest.textures.iter().find(|t| t.name == a.name);
            rows.push(info(format!(
                "texture_size {}",
                engine
                    .texture_size(&q)
                    .map_or("(not uploaded)".into(), |(w, h)| format!("{w} x {h}"))
            )));
            if let Some(t) = tex {
                rows.push(info(format!("format       {}", t.format.as_deref().unwrap_or("(raw)"))));
                let res = &entry.rom.resources;
                let companion = |src: &Option<String>, blob: Option<&[u8]>| match (src, blob) {
                    (Some(s), Some(b)) => format!("{s} ({})", human_bytes(b.len())),
                    (Some(s), None) => format!("{s} (missing)"),
                    (None, _) => "-".into(),
                };
                rows.push(info(format!(
                    "depth        {}",
                    companion(&t.depth, res.get(classic_rom::ResourceKind::Depth, &a.name))
                )));
                rows.push(info(format!(
                    "normal       {}",
                    companion(&t.normal, res.get(classic_rom::ResourceKind::Normal, &a.name))
                )));
                rows.push(info(format!(
                    "frames       {}",
                    companion(&t.frames, res.get(classic_rom::ResourceKind::Frames, &a.name))
                )));
            }
            if let Some(ft) = engine.frame_tables.get(&q) {
                rows.push(info(format!(
                    "frame table  v{}: {} frames, {} sheets",
                    ft.version,
                    ft.frames.len(),
                    ft.sheets.len()
                )));
            }
        }
        AssetKind::Font => {
            if let Some(f) = engine.sdf_fonts.get(&q) {
                rows.push(info(format!(
                    "font         {} ({} glyphs, glyph {} px)",
                    f.family,
                    f.glyphs.len(),
                    f.glyph_size
                )));
            }
        }
        AssetKind::Animation => {
            if let Some(an) = engine.animations.get(&q) {
                rows.push(info(format!(
                    "animation    {} frames @ {} fps, texture {}",
                    an.sequence.len(),
                    an.rate,
                    an.src
                )));
            }
        }
        AssetKind::Vehicle => {
            if let Some(v) = engine.vehicles.get(&q) {
                rows.push(info(format!(
                    "vehicle      v{}: {} directions, {} layers",
                    v.version,
                    v.directions,
                    v.layers.len()
                )));
            }
        }
        AssetKind::Model => {
            if let Some(m) = engine.models.get(&q) {
                rows.push(info(format!(
                    "model        {} nodes, {} meshes, {} clips",
                    m.nodes.len(),
                    m.meshes.len(),
                    m.clips.len()
                )));
            }
        }
        AssetKind::Data | AssetKind::Code | AssetKind::Grid => {}
    }
    rows
}

fn v3(v: glam::Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

/// Whether an entity lives on the map (its `Transform` is an iso tile
/// position) rather than on the screen (UI, `ignore_cam` sprites/text) or
/// being the map itself.
fn on_map(engine: &Engine, e: hecs::Entity) -> bool {
    let w = &engine.world;
    if w.get::<&UiNode>(e).is_ok() || w.get::<&Tilemap>(e).is_ok() || w.get::<&NavMesh>(e).is_ok() {
        return false;
    }
    if w.get::<&SpriteRender>(e).is_ok_and(|s| s.ignore_cam)
        || w.get::<&SdfTextRender>(e).is_ok_and(|s| s.ignore_cam)
    {
        return false;
    }
    engine.entity_by_role(classic_core::RoleKind::Tilemap).is_some()
}

/// The transform block of an entity: raw `Transform`, then (on the map) its
/// tile + terrain height, world point and screen point.
pub fn transform_rows(engine: &Engine, name: &str) -> Vec<Row> {
    let Some(&e) = engine.names.get(name) else { return vec![dim("(entity gone)")] };
    let mut rows = vec![heading("Transform")];
    let tf = engine.world.get::<&Transform>(e).ok().map(|t| (t.position, t.scale));
    match tf {
        Some((p, s)) => {
            rows.push(info(format!("  position   {}", v3(p))));
            rows.push(info(format!("  scale      {}", v3(s))));
        }
        None => rows.push(dim("  (no Transform)")),
    }
    if !on_map(engine, e) {
        return rows;
    }
    if let Some((p, _)) = tf {
        rows.push(info(format!(
            "  tile       ({:.2}, {:.2})   ground {:.2} m",
            p.x,
            p.y,
            engine.height_at(p.x, p.y)
        )));
    }
    if let Some(w) = engine.named_world_point(name) {
        rows.push(info(format!("  world      {} m", v3(w))));
        let (sx, sy) = engine.world_to_screen(w);
        rows.push(info(format!("  screen     ({sx:.0}, {sy:.0}) px")));
    }
    rows
}

/// The assets and entities an entity's components name, each resolved the
/// way the engine resolves it (`resolve_resource` in the entity's namespace).
pub fn related(engine: &Engine, name: &str) -> Vec<(String, Option<Selection>)> {
    let Some(&e) = engine.names.get(name) else { return Vec::new() };
    let ns = Engine::namespace_of(name);
    let w = &engine.world;
    let mut assets: Vec<(ResourceKind, AssetKind, String)> = Vec::new();
    let mut entities: Vec<String> = Vec::new();
    if let Ok(s) = w.get::<&IsoSprite>(e) {
        assets.push((ResourceKind::Texture, AssetKind::Texture, s.texture.clone()));
    }
    if let Ok(s) = w.get::<&IsoAgent>(e) {
        assets.push((ResourceKind::Texture, AssetKind::Texture, s.texture.clone()));
    }
    if let Ok(s) = w.get::<&SpriteRender>(e) {
        assets.push((ResourceKind::Texture, AssetKind::Texture, s.texture.clone()));
    }
    if let Ok(s) = w.get::<&SdfTextRender>(e) {
        assets.push((ResourceKind::Font, AssetKind::Font, s.atlas_name.clone()));
    }
    if let Ok(a) = w.get::<&Animator>(e) {
        if let Some(anim) = &a.animation {
            assets.push((ResourceKind::Animation, AssetKind::Animation, anim.clone()));
        }
        entities.push(a.target.split('.').next().unwrap_or_default().to_string());
    }
    if let Ok(m) = w.get::<&Model>(e) {
        assets.push((ResourceKind::Model, AssetKind::Model, m.model.clone()));
    }
    if let Ok(t) = w.get::<&Tilemap>(e) {
        assets.push((ResourceKind::Texture, AssetKind::Texture, t.tile_set.clone()));
    }
    if let Ok(n) = w.get::<&NavMesh>(e) {
        assets.push((ResourceKind::Texture, AssetKind::Texture, n.tile_set.clone()));
    }
    if let Ok(v) = w.get::<&IsoVehicle>(e) {
        if !v.def_name.is_empty() {
            assets.push((ResourceKind::Vehicle, AssetKind::Vehicle, v.def_name.clone()));
        }
        entities.extend(v.layers.iter().map(|l| l.entity.clone()));
        entities.extend(v.wheel_entities.iter().cloned());
        entities.extend(v.tire_entities.iter().cloned());
        entities.extend(v.attachments.iter().cloned());
    }

    let mut out: Vec<(String, Option<Selection>)> = Vec::new();
    for (rk, kind, raw) in assets {
        if raw.is_empty() {
            continue;
        }
        let resolved = engine.resolve_resource(&ns, rk, &raw);
        let shown = resolved.clone().unwrap_or_else(|| raw.clone());
        let target = resolved.and_then(|q| tree::find_asset(engine, kind, &q));
        let suffix = if target.is_some() { "" } else { "  (runtime)" };
        let row = (format!("  {:<9} {shown}{suffix}", kind.label()), target.map(Selection::Asset));
        if !out.contains(&row) {
            out.push(row);
        }
    }
    for raw in entities {
        if raw.is_empty() || raw == name {
            continue;
        }
        let Some(full) = resolve_entity(engine, &ns, &raw) else { continue };
        let row = (format!("  entity    {full}"), Some(Selection::Entity(full.clone())));
        if !out.contains(&row) {
            out.push(row);
        }
    }
    out
}

/// An entity reference as written in a component: exact, else in the
/// referring namespace.
fn resolve_entity(engine: &Engine, ns: &str, raw: &str) -> Option<String> {
    if engine.names.contains_key(raw) {
        return Some(raw.to_string());
    }
    let q = tree::qualified(ns, raw);
    engine.names.contains_key(&q).then_some(q)
}

fn entity_rows(engine: &Engine, name: &str) -> Vec<Row> {
    let Some(&e) = engine.names.get(name) else { return vec![dim("(entity gone)")] };
    let ns = Engine::namespace_of(name);
    let rom = engine.loaded_roms.iter().position(|r| engine.rom_namespace(&r.name) == ns);
    let mut rows = vec![
        info(format!("name         {name}")),
        match rom {
            Some(i) => Row {
                text: format!("rom          {}", engine.loaded_roms[i].name),
                color: LINK,
                action: Some(RowAction::Select(Selection::Rom(i))),
            },
            None => dim(format!("rom          (none: namespace {})", display_ns(&ns))),
        },
        info(format!("hidden       {}", engine.is_disabled(e))),
    ];
    rows.extend(transform_rows(engine, name));
    let rel = related(engine, name);
    if !rel.is_empty() {
        rows.push(heading("Related"));
        for (text, target) in rel {
            match target {
                Some(sel) => {
                    rows.push(Row { text, color: LINK, action: Some(RowAction::Select(sel)) })
                }
                None => rows.push(dim(text)),
            }
        }
    }
    rows
}

/// The component dump of an entity as display rows: one heading per
/// component (its `type`), its fields as compact JSON below.
pub fn component_rows(engine: &Engine, name: &str) -> Vec<Row> {
    let Some(&e) = engine.names.get(name) else { return Vec::new() };
    let comps = engine.dump_entity_components(e);
    let mut rows = vec![heading(format!("Components ({})", comps.len()))];
    let mut lines = Vec::new();
    for c in &comps {
        let ty = c.get("type").and_then(Value::as_str).unwrap_or("?");
        lines.push((format!("  {ty}"), true));
        if let Value::Object(map) = c {
            for (k, v) in map.iter().filter(|(k, _)| k.as_str() != "type") {
                json_field(k, v, 2, &mut lines);
            }
        }
        if lines.len() > MAX_COMPONENT_LINES {
            break;
        }
    }
    let cut = lines.len() > MAX_COMPONENT_LINES;
    lines.truncate(MAX_COMPONENT_LINES);
    for (text, is_head) in lines {
        rows.push(if is_head {
            Row { text, color: tree::SELECTED, action: None }
        } else {
            info(text)
        });
    }
    if cut {
        rows.push(dim("  ... (cut short)"));
    }
    rows
}

fn pad(depth: usize) -> String {
    "  ".repeat(depth)
}

/// `key: value` lines for one field at `depth`, nesting objects and long
/// arrays, inlining what fits.
pub fn json_field(key: &str, v: &Value, depth: usize, out: &mut Vec<(String, bool)>) {
    if let Some(inline) = inline_json(v) {
        out.push((format!("{}{key}: {inline}", pad(depth)), false));
        return;
    }
    match v {
        Value::Object(map) => {
            out.push((format!("{}{key}:", pad(depth)), false));
            for (k, v) in map {
                json_field(k, v, depth + 1, out);
            }
        }
        Value::Array(items) => {
            out.push((format!("{}{key}: [{} items]", pad(depth), items.len()), false));
            for (i, item) in items.iter().enumerate() {
                json_field(&i.to_string(), item, depth + 1, out);
            }
        }
        _ => out.push((format!("{}{key}: {v}", pad(depth)), false)),
    }
}

fn number(n: &serde_json::Number) -> String {
    if n.is_f64() {
        let f = n.as_f64().unwrap_or(0.0);
        let s = format!("{f:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".into()
        } else {
            s.to_string()
        }
    } else {
        n.to_string()
    }
}

/// A one-line rendering of `v` when it is short enough: scalars always,
/// arrays and objects when they fit [`INLINE_WIDTH`], and long scalar
/// arrays as a head plus a count.
pub fn inline_json(v: &Value) -> Option<String> {
    let s = match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number(n),
        Value::String(s) => format!("{s:?}"),
        Value::Array(items) => {
            let scalars = items.iter().all(|i| !i.is_array() && !i.is_object());
            if items.len() > MAX_INLINE_ITEMS {
                if !scalars {
                    return None;
                }
                let head: Vec<String> = items.iter().take(4).filter_map(inline_json).collect();
                return Some(format!("[{}, ... {} items]", head.join(", "), items.len()));
            }
            let parts: Option<Vec<String>> = items.iter().map(inline_json).collect();
            format!("[{}]", parts?.join(", "))
        }
        Value::Object(map) => {
            if map.values().any(|v| v.is_array() || v.is_object()) {
                return None;
            }
            let parts: Option<Vec<String>> =
                map.iter().map(|(k, v)| inline_json(v).map(|s| format!("{k}: {s}"))).collect();
            format!("{{{}}}", parts?.join(", "))
        }
    };
    (s.len() <= INLINE_WIDTH || !(v.is_array() || v.is_object())).then_some(s)
}
