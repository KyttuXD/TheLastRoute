//! The sewers' emergency lights: pulsing red and amber wall fixtures (`sewerEmergencyRed`, `sewerEmergencyAmber`,
//! data/props/p11.json) added as live `Facility` props by `apply_sewer_floor`, after the generator's own props. They are
//! a separate layer, so `SewerChunk::region_props` (and with it the sewer layout goldens) is unchanged. How many, how far
//! apart and at what height is data/sewer_emergency.json (`lr.sewer_emergency/1`). A fixture stands on a dry cell
//! with brick behind it, is never solid, and is never on a cell another prop holds.

use super::facility::{Cell, Prop, Room};
use super::rng::Rng;
use super::route::layout::Coord;
use super::route::sewer::core::{chunk_id, DRY, N, SOLID};
use super::route::sewer::SewerChunk;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const SCHEMA: &str = "lr.sewer_emergency/1";
const PATH: &str = "sewer_emergency.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    per_chunk: HashMap<i32, usize>,
    pub min_gap_cells: i32,
    pub lamp_gap_cells: i32,
    pub mount_y: f64,
    pub types: Vec<String>,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{PATH}: {e}"))?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("{PATH}: schema: expected {SCHEMA}"));
        }
        let whole = |k: &str| v.get(k).and_then(Value::as_i64).filter(|n| *n >= 0).map(|n| n as i32).ok_or_else(|| format!("{PATH}: {k}: expected a whole number of 0 or more"));
        let mut per_chunk = HashMap::new();
        for (key, n) in v.get("per_chunk").and_then(Value::as_object).ok_or_else(|| format!("{PATH}: per_chunk: expected an object"))? {
            let level = key.strip_prefix("level.").and_then(|l| l.parse::<i32>().ok()).ok_or_else(|| format!("{PATH}: per_chunk.{key}: expected `level.<n>`"))?;
            let n = n.as_u64().ok_or_else(|| format!("{PATH}: per_chunk.{key}: expected a whole number"))?;
            per_chunk.insert(level, n as usize);
        }
        let types: Vec<String> = v
            .get("types")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if types.is_empty() {
            return Err(format!("{PATH}: types: expected at least one prop id"));
        }
        Ok(Rules {
            per_chunk,
            min_gap_cells: whole("min_gap_cells")?,
            lamp_gap_cells: whole("lamp_gap_cells")?,
            mount_y: v.get("mount_y").and_then(Value::as_f64).filter(|y| *y > 0.0).ok_or_else(|| format!("{PATH}: mount_y: expected a positive number"))?,
            types,
        })
    }

    pub fn count(&self, level: i32) -> usize {
        self.per_chunk.get(&level).copied().unwrap_or(0)
    }
}

/// The rules compiled into the executable. A file that fails to parse is reported on stderr and no emergency lights are placed.
pub fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        let text = crate::content::embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Rules::parse(text).unwrap_or_else(|e| {
            eprintln!("{e}");
            Rules { per_chunk: HashMap::new(), min_gap_cells: 0, lamp_gap_cells: 0, mount_y: 2.7, types: vec!["sewerEmergencyRed".into()] }
        })
    })
}

/// The emergency lights of one sewer chunk on `level`: seeded by the world, the chunk and the level.
pub fn props(sc: &SewerChunk, seed: &str, cx: i32, cy: i32, level: i32, rooms: &[Room]) -> Vec<Prop> {
    let r = rules();
    let want = r.count(level);
    if want == 0 {
        return vec![];
    }
    let taken: Vec<(i32, i32)> = sc.region_props.iter().map(|p| (p.cell.x, p.cell.y)).collect();
    let lamps: Vec<(i32, i32)> = sc.region_props.iter().filter(|p| p.prop_type == "sewerLamp").map(|p| (p.cell.x, p.cell.y)).collect();
    // every dry cell with brick behind it, in grid order
    let mut spots: Vec<(i32, i32, (i32, i32))> = Vec::new();
    for z in 0..N as i32 {
        for x in 0..N as i32 {
            if sc.sewer_grid[z as usize][x as usize] != DRY || taken.contains(&(x, z)) {
                continue;
            }
            for wall in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (bx, bz) = (x + wall.0, z + wall.1);
                if bx >= 0 && bz >= 0 && bx < N as i32 && bz < N as i32 && sc.sewer_grid[bz as usize][bx as usize] == SOLID {
                    spots.push((x, z, wall));
                }
            }
        }
    }
    let far = |a: (i32, i32), b: (i32, i32), gap: i32| (a.0 - b.0).abs().max((a.1 - b.1).abs()) >= gap;
    let mut rng = Rng::new(&format!("{seed}:SEWER-EMERGENCY:{cx}:{level}:{cy}"));
    let mut chosen: Vec<(i32, i32, (i32, i32))> = Vec::new();
    while chosen.len() < want && !spots.is_empty() {
        let (x, z, wall) = spots.remove(rng.next_index(spots.len()));
        let at = (x, z);
        if lamps.iter().all(|l| far(at, *l, r.lamp_gap_cells)) && chosen.iter().all(|c| far(at, (c.0, c.1), r.min_gap_cells)) {
            chosen.push((x, z, wall));
        }
    }
    let id = chunk_id(Coord::new(cx, level, cy));
    chosen
        .into_iter()
        .enumerate()
        .map(|(n, (x, z, wall))| {
            let yaw = (wall.0 as f64).atan2(wall.1 as f64);
            let rotation = ((yaw / std::f64::consts::FRAC_PI_2).round() as i32).rem_euclid(4);
            Prop {
                id: format!("{id}:PROP:E{n}"),
                cell: Cell::new(x, z),
                ptype: r.types[(rng.next_index(r.types.len()) + n) % r.types.len()].clone(),
                room: rooms.iter().position(|rm| x >= rm.x && z >= rm.y && x < rm.x + rm.w && z < rm.y + rm.h).unwrap_or(0),
                rotation,
                mount_y: Some(r.mount_y),
                solid: false,
                half: (0.25, 0.25),
                offset: (wall.0 as f32 * 0.5, wall.1 as f32 * 0.5),
                door: None,
                sewer: None,
                crawl_under: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::sewer::{self, Inputs};

    const FILE: &str = include_str!("../data/sewer_emergency.json");
    const WORLDS: [&str; 3] = ["wired-sewer", "1426541270", "water-a"];

    fn chunk(world: &str, cx: i32, cy: i32, level: i32) -> SewerChunk {
        sewer::chunk(Coord::new(cx, level, cy), &Inputs { world }).expect("a sewer chunk")
    }

    #[test]
    fn the_shipped_file_parses_and_names_known_props() {
        let r = Rules::parse(FILE).unwrap();
        assert_eq!(&r, rules());
        assert!(r.count(-1) > 0 && r.count(-2) > 0);
        assert_eq!(r.count(0), 0);
        let content = crate::content::Content::load();
        for t in &r.types {
            let light = content.props.get(t).and_then(|d| d.light.clone()).expect("an emergency type has a light");
            assert!(light.flicker_hz > 0.0 && light.flicker_depth > 0.0, "{t} pulses");
        }
        assert!(Rules::parse(&FILE.replace("lr.sewer_emergency/1", "x")).is_err());
        assert!(Rules::parse(&FILE.replace("\"types\": [\"sewerEmergencyRed\", \"sewerEmergencyAmber\"]", "\"types\": []")).is_err());
        assert!(Rules::parse(&FILE.replace("level.-1", "floor.1")).is_err());
    }

    #[test]
    fn lights_stand_on_dry_cells_with_brick_behind_apart_and_off_other_props() {
        let r = rules();
        let mut total = 0;
        for world in WORLDS {
            for level in [-1, -2] {
                for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1)] {
                    let sc = chunk(world, cx, cy, level);
                    let out = props(&sc, world, cx, cy, level, &[]);
                    assert!(out.len() <= r.count(level));
                    total += out.len();
                    for (i, p) in out.iter().enumerate() {
                        assert!(["sewerEmergencyRed", "sewerEmergencyAmber"].contains(&p.ptype.as_str()));
                        assert!(!p.solid);
                        assert!(p.id.starts_with(&format!("SEWER:{cx}:{level}:{cy}:PROP:E")), "{}", p.id);
                        assert_eq!(sc.sewer_grid[p.cell.y as usize][p.cell.x as usize], DRY);
                        let back = (p.cell.x + (p.offset.0 * 2.0) as i32, p.cell.y + (p.offset.1 * 2.0) as i32);
                        assert_eq!(sc.sewer_grid[back.1 as usize][back.0 as usize], SOLID, "{} has no brick behind it", p.id);
                        // the yaw points at the wall (Godot's wall_yaw): the quarter turn's (sin, cos) is the wall step, so the light faces the open cell
                        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
                        let toward_wall = (yaw.sin().round() as i32, yaw.cos().round() as i32);
                        assert_eq!(toward_wall, (back.0 - p.cell.x, back.1 - p.cell.y), "{} faces the wrong way", p.id);
                        assert!(sc.region_props.iter().all(|o| (o.cell.x, o.cell.y) != (p.cell.x, p.cell.y)));
                        for o in &out[..i] {
                            let d = (o.cell.x - p.cell.x).abs().max((o.cell.y - p.cell.y).abs());
                            assert!(d >= r.min_gap_cells);
                        }
                    }
                }
            }
        }
        assert!(total > 10, "placed {total}");
    }

    #[test]
    fn placement_is_deterministic_and_leaves_the_generators_props_alone() {
        let sc = chunk("wired-sewer", 0, 0, -1);
        let before = sc.region_props.clone();
        let a = props(&sc, "wired-sewer", 0, 0, -1, &[]);
        assert_eq!(a.iter().map(|p| (p.id.clone(), p.cell, p.ptype.clone())).collect::<Vec<_>>(), props(&sc, "wired-sewer", 0, 0, -1, &[]).iter().map(|p| (p.id.clone(), p.cell, p.ptype.clone())).collect::<Vec<_>>());
        assert_eq!(sc.region_props, before);
        assert!(props(&sc, "wired-sewer", 0, 0, 0, &[]).is_empty(), "no lights off the sewer levels");
    }
}
