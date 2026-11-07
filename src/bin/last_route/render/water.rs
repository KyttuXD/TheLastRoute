//! The sewer's water surface as a mesh: one flat quad per water cell at the water height, tinted by depth.
//! The tint runs from `shallow` at the shore to `deep` over the bed (`world::route::sewer::water` holds the
//! numbers, read from `sewer_water.json`). Vertex colour carries the tint, so the water deepens toward the
//! channel's middle without a texture. Drawn lit (`shader::LIT_MODE`), so it goes dark with the sewer
//! instead of sitting at full brightness like an emissive surface.

use super::builder::Builder;
use crate::world::route::sewer::core::{Grid, N, WATER};
use crate::world::route::sewer::water::{params, WaterParams};
use macroquad::prelude::*;

/// The tint at `depth` metres of water: linear in the fraction of the full depth, bent by `deepen_power`.
pub fn tint_at_depth(depth: f32, p: &WaterParams) -> [u8; 4] {
    let t = (depth / p.full_depth()).clamp(0.0, 1.0).powf(p.deepen_power);
    let mut out = [0u8; 4];
    for k in 0..4 {
        out[k] = (p.shallow[k] as f32 + (p.deep[k] as f32 - p.shallow[k] as f32) * t).round() as u8;
    }
    out
}

fn is_water(grid: &Grid, x: i32, z: i32) -> bool {
    x >= 0 && z >= 0 && x < N as i32 && z < N as i32 && grid[z as usize][x as usize] == WATER
}

/// The depth a corner of the grid reads: the full depth where all four cells around it are water, the shore
/// fraction of it where any is not.
pub fn corner_depth(grid: &Grid, cx: i32, cz: i32, p: &WaterParams) -> f32 {
    let all = [(cx - 1, cz - 1), (cx, cz - 1), (cx, cz), (cx - 1, cz)].iter().all(|(x, z)| is_water(grid, *x, *z));
    p.full_depth() * if all { 1.0 } else { p.shore_depth_fraction }
}

/// The water quads of a chunk's grid: chunk cell (0,0) at `origin` (x, z in metres), `cell` metres a side, the
/// surface at `floor_y` plus the surface offset. Returns the number of quads added.
pub fn add_water(b: &mut Builder, grid: &Grid, origin: Vec2, cell: f32, floor_y: f32) -> usize {
    add_water_cells(b, grid, origin, cell, floor_y, &|_, _| true)
}

/// `add_water` for the cells `keep(x, z)` accepts only (a streamed part draws the cells it owns, so no cell is
/// drawn twice); the shore depth still reads every neighbour.
pub fn add_water_cells(b: &mut Builder, grid: &Grid, origin: Vec2, cell: f32, floor_y: f32, keep: &dyn Fn(i32, i32) -> bool) -> usize {
    let p = params();
    let y = floor_y + p.water_surface;
    let mut count = 0;
    for z in 0..N as i32 {
        for x in 0..N as i32 {
            if !is_water(grid, x, z) || !keep(x, z) {
                continue;
            }
            let corner = |cx: i32, cz: i32| (vec3(origin.x + cx as f32 * cell, y, origin.y + cz as f32 * cell), corner_depth(grid, cx, cz, p));
            let [a, bb, c, d] = [corner(x, z), corner(x + 1, z), corner(x + 1, z + 1), corner(x, z + 1)];
            let verts = [a, bb, c, d];
            b.quad(
                [a.0, bb.0, c.0, d.0],
                [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)],
                vec3(0., 1., 0.),
                WHITE,
                super::shader::LIT_MODE,
            );
            // quad() may have flushed a full mesh first, so the four new vertices are the last four
            let n = b.verts.len();
            for (k, v) in verts.iter().enumerate() {
                b.verts[n - 4 + k].color = tint_at_depth(v.1, p);
            }
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::sewer::core::SOLID;

    #[test]
    fn tint_deepens_with_depth() {
        let p = params();
        let shallow = tint_at_depth(0.0, p);
        let deep = tint_at_depth(p.full_depth(), p);
        assert_eq!(shallow, p.shallow);
        assert_eq!(deep, p.deep);
        let mid = tint_at_depth(p.full_depth() * 0.5, p);
        assert!(mid[1] < shallow[1] && mid[1] > deep[1], "green falls from shallow to deep");
        assert_eq!(tint_at_depth(99.0, p), p.deep);
    }

    #[test]
    fn shore_corners_read_shallower_than_open_water() {
        let mut g: Grid = [[SOLID; N]; N];
        for z in 4..9 {
            for x in 4..9 {
                g[z][x] = WATER;
            }
        }
        let p = params();
        assert_eq!(corner_depth(&g, 6, 6, p), p.full_depth());
        assert!(corner_depth(&g, 4, 4, p) < corner_depth(&g, 6, 6, p));
    }

    #[test]
    fn the_surface_sits_at_the_data_depth_and_only_kept_cells_are_drawn() {
        let mut g: Grid = [[SOLID; N]; N];
        for z in 4..9 {
            for x in 4..9 {
                g[z][x] = WATER;
            }
        }
        let p = params();
        let mut all = Builder::new(None);
        assert_eq!(add_water(&mut all, &g, vec2(10.0, 20.0), 1.0, 0.0), 25);
        assert!(all.verts.iter().all(|v| (v.position.y - p.water_surface).abs() < 1e-5), "surface = floor + water_surface");
        assert!(all.verts.iter().all(|v| (v.normal.w - crate::render::shader::LIT_MODE).abs() < 1e-5), "the surface is lit, not emissive");
        assert!(all.verts.iter().all(|v| v.position.x >= 14.0 && v.position.x <= 19.0 && v.position.z >= 24.0));
        let mut part = Builder::new(None);
        let kept = add_water_cells(&mut part, &g, vec2(0.0, 0.0), 1.0, 2.0, &|x, _| x < 6);
        assert_eq!(kept, 10, "two columns of five cells");
        assert!(part.verts.iter().all(|v| (v.position.y - (2.0 + p.water_surface)).abs() < 1e-5));
    }
}
