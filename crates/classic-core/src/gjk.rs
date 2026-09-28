//! # Skill: `classic-physics`
//!
//! **Read `.agents/skills/classic-physics/SKILL.md` before working on this module.**
//!
//! GJK (Gilbert-Johnson-Keerthi) collision detection.

use glam::Vec3;

/// A shape that can be used with the GJK algorithm.
pub trait GjkShape {
    fn center(&self) -> Vec3;
    fn support(&self, dir: Vec3) -> Option<Vec3>;
}

/// Result of one simplex-evolution step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolveResult {
    NoIntersection,
    Intersection,
    StillEvolving,
}

#[inline]
fn triple_product(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    a.cross(b).cross(c)
}

/// Squared length below which a search direction counts as degenerate.
const MIN_DIR_LEN_SQ: f32 = 1e-12;

/// Direction used when the two shape centres coincide.  Any fixed axis works
/// as GJK's first guess; what matters is that it is non-zero, because
/// `Shape::support` normalizes `dir` for a circle, so a zero `dir` yields a
/// NaN support point — which then reads as "no intersection".
const FALLBACK_DIR: Vec3 = Vec3::X;

/// GJK algorithm state machine.
pub struct GjkContext<'a> {
    shape_a: &'a dyn GjkShape,
    shape_b: &'a dyn GjkShape,
    direction: Vec3,
    pub verts: Vec<Vec3>,
}

impl<'a> GjkContext<'a> {
    pub fn new(shape_a: &'a dyn GjkShape, shape_b: &'a dyn GjkShape) -> Self {
        Self { shape_a, shape_b, direction: Vec3::ZERO, verts: Vec::new() }
    }

    /// Add a Minkowski-difference support point in the given direction.
    /// Returns `false` if either shape returns `None` from `support`, or
    /// if the new point does not reach past the origin.
    fn add_support(&mut self, dir: Vec3) -> bool {
        let n_dir = -dir;

        let sup_a = self.shape_a.support(dir);
        let sup_b = self.shape_b.support(n_dir);

        let sup_a = match sup_a {
            Some(v) => v,
            None => return false,
        };
        let sup_b = match sup_b {
            Some(v) => v,
            None => return false,
        };

        let diff = sup_a - sup_b;
        self.verts.push(diff);
        dir.dot(diff) >= 0.0
    }

    /// Evolve the simplex one step.
    pub fn evolve_simplex(&mut self) -> EvolveResult {
        match self.verts.len() {
            0 => {
                let delta = self.shape_b.center() - self.shape_a.center();
                // Concentric shapes give a zero delta — fall back to a fixed
                // axis rather than searching along nothing.
                self.direction =
                    if delta.length_squared() > MIN_DIR_LEN_SQ { delta } else { FALLBACK_DIR };
            }
            1 => {
                self.direction = -self.direction;
            }
            2 => {
                let b = self.verts[1];
                let c = self.verts[0];
                let cb = b - c;
                let c0 = -c;
                let perp = triple_product(cb, c0, cb);
                if perp.length_squared() <= MIN_DIR_LEN_SQ {
                    // `c` and `b` were found along opposite directions, so the
                    // origin already lies between them; a degenerate perp means
                    // it sits *on* segment CB and is therefore inside the
                    // Minkowski difference.  Searching along a zero direction
                    // here is what made concentric shapes read as disjoint.
                    return EvolveResult::Intersection;
                }
                self.direction = perp;
            }
            3 => {
                let a = self.verts[2];
                let b = self.verts[1];
                let c = self.verts[0];

                let a0 = -a;
                let ab = b - a;
                let ac = c - a;

                let ab_perp = triple_product(ac, ab, ab);
                let ac_perp = triple_product(ab, ac, ac);

                if ab_perp.dot(a0) > 0.0 {
                    self.verts.remove(0); // drop c
                    self.direction = ab_perp;
                } else if ac_perp.dot(a0) > 0.0 {
                    self.verts.remove(1); // drop b
                    self.direction = ac_perp;
                } else {
                    return EvolveResult::Intersection;
                }
            }
            _ => panic!("GJK: only 2D simplex supported"),
        }

        if self.add_support(self.direction) {
            EvolveResult::StillEvolving
        } else {
            EvolveResult::NoIntersection
        }
    }

    /// Run the full GJK test.
    /// Returns `true` if the two shapes intersect.
    pub fn perform_test(&mut self) -> bool {
        let max_iter = 1000;

        for _ in 0..max_iter {
            let res = self.evolve_simplex();
            match res {
                EvolveResult::StillEvolving => continue,
                EvolveResult::Intersection => return true,
                EvolveResult::NoIntersection => return false,
            }
        }

        panic!("GJK: max iterations ({max_iter}) reached");
    }
}
