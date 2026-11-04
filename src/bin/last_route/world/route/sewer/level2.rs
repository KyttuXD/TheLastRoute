//! The second sewer level, below the first: its own layout rules and water, joined to the first level by drops
//! where the waterfalls of `falls_between` fall. Every number is in the `layout` block of data/regions/sewer2.json
//! (`Rules2`); the code holds only the behaviour. Drop cells are offsets from the falls centre and the landing
//! pool is the falls void grown by `landing_grow`, so the two levels line up for any data that keeps the drops
//! inside the void (`chunk` skips a drop whose cell is outside the void).

use serde_json::Value;

use super::core::{falls_between, Falls, Grid, Inputs, Rect, DRY, N, SOLID, WATER};
use crate::world::rng::Rng;
use crate::world::route::layout::V2;

/// One drop in the data: where it is, relative to the falls centre.
#[derive(Clone, Debug, PartialEq)]
pub struct DropSpec {
    pub id: String,
    pub offset: V2,
}

/// The layout rules of the second level, from the `layout` block of regions/sewer2.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules2 {
    /// The storey number of this level.
    pub level: i32,
    pub tunnel_half: i32,
    /// Half-width of the water channel down the middle of each tunnel.
    pub channel_half: i32,
    pub junction_half: i32,
    /// How far the landing pool reaches beyond the falls void.
    pub landing_grow: i32,
    /// Half-width of the dry path from the landing pool to the tunnel.
    pub exit_half: i32,
    /// Water depth in metres (the swim and plunge code reads this).
    pub water_depth: f64,
    pub side_pool_chance: f64,
    pub side_pool_size: [i32; 2],
    pub side_pool_centers: Vec<V2>,
    pub drops: Vec<DropSpec>,
}

/// The rules from the built-in sewer2 region text, read once.
pub fn rules() -> &'static Rules2 {
    static RULES: std::sync::OnceLock<Rules2> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        // The packs list may not name this file yet; the fallback is the same file.
        let text = crate::packs::regions::BUILTIN_FILES
            .iter()
            .find(|(file, _)| *file == "regions/sewer2.json")
            .map(|(_, text)| *text)
            .unwrap_or(include_str!("../../../../../../data/regions/sewer2.json"));
        Rules2::from_region_text(text).unwrap_or_else(|e| panic!("sewer2 layout rules: {e}"))
    })
}

fn int(v: &Value, key: &str) -> Result<i32, String> {
    v.get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| format!("layout.{key} must be an integer"))
}

fn num(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("layout.{key} must be a number"))
}

fn pair(v: &Value, key: &str) -> Result<[i32; 2], String> {
    let bad = || format!("layout.{key} must be an array of 2 integers");
    let items = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
    let a = items[0].as_i64().and_then(|n| i32::try_from(n).ok()).ok_or_else(bad)?;
    let b = items[1].as_i64().and_then(|n| i32::try_from(n).ok()).ok_or_else(bad)?;
    Ok([a, b])
}

impl Rules2 {
    pub fn from_region_text(text: &str) -> Result<Rules2, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("sewer2.json: {e}"))?;
        let v = root.get("layout").ok_or_else(|| "sewer2.json has no \"layout\" block".to_string())?;
        let rows = |key: &str| -> Result<&Vec<Value>, String> {
            v.get(key)
                .and_then(Value::as_array)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| format!("layout.{key} must be a non-empty array"))
        };
        let drops = rows("drops")?
            .iter()
            .map(|d| -> Result<DropSpec, String> {
                let id = d.get("id").and_then(Value::as_str).ok_or("layout.drops[].id must be text")?.to_string();
                let [x, y] = pair(d.get("offset").ok_or("layout.drops[].offset is missing")?, "drops[].offset")?;
                Ok(DropSpec { id, offset: V2::new(x, y) })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let side_pool_centers = rows("side_pool_centers")?
            .iter()
            .map(|c| pair(c, "side_pool_centers").map(|[x, y]| V2::new(x, y)))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Rules2 {
            level: int(v, "level")?,
            tunnel_half: int(v, "tunnel_half")?,
            channel_half: int(v, "channel_half")?,
            junction_half: int(v, "junction_half")?,
            landing_grow: int(v, "landing_grow")?,
            exit_half: int(v, "exit_half")?,
            water_depth: num(v, "water_depth")?,
            side_pool_chance: num(v, "side_pool_chance")?,
            side_pool_size: pair(v.get("side_pool_size").ok_or("layout.side_pool_size is missing")?, "side_pool_size")?,
            side_pool_centers,
            drops,
        })
    }
}

/// A drop: the cell where the first level's waterfall falls, and where the player lands (the same cell, one
/// level down, which is always water of the landing pool).
#[derive(Clone, Debug, PartialEq)]
pub struct Drop {
    pub id: String,
    pub upper_cell: V2,
    pub lower_cell: V2,
}

/// One chunk of the second level.
#[derive(Clone, Debug)]
pub struct Level2Chunk {
    pub x: i32,
    pub z: i32,
    pub grid: Grid,
    pub drops: Vec<Drop>,
    pub landing_pool: Option<Rect>,
    pub side_pool: Option<Rect>,
}

fn carve(grid: &mut Grid, r: Rect, code: u8) {
    for z in r.y.max(0)..r.end().y.min(N as i32) {
        for x in r.x.max(0)..r.end().x.min(N as i32) {
            grid[z as usize][x as usize] = code;
        }
    }
}

/// The landing pool under a falls: its void grown by `landing_grow`.
pub fn landing_pool(falls: &Falls) -> Rect {
    falls.void.grow(rules().landing_grow)
}

/// Every cell of a falls void, row by row: the cells the first level leaves open (`FALL`) and, in the same x/z on this
/// level, the cells under them (the air above the landing pool, whose rim is `landing_pool`). The playing facility
/// reads the first level's `FALL` cells for both (`flight_links::fall_cells`); this is the same set from the falls alone.
pub fn void_cells(falls: &Falls) -> Vec<V2> {
    let v = falls.void;
    let mut out = Vec::with_capacity((v.w.max(0) * v.h.max(0)) as usize);
    for z in v.y..v.end().y {
        for x in v.x..v.end().x {
            out.push(V2::new(x, z));
        }
    }
    out
}

/// The second level's chunk at column (x, z); the entry point of the generator. The tunnels cross at the chunk
/// centre; a falls in the first level (`falls_between`) adds a landing pool, a dry path to the tunnel and the
/// drops.
pub fn chunk(inputs: &Inputs, x: i32, z: i32) -> Level2Chunk {
    let r = rules();
    let n = N as i32;
    let c = n / 2;
    let mut grid: Grid = [[SOLID; N]; N];
    carve(&mut grid, Rect::new(0, c - r.tunnel_half, n, 2 * r.tunnel_half + 1), DRY);
    carve(&mut grid, Rect::new(c - r.tunnel_half, 0, 2 * r.tunnel_half + 1, n), DRY);
    carve(&mut grid, Rect::new(c - r.junction_half, c - r.junction_half, 2 * r.junction_half + 1, 2 * r.junction_half + 1), DRY);
    carve(&mut grid, Rect::new(0, c - r.channel_half, n, 2 * r.channel_half + 1), WATER);
    carve(&mut grid, Rect::new(c - r.channel_half, 0, 2 * r.channel_half + 1, n), WATER);

    let mut drops = Vec::new();
    let mut pool = None;
    let falls = falls_between(inputs, x, z);
    if let Some(f) = &falls {
        let p = landing_pool(f);
        // the dry path from the pool to the tunnel it faces, then the pool over it
        let (z0, z1) = if f.north { (p.end().y, c - r.tunnel_half) } else { (c + r.tunnel_half + 1, p.y) };
        carve(&mut grid, Rect::new(f.center.x - r.exit_half, z0, 2 * r.exit_half + 1, (z1 - z0).max(0)), DRY);
        carve(&mut grid, p, WATER);
        for d in &r.drops {
            let cell = V2::new(f.center.x + d.offset.x, f.center.y + d.offset.y);
            if f.void.has_point(cell) {
                drops.push(Drop { id: d.id.clone(), upper_cell: cell, lower_cell: cell });
            }
        }
        pool = Some(p);
    }

    // an optional side pool in a place the falls does not use
    let mut rng = Rng::new(&format!("{}:SEWER2:{}:{}", inputs.world, x, z));
    let mut side = None;
    if rng.next_float() < r.side_pool_chance {
        let centre = r.side_pool_centers[rng.next_index(r.side_pool_centers.len())];
        if falls.as_ref().map_or(true, |f| f.center != centre) {
            let [w, h] = r.side_pool_size;
            let rect = Rect::new(centre.x - w / 2, centre.y - h / 2, w, h);
            carve(&mut grid, rect, WATER);
            side = Some(rect);
        }
    }
    Level2Chunk { x, z, grid, drops, landing_pool: pool, side_pool: side }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::layout::Coord;
    use crate::world::route::sewer::core::{chunk as upper_chunk, FALL};

    fn inputs() -> Inputs<'static> {
        Inputs { world: "level2-test" }
    }

    #[test]
    fn the_standard_data_parses_and_is_consistent() {
        let r = rules();
        assert!(r.channel_half < r.tunnel_half && r.tunnel_half < r.junction_half);
        assert!(r.water_depth > 0.0 && !r.drops.is_empty());
    }

    #[test]
    fn a_bad_layout_names_the_key() {
        let err = Rules2::from_region_text(r#"{"layout":{"level":-2}}"#).unwrap_err();
        assert!(err.contains("drops"), "{err}");
    }

    #[test]
    fn the_drop_cells_line_up_between_the_two_levels() {
        let mut found = 0;
        for x in -6..6 {
            for z in -6..6 {
                let lower = chunk(&inputs(), x, z);
                let upper = upper_chunk(Coord::new(x, -1, z), &inputs()).expect("level -1 chunk");
                // a drop exists exactly where the first level has a falls
                assert_eq!(lower.drops.is_empty(), upper.falls.is_none(), "({x}, {z})");
                for d in &lower.drops {
                    found += 1;
                    let f = upper.falls.as_ref().unwrap();
                    assert_eq!(d.upper_cell, d.lower_cell);
                    assert!(f.void.has_point(d.upper_cell));
                    assert_eq!(upper.sewer_grid[d.upper_cell.y as usize][d.upper_cell.x as usize], FALL);
                    assert_eq!(lower.grid[d.lower_cell.y as usize][d.lower_cell.x as usize], WATER);
                    assert!(lower.landing_pool.unwrap().has_point(d.lower_cell));
                }
                // the playing facilities agree too (a facility is costly, so the chunks without falls are sampled)
                match &upper.falls {
                    Some(falls) => facilities_line_up(x, z, Some(falls)),
                    None if (x + z).rem_euclid(5) == 0 => facilities_line_up(x, z, None),
                    None => {}
                }
            }
        }
        assert!(found > 0, "no falls in the scanned area");
    }

    /// The playing facilities of the two levels agree on the void: every drop cell of level -1 is a hole with no wall,
    /// the same x/z on level -2 is air above with no ceiling, the drop cells lie in the falls void, and the two floors
    /// are one storey apart in absolute y.
    fn facilities_line_up(x: i32, z: i32, falls: Option<&Falls>) {
        use crate::content::loot::LootTables;
        use crate::world::catalog::Catalog;
        use crate::world::facility::Cell;
        use crate::world::infinite::{chunk as build, ChunkKey};
        use crate::world::vertical::STOREY;
        use std::sync::OnceLock;
        static CAT: OnceLock<Catalog> = OnceLock::new();
        let cat = CAT.get_or_init(Catalog::load);
        let loot = LootTables::default();
        let world = inputs().world;
        let (up, _) = build(world, ChunkKey { cx: x, cy: z, level: -1 }, cat, &loot);
        let (lo, _) = build(world, ChunkKey { cx: x, cy: z, level: -2 }, cat, &loot);
        // floor heights: the upper floor minus the lower floor is one storey
        let (up_y, lo_y) = (up.level as f32 * STOREY, lo.level as f32 * STOREY);
        assert!((up_y - lo_y - STOREY).abs() < 1e-4, "({x}, {z}): floors {up_y} and {lo_y}");
        let Some(falls) = falls else {
            assert!(up.drop_cells().is_empty() && lo.air_above_cells().is_empty(), "({x}, {z}): a drop with no falls");
            return;
        };
        let void = void_cells(falls);
        assert!(!up.drop_cells().is_empty(), "({x}, {z}): a falls with no drop cells");
        assert_eq!(up.drop_cells(), lo.air_above_cells(), "({x}, {z}): the drop and the air above differ");
        for c in up.drop_cells() {
            assert!(void.contains(&V2::new(c.x, c.y)), "({x}, {z}): drop {c:?} is outside the falls void");
            assert!(up.is_drop(*c) && up.is_hole(c.x, c.y) && !up.is_wall_cell(c.x, c.y), "({x}, {z}): {c:?} is not an open hole");
            assert!(up.open_below(*c) && !up.open_above(*c));
            assert!(lo.is_air_above(*c) && lo.open_above(*c) && !lo.is_drop(*c), "({x}, {z}): {c:?} has a ceiling below");
            assert!(!lo.is_wall_cell(c.x, c.y), "({x}, {z}): the pool under {c:?} is rock");
        }
        // around the void the walls are untouched: the spur and the overlook stay floor
        let stand = |r: Rect| {
            (r.y..r.y + r.h).any(|yy| (r.x..r.x + r.w).any(|xx| !up.is_wall_cell(xx, yy) && !up.is_drop(Cell::new(xx, yy))))
        };
        assert!(stand(falls.spur) || stand(falls.overlook), "({x}, {z}): no floor on the spur or the overlook");
    }

    #[test]
    fn the_void_cells_are_the_whole_falls_void() {
        let mut checked = 0;
        for x in -6..6 {
            for z in -6..6 {
                if let Some(f) = falls_between(&inputs(), x, z) {
                    let cells = void_cells(&f);
                    assert_eq!(cells.len() as i32, f.void.w * f.void.h);
                    assert!(cells.iter().all(|c| f.void.has_point(*c)));
                    assert!(cells.iter().all(|c| landing_pool(&f).has_point(*c)), "the pool lies under the whole void");
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn the_tunnels_are_open_and_carry_water() {
        let c = chunk(&inputs(), 0, 0);
        let mid = N / 2;
        assert_eq!(c.grid[mid][0], WATER);
        assert_eq!(c.grid[0][mid], WATER);
        assert_eq!(c.grid[mid + rules().tunnel_half as usize][0], DRY);
        assert_eq!(c.grid[0][0], SOLID);
    }

    #[test]
    fn the_chunk_is_deterministic() {
        assert_eq!(chunk(&inputs(), 3, 4).grid, chunk(&inputs(), 3, 4).grid);
    }
}
