//! Host-owned field-buffer registry: named intermediate grids ROM guests can
//! allocate, seed, transform with [`crate::terrain::kernels`], and download.
//!
//! The whole point is that grids never round-trip through guest linear memory
//! mid-generation: the guest allocates a field by name, drives kernels over it
//! by name, and only downloads the final grids.  This sidesteps the SAB bridge's
//! bulk-payload cap on web and keeps the heavy O(n²) loops host-side.
//!
//! `FieldRegistry` is deliberately `Send` and independent of the engine so the
//! background guest worker (Tier 3) can own its own scratch registry off the
//! render thread.

use std::collections::HashMap;

use crate::terrain::kernels;

/// The element type of a registered field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldDtype {
    F32,
    U32,
}

impl FieldDtype {
    /// Decode the ABI dtype code (`0` = f32, `1` = u32).
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => FieldDtype::U32,
            _ => FieldDtype::F32,
        }
    }
}

/// A named host-resident grid.
#[derive(Clone, Debug)]
pub enum Field {
    F32 { w: i32, h: i32, data: Vec<f32> },
    U32 { w: i32, h: i32, data: Vec<u32> },
}

impl Field {
    pub fn dtype(&self) -> FieldDtype {
        match self {
            Field::F32 { .. } => FieldDtype::F32,
            Field::U32 { .. } => FieldDtype::U32,
        }
    }

    pub fn dims(&self) -> (i32, i32) {
        match self {
            Field::F32 { w, h, .. } | Field::U32 { w, h, .. } => (*w, *h),
        }
    }
}

/// The named-field registry itself, plus the kernel dispatch that operates on
/// those fields.  Pure Rust, no GL — fully unit-testable.
#[derive(Default)]
pub struct FieldRegistry {
    fields: HashMap<String, Field>,
}

impl FieldRegistry {
    // ---- lifecycle -------------------------------------------------------

    /// Allocate a zero-filled `w`×`h` field under `name` (replacing any field
    /// of the same name).  Returns `false` on non-positive dimensions.
    pub fn alloc(&mut self, name: &str, w: i32, h: i32, dtype: FieldDtype) -> bool {
        if w <= 0 || h <= 0 {
            return false;
        }
        let len = (w * h) as usize;
        let field = match dtype {
            FieldDtype::F32 => Field::F32 { w, h, data: vec![0.0f32; len] },
            FieldDtype::U32 => Field::U32 { w, h, data: vec![0u32; len] },
        };
        self.fields.insert(name.to_string(), field);
        true
    }

    /// Remove a field by name.  Returns whether it existed.
    pub fn free(&mut self, name: &str) -> bool {
        self.fields.remove(name).is_some()
    }

    /// Overwrite a field's data (dims must already match the allocated field).
    pub fn write(&mut self, name: &str, data: &[f32]) -> bool {
        match self.fields.get_mut(name) {
            Some(Field::F32 { data: dst, .. }) if dst.len() == data.len() => {
                dst.copy_from_slice(data);
                true
            }
            _ => false,
        }
    }

    /// Overwrite a `u32` field's data.
    pub fn write_u32(&mut self, name: &str, data: &[u32]) -> bool {
        match self.fields.get_mut(name) {
            Some(Field::U32 { data: dst, .. }) if dst.len() == data.len() => {
                dst.copy_from_slice(data);
                true
            }
            _ => false,
        }
    }

    // ---- accessors -------------------------------------------------------

    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }

    pub fn f32(&self, name: &str) -> Option<(&[f32], i32, i32)> {
        match self.fields.get(name)? {
            Field::F32 { w, h, data } => Some((data, *w, *h)),
            Field::U32 { .. } => None,
        }
    }

    pub fn f32_mut(&mut self, name: &str) -> Option<(&mut [f32], i32, i32)> {
        match self.fields.get_mut(name)? {
            Field::F32 { w, h, data } => Some((data, *w, *h)),
            Field::U32 { .. } => None,
        }
    }

    pub fn u32(&self, name: &str) -> Option<(&[u32], i32, i32)> {
        match self.fields.get(name)? {
            Field::U32 { w, h, data } => Some((data, *w, *h)),
            Field::F32 { .. } => None,
        }
    }

    pub fn u32_mut(&mut self, name: &str) -> Option<(&mut [u32], i32, i32)> {
        match self.fields.get_mut(name)? {
            Field::U32 { w, h, data } => Some((data, *w, *h)),
            Field::F32 { .. } => None,
        }
    }

    /// Install (or replace) a field, used by kernels that produce a new grid
    /// of derived dimensions (e.g. `gradient_magnitude`).
    fn install(&mut self, name: &str, field: Field) {
        self.fields.insert(name.to_string(), field);
    }

    // ---- kernel dispatch ---------------------------------------------------

    /// In-place `dst = dst op src` over two `f32` fields of equal size.
    pub fn map_field(&mut self, op: kernels::FieldOp, dst: &str, src: &str) -> bool {
        let src_data = match self.get(src) {
            Some(Field::F32 { data, .. }) => data.clone(),
            _ => return false,
        };
        let (d, _, _) = match self.f32_mut(dst) {
            Some(v) => v,
            None => return false,
        };
        if d.len() != src_data.len() {
            return false;
        }
        kernels::map_field(op, d, &src_data);
        true
    }

    /// In-place `dst = dst op scalar` over an `f32` field.
    pub fn map_scalar(&mut self, op: kernels::FieldOp, dst: &str, scalar: f32) -> bool {
        let Some((d, _, _)) = self.f32_mut(dst) else { return false };
        kernels::map_scalar(op, d, scalar);
        true
    }

    /// In-place N×N box blur of an `f32` field.
    pub fn blur_box(&mut self, name: &str, radius: i32) -> bool {
        let (data, w, h) = match self.f32(name) {
            Some(v) => v,
            None => return false,
        };
        let blurred = kernels::blur_box(data, w, h, radius);
        self.write(name, &blurred)
    }

    /// In-place slope relaxation of an `f32` field; `pinned` is an optional
    /// `u32` field name marking fixed cells.  Returns `(iterations, worst)`.
    pub fn relax_slopes(
        &mut self,
        name: &str,
        max_slope: f32,
        iterations: u32,
        tolerance: f32,
        pinned: Option<&str>,
    ) -> Option<(u32, f32)> {
        // Fetch the pinned mask first (owned) so it does not borrow `self`
        // across the mutable `data` borrow below.
        let pinned_mask: Option<(Vec<bool>, (i32, i32))> = match pinned {
            Some(p) => {
                let (m, pw, ph) = self.u32(p)?;
                Some((m.iter().map(|&v| v != 0).collect(), (pw, ph)))
            }
            None => None,
        };
        let (data, w, h) = self.f32_mut(name)?;
        let result = match pinned_mask {
            Some((bools, (pw, ph))) => {
                if (pw, ph) != (w, h) {
                    return None;
                }
                kernels::relax_slopes(
                    data,
                    w as usize,
                    h as usize,
                    max_slope,
                    iterations,
                    tolerance,
                    Some(&bools),
                )
            }
            None => kernels::relax_slopes(
                data, w as usize, h as usize, max_slope, iterations, tolerance, None,
            ),
        };
        Some(result)
    }

    /// Derive a per-tile `f32` gradient field (`w`×`h`) from an `f32` vertex
    /// height field (`(w+1)`×`(h+1)`), installed under `dst`.
    pub fn gradient_magnitude(&mut self, heights: &str, dst: &str) -> bool {
        let (data, w, h) = match self.f32(heights) {
            Some(v) => v,
            None => return false,
        };
        let grad = kernels::gradient_magnitude(data, w - 1, h - 1);
        self.install(dst, Field::F32 { w: w - 1, h: h - 1, data: grad });
        true
    }

    /// Threshold an `f32` field into a `u32` field (`1` where `<= t`) under `dst`.
    pub fn threshold_le(&mut self, src: &str, dst: &str, t: f32) -> bool {
        let (data, w, h) = match self.f32(src) {
            Some(v) => v,
            None => return false,
        };
        let nav = kernels::threshold_le(data, t);
        self.install(dst, Field::U32 { w, h, data: nav });
        true
    }

    /// Prune every walkable cell not in the largest component of a `u32` field.
    pub fn prune_components(&mut self, name: &str) -> bool {
        let (data, w, h) = match self.u32_mut(name) {
            Some(v) => v,
            None => return false,
        };
        kernels::prune_to_main_component(data, w, h);
        true
    }

    /// Reduce an `f32` field to a single statistic.
    pub fn reduce(&self, name: &str, op: kernels::Reduce) -> Option<f32> {
        let (data, _, _) = self.f32(name)?;
        Some(kernels::reduce_field(data, op))
    }

    /// Stamp a radial profile into an `f32` field.
    #[allow(clippy::too_many_arguments)]
    pub fn stamp_radial(
        &mut self,
        name: &str,
        cx: f32,
        cy: f32,
        radius: f32,
        amplitude: f32,
        op: kernels::FieldOp,
    ) -> bool {
        let (data, w, h) = match self.f32_mut(name) {
            Some(v) => v,
            None => return false,
        };
        kernels::stamp_radial(data, w, h, cx, cy, radius, amplitude, op);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::kernels::{FieldOp, Reduce};

    fn seed_height_field(reg: &mut FieldRegistry, name: &str, vals: &[f32], w: i32) {
        let h = vals.len() as i32 / w;
        assert!(reg.alloc(name, w, h, FieldDtype::F32));
        assert!(reg.write(name, vals));
    }

    #[test]
    fn alloc_write_read_roundtrip() {
        let mut reg = FieldRegistry::default();
        assert!(reg.alloc("h", 2, 2, FieldDtype::F32));
        assert!(reg.write("h", &[1.0, 2.0, 3.0, 4.0]));
        let (data, w, h) = reg.f32("h").unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(data, &[1.0, 2.0, 3.0, 4.0]);
        assert!(reg.free("h"));
        assert!(reg.f32("h").is_none());
    }

    #[test]
    fn map_field_and_scalar() {
        let mut reg = FieldRegistry::default();
        seed_height_field(&mut reg, "a", &[1.0, 2.0, 3.0], 3);
        seed_height_field(&mut reg, "b", &[10.0, 20.0, 30.0], 3);
        assert!(reg.map_field(FieldOp::Add, "a", "b"));
        assert_eq!(reg.f32("a").unwrap().0, &[11.0, 22.0, 33.0]);
        assert!(reg.map_scalar(FieldOp::Mul, "a", 2.0));
        assert_eq!(reg.f32("a").unwrap().0, &[22.0, 44.0, 66.0]);
    }

    #[test]
    fn blur_box_smooths() {
        let mut reg = FieldRegistry::default();
        seed_height_field(&mut reg, "h", &[0.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, 0.0, 0.0], 3);
        assert!(reg.blur_box("h", 1));
        let (data, _, _) = reg.f32("h").unwrap();
        assert_eq!(data[4], 1.0);
    }

    #[test]
    fn relax_slopes_bounds_a_spike() {
        let mut reg = FieldRegistry::default();
        let mut field = vec![0.0f32; 25];
        field[12] = 10.0;
        seed_height_field(&mut reg, "h", &field, 5);
        let (used, worst) = reg.relax_slopes("h", 1.0, 100, 0.01, None).unwrap();
        assert!(used > 0);
        assert!(worst < 1.05, "worst slope: {worst}");
    }

    #[test]
    fn gradient_and_threshold_derive_grids() {
        let mut reg = FieldRegistry::default();
        // A 4x4 vertex grid (so a 3x3 tile grid) ramping in x.
        let mut heights = vec![0.0f32; 16];
        for y in 0..4 {
            for x in 0..4 {
                heights[y * 4 + x] = x as f32;
            }
        }
        seed_height_field(&mut reg, "heights", &heights, 4);
        assert!(reg.gradient_magnitude("heights", "slopes"));
        let (grad, w, h) = reg.f32("slopes").unwrap();
        assert_eq!((w, h), (3, 3));
        assert!(grad.iter().all(|&g| (g - 1.0).abs() < 1e-6));

        assert!(reg.threshold_le("slopes", "nav", 0.5));
        let (nav, w, h) = reg.u32("nav").unwrap();
        assert_eq!((w, h), (3, 3));
        assert!(nav.iter().all(|&v| v == 0));
    }

    #[test]
    fn prune_and_reduce() {
        let mut reg = FieldRegistry::default();
        reg.alloc("nav", 3, 1, FieldDtype::U32);
        assert!(reg.write_u32("nav", &[1, 0, 1]));
        assert!(reg.prune_components("nav"));
        let (nav, _, _) = reg.u32("nav").unwrap();
        assert_eq!(nav.iter().filter(|&&v| v == 1).count(), 1);

        seed_height_field(&mut reg, "h", &[1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(reg.reduce("h", Reduce::Min), Some(1.0));
        assert_eq!(reg.reduce("h", Reduce::Max), Some(4.0));
        assert_eq!(reg.reduce("h", Reduce::Mean), Some(2.5));
    }
}
