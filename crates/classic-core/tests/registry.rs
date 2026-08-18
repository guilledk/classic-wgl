use classic_core::registry;

/// A dummy "component" — just data we want to spawn.
#[derive(Debug, PartialEq)]
struct Dummy {
    value: i32,
}
struct Other {}

fn spawn_dummy(b: &mut hecs::EntityBuilder, v: serde_json::Value) -> anyhow::Result<()> {
    let d: i32 = v.get("value").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    b.add(Dummy { value: d });
    Ok(())
}

fn spawn_other(_b: &mut hecs::EntityBuilder, _v: serde_json::Value) -> anyhow::Result<()> {
    _b.add(Other {});
    Ok(())
}

#[test]
fn registers_and_looks_up() {
    registry::clear();
    registry::register_spawner("Dummy", spawn_dummy);
    assert!(registry::lookup("Dummy").is_some());
}

#[test]
fn has_reflects_state() {
    registry::clear();
    assert!(!registry::has("Dummy"));
    registry::register_spawner("Dummy", spawn_dummy);
    assert!(registry::has("Dummy"));
}

#[test]
fn lookup_returns_none_for_unknown() {
    registry::clear();
    assert!(registry::lookup("Nope").is_none());
}

#[test]
fn overwriting_registers_warns() {
    registry::clear();
    registry::register_spawner("Dummy", spawn_dummy);
    registry::register_spawner("Dummy", spawn_other);
    assert!(registry::has("Dummy"));
}

#[test]
fn lists_all_names() {
    registry::clear();
    registry::register_spawner("Dummy", spawn_dummy);
    registry::register_spawner("Other", spawn_other);
    let mut names = registry::names();
    names.sort();
    assert_eq!(names, vec!["Dummy", "Other"]);
}

#[test]
fn clear_removes_all() {
    registry::clear();
    registry::register_spawner("Dummy", spawn_dummy);
    registry::clear();
    assert!(registry::names().is_empty());
    assert!(!registry::has("Dummy"));
}
