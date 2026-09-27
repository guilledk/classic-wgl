use glam::Vec3;

use classic_core::collision::{polygon_from_verts, PhysicsProvider};
use classic_core::components::{ColliderData, Shape};
use classic_core::gjk::{GjkContext, GjkShape};

/// A simple unit-square shape for testing.
struct UnitSquare {
    pos: Vec3,
    scale: Vec3,
}

impl UnitSquare {
    fn new(x: f32, y: f32) -> Self {
        Self { pos: Vec3::new(x, y, 0.0), scale: Vec3::new(1.0, 1.0, 1.0) }
    }

    fn vertices(&self) -> [Vec3; 4] {
        [
            Vec3::new(self.pos.x, self.pos.y, 0.0),
            Vec3::new(self.pos.x + self.scale.x, self.pos.y, 0.0),
            Vec3::new(self.pos.x + self.scale.x, self.pos.y + self.scale.y, 0.0),
            Vec3::new(self.pos.x, self.pos.y + self.scale.y, 0.0),
        ]
    }
}

impl GjkShape for UnitSquare {
    fn center(&self) -> Vec3 {
        self.pos + self.scale * 0.5
    }

    fn support(&self, dir: Vec3) -> Option<Vec3> {
        let verts = self.vertices();
        let mut best = verts[0];
        let mut best_dot = dir.dot(best);
        for v in &verts[1..] {
            let d = dir.dot(*v);
            if d > best_dot {
                best_dot = d;
                best = *v;
            }
        }
        Some(best)
    }
}

#[test]
fn support_returns_furthest_vertex() {
    let sq = UnitSquare::new(0.0, 0.0);
    let sqrt2 = 2.0f32.sqrt() / 2.0;

    // π/4 direction → furthest should be top-right (1,0,0)
    assert_eq!(sq.support(Vec3::new(sqrt2, -sqrt2, 0.0)), Some(Vec3::new(1.0, 0.0, 0.0)));
    // 3π/4 direction → furthest should be top-left (0,0,0)
    assert_eq!(sq.support(Vec3::new(-sqrt2, -sqrt2, 0.0)), Some(Vec3::new(0.0, 0.0, 0.0)));
    // 5π/4 direction → furthest should be bottom-left (0,1,0)
    assert_eq!(sq.support(Vec3::new(-sqrt2, sqrt2, 0.0)), Some(Vec3::new(0.0, 1.0, 0.0)));
    // 7π/4 direction → furthest should be bottom-right (1,1,0)
    assert_eq!(sq.support(Vec3::new(sqrt2, sqrt2, 0.0)), Some(Vec3::new(1.0, 1.0, 0.0)));
}

#[test]
fn support_accounts_for_position_offset() {
    let sq = UnitSquare::new(5.0, 5.0);
    assert_eq!(sq.support(Vec3::new(1.0, 0.0, 0.0)), Some(Vec3::new(6.0, 5.0, 0.0)));
}

#[test]
fn detects_overlapping_squares() {
    let a = UnitSquare::new(0.0, 0.0);
    let b = UnitSquare::new(0.5, 0.5);
    assert!(GjkContext::new(&a, &b).perform_test());
}

#[test]
fn detects_no_collision_between_disjoint() {
    let a = UnitSquare::new(0.0, 0.0);
    let b = UnitSquare::new(10.0, 10.0);
    assert!(!GjkContext::new(&a, &b).perform_test());
}

#[test]
fn detects_containment() {
    // Big square at (0,0) scale 10,10 — contains small at (4,4) scale 1,1
    struct BigSquare {
        pos: Vec3,
        scale: Vec3,
    }
    impl GjkShape for BigSquare {
        fn center(&self) -> Vec3 {
            self.pos + self.scale * 0.5
        }
        fn support(&self, dir: Vec3) -> Option<Vec3> {
            let hw = self.scale.x / 2.0;
            let hh = self.scale.y / 2.0;
            let c = self.center();
            Some(Vec3::new(c.x + hw * dir.x.signum(), c.y + hh * dir.y.signum(), 0.0))
        }
    }

    let big = BigSquare { pos: Vec3::new(0.0, 0.0, 0.0), scale: Vec3::new(10.0, 10.0, 1.0) };
    let small = UnitSquare::new(4.0, 4.0);

    assert!(GjkContext::new(&big, &small).perform_test());
}

#[test]
fn touching_edges_are_colliding() {
    let a = UnitSquare::new(0.0, 0.0);
    let b = UnitSquare::new(1.0, 0.0);
    assert!(GjkContext::new(&a, &b).perform_test());
}

#[test]
fn symmetric() {
    let a = UnitSquare::new(0.0, 0.0);
    let b = UnitSquare::new(0.5, 0.5);
    assert_eq!(GjkContext::new(&a, &b).perform_test(), GjkContext::new(&b, &a).perform_test());
}

#[test]
#[should_panic(expected = "only 2D simplex supported")]
fn panics_on_4d_simplex() {
    let a = UnitSquare::new(0.0, 0.0);
    let b = UnitSquare::new(0.5, 0.5);
    let mut ctx = GjkContext::new(&a, &b);
    ctx.verts.push(Vec3::ZERO);
    ctx.verts.push(Vec3::ZERO);
    ctx.verts.push(Vec3::ZERO);
    ctx.verts.push(Vec3::ZERO);
    ctx.evolve_simplex();
}

// ---------------------------------------------------------------------------
// Concentric shapes
//
// A zero centre-delta used to leave the first search direction at `Vec3::ZERO`.
// `Shape::support` normalizes `dir` for a circle, so that produced a NaN
// support point, `dir.dot(diff) >= 0.0` was false, and two fully overlapping
// shapes reported *no* collision.
// ---------------------------------------------------------------------------

/// A circle usable directly as a `GjkShape`, going through the real
/// `Shape::support` implementation (the one that normalizes `dir`).
struct Circle {
    pos: Vec3,
    diameter: f32,
}

impl GjkShape for Circle {
    fn center(&self) -> Vec3 {
        self.pos
    }

    fn support(&self, dir: Vec3) -> Option<Vec3> {
        Shape::Circle { diameter: self.diameter }.support(self.pos, Vec3::ONE, dir)
    }
}

#[test]
fn detects_concentric_circles() {
    let a = Circle { pos: Vec3::new(100.0, 100.0, 0.0), diameter: 20.0 };
    let b = Circle { pos: Vec3::new(100.0, 100.0, 0.0), diameter: 10.0 };
    assert!(GjkContext::new(&a, &b).perform_test());
    // ...and the other way round, since the first direction is derived from
    // `b.center() - a.center()`.
    assert!(GjkContext::new(&b, &a).perform_test());
}

#[test]
fn detects_identical_circles() {
    let a = Circle { pos: Vec3::new(-3.5, 12.0, 0.0), diameter: 8.0 };
    let b = Circle { pos: Vec3::new(-3.5, 12.0, 0.0), diameter: 8.0 };
    assert!(GjkContext::new(&a, &b).perform_test());
}

#[test]
fn detects_concentric_circle_and_polygon() {
    // Unit square centred on the origin, so it shares the circle's centre.
    let square = polygon_from_verts(vec![
        Vec3::new(-0.5, -0.5, 0.0),
        Vec3::new(0.5, -0.5, 0.0),
        Vec3::new(0.5, 0.5, 0.0),
        Vec3::new(-0.5, 0.5, 0.0),
    ]);
    let poly = ShapeAt { shape: square, pos: Vec3::ZERO };
    let circle = Circle { pos: Vec3::ZERO, diameter: 4.0 };

    assert!(GjkContext::new(&circle, &poly).perform_test());
    assert!(GjkContext::new(&poly, &circle).perform_test());
}

/// A `Shape` placed at a position, as a `GjkShape`.
struct ShapeAt {
    shape: Shape,
    pos: Vec3,
}

impl GjkShape for ShapeAt {
    fn center(&self) -> Vec3 {
        self.shape.center(self.pos, Vec3::ONE)
    }

    fn support(&self, dir: Vec3) -> Option<Vec3> {
        self.shape.support(self.pos, Vec3::ONE, dir)
    }
}

#[test]
fn circle_support_of_zero_dir_is_finite() {
    let s = Shape::Circle { diameter: 6.0 };
    let p = s.support(Vec3::new(7.0, -2.0, 0.0), Vec3::ONE, Vec3::ZERO).unwrap();
    assert!(p.is_finite(), "zero-dir support must not be NaN, got {p:?}");
}

#[test]
fn concentric_colliders_collide_through_the_provider() {
    let mut physics = PhysicsProvider::new();
    let a = physics.register_collider(ColliderData {
        position: Vec3::new(100.0, 100.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 20.0 })
    });
    let b = physics.register_collider(ColliderData {
        position: Vec3::new(100.0, 100.0, 0.0),
        ..ColliderData::new(Shape::Circle { diameter: 10.0 })
    });
    assert!(physics.gjk_test(a, b));
    assert!(physics.gjk_test(b, a));
}
