use std::sync::{Mutex, MutexGuard};

use classic_core::instrument::{Chan, Level};

fn enabled(ch: Chan, lvl: Level) -> bool {
    classic_core::instrument::enabled(ch, lvl)
}

static SETUP_LOCK: Mutex<()> = Mutex::new(());

/// Configure the global log-channel table.  `init` writes the whole table
/// non-atomically, so serialise setup + assertions across tests.
fn setup(spec: &str) -> MutexGuard<'static, ()> {
    let guard = SETUP_LOCK.lock().unwrap();
    classic_core::instrument::reset_for_test();
    classic_core::instrument::init(spec);
    guard
}

#[test]
fn default_is_all_off() {
    let _guard = setup("");
    for &ch in &[Chan::Frame, Chan::Ui, Chan::Collision, Chan::Click] {
        assert!(!enabled(ch, Level::Error));
        assert!(!enabled(ch, Level::Info));
    }
}

#[test]
fn single_channel_enables_info() {
    let _guard = setup("ui");
    assert!(enabled(Chan::Ui, Level::Info));
    assert!(!enabled(Chan::Ui, Level::Debug));
    assert!(!enabled(Chan::Frame, Level::Info));
}

#[test]
fn channel_with_level() {
    let _guard = setup("collision=trace");
    assert!(enabled(Chan::Collision, Level::Info));
    assert!(enabled(Chan::Collision, Level::Trace));
    assert!(!enabled(Chan::Ui, Level::Error));
}

#[test]
fn all_with_level() {
    let _guard = setup("all=info");
    for &ch in &[Chan::Frame, Chan::Ui, Chan::Text, Chan::Iso] {
        assert!(enabled(ch, Level::Info));
        assert!(!enabled(ch, Level::Debug));
    }
}

#[test]
fn negate_single_channel() {
    let _guard = setup("all=info,-ui");
    assert!(enabled(Chan::Frame, Level::Info));
    assert!(!enabled(Chan::Ui, Level::Error));
    assert!(!enabled(Chan::Ui, Level::Info));
}

#[test]
fn negate_all() {
    let _guard = setup("all=info,-all");
    for &ch in &[Chan::Frame, Chan::Ui, Chan::Collision] {
        assert!(!enabled(ch, Level::Error));
        assert!(!enabled(ch, Level::Info));
    }
}

#[test]
fn negate_with_level_ignores_level() {
    let _guard = setup("-ui=trace");
    assert!(!enabled(Chan::Ui, Level::Error));
    assert!(!enabled(Chan::Ui, Level::Trace));
}

#[test]
fn physics_alias_expands() {
    let _guard = setup("physics=debug");
    assert!(enabled(Chan::Collision, Level::Debug));
    assert!(enabled(Chan::Click, Level::Debug));
    assert!(!enabled(Chan::Ui, Level::Error));
}

#[test]
fn glstate_alias_works() {
    let _guard = setup("glstate=trace");
    assert!(enabled(Chan::GlState, Level::Trace));
}

#[test]
fn comma_separated_channels() {
    let _guard = setup("ui=trace,frame=debug");
    assert!(enabled(Chan::Ui, Level::Trace));
    assert!(enabled(Chan::Frame, Level::Debug));
    assert!(!enabled(Chan::Frame, Level::Trace));
    assert!(!enabled(Chan::Collision, Level::Error));
}

#[test]
fn unknown_channel_does_not_panic() {
    let _guard = setup("nonexistant=trace");
}

#[test]
fn unknown_level_defaults_to_info() {
    let _guard = setup("ui=nonexistant");
    assert!(enabled(Chan::Ui, Level::Info));
}
