use classic_core::registry::{self, ComponentReg};

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

fn spawn_other(b: &mut hecs::EntityBuilder, _v: serde_json::Value) -> anyhow::Result<()> {
    b.add(Other {});
    Ok(())
}

// The registry is a process-global `OnceLock`, so all assertions live in one
// test (the "first init wins" semantics make independent tests interfere).
#[test]
fn registry_init_lookup_and_idempotency() {
    registry::init(vec![
        ComponentReg { name: "Dummy", spawn: spawn_dummy, dump: None, order: 0, subsumes: &[] },
        ComponentReg { name: "Other", spawn: spawn_other, dump: None, order: 0, subsumes: &[] },
    ]);

    assert!(registry::lookup("Dummy").is_some());
    assert!(registry::has("Dummy"));
    assert!(registry::has("Other"));
    assert!(!registry::has("Nope"));
    assert!(registry::lookup("Nope").is_none());

    let mut names = registry::names();
    names.sort();
    assert_eq!(names, vec!["Dummy", "Other"]);

    // Second init is ignored (first wins).
    registry::init(vec![ComponentReg {
        name: "Second",
        spawn: spawn_other,
        dump: None,
        order: 0,
        subsumes: &[],
    }]);
    assert!(registry::has("Dummy"));
    assert!(!registry::has("Second"));
}
