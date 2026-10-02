use std::cell::Cell;
use std::rc::Rc;

use glam::{Mat4, Vec3};

use classic_core::collision::{polygon_from_verts, HandlerKind, PhysicsProvider};
use classic_core::components::{ColliderData, Shape};

#[test]
fn registers_and_retrieves_collider() {
    let mut physics = PhysicsProvider::new();
    let c = ColliderData::new(Shape::Circle { diameter: 10.0 });
    let pid = physics.register_collider(c);
    assert!(pid >= 2);
}

#[test]
fn gjk_detects_overlapping_circles() {
    let mut physics = PhysicsProvider::new();
    let pid1 = physics.register_collider(ColliderData {
        position: glam::Vec3::new(100.0, 100.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 20.0 })
    });
    let pid2 = physics.register_collider(ColliderData {
        position: glam::Vec3::new(105.0, 102.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 20.0 })
    });
    assert!(physics.gjk_test(pid1, pid2));
}

#[test]
fn gjk_detects_disjoint_circles() {
    let mut physics = PhysicsProvider::new();
    let pid1 = physics.register_collider(ColliderData {
        position: glam::Vec3::new(100.0, 100.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 10.0 })
    });
    let pid2 = physics.register_collider(ColliderData {
        position: glam::Vec3::new(500.0, 400.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 10.0 })
    });
    assert!(!physics.gjk_test(pid1, pid2));
}

#[test]
fn gjk_mouse_vs_collider() {
    let mut physics = PhysicsProvider::new();
    let pid = physics.register_collider(ColliderData {
        position: glam::Vec3::new(200.0, 150.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 30.0 })
    });
    physics.mouse.position = glam::Vec3::new(203.0, 152.0, 0.0);
    assert!(physics.gjk_test(0, pid));
}

#[test]
fn click_does_not_fire_without_mouse_clicked() {
    let mut physics = PhysicsProvider::new();
    physics.resize_screen(800.0, 600.0);

    let clicked = Rc::new(Cell::new(false));
    let pid = physics.register_collider(ColliderData {
        position: glam::Vec3::new(200.0, 150.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 30.0 })
    });
    physics.add_collider_handler(pid, HandlerKind::Click, {
        // `Rc`, not `Cell::clone` — a cloned `Cell` is a detached copy, so the
        // assert below could never fail no matter what the handler did.
        let cl = Rc::clone(&clicked);
        move || {
            cl.set(true);
            false
        }
    });

    physics.mouse.position = glam::Vec3::new(203.0, 152.0, 0.0);
    physics.mouse_clicked = false;
    physics.begin_frame();
    physics.perform_calls();
    assert!(!clicked.get());
}

// ---------------------------------------------------------------------------
// World-space colliders are queried in screen space
//
// `begin_frame` inserts a `ColliderSpace::World` collider into the *screen*
// quadtree by its projected rect, and `shape_of` reads it back projected.
// `handle_for` used to hand back the un-projected *world* rect, so
// `perform_calls` retrieved candidates for the wrong quadtree region and
// Enter/Exit never fired for world colliders under a non-identity camera.
//
// The quadtree only subdivides past `max_objects` (10), and `retrieve` always
// returns everything held at a level — so this needs enough fillers to force a
// split, otherwise a wrong-space rect still finds everything by accident.
// ---------------------------------------------------------------------------

fn quad(x: f32, y: f32, w: f32, h: f32) -> Shape {
    polygon_from_verts(vec![
        Vec3::new(x, y, 0.0),
        Vec3::new(x + w, y, 0.0),
        Vec3::new(x + w, y + h, 0.0),
        Vec3::new(x, y + h, 0.0),
    ])
}

/// Push the screen quadtree past `max_objects` with colliders parked strictly
/// inside the bottom-left quadrant, so the root splits and the top-right
/// quadrant becomes distinguishable from the top-left one.
fn fill_bottom_left(physics: &mut PhysicsProvider) {
    for i in 0..12 {
        physics.register_collider(ColliderData {
            position: Vec3::new(100.0 + i as f32 * 20.0, 500.0, 0.0),
            ..ColliderData::new(Shape::Circle { diameter: 10.0 })
        });
    }
}

#[test]
fn world_collider_enter_fires_under_non_identity_camera() {
    let mut physics = PhysicsProvider::new();
    physics.resize_screen(1280.0, 720.0);
    // Non-identity: the engine's iso camera is a scale + translate.  Chosen so
    // the two colliders project into the *top-right* quadrant while their
    // world rects sit in the *top-left* one.
    physics.set_world_to_screen(
        Mat4::from_translation(Vec3::new(600.0, 200.0, 0.0))
            * Mat4::from_scale(Vec3::new(0.32, 0.32, 1.0)),
    );
    fill_bottom_left(&mut physics);

    let a = physics.register_collider(ColliderData::world(quad(200.0, 50.0, 60.0, 60.0)));
    let _b = physics.register_collider(ColliderData::world(quad(230.0, 80.0, 60.0, 60.0)));

    let entered = Rc::new(Cell::new(false));
    physics.add_collider_handler(a, HandlerKind::Enter, {
        let e = Rc::clone(&entered);
        move || {
            e.set(true);
            false
        }
    });

    physics.begin_frame();
    physics.perform_calls();
    assert!(entered.get(), "Enter must fire for a world-space collider");
}

#[test]
fn world_collider_exit_fires_when_the_pair_separates() {
    let mut physics = PhysicsProvider::new();
    physics.resize_screen(1280.0, 720.0);
    physics.set_world_to_screen(
        Mat4::from_translation(Vec3::new(600.0, 200.0, 0.0))
            * Mat4::from_scale(Vec3::new(0.32, 0.32, 1.0)),
    );
    fill_bottom_left(&mut physics);

    let a = physics.register_collider(ColliderData::world(quad(200.0, 50.0, 60.0, 60.0)));
    let b = physics.register_collider(ColliderData::world(quad(230.0, 80.0, 60.0, 60.0)));

    let exited = Rc::new(Cell::new(false));
    physics.add_collider_handler(a, HandlerKind::Exit, {
        let e = Rc::clone(&exited);
        move || {
            e.set(true);
            false
        }
    });

    // Frame 1: overlapping.
    physics.begin_frame();
    physics.perform_calls();
    assert!(!exited.get(), "no Exit on the frame the pair is still touching");

    // Frame 2: drive `b` away, still inside the same screen quadrant.
    physics.update_world_shape(b, quad(330.0, 180.0, 60.0, 60.0));
    physics.begin_frame();
    physics.perform_calls();
    assert!(exited.get(), "Exit must fire once a world-space pair separates");
}

// ---------------------------------------------------------------------------
// World-space circles keep their position when projected
//
// `project_shape` used to scale a circle's diameter and drop its position, so
// every `ColliderSpace::World` circle landed at screen origin whatever its
// world position.
// ---------------------------------------------------------------------------

/// A world circle at (1000, 500) under `translate(100, 50) * scale(0.5)`
/// projects to screen (600, 300), diameter 20.
fn world_circle_physics() -> (PhysicsProvider, u32, Rc<Cell<bool>>) {
    let mut physics = PhysicsProvider::new();
    physics.resize_screen(1280.0, 720.0);
    physics.set_world_to_screen(
        Mat4::from_translation(Vec3::new(100.0, 50.0, 0.0))
            * Mat4::from_scale(Vec3::new(0.5, 0.5, 1.0)),
    );
    let pid = physics.register_collider(ColliderData {
        position: Vec3::new(1000.0, 500.0, 0.0),
        ..ColliderData::world(Shape::Circle { diameter: 40.0 })
    });
    let clicked = Rc::new(Cell::new(false));
    physics.add_collider_handler(pid, HandlerKind::Click, {
        let cl = Rc::clone(&clicked);
        move || {
            cl.set(true);
            false
        }
    });
    (physics, pid, clicked)
}

#[test]
fn world_circle_is_hit_at_its_projected_position() {
    let (mut physics, pid, clicked) = world_circle_physics();
    physics.mouse.position = Vec3::new(605.0, 302.0, 0.0);
    physics.mouse_clicked = true;
    physics.begin_frame();
    assert!(physics.gjk_test(0, pid), "mouse over the projected centre must hit the circle");
    physics.perform_calls();
    assert!(clicked.get(), "Click must fire on a world circle at its projected position");
}

#[test]
fn world_circle_is_not_hit_at_screen_origin() {
    let (mut physics, pid, clicked) = world_circle_physics();
    physics.mouse.position = Vec3::new(2.0, 2.0, 0.0);
    physics.mouse_clicked = true;
    physics.begin_frame();
    assert!(!physics.gjk_test(0, pid), "a world circle must not project to screen origin");
    physics.perform_calls();
    assert!(!clicked.get());
}
