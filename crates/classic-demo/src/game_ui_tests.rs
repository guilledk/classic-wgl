//! The game UI (a guest's managed UI) beside the dev HUD: sizes, label
//! placement and selection.

use classic_core::components::{Selectable, Transform, UiAnchor, UiNode};
use classic_engine::Engine;
use glam::Vec2;

const VW: f32 = 1280.0;
const VH: f32 = 720.0;

fn frame(engine: &mut Engine) {
    let mut platform = engine.input.clone();
    engine.frame(&mut platform, VW, VH, 1.0 / 60.0);
}

/// A guest-style button: built, attached to the root at `anchor`.
fn button(engine: &mut Engine, name: &str, w: f32, h: f32, anchor: UiAnchor) {
    assert!(engine.ui_button(name, "Go", w, h, [0.2, 0.6, 0.2, 1.0]));
    assert!(engine.ui_add_to_root(name, anchor, anchor));
}

/// `name`'s top-left and size, and its label's top-left and size.
fn rects(engine: &Engine, name: &str) -> ((Vec2, Vec2), (Vec2, Vec2)) {
    let e = engine.names[name];
    let node = engine.world.get::<&UiNode>(e).unwrap();
    let label = node.children[0].entity;
    let at = |e| engine.world.get::<&Transform>(e).unwrap().position.truncate();
    let size = |e| engine.world.get::<&UiNode>(e).unwrap().size;
    ((at(e), node.size), (at(label), size(label)))
}

/// The label sits centred on its button.
fn assert_label_follows(engine: &Engine, name: &str, when: &str) {
    let ((b, bs), (l, ls)) = rects(engine, name);
    let want = b + bs / 2.0 - ls / 2.0;
    assert!((l - want).length() < 0.5, "{when}: label at {l}, its button at {b} {bs}");
}

#[test]
fn a_guest_button_keeps_its_size_when_the_dev_hud_adopts_the_root() {
    let mut engine = Engine::new_for_test();
    // the guest's init: a 220 x 64 button on the game UI's root
    button(&mut engine, "go", 220.0, 64.0, UiAnchor::BotCenter);
    // then the dev HUD takes the root (and resizes it to the viewport)
    crate::editor::init_ui(&mut engine);
    frame(&mut engine);
    let ((_, size), _) = rects(&engine, "go");
    assert_eq!(size, Vec2::new(220.0, 64.0), "not stretched to the viewport");
    // a viewport resize keeps it too, while the dev top bar spans the width
    let mut platform = engine.input.clone();
    engine.frame(&mut platform, 1600.0, 900.0, 1.0 / 60.0);
    let ((_, size), _) = rects(&engine, "go");
    assert_eq!(size, Vec2::new(220.0, 64.0));
    let root = engine.ui.as_ref().unwrap().root;
    let bar_w = engine
        .world
        .get::<&UiNode>(root)
        .unwrap()
        .children
        .iter()
        .filter_map(|c| {
            engine.world.get::<&UiNode>(c.entity).ok().map(|n| (n.fill_width, n.size.x))
        })
        .find(|(fill, _)| *fill)
        .expect("the dev top bar fills the width")
        .1;
    assert_eq!(bar_w, 1600.0);
}

#[test]
fn a_buttons_label_follows_it_through_create_show_and_resize() {
    let mut engine = Engine::new_for_test();
    button(&mut engine, "go", 220.0, 64.0, UiAnchor::BotCenter);
    frame(&mut engine);
    assert_label_follows(&engine, "go", "created in init");
    // resized
    assert!(engine.ui_set_size("go", 400.0, 120.0));
    frame(&mut engine);
    assert_label_follows(&engine, "go", "after ui_set_size");
    // re-anchored
    assert!(engine.ui_set_anchor("go", UiAnchor::TopRight));
    frame(&mut engine);
    assert_label_follows(&engine, "go", "after ui_set_anchor");
    // hidden, resized while hidden, shown
    assert!(engine.set_enabled_named("go", false));
    frame(&mut engine);
    assert!(engine.ui_set_size("go", 300.0, 90.0));
    frame(&mut engine);
    assert!(engine.set_enabled_named("go", true));
    frame(&mut engine);
    assert_label_follows(&engine, "go", "shown again");
    // built mid-run and shown at once, beside the first
    for _ in 0..3 {
        frame(&mut engine);
    }
    button(&mut engine, "fire", 180.0, 48.0, UiAnchor::BotRight);
    frame(&mut engine);
    assert_label_follows(&engine, "fire", "built mid-run");
    assert_label_follows(&engine, "go", "the first, unmoved");
}

#[test]
fn rts_selection_needs_no_dev_layer() {
    // No dev HUD (as under CLASSIC_NO_UI, or without host features): a
    // click on a selectable selects it.
    let mut engine = Engine::new_for_test();
    assert!(engine.guest_flag("rts_selection"), "on by default");
    assert!(engine.spawn_named("unit"));
    let e = engine.names["unit"];
    engine.world.insert_one(e, Selectable { priority: 0, group: 0 }).unwrap();
    assert!(engine.spawn_collider("unit", 600.0, 320.0, 80.0, 80.0));
    for f in 0..5 {
        if f == 1 {
            engine.scripted_input.click(Vec2::new(640.0, 360.0), 0);
        }
        frame(&mut engine);
    }
    assert_eq!(engine.selected_names(), vec!["unit".to_string()]);
}
