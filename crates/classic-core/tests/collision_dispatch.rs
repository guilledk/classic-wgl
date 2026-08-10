use std::cell::Cell;

use classic_core::collision::{HandlerKind, PhysicsProvider};
use classic_core::components::{Collider, Shape};

#[test]
fn registers_and_retrieves_collider() {
    let mut physics = PhysicsProvider::new();
    let c = Collider::new(Shape::Circle { diameter: 10.0 });
    let pid = physics.register_collider(c);
    assert!(pid >= 2);
}

#[test]
fn gjk_detects_overlapping_circles() {
    let mut physics = PhysicsProvider::new();
    let pid1 = physics.register_collider(Collider {
        position: glam::Vec3::new(100.0, 100.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 20.0 })
    });
    let pid2 = physics.register_collider(Collider {
        position: glam::Vec3::new(105.0, 102.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 20.0 })
    });
    assert!(physics.gjk_test(pid1, pid2));
}

#[test]
fn gjk_detects_disjoint_circles() {
    let mut physics = PhysicsProvider::new();
    let pid1 = physics.register_collider(Collider {
        position: glam::Vec3::new(100.0, 100.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 10.0 })
    });
    let pid2 = physics.register_collider(Collider {
        position: glam::Vec3::new(500.0, 400.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 10.0 })
    });
    assert!(!physics.gjk_test(pid1, pid2));
}

#[test]
fn gjk_mouse_vs_collider() {
    let mut physics = PhysicsProvider::new();
    let pid = physics.register_collider(Collider {
        position: glam::Vec3::new(200.0, 150.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 30.0 })
    });
    physics.mouse.position = glam::Vec3::new(203.0, 152.0, 0.0);
    assert!(physics.gjk_test(0, pid));
}

#[test]
fn click_does_not_fire_without_mouse_clicked() {
    let mut physics = PhysicsProvider::new();
    physics.resize_screen(800.0, 600.0);

    let clicked = Cell::new(false);
    let mut c = Collider {
        position: glam::Vec3::new(200.0, 150.0, 0.0),
        ..Collider::new(Shape::Circle { diameter: 30.0 })
    };
    c.add_handler(HandlerKind::Click, {
        let cl = clicked.clone();
        move || {
            cl.set(true);
            false
        }
    });
    physics.register_collider(c);

    physics.mouse.position = glam::Vec3::new(203.0, 152.0, 0.0);
    physics.mouse_clicked = false;
    physics.begin_frame();
    physics.perform_calls();
    assert!(!clicked.get());
}
