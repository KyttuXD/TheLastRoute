//! Sewer channel railings (Godot's `_spawn_channel_railings`): iron rails on every walkable cell edge that meets
//! channel water or the falls' drop, so the water collider has a visible reason. `channel_rail_segments` is the pure
//! edge walk; `rail_boxes` turns one segment into boxes. Sizes and colour are data/render/sewer_rails.json.

use crate::content::embedded;
use crate::world::route::sewer::core::{Grid, BRIDGE, DRY, FALL, N, WATER};
use std::sync::OnceLock;

pub const PATH: &str = "render/sewer_rails.json";

#[derive(Clone, Debug, PartialEq)]
pub struct RailCfg {
    pub inset_m: f32,
    pub height_m: f32,
    pub top_rail_drop_m: f32,
    pub mid_rail_fraction: f32,
    pub rail_thickness_m: f32,
    pub post_size_m: f32,
    pub post_spacing_m: f32,
    pub color: [f32; 3],
    pub collision: RailCollision,
}

/// The railing as a wall (`collision` in the data file).
#[derive(Clone, Debug, PartialEq)]
pub struct RailCollision {
    /// Whether the rails block bodies and routes at all.
    pub solid: bool,
    /// The wall's thickness (m) across the rail line.
    pub thickness_m: f32,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32).ok_or_else(|| format!("{key}: expected a number"))
}

impl RailCollision {
    fn parse(v: Option<&serde_json::Value>) -> Result<RailCollision, String> {
        let v = v.ok_or("collision: expected an object")?;
        let solid = v.get("solid").and_then(|x| x.as_bool()).ok_or("collision.solid: expected true or false")?;
        let thickness_m = num(v, "thickness_m").map_err(|e| format!("collision.{e}"))?;
        if !(thickness_m > 0.0 && thickness_m < 0.5) {
            return Err(format!("collision.thickness_m: {thickness_m} must be above 0 and under half a cell"));
        }
        Ok(RailCollision { solid, thickness_m })
    }
}

impl RailCfg {
    pub fn parse(text: &str) -> Result<RailCfg, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let color = v
            .get("color")
            .and_then(|x| x.as_array())
            .filter(|a| a.len() == 3)
            .and_then(|a| Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32]))
            .ok_or("color: expected 3 numbers")?;
        let cfg = RailCfg {
            inset_m: num(&v, "inset_m")?,
            height_m: num(&v, "height_m")?,
            top_rail_drop_m: num(&v, "top_rail_drop_m")?,
            mid_rail_fraction: num(&v, "mid_rail_fraction")?,
            rail_thickness_m: num(&v, "rail_thickness_m")?,
            post_size_m: num(&v, "post_size_m")?,
            post_spacing_m: num(&v, "post_spacing_m")?,
            color,
            collision: RailCollision::parse(v.get("collision"))?,
        };
        if cfg.height_m <= 0.0 || cfg.post_spacing_m <= 0.0 || !(0.0..0.5).contains(&cfg.inset_m) {
            return Err("height and post spacing must be positive and the inset under half a cell".into());
        }
        // a hunter must still walk a one-cell walkway with a railing on both sides
        let room = 1.0 - 2.0 * cfg.inset_m - cfg.collision.thickness_m;
        if cfg.collision.solid && room <= 2.0 * crate::config::HUNTER_RADIUS {
            return Err(format!(
                "collision.thickness_m: {} leaves {room:.2} m between the rails of a one-cell walkway, a hunter needs {:.2} m",
                cfg.collision.thickness_m,
                2.0 * crate::config::HUNTER_RADIUS
            ));
        }
        Ok(cfg)
    }
}

/// The railing tunables compiled in. A bad file is reported on stderr and the built-in copy is used.
pub fn config() -> &'static RailCfg {
    static CFG: OnceLock<RailCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        RailCfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            RailCfg::parse(include_str!("../data/render/sewer_rails.json")).expect("sewer_rails.json is valid")
        })
    })
}

/// One run of railing: the walkable cell (`x`, `z`) in the chunk grid and the unit step to the water side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RailSegment {
    pub x: i32,
    pub z: i32,
    pub step: (i32, i32),
}

const DIRS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// A segment for every edge of a walkable (dry or bridge) cell whose neighbour is water or the falls' drop, except
/// the deliberate jump cell `gap`: no rail faces it, and when the jump cell is itself walkable (the lower storey's
/// deck, beside the pool it drops into) none stands on its own water edges either, so a body that lands in the pool
/// can climb out through it. Edges at the grid's border get none.
pub fn channel_rail_segments(grid: &Grid, gap: Option<(i32, i32)>) -> Vec<RailSegment> {
    let mut out = Vec::new();
    for z in 0..N as i32 {
        for x in 0..N as i32 {
            let code = grid[z as usize][x as usize];
            if code != DRY && code != BRIDGE {
                continue;
            }
            if gap == Some((x, z)) {
                continue;
            }
            for step in DIRS {
                let (nx, nz) = (x + step.0, z + step.1);
                if nx < 0 || nz < 0 || nx >= N as i32 || nz >= N as i32 {
                    continue;
                }
                let n = grid[nz as usize][nx as usize];
                if (n == WATER || n == FALL) && gap != Some((nx, nz)) {
                    out.push(RailSegment { x, z, step });
                }
            }
        }
    }
    out
}

/// The posts on one segment.
pub fn posts_per_segment(cfg: &RailCfg) -> usize {
    (1.0 / cfg.post_spacing_m).round().max(1.0) as usize
}

/// The boxes of one segment as (centre, size) relative to the cell's corner (x, floor, z) of the grid: two rails
/// along the edge and a post every spacing metres from its start corner.
pub fn rail_boxes(seg: RailSegment, cfg: &RailCfg) -> Vec<([f32; 3], [f32; 3])> {
    let edge = 0.5 - cfg.inset_m;
    let mid = [0.5 + seg.step.0 as f32 * edge, 0.5 + seg.step.1 as f32 * edge];
    // the posts walk along the edge from its start corner. Godot builds the segment facing north/south and turns it a
    // quarter turn about Y for an east/west step, which turns its +x into world -z: there the start corner is +z
    let along = if seg.step.0 != 0 { [0.0, -1.0] } else { [1.0, 0.0] };
    let t = cfg.rail_thickness_m;
    let rail_size = if seg.step.0 != 0 { [t, t, 1.0] } else { [1.0, t, t] };
    let mut out = vec![
        ([mid[0], cfg.height_m - cfg.top_rail_drop_m, mid[1]], rail_size),
        ([mid[0], cfg.height_m * cfg.mid_rail_fraction, mid[1]], rail_size),
    ];
    let p = cfg.post_size_m;
    for k in 0..posts_per_segment(cfg) {
        let d = -0.5 + k as f32 * cfg.post_spacing_m;
        out.push(([mid[0] + along[0] * d, cfg.height_m * 0.5, mid[1] + along[1] * d], [p, cfg.height_m, p]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::sewer::core::SOLID;

    fn channel() -> Grid {
        // a dry row at z = 4, water at z = 5, solid elsewhere
        let mut g = [[SOLID; N]; N];
        for x in 2..8 {
            g[4][x] = DRY;
            g[5][x] = WATER;
        }
        g
    }

    #[test]
    fn one_segment_per_walkable_to_water_edge() {
        let segs = channel_rail_segments(&channel(), None);
        assert_eq!(segs.len(), 6);
        assert!(segs.iter().all(|s| s.z == 4 && s.step == (0, 1)));
    }

    #[test]
    fn the_jump_gap_is_skipped() {
        let mut g = channel();
        g[5][4] = FALL;
        let segs = channel_rail_segments(&g, Some((4, 5)));
        assert_eq!(segs.len(), 5);
        assert!(!segs.iter().any(|s| s.x == 4));
        assert_eq!(channel_rail_segments(&g, None).len(), 6);
    }

    #[test]
    fn a_walkable_jump_cell_keeps_its_water_edges_open() {
        // the lower storey's deck is the jump cell itself, beside the pool: the pool stays a way out
        let g = channel();
        let segs = channel_rail_segments(&g, Some((4, 4)));
        assert_eq!(segs.len(), 5);
        assert!(!segs.iter().any(|s| s.x == 4 && s.z == 4));
        assert!(segs.iter().any(|s| s.x == 3 && s.z == 4), "the neighbours keep their rails");
    }

    #[test]
    fn a_grid_without_water_has_no_rails() {
        // level 0 grids carry no WATER or FALL codes
        let mut g = [[SOLID; N]; N];
        g[3][3] = DRY;
        g[3][4] = BRIDGE;
        assert!(channel_rail_segments(&g, None).is_empty());
    }

    #[test]
    fn the_instance_count_is_bounded_per_chunk() {
        let cfg = config();
        let mut g = [[DRY; N]; N];
        for z in (0..N).step_by(2) {
            for x in 0..N {
                g[z][x] = WATER;
            }
        }
        let boxes: usize = channel_rail_segments(&g, None).into_iter().map(|s| rail_boxes(s, cfg).len()).sum();
        assert!(boxes <= N * N * 4 * (2 + posts_per_segment(cfg)));
        assert!(boxes > 0);
    }

    #[test]
    fn boxes_sit_on_the_walkway_side_of_the_edge() {
        let cfg = config();
        let b = rail_boxes(RailSegment { x: 0, z: 0, step: (0, 1) }, cfg);
        assert_eq!(b.len(), 2 + posts_per_segment(cfg));
        assert!((b[0].0[2] - (1.0 - cfg.inset_m)).abs() < 1e-6);
        assert_eq!(b[0].1[0], 1.0);
    }

    #[test]
    fn posts_start_at_the_godot_corner_on_every_edge() {
        let cfg = config();
        let post_axis = |step: (i32, i32)| {
            let b = rail_boxes(RailSegment { x: 0, z: 0, step }, cfg);
            let first = &b[2];
            (first.0[0] - (0.5 + step.0 as f32 * (0.5 - cfg.inset_m)), first.0[2] - (0.5 + step.1 as f32 * (0.5 - cfg.inset_m)))
        };
        // north and south edges start at -x, east and west edges at +z
        assert_eq!(post_axis((0, 1)), (-0.5, 0.0));
        assert_eq!(post_axis((0, -1)), (-0.5, 0.0));
        assert_eq!(post_axis((1, 0)), (0.0, 0.5));
        assert_eq!(post_axis((-1, 0)), (0.0, 0.5));
        let two = RailCfg { post_spacing_m: 0.5, ..cfg.clone() };
        let b = rail_boxes(RailSegment { x: 0, z: 0, step: (1, 0) }, &two);
        assert_eq!((b[2].0[2], b[3].0[2]), (1.0, 0.5), "the posts walk toward -z");
    }

    fn real_chunk(level: i32, seed: &str, cx: i32, cz: i32) -> Option<crate::world::route::sewer::SewerChunk> {
        use crate::world::route::layout::Coord;
        crate::world::route::sewer::chunk(Coord::new(cx, level, cz), &crate::world::route::sewer::Inputs { world: seed })
    }

    #[test]
    fn a_real_falls_chunk_skips_the_jump_cell() {
        use crate::world::route::sewer::core::{LEVEL, LOWER_LEVEL};
        let mut checked = 0;
        for seed in ["ROOM-RAILS-1", "ROOM-RAILS-2", "ROOM-RAILS-3", "ROOM-RAILS-4"] {
            for level in [LEVEL, LOWER_LEVEL] {
                for (cx, cz) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, -1), (2, 0), (0, 2)] {
                    let Some(c) = real_chunk(level, seed, cx, cz) else { continue };
                    let Some(falls) = c.falls.as_ref() else { continue };
                    let jump = (falls.jump.cell.x, falls.jump.cell.y);
                    let with_gap = channel_rail_segments(&c.sewer_grid, Some(jump));
                    let all = channel_rail_segments(&c.sewer_grid, None);
                    let faces = |s: &RailSegment| (s.x + s.step.0, s.z + s.step.1) == jump;
                    // the jump cell's own edges go too when it is walkable (the lower deck beside its pool)
                    let own = |s: &RailSegment| (s.x, s.z) == jump;
                    assert!(!with_gap.iter().any(faces), "{seed} {level} {cx},{cz}: a rail faces the jump cell");
                    assert!(!with_gap.iter().any(own), "{seed} {level} {cx},{cz}: a rail stands on the jump cell");
                    assert_eq!(all.len() - with_gap.len(), all.iter().filter(|s| faces(s) || own(s)).count());
                    assert!(!all.is_empty(), "a falls chunk has water edges to rail");
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no sample sewer chunk has falls");
    }

    #[test]
    fn a_level_zero_chunk_has_no_sewer_grid_so_no_rails() {
        for (cx, cz) in [(0, 0), (1, 0), (0, 1)] {
            assert!(real_chunk(0, "ROOM-RAILS-1", cx, cz).is_none(), "level 0 is not a sewer chunk");
        }
    }

    #[test]
    fn the_shipped_file_parses() {
        assert!(RailCfg::parse(include_str!("../data/render/sewer_rails.json")).is_ok());
    }

    /// The shipped file with its `collision` block replaced by `collision` (a JSON fragment).
    fn with_collision(collision: &str) -> String {
        let text = include_str!("../data/render/sewer_rails.json");
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["collision"] = serde_json::from_str(collision).unwrap_or(serde_json::Value::Null);
        v.to_string()
    }

    #[test]
    fn a_bad_collision_block_is_refused_on_load() {
        assert!(RailCfg::parse(&with_collision(r#"{"solid": true, "thickness_m": 0.07}"#)).is_ok());
        assert!(RailCfg::parse(&with_collision(r#"{"solid": false, "thickness_m": 0.07}"#)).is_ok());
        // no block, a non-boolean solid, a missing or non-positive thickness, a thickness of half a cell or more
        assert!(RailCfg::parse(&with_collision("null")).is_err());
        assert!(RailCfg::parse(&with_collision(r#"{"solid": "yes", "thickness_m": 0.07}"#)).is_err());
        assert!(RailCfg::parse(&with_collision(r#"{"solid": true}"#)).is_err());
        assert!(RailCfg::parse(&with_collision(r#"{"solid": true, "thickness_m": 0.0}"#)).is_err());
        assert!(RailCfg::parse(&with_collision(r#"{"solid": true, "thickness_m": 0.5}"#)).is_err());
    }

    #[test]
    fn a_railing_too_thick_for_a_hunter_to_pass_between_is_refused() {
        // the room between the rails of a one-cell walkway must fit a hunter
        let thick = with_collision(r#"{"solid": true, "thickness_m": 0.45}"#);
        let err = RailCfg::parse(&thick).unwrap_err();
        assert!(err.contains("collision.thickness_m"), "{err}");
        // a solid-false railing has no room to check
        assert!(RailCfg::parse(&with_collision(r#"{"solid": false, "thickness_m": 0.45}"#)).is_ok());
    }
}
