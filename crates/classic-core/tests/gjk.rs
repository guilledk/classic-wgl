use glam::Vec3;

use classic_core::gjk::{GjkContext, GjkShape};

/// A simple unit-square shape for testing, matching the TS test helpers.
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
