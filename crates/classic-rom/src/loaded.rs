//! Multi-ROM loading: a cycle-checked, topologically ordered dependency DAG.
//!
//! A ROM's [`RomManifest::deps`](crate::RomManifest) declares the names of the
//! ROMs it depends on.  [`LoadedRoms::resolve`] walks that graph from a root
//! name — loading each ROM once, rejecting cycles, and returning the ROMs in
//! an order where every dependency precedes its dependents.  Byte
//! materialisation and parsing are the caller's job (the platform layer owns
//! the name -> location index and the native/web fetch paths); this module is
//! pure graph logic over already-loaded [`Rom`]s.

use std::collections::BTreeSet;
use std::future::Future;

use anyhow::Context;

use crate::rom::Rom;

/// One ROM in a resolved multi-ROM dependency DAG.
#[derive(Clone, Debug)]
pub struct LoadedRom {
    /// The name this ROM was resolved under (the name -> location index key).
    pub name: String,
    /// The ROM's declared namespace (verbatim from the manifest; empty =
    /// global, un-namespaced).  The engine derives the effective namespace
    /// (defaulting to the entrypoint for multi-ROM participants) at load time.
    pub namespace: String,
    /// The parsed, resource-populated ROM.
    pub rom: Rom,
}

/// A resolved multi-ROM dependency DAG in load order (deps before dependents).
#[derive(Clone, Debug, Default)]
pub struct LoadedRoms {
    /// The name the graph was rooted at.
    pub root: String,
    /// ROMs in topological order: dependencies first, the root last.
    pub order: Vec<LoadedRom>,
}

impl LoadedRoms {
    /// Resolve the dependency DAG rooted at `root_name`.
    ///
    /// `load` maps a ROM name to a fully-loaded [`Rom`] and is called at most
    /// once per distinct name (diamonds are de-duplicated).  The graph is
    /// walked depth-first with cycle detection; the returned `order` places
    /// every dependency before its dependents.
    pub fn resolve(
        root_name: &str,
        mut load: impl FnMut(&str) -> anyhow::Result<Rom>,
    ) -> anyhow::Result<Self> {
        let mut order = Vec::new();
        let mut done = BTreeSet::new();
        let mut visiting = Vec::new();
        visit(root_name, &mut load, &mut done, &mut visiting, &mut order)?;
        Ok(Self { root: root_name.to_string(), order })
    }

    /// Async counterpart to [`LoadedRoms::resolve`] for platforms whose ROM
    /// bytes are fetched (web).  `load` maps a ROM name to a future that
    /// resolves to a fully-loaded [`Rom`]; the same cycle / dedup / topological
    /// guarantees apply.
    pub async fn resolve_async<F, Fut>(root_name: &str, mut load: F) -> anyhow::Result<Self>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = anyhow::Result<Rom>>,
    {
        let mut order = Vec::new();
        let mut done = BTreeSet::new();
        let mut visiting = Vec::new();
        visit_async(root_name.to_string(), &mut load, &mut done, &mut visiting, &mut order).await?;
        Ok(Self { root: root_name.to_string(), order })
    }

    /// Iterate the ROMs in topological order (deps before dependents).
    pub fn iter(&self) -> impl Iterator<Item = &LoadedRom> {
        self.order.iter()
    }

    /// The root ROM (the last entry in topological order).
    pub fn root_rom(&self) -> Option<&Rom> {
        self.order.last().map(|e| &e.rom)
    }
}

fn visit(
    name: &str,
    load: &mut dyn FnMut(&str) -> anyhow::Result<Rom>,
    done: &mut BTreeSet<String>,
    visiting: &mut Vec<String>,
    order: &mut Vec<LoadedRom>,
) -> anyhow::Result<()> {
    if done.contains(name) {
        return Ok(());
    }
    if let Some(pos) = visiting.iter().position(|n| n == name) {
        let cycle = visiting[pos..].to_vec().join(" -> ");
        anyhow::bail!("ROM dependency cycle: {cycle} -> {name}");
    }

    visiting.push(name.to_string());
    let rom = load(name).with_context(|| format!("load ROM dependency `{name}`"))?;
    let namespace = rom.manifest.namespace.clone();
    for dep in &rom.manifest.deps {
        visit(dep, load, done, visiting, order)?;
    }
    visiting.pop();
    done.insert(name.to_string());
    order.push(LoadedRom { name: name.to_string(), namespace, rom });
    Ok(())
}

async fn visit_async<F, Fut>(
    name: String,
    load: &mut F,
    done: &mut BTreeSet<String>,
    visiting: &mut Vec<String>,
    order: &mut Vec<LoadedRom>,
) -> anyhow::Result<()>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<Rom>>,
{
    if done.contains(&name) {
        return Ok(());
    }
    if let Some(pos) = visiting.iter().position(|n| *n == name) {
        let cycle = visiting[pos..].to_vec().join(" -> ");
        anyhow::bail!("ROM dependency cycle: {cycle} -> {name}");
    }

    visiting.push(name.clone());
    let rom = load(name.clone()).await?;
    let namespace = rom.manifest.namespace.clone();
    let deps = rom.manifest.deps.clone();
    for dep in deps {
        // Box the recursive future so async recursion type-checks.
        Box::pin(visit_async(dep, &mut *load, done, visiting, order)).await?;
    }
    visiting.pop();
    done.insert(name.clone());
    order.push(LoadedRom { name, namespace, rom });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::RomManifest;
    use crate::resource::ResourceSet;
    use std::collections::HashMap;

    fn rom_with(name: &str, namespace: &str, deps: &[&str]) -> Rom {
        let deps_json = deps.iter().map(|d| format!("\"{d}\"")).collect::<Vec<_>>().join(",");
        let manifest_json = format!(
            r#"{{
                "entrypoint": "{name}",
                "namespace": "{namespace}",
                "deps": [{deps_json}],
                "shaders": [],
                "textures": [],
                "animations": []
            }}"#
        );
        let manifest: RomManifest = serde_json::from_str(&manifest_json).unwrap();
        Rom {
            manifest,
            manifest_json,
            resources: ResourceSet::default(),
            state: "{\"entities\":{}}".into(),
        }
    }

    fn loader(roms: HashMap<&'static str, Rom>) -> impl FnMut(&str) -> anyhow::Result<Rom> {
        move |name: &str| {
            roms.get(name).cloned().ok_or_else(|| anyhow::anyhow!("unknown ROM `{name}`"))
        }
    }

    fn names(loaded: &LoadedRoms) -> Vec<&str> {
        loaded.order.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn single_rom_with_no_deps() {
        let loaded = LoadedRoms::resolve(
            "demo",
            loader(HashMap::from([("demo", rom_with("demo", "", &[]))])),
        )
        .unwrap();
        assert_eq!(loaded.root, "demo");
        assert_eq!(names(&loaded), vec!["demo"]);
    }

    #[test]
    fn resolves_linear_chain_in_topological_order() {
        let loaded = LoadedRoms::resolve(
            "scene",
            loader(HashMap::from([
                ("scene", rom_with("scene", "scene", &["vehicles"])),
                ("vehicles", rom_with("vehicles", "vehicles", &["common"])),
                ("common", rom_with("common", "common", &[])),
            ])),
        )
        .unwrap();
        assert_eq!(names(&loaded), vec!["common", "vehicles", "scene"]);
    }

    #[test]
    fn dedups_diamond_dependencies() {
        let loaded = LoadedRoms::resolve(
            "scene",
            loader(HashMap::from([
                ("scene", rom_with("scene", "scene", &["a", "b"])),
                ("a", rom_with("a", "a", &["common"])),
                ("b", rom_with("b", "b", &["common"])),
                ("common", rom_with("common", "common", &[])),
            ])),
        )
        .unwrap();
        // `common` must appear exactly once, before both a and b, all before
        // the root.
        assert_eq!(names(&loaded), vec!["common", "a", "b", "scene"]);
    }

    #[test]
    fn records_declared_namespaces() {
        let loaded = LoadedRoms::resolve(
            "scene",
            loader(HashMap::from([
                ("scene", rom_with("scene", "lunar", &["common"])),
                ("common", rom_with("common", "", &[])),
            ])),
        )
        .unwrap();
        let by_name: HashMap<&str, &str> =
            loaded.order.iter().map(|e| (e.name.as_str(), e.namespace.as_str())).collect();
        assert_eq!(by_name["common"], "");
        assert_eq!(by_name["scene"], "lunar");
    }

    #[test]
    fn rejects_direct_cycle() {
        let err = LoadedRoms::resolve(
            "a",
            loader(HashMap::from([
                ("a", rom_with("a", "a", &["b"])),
                ("b", rom_with("b", "b", &["a"])),
            ])),
        )
        .unwrap_err();
        assert!(err.to_string().contains("cycle"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_self_cycle() {
        let err =
            LoadedRoms::resolve("a", loader(HashMap::from([("a", rom_with("a", "a", &["a"]))])))
                .unwrap_err();
        assert!(err.to_string().contains("cycle"), "unexpected error: {err}");
    }

    #[test]
    fn surfaces_missing_dependency_name() {
        let err = LoadedRoms::resolve(
            "scene",
            loader(HashMap::from([("scene", rom_with("scene", "scene", &["missing"]))])),
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_async_matches_sync_topological_order() {
        let roms = HashMap::from([
            ("scene", rom_with("scene", "scene", &["vehicles"])),
            ("vehicles", rom_with("vehicles", "vehicles", &["common"])),
            ("common", rom_with("common", "common", &[])),
        ]);
        let mut loader = {
            let roms = roms.clone();
            move |name: String| {
                let roms = roms.clone();
                async move {
                    roms.get(name.as_str())
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("unknown ROM `{name}`"))
                }
            }
        };
        let loaded = pollster::block_on(LoadedRoms::resolve_async("scene", &mut loader)).unwrap();
        assert_eq!(names(&loaded), vec!["common", "vehicles", "scene"]);
    }

    #[test]
    fn resolve_async_rejects_cycle() {
        let roms =
            HashMap::from([("a", rom_with("a", "a", &["b"])), ("b", rom_with("b", "b", &["a"]))]);
        let mut loader = {
            let roms = roms.clone();
            move |name: String| {
                let roms = roms.clone();
                async move {
                    roms.get(name.as_str())
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("unknown ROM `{name}`"))
                }
            }
        };
        let err = pollster::block_on(LoadedRoms::resolve_async("a", &mut loader)).unwrap_err();
        assert!(err.to_string().contains("cycle"), "unexpected error: {err}");
    }
}
