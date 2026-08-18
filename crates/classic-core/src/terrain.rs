//! Re-export of the [`classic_terrain`] noise toolkit (see that crate's docs).
//!
//! The open terrain/noise primitives live in the `#![no_std]` `classic-terrain`
//! crate so ROM guests can link them; this module keeps `classic_core::terrain::*`
//! working for the host engine and demo.  Map-specific generators (e.g. the
//! lunar algorithm) live in ROM guest code, not here.

pub use classic_terrain::fractal;
pub use classic_terrain::kernels;
pub use classic_terrain::noise_fields;
