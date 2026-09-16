//! Dynamic lights: the std140 `LightBlock` uniform buffer.

use classic_core::components::Light;

use crate::GlBuffer;

// ---------------------------------------------------------------------------
// Dynamic lights (std140 uniform block)
// ---------------------------------------------------------------------------

/// Maximum number of dynamic lights uploadable to the `LightBlock` UBO.
/// The block is 3 `vec4`s per light, so `256` lights occupy
/// `16 + 256 * 48 = 12304` bytes — comfortably within the WebGL2-guaranteed
/// 16 KB `MAX_UNIFORM_BLOCK_SIZE`.
pub const MAX_LIGHTS: usize = 256;

/// The UBO binding point shared by every shader that declares `LightBlock`.
pub const LIGHT_UBO_BINDING: u32 = 1;

/// Edge length of the square directional shadow map (depth texture).
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// Slope-scaled depth offset applied to sprite billboard shadow casters, in
/// OpenGL polygon-offset factor units.  See `Gfx::set_shadow_sprite_offset`.
pub const SHADOW_SPRITE_SLOPE_OFFSET: f32 = 4.0;

/// Constant depth offset paired with `SHADOW_SPRITE_SLOPE_OFFSET`.
pub const SHADOW_SPRITE_UNIT_OFFSET: f32 = 8.0;

/// Texture unit the directional shadow map is bound to in the lit shaders.
/// Tilemap uses units 0/1, sprites use 0/1/2 — unit 3 is free for both.
pub const SHADOW_MAP_UNIT: u32 = 3;

/// Pack a slice of [`Light`]s into the flat `f32` buffer consumed by the
/// `LightBlock` `std140` uniform block:
///
/// ```text
/// offset 0        : vec4 count            (x = active light count)
/// per light i     : vec4 pos_radius       (xyz = position, w = radius)
///                 : vec4 color_intensity  (rgb = color, a = intensity)
///                 : vec4 dir_cone         (xyz = direction, w = cone_angle)
/// ```
///
/// `cone_angle <= 0` is the point-light sentinel (the shader skips the cone
/// term), so both `LightKind::Point` and `LightKind::Spot` share one layout.
/// The returned buffer is always `(1 + MAX_LIGHTS * 3) * 4` floats; trailing
/// lights beyond `capacity` are silently dropped.
pub fn pack_lights(lights: &[Light], capacity: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; (1 + capacity * 3) * 4];
    out[0] = (lights.len().min(capacity)) as f32;
    for (i, l) in lights.iter().take(capacity).enumerate() {
        let base = (1 + i * 3) * 4;
        out[base..base + 4].copy_from_slice(&[l.position.x, l.position.y, l.position.z, l.radius]);
        out[base + 4..base + 8].copy_from_slice(&[l.color[0], l.color[1], l.color[2], l.intensity]);
        let cone =
            if l.kind == classic_core::components::LightKind::Point { 0.0 } else { l.cone_angle };
        out[base + 8..base + 12].copy_from_slice(&[l.dir.x, l.dir.y, l.dir.z, cone]);
    }
    out
}

/// A host-side UBO backing the `LightBlock` uniform block.  Owns the CPU-side
/// capacity and the GPU buffer; uploaded once per frame by [`Gfx::upload_lights`].
pub struct LightBuffer {
    buffer: GlBuffer,
    capacity: usize,
}

impl LightBuffer {
    pub fn new(gl: &glow::Context, capacity: usize) -> Self {
        let floats = (1 + capacity * 3) * 4;
        let zeros = vec![0.0f32; floats];
        let buffer = GlBuffer::from_slice(gl, glow::UNIFORM_BUFFER, &zeros, glow::DYNAMIC_DRAW);
        Self { buffer, capacity }
    }

    /// Upload the packed light block and bind it to [`LIGHT_UBO_BINDING`].
    pub fn upload(&self, gl: &glow::Context, lights: &[Light]) {
        let data = pack_lights(lights, self.capacity);
        self.buffer.sub_data(gl, &data);
        self.buffer.bind_base(gl, LIGHT_UBO_BINDING);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::{Light, LightKind};

    #[test]
    fn pack_lights_std140_layout() {
        let lights = vec![Light {
            kind: LightKind::Point,
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            color: [0.1, 0.2, 0.3],
            intensity: 4.0,
            radius: 50.0,
            dir: glam::Vec3::new(5.0, 6.0, 7.0),
            cone_angle: 0.5,
            parent: None,
        }];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(buf.len(), (1 + MAX_LIGHTS * 3) * 4);
        // count vec4
        assert_eq!(buf[0], 1.0);
        assert_eq!(buf[1], 0.0);
        // light 0: [pos.xyz | radius]
        assert_eq!(&buf[4..8], &[1.0, 2.0, 3.0, 50.0]);
        // light 0: [color.rgb | intensity]
        assert_eq!(&buf[8..12], &[0.1, 0.2, 0.3, 4.0]);
        // light 0: [dir.xyz | cone]; a Point light forces cone_angle to 0.
        assert_eq!(&buf[12..16], &[5.0, 6.0, 7.0, 0.0]);
    }

    #[test]
    fn pack_lights_spot_keeps_cone_angle() {
        let lights = vec![Light {
            kind: LightKind::Spot,
            dir: glam::Vec3::new(0.0, 0.0, 1.0),
            cone_angle: 0.7,
            ..Default::default()
        }];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(&buf[12..16], &[0.0, 0.0, 1.0, 0.7]);
    }

    #[test]
    fn pack_lights_truncates_beyond_capacity() {
        let lights = vec![Light::default(); MAX_LIGHTS + 5];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(buf[0], MAX_LIGHTS as f32);
    }
}
