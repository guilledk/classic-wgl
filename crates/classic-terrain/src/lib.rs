//! Procedural noise primitives for terrain generation.
//!
//! `#![no_std]` (with `alloc`) so it can compile into ROM guest `.wasm`
//! modules as well as the native host.  Everything here is a pure function of
//! `(seed, dims, params)` — no system clock, no GL — so output is reproducible
//! across targets and stable for golden traces.
//!
//! This is the *open* terrain/noise toolkit: it has no map-specific knowledge.
//! Map algorithms (e.g. the lunar generator) live in the ROM guest code and
//! build on these primitives.
//!
//! - [`simplex_noise`] — seedable 2D/3D/4D simplex + a deterministic [`Random`].
//! - [`fractal`] — multi-octave combinators over `simplex_noise`.
//! - [`noise_fields`] — bulk field-fill helpers behind the host SDK.

#![no_std]

extern crate alloc;

pub mod fractal;
pub mod kernels;
pub mod noise_fields;
pub mod simplex_noise;
