//! Developer utility: generate a lunar map and print diagnostics plus ASCII
//! previews of the height field, materials and navigation grid.
//!
//! `cargo run -p classic-core --example dbg_lunar -- [seed] [size]`

use classic_core::terrain::lunar::*;
use classic_core::terrain::material::{material_for_tile_id, LunarMaterial};

fn main() {
    let mut args = std::env::args().skip(1);
    let seed = args.next().unwrap_or_else(|| "apollo".into());
    let size: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);

    let p = LunarParams { seed, size_x: size, size_y: size, ..Default::default() };
    let t0 = std::time::Instant::now();
    let t = generate_lunar(&p);
    let dt = t0.elapsed();

    println!("generated in {dt:?}");
    println!("{:#?}", t.stats);
    println!("zones: {:?}", t.landing_zones);
    println!("spawns: {:?}", t.spawn_points);

    let cols = 100usize.min(t.size_x as usize);
    let rows = 50usize.min(t.size_y as usize);
    let sx = t.size_x as usize;
    let vw = sx + 1;

    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for h in &t.heights {
        lo = lo.min(*h);
        hi = hi.max(*h);
    }

    println!("\nheight field ({lo:.2} .. {hi:.2}), ' .:-=+*#%@' low->high:");
    let ramp: Vec<char> = " .:-=+*#%@".chars().collect();
    for r in 0..rows {
        let y = r * t.size_y as usize / rows;
        let mut line = String::new();
        for c in 0..cols {
            let x = c * sx / cols;
            let h = t.heights[y * vw + x];
            let k = (((h - lo) / (hi - lo)) * (ramp.len() - 1) as f32).round() as usize;
            line.push(ramp[k.min(ramp.len() - 1)]);
        }
        println!("{line}");
    }

    println!(
        "\nmaterials (m=mare M=dark r=regolith R=coarse ^=rocky o=crater floor O=rim *=ray L=pad):"
    );
    for r in 0..rows {
        let y = r * t.size_y as usize / rows;
        let mut line = String::new();
        for c in 0..cols {
            let x = c * sx / cols;
            let id = t.tiles[y * sx + x];
            let ch = match material_for_tile_id(id).map(|(m, _)| m) {
                Some(LunarMaterial::MareSmooth) => 'm',
                Some(LunarMaterial::MareDark) => 'M',
                Some(LunarMaterial::Regolith) => 'r',
                Some(LunarMaterial::RegolithCoarse) => 'R',
                Some(LunarMaterial::Rocky) => '^',
                Some(LunarMaterial::CraterFloor) => 'o',
                Some(LunarMaterial::RimBright) => 'O',
                Some(LunarMaterial::Ray) => '*',
                Some(LunarMaterial::LandingPad) => 'L',
                None => '?',
            };
            line.push(ch);
        }
        println!("{line}");
    }

    println!("\nnavigation ('#' = blocked):");
    for r in 0..rows {
        let y = r * t.size_y as usize / rows;
        let mut line = String::new();
        for c in 0..cols {
            let x = c * sx / cols;
            line.push(if t.nav[y * sx + x] == 1 { '.' } else { '#' });
        }
        println!("{line}");
    }
}
