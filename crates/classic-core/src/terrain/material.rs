//! Lunar surface material classes.
//!
//! This table is the single source of truth shared by the terrain generator
//! (which classifies each tile into a material) and the tileset generator
//! (which paints the pixels for that material).  Keeping them in one place is
//! what stops the two from drifting apart.
//!
//! Tile id `0` is deliberately never assigned: `build_mesh` skips any tile
//! whose id is `0` *and* whose four corner heights are all zero, which would
//! punch a hole in the map.  Ids therefore start at `1`.

/// Surface material class assigned to a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LunarMaterial {
    /// Flat mare basalt plain — the primary buildable surface.
    MareSmooth,
    /// Darker, fresher mare basalt patches.
    MareDark,
    /// Ordinary highland regolith.
    Regolith,
    /// Coarser, blockier regolith on moderate slopes.
    RegolithCoarse,
    /// Steep exposed bedrock / boulder fields.
    Rocky,
    /// Shadowed, dust-filled crater floor.
    CraterFloor,
    /// Freshly excavated bright material on a crater rim.
    RimBright,
    /// High-albedo ejecta ray streak from a young crater.
    Ray,
    /// Smooth compacted dust of a landing zone.
    LandingPad,
}

/// Appearance parameters for one material class.
#[derive(Clone, Copy, Debug)]
pub struct MaterialSpec {
    pub material: LunarMaterial,
    /// Number of visually distinct tiles generated for this class.  Multiple
    /// variants break up the 32px repetition when large areas share a class.
    pub variants: u32,
    /// Base albedo as 8-bit RGB.  Regolith is a very slightly warm grey, so
    /// R > G > B by a couple of counts throughout.
    pub albedo: [u8; 3],
    /// Peak-to-peak amplitude of the noise speckle, in 8-bit counts.
    pub speckle: f32,
    /// Speckle cycles across one tile.
    pub speckle_freq: f64,
    /// Number of microcraters stamped into each tile of this class.
    pub craterlets: u32,
}

/// All material classes, in tile-id order.
pub const MATERIALS: &[MaterialSpec] = &[
    MaterialSpec {
        material: LunarMaterial::MareSmooth,
        variants: 3,
        albedo: [94, 92, 89],
        speckle: 10.0,
        speckle_freq: 3.0,
        craterlets: 2,
    },
    MaterialSpec {
        material: LunarMaterial::MareDark,
        variants: 3,
        albedo: [76, 75, 74],
        speckle: 8.0,
        speckle_freq: 2.0,
        craterlets: 1,
    },
    MaterialSpec {
        material: LunarMaterial::Regolith,
        variants: 4,
        albedo: [150, 146, 140],
        speckle: 20.0,
        speckle_freq: 4.0,
        craterlets: 4,
    },
    MaterialSpec {
        material: LunarMaterial::RegolithCoarse,
        variants: 3,
        albedo: [166, 161, 154],
        speckle: 30.0,
        speckle_freq: 6.0,
        craterlets: 6,
    },
    MaterialSpec {
        material: LunarMaterial::Rocky,
        variants: 3,
        albedo: [192, 188, 181],
        speckle: 42.0,
        speckle_freq: 8.0,
        craterlets: 3,
    },
    MaterialSpec {
        material: LunarMaterial::CraterFloor,
        variants: 2,
        albedo: [88, 86, 86],
        speckle: 9.0,
        speckle_freq: 2.5,
        craterlets: 2,
    },
    MaterialSpec {
        material: LunarMaterial::RimBright,
        variants: 2,
        albedo: [190, 186, 180],
        speckle: 26.0,
        speckle_freq: 5.0,
        craterlets: 2,
    },
    MaterialSpec {
        material: LunarMaterial::Ray,
        variants: 2,
        albedo: [238, 235, 230],
        speckle: 16.0,
        speckle_freq: 3.5,
        craterlets: 1,
    },
    MaterialSpec {
        material: LunarMaterial::LandingPad,
        variants: 2,
        // Deliberately close to plain regolith: the pad is a dust-filled
        // basin floor, not a paved apron.  Its flatness is the feature, and a
        // high-contrast albedo just makes the tile-quantised rim obvious.
        albedo: [124, 121, 117],
        speckle: 17.0,
        speckle_freq: 3.0,
        craterlets: 3,
    },
];

/// Total number of generated tiles across all material classes.
pub fn tile_count() -> u32 {
    MATERIALS.iter().map(|m| m.variants).sum()
}

/// First tile id belonging to `material` (ids are 1-based).
pub fn base_tile_id(material: LunarMaterial) -> u32 {
    let mut id = 1;
    for spec in MATERIALS {
        if spec.material == material {
            return id;
        }
        id += spec.variants;
    }
    1
}

/// Tile id for a `(material, variant)` pair.  `variant` wraps modulo the
/// number of variants the material declares.
pub fn tile_id(material: LunarMaterial, variant: u32) -> u32 {
    let spec = spec_for(material);
    base_tile_id(material) + variant % spec.variants.max(1)
}

/// Look up the spec for a material class.
pub fn spec_for(material: LunarMaterial) -> &'static MaterialSpec {
    MATERIALS.iter().find(|m| m.material == material).unwrap_or(&MATERIALS[0])
}

/// Reverse lookup: which `(material, variant)` does a tile id denote?
pub fn material_for_tile_id(id: u32) -> Option<(LunarMaterial, u32)> {
    if id == 0 {
        return None;
    }
    let mut base = 1;
    for spec in MATERIALS {
        if id < base + spec.variants {
            return Some((spec.material, id - base));
        }
        base += spec.variants;
    }
    None
}
