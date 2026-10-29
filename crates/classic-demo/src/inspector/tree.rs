//! The inspector's left panel: the loaded ROMs (topological order, deps
//! first), each expanding to its **Assets** (by kind, from the manifest) and
//! its **Entities** (`name_order` filtered by namespace, paged).
//!
//! Everything here is a pure function of the engine plus the tree state, so
//! the panel is rebuilt from scratch whenever that state changes; the
//! per-ROM entity lists are cached in [`EntityIndex`] and rebuilt only when
//! the entity count or the nav version moves (or on a Refresh click).

use classic_engine::Engine;
use classic_rom::Rom;

use super::{AssetRef, InspectorState, Row, RowAction, Selection};

/// Rows per page of a leaf list (entities, or one asset kind).
pub const PAGE: usize = 50;

/// The asset kinds the tree lists, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    Texture,
    Font,
    Animation,
    Vehicle,
    Model,
    Data,
    Code,
    Grid,
}

impl AssetKind {
    pub const ALL: [AssetKind; 8] = [
        AssetKind::Texture,
        AssetKind::Font,
        AssetKind::Animation,
        AssetKind::Vehicle,
        AssetKind::Model,
        AssetKind::Data,
        AssetKind::Code,
        AssetKind::Grid,
    ];

    /// Singular label (detail headers, related-asset rows).
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Texture => "texture",
            AssetKind::Font => "font",
            AssetKind::Animation => "animation",
            AssetKind::Vehicle => "vehicle",
            AssetKind::Model => "model",
            AssetKind::Data => "data",
            AssetKind::Code => "code",
            AssetKind::Grid => "grid",
        }
    }

    /// Plural group label.
    pub fn plural(self) -> &'static str {
        match self {
            AssetKind::Texture => "Textures",
            AssetKind::Font => "Fonts",
            AssetKind::Animation => "Animations",
            AssetKind::Vehicle => "Vehicles",
            AssetKind::Model => "Models",
            AssetKind::Data => "Data",
            AssetKind::Code => "Code",
            AssetKind::Grid => "Grids",
        }
    }

    /// The ROM resource store this kind's bytes live in.
    pub fn resource_kind(self) -> classic_rom::ResourceKind {
        use classic_rom::ResourceKind as R;
        match self {
            AssetKind::Texture => R::Texture,
            AssetKind::Font => R::Font,
            AssetKind::Animation => R::Animation,
            AssetKind::Vehicle => R::Vehicle,
            AssetKind::Model => R::Model,
            AssetKind::Data => R::Data,
            AssetKind::Code => R::Code,
            AssetKind::Grid => R::Grid,
        }
    }
}

/// An expandable node under a ROM (or the unscoped-entities slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Assets,
    Kind(AssetKind),
    Entities,
}

/// One manifest asset: its bare (manifest) name and source path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    pub name: String,
    pub src: String,
}

/// The assets of one kind a ROM declares, in manifest order.
pub fn rom_assets(rom: &Rom, kind: AssetKind) -> Vec<AssetEntry> {
    let m = &rom.manifest;
    let pairs: Vec<(&str, &str)> = match kind {
        AssetKind::Texture => {
            m.manifest.textures.iter().map(|t| (t.name.as_str(), t.src.as_str())).collect()
        }
        AssetKind::Font => {
            m.manifest.sdf_fonts.iter().map(|f| (f.name.as_str(), f.metrics.as_str())).collect()
        }
        AssetKind::Animation => {
            m.manifest.animations.iter().map(|a| (a.name.as_str(), a.src.as_str())).collect()
        }
        AssetKind::Vehicle => {
            m.manifest.vehicles.iter().map(|v| (v.name.as_str(), v.src.as_str())).collect()
        }
        AssetKind::Model => {
            m.manifest.models.iter().map(|v| (v.name.as_str(), v.src.as_str())).collect()
        }
        AssetKind::Data => {
            m.manifest.data.iter().map(|v| (v.name.as_str(), v.src.as_str())).collect()
        }
        AssetKind::Code => m.code.iter().map(|c| (c.name.as_str(), c.src.as_str())).collect(),
        AssetKind::Grid => m.grids.iter().map(|g| (g.name.as_str(), g.src.as_str())).collect(),
    };
    pairs
        .into_iter()
        .map(|(name, src)| AssetEntry { name: name.to_string(), src: src.to_string() })
        .collect()
}

/// `ns::name`, or the bare name in the global namespace (or when already
/// qualified) — the engine's resource qualification.
pub fn qualified(ns: &str, name: &str) -> String {
    if ns.is_empty() || name.contains("::") {
        name.to_string()
    } else {
        format!("{ns}::{name}")
    }
}

/// The bare part of a qualified `ns::name`.
pub fn bare(name: &str) -> &str {
    name.rsplit_once("::").map_or(name, |(_, b)| b)
}

/// The effective namespace of every loaded ROM, in load order.
pub fn rom_namespaces(engine: &Engine) -> Vec<String> {
    engine.loaded_roms.iter().map(|r| engine.rom_namespace(&r.name)).collect()
}

/// Map a (qualified) resource name the engine resolved back to the manifest
/// entry that declared it: the first ROM whose namespace qualifies one of its
/// `kind` entries to `name`.
pub fn find_asset(engine: &Engine, kind: AssetKind, name: &str) -> Option<AssetRef> {
    let namespaces = rom_namespaces(engine);
    for (i, entry) in engine.loaded_roms.iter().enumerate() {
        for asset in rom_assets(&entry.rom, kind) {
            if qualified(&namespaces[i], &asset.name) == name {
                return Some(AssetRef { rom: i, kind, name: asset.name });
            }
        }
    }
    None
}

/// Split names into one list per namespace (`namespaces[i]` → list `i`) plus
/// the names no namespace claims.  A name goes to the first matching slot.
pub fn group_by_namespace(
    names: &[String],
    namespaces: &[String],
) -> (Vec<Vec<String>>, Vec<String>) {
    let mut per = vec![Vec::new(); namespaces.len()];
    let mut other = Vec::new();
    for name in names {
        let ns = Engine::namespace_of(name);
        match namespaces.iter().position(|n| *n == ns) {
            Some(i) => per[i].push(name.clone()),
            None => other.push(name.clone()),
        }
    }
    (per, other)
}

/// The window of a paged list: `(start, end, page, pages)` for a list of
/// `len` items asked for `page` (clamped), at most [`PAGE`] per page.
pub fn page_range(len: usize, page: usize) -> (usize, usize, usize, usize) {
    let pages = len.div_ceil(PAGE).max(1);
    let page = page.min(pages - 1);
    let start = page * PAGE;
    (start, (start + PAGE).min(len), page, pages)
}

/// The page a list index falls on.
pub fn page_of(index: usize) -> usize {
    index / PAGE
}

/// The per-ROM entity lists, cached between rebuilds.
#[derive(Debug, Default)]
pub struct EntityIndex {
    /// `(entity count, nav version)` the lists were built at.
    key: Option<(usize, u64)>,
    /// One list per loaded ROM, in `loaded_roms` order.
    pub per_rom: Vec<Vec<String>>,
    /// Names whose namespace matches no loaded ROM.
    pub unscoped: Vec<String>,
}

impl EntityIndex {
    /// Rebuild when the entity count or nav version changed (or `force`).
    /// Returns whether it rebuilt.
    pub fn refresh(&mut self, engine: &Engine, force: bool) -> bool {
        let key = (engine.name_order.len(), engine.nav_version());
        if !force && self.key == Some(key) {
            return false;
        }
        let (per_rom, unscoped) = group_by_namespace(&engine.name_order, &rom_namespaces(engine));
        self.per_rom = per_rom;
        self.unscoped = unscoped;
        self.key = Some(key);
        true
    }

    /// The entity list of a slot (`loaded_roms.len()` = the unscoped one).
    pub fn slot(&self, slot: usize) -> &[String] {
        self.per_rom.get(slot).map_or(&self.unscoped, |v| v.as_slice())
    }

    /// The slot that lists `name`, and its index there.
    pub fn locate(&self, name: &str) -> Option<(usize, usize)> {
        for (slot, list) in self.per_rom.iter().enumerate() {
            if let Some(i) = list.iter().position(|n| n == name) {
                return Some((slot, i));
            }
        }
        let i = self.unscoped.iter().position(|n| n == name)?;
        Some((self.per_rom.len(), i))
    }
}

pub(super) const WHITE: [f32; 4] = [0.95, 0.95, 0.95, 1.0];
pub(super) const GROUP: [f32; 4] = [0.75, 0.78, 0.85, 1.0];
pub(super) const LINK: [f32; 4] = [0.55, 0.8, 1.0, 1.0];
pub(super) const SELECTED: [f32; 4] = [1.0, 0.85, 0.3, 1.0];
pub(super) const DIM: [f32; 4] = [0.55, 0.57, 0.62, 1.0];

fn mark(open: bool) -> &'static str {
    if open {
        "[-]"
    } else {
        "[+]"
    }
}

/// A row that selects `sel`, highlighted when it is the current selection.
fn select_row(st: &InspectorState, text: String, sel: Selection) -> Row {
    let color = if st.selection.as_ref() == Some(&sel) { SELECTED } else { LINK };
    Row { text, color, action: Some(RowAction::Select(sel)) }
}

/// The paged leaf rows of one group: a page header (with a "prev" link), the
/// items, then a "next" link.
fn paged<T>(
    rows: &mut Vec<Row>,
    st: &InspectorState,
    slot: usize,
    group: Group,
    items: &[T],
    indent: &str,
    mut row: impl FnMut(&T) -> Row,
) {
    let want = st.pages.get(&(slot, group)).copied().unwrap_or(0);
    let (start, end, page, pages) = page_range(items.len(), want);
    if pages > 1 {
        let head = format!(
            "{indent}page {}/{} ({}-{} of {})",
            page + 1,
            pages,
            start + 1,
            end,
            items.len()
        );
        if page > 0 {
            rows.push(Row {
                text: format!("{head}   < prev"),
                color: LINK,
                action: Some(RowAction::Page(slot, group, page - 1)),
            });
        } else {
            rows.push(Row { text: head, color: DIM, action: None });
        }
    }
    for item in &items[start..end] {
        rows.push(row(item));
    }
    if page + 1 < pages {
        rows.push(Row {
            text: format!("{indent}next {} >", PAGE.min(items.len() - end)),
            color: LINK,
            action: Some(RowAction::Page(slot, group, page + 1)),
        });
    }
}

/// Every row of the tree panel for the current state.
pub fn build_rows(engine: &Engine, st: &InspectorState) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, entry) in engine.loaded_roms.iter().enumerate() {
        let open = st.expanded_roms.contains(&i);
        let n_entities = st.index.slot(i).len();
        let text = format!("{} {}  ({} entities)", mark(open), entry.name, n_entities);
        let color = if st.selection == Some(Selection::Rom(i)) { SELECTED } else { WHITE };
        rows.push(Row { text, color, action: Some(RowAction::ToggleRom(i)) });
        if !open {
            continue;
        }

        let kinds: Vec<(AssetKind, Vec<AssetEntry>)> = AssetKind::ALL
            .iter()
            .map(|&k| (k, rom_assets(&entry.rom, k)))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let n_assets: usize = kinds.iter().map(|(_, v)| v.len()).sum();
        let assets_open = st.expanded.contains(&(i, Group::Assets));
        rows.push(Row {
            text: format!("    {} Assets ({n_assets})", mark(assets_open)),
            color: GROUP,
            action: Some(RowAction::ToggleGroup(i, Group::Assets)),
        });
        if assets_open {
            for (kind, list) in &kinds {
                let group = Group::Kind(*kind);
                let kind_open = st.expanded.contains(&(i, group));
                rows.push(Row {
                    text: format!("        {} {} ({})", mark(kind_open), kind.plural(), list.len()),
                    color: GROUP,
                    action: Some(RowAction::ToggleGroup(i, group)),
                });
                if kind_open {
                    paged(&mut rows, st, i, group, list, "            ", |a| {
                        let sel = Selection::Asset(AssetRef {
                            rom: i,
                            kind: *kind,
                            name: a.name.clone(),
                        });
                        select_row(st, format!("            {}", a.name), sel)
                    });
                }
            }
        }

        entities_rows(&mut rows, st, i, "    ", "Entities");
    }

    let other = engine.loaded_roms.len();
    if !st.index.unscoped.is_empty() {
        entities_rows(&mut rows, st, other, "", "Other entities");
    }
    rows.push(Row { text: "[ Refresh ]".into(), color: LINK, action: Some(RowAction::Refresh) });
    rows
}

fn entities_rows(rows: &mut Vec<Row>, st: &InspectorState, slot: usize, indent: &str, label: &str) {
    let list = st.index.slot(slot);
    let open = st.expanded.contains(&(slot, Group::Entities));
    rows.push(Row {
        text: format!("{indent}{} {label} ({})", mark(open), list.len()),
        color: GROUP,
        action: Some(RowAction::ToggleGroup(slot, Group::Entities)),
    });
    if open {
        let leaf = format!("{indent}    ");
        paged(rows, st, slot, Group::Entities, list, &leaf, |name| {
            select_row(st, format!("{leaf}{}", bare(name)), Selection::Entity(name.clone()))
        });
    }
}
