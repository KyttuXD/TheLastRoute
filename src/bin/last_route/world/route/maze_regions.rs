//! Level 0 maze variants: a courtyard, a warehouse, a library and a boiler room, each a data file
//! `src/bin/last_route/data/regions/maze_<name>.json` (`lr.maze_region/1`, embedded by build.rs). A variant has floors, room
//! roles, a prop set, spawn rules and a weight; the picker mixes them into the level 0 maze area.
//!
//! These files are not `lr.region/1` (that schema has no weight or spawn rules), so the region catalogue in
//! `packs::regions` does not read them. This module does. The plain maze (`regions/maze.json`) stays the
//! default: it competes with the variants at `plain_weight`, which every file must state the same.
//!
//! RNG: `"<world>:MAZE-VARIANT:<rx>:<rz>"`, one `next_float` per region block, so a block's variant is a pure
//! function of (world seed text, region x, region z). The Godot region draw is not touched.

use crate::content::embedded;
use crate::world::rng::Rng;
use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.maze_region/1";
const FILE_PREFIX: &str = "regions/maze_";

/// One prop of a variant's set.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantProp {
    pub kind: String,
    pub size: [f32; 3],
    pub solid: bool,
}

/// Spawn rules: scales on the level 0 base rates (1.0 is the plain maze).
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnRules {
    pub hunter_chance_scale: f64,
    pub loot_scale: f64,
    pub prop_density: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MazeVariant {
    pub id: String,
    pub label: String,
    pub weight: f64,
    pub plain_weight: f64,
    pub hall_floor: Vec<String>,
    pub hall_runner: bool,
    pub room_default_floor: Vec<String>,
    pub room_roles: Vec<String>,
    pub prop_set: Vec<VariantProp>,
    pub spawn_rules: SpawnRules,
}

fn strings(v: &Value, file: &str, key: &str) -> Result<Vec<String>, String> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{file}: \"{key}\" must be an array of strings"))?;
    let out: Vec<String> = arr.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
    if out.len() != arr.len() || out.is_empty() {
        return Err(format!("{file}: \"{key}\" must be a non-empty array of strings"));
    }
    Ok(out)
}

fn number(v: &Value, file: &str, key: &str, min: f64) -> Result<f64, String> {
    match v.get(key).and_then(Value::as_f64) {
        Some(n) if n >= min && n.is_finite() => Ok(n),
        _ => Err(format!("{file}: \"{key}\" must be a number of at least {min}")),
    }
}

/// Parses one variant file.
pub fn parse_variant(file: &str, text: &str) -> Result<MazeVariant, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("{file}: \"schema\" must be \"{SCHEMA}\""));
    }
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{file}: \"id\" must be a non-empty string"))?;
    let props = v
        .get("prop_set")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{file}: \"prop_set\" must be an array"))?;
    let mut prop_set = Vec::new();
    for p in props {
        let kind = p
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{file}: a prop needs a \"type\""))?;
        let size: Vec<f32> = p
            .get("size")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|n| n.as_f64()).map(|n| n as f32).collect())
            .unwrap_or_default();
        if size.len() != 3 || size.iter().any(|n| *n <= 0.0) {
            return Err(format!("{file}: prop \"{kind}\" needs a \"size\" of three positive numbers"));
        }
        prop_set.push(VariantProp {
            kind: kind.to_string(),
            size: [size[0], size[1], size[2]],
            solid: p.get("solid").and_then(Value::as_bool).unwrap_or(true),
        });
    }
    if prop_set.is_empty() {
        return Err(format!("{file}: \"prop_set\" must not be empty"));
    }
    let rules = v
        .get("spawn_rules")
        .ok_or_else(|| format!("{file}: missing \"spawn_rules\""))?;
    Ok(MazeVariant {
        id: id.to_string(),
        label: v.get("label").and_then(Value::as_str).unwrap_or(id).to_string(),
        weight: number(&v, file, "weight", 0.0)?,
        plain_weight: number(&v, file, "plain_weight", 0.0)?,
        hall_floor: strings(&v, file, "hall_floor")?,
        hall_runner: v.get("hall_runner").and_then(Value::as_bool).unwrap_or(false),
        room_default_floor: strings(&v, file, "room_default_floor")?,
        room_roles: strings(&v, file, "room_roles")?,
        prop_set,
        spawn_rules: SpawnRules {
            hunter_chance_scale: number(rules, file, "hunter_chance_scale", 0.0)?,
            loot_scale: number(rules, file, "loot_scale", 0.0)?,
            prop_density: number(rules, file, "prop_density", 0.0)?,
        },
    })
}

/// Parses `(file, text)` pairs, sorted by id. Every file must state the same `plain_weight`.
pub fn parse_all<'a>(files: impl Iterator<Item = (&'a str, &'a str)>) -> Result<Vec<MazeVariant>, String> {
    let mut out: Vec<MazeVariant> = Vec::new();
    for (file, text) in files {
        let v = parse_variant(file, text)?;
        if out.iter().any(|o| o.id == v.id) {
            return Err(format!("{file}: duplicate id {}", v.id));
        }
        if let Some(first) = out.first() {
            if first.plain_weight != v.plain_weight {
                return Err(format!("{file}: \"plain_weight\" differs from {}", first.id));
            }
        }
        out.push(v);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// The embedded variants. A broken built-in file is a build error, so this panics with the message.
pub fn variants() -> &'static [MazeVariant] {
    static CELL: OnceLock<Vec<MazeVariant>> = OnceLock::new();
    CELL.get_or_init(|| {
        parse_all(embedded::data_under(FILE_PREFIX)).unwrap_or_else(|e| panic!("maze variants: {e}"))
    })
}

/// The pick of one region block from `variants`: `None` is the plain maze. Pure in (world, rx, rz).
pub fn pick_from<'a>(variants: &'a [MazeVariant], world: &str, rx: i32, rz: i32) -> Option<&'a MazeVariant> {
    let first = variants.first()?;
    let total: f64 = first.plain_weight + variants.iter().map(|v| v.weight).sum::<f64>();
    if total <= 0.0 {
        return None;
    }
    let mut roll = Rng::new(&format!("{world}:MAZE-VARIANT:{rx}:{rz}")).next_float() * total;
    if roll < first.plain_weight {
        return None;
    }
    roll -= first.plain_weight;
    for v in variants {
        if roll < v.weight {
            return Some(v);
        }
        roll -= v.weight;
    }
    None
}

/// The variant of the maze block (rx, rz) on level 0; `None` keeps the plain maze. Call it only for a
/// chunk whose `regions::region_for` is "maze".
pub fn maze_variant(world: &str, rx: i32, rz: i32) -> Option<&'static MazeVariant> {
    pick_from(variants(), world, rx, rz)
}

/// The same, for a chunk coordinate (cx, cz): the region block is `REGION_SIZE_CHUNKS` chunks wide.
pub fn maze_variant_for_chunk(world: &str, cx: i32, cz: i32) -> Option<&'static MazeVariant> {
    let s = super::regions::REGION_SIZE_CHUNKS;
    maze_variant(world, cx.div_euclid(s), cz.div_euclid(s))
}

/// The variant that dresses the chunk (cx, cz) on `level`: `None` for any level but 0, for a chunk whose region is not
/// the maze, and for a plain-maze pick. This is the one gate the level 0 chunk build, the hunter spawn chance and the
/// renderer go through, so they agree on a chunk's variant.
pub fn variant_at(world: &str, cx: i32, cz: i32, level: i32) -> Option<&'static MazeVariant> {
    if level != 0 {
        return None;
    }
    // the Godot goldens (`layout::with_godot_regions`) read Godot's draw, every other call the concept map
    let region = if super::layout::godot_regions() {
        super::regions::godot_region_for(world, cx, cz, 0)
    } else {
        super::regions::region_for(world, cx, cz, 0)
    };
    if region != "maze" {
        return None;
    }
    maze_variant_for_chunk(world, cx, cz)
}

/// `base` scaled by a variant's `hunter_chance_scale`, kept in 0..=1. No variant leaves `base` unchanged.
pub fn scaled_chance(base: f64, v: Option<&MazeVariant>) -> f64 {
    match v {
        Some(v) => (base * v.spawn_rules.hunter_chance_scale).clamp(0.0, 1.0),
        None => base,
    }
}

/// The hunter spawn chance of chunk (cx, cz) on `level` from the level's `base` chance (0.2848 on level 0).
pub fn hunter_chance_at(base: f64, world: &str, cx: i32, cz: i32, level: i32) -> f64 {
    scaled_chance(base, variant_at(world, cx, cz, level))
}

/// Whether the `n`th of `count` things (loot or props, in layout order) of a chunk survives `scale` (a variant's
/// `loot_scale` or `prop_density`): a scale of 1 or more keeps all, below 1 keeps that share, deterministically.
pub fn keeps(scale: f64, seed: &str, tag: &str, n: usize) -> bool {
    if scale >= 1.0 {
        return true;
    }
    Rng::new(&format!("{seed}:MAZE-KEEP:{tag}:{n}")).next_float() < scale.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(id: &str, weight: f64, plain: f64) -> String {
        format!(
            r#"{{"schema":"lr.maze_region/1","id":"{id}","weight":{weight},"plain_weight":{plain},"hall_floor":["a"],"room_default_floor":["a"],"room_roles":["r"],"prop_set":[{{"type":"crate","size":[1,1,1]}}],"spawn_rules":{{"hunter_chance_scale":1,"loot_scale":1,"prop_density":1}}}}"#
        )
    }

    #[test]
    fn the_four_built_in_variants_load_with_full_data() {
        let v = variants();
        let ids: Vec<&str> = v.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["maze_boiler", "maze_courtyard", "maze_library", "maze_warehouse"]);
        for x in v {
            assert!(x.weight > 0.0 && x.plain_weight > 0.0, "{}", x.id);
            assert!(!x.hall_floor.is_empty() && !x.room_default_floor.is_empty() && !x.room_roles.is_empty());
            assert!(x.prop_set.iter().any(|p| p.solid), "{} has a solid prop", x.id);
            assert!(x.spawn_rules.hunter_chance_scale > 0.0);
        }
    }

    #[test]
    fn the_pick_is_deterministic_and_mixes_plain_and_variants() {
        let mut plain = 0;
        let mut seen = std::collections::HashSet::new();
        for rx in -20..20 {
            for rz in -20..20 {
                let a = maze_variant("12345", rx, rz).map(|v| v.id.clone());
                assert_eq!(a, maze_variant("12345", rx, rz).map(|v| v.id.clone()));
                match a {
                    None => plain += 1,
                    Some(id) => {
                        seen.insert(id);
                    }
                }
            }
        }
        assert_eq!(seen.len(), 4, "every variant appears");
        // plain weight 10 of 19.5 is about 51 percent of 1600 blocks
        assert!((500..=1100).contains(&plain), "plain share {plain}/1600");
    }

    #[test]
    fn a_changed_weight_changes_the_mix() {
        let heavy_text = one("a", 1000.0, 1.0);
        let heavy = parse_all([("a.json", heavy_text.as_str())].into_iter()).unwrap();
        let hits = (0..200).filter(|i| pick_from(&heavy, "w", *i, 0).is_some()).count();
        assert!(hits > 190, "{hits}");
        let none_text = one("a", 0.0, 1.0);
        let none = parse_all([("a.json", none_text.as_str())].into_iter()).unwrap();
        assert!((0..200).all(|i| pick_from(&none, "w", i, 0).is_none()));
    }

    #[test]
    fn a_chunk_maps_to_its_region_block() {
        let s = super::super::regions::REGION_SIZE_CHUNKS;
        let a = maze_variant_for_chunk("9", 0, 0).map(|v| v.id.clone());
        assert_eq!(a, maze_variant_for_chunk("9", s - 1, s - 1).map(|v| v.id.clone()));
        assert_eq!(
            maze_variant_for_chunk("9", -1, -1).map(|v| v.id.clone()),
            maze_variant("9", -1, -1).map(|v| v.id.clone())
        );
    }

    #[test]
    fn bad_files_are_rejected() {
        assert!(parse_variant("f", &one("a", 1.0, 1.0).replace("lr.maze_region/1", "x")).is_err());
        assert!(parse_variant("f", &one("a", 1.0, 1.0).replace("\"size\":[1,1,1]", "\"size\":[1,1]")).is_err());
        let a = one("a", 1.0, 1.0);
        let b = one("b", 1.0, 2.0);
        assert!(parse_all([("a", a.as_str()), ("b", b.as_str())].into_iter()).is_err());
        assert!(parse_all([("a", a.as_str()), ("a2", a.as_str())].into_iter()).is_err());
    }

    #[test]
    fn a_variant_applies_only_to_level_zero_maze_chunks() {
        // the level 0 maze is the concept map's white area (and its utility area), so look over several seeds
        let mut seen = 0;
        for world in ["5", "6", "7", "8", "9", "10", "11", "12"] {
            for cx in -12..12 {
                for cz in -12..12 {
                    assert!(variant_at(world, cx, cz, -1).is_none());
                    if let Some(v) = variant_at(world, cx, cz, 0) {
                        seen += 1;
                        assert_eq!(super::super::regions::region_for(world, cx, cz, 0), "maze");
                        assert_eq!(Some(v.id.clone()), maze_variant_for_chunk(world, cx, cz).map(|v| v.id.clone()));
                    }
                }
            }
        }
        assert!(seen > 0, "no chunk got a variant");
    }

    #[test]
    fn the_hunter_chance_scales_by_the_variant_data_and_stays_a_chance() {
        let v = variants().first().unwrap();
        assert_eq!(scaled_chance(0.2848, None), 0.2848);
        let s = scaled_chance(0.2848, Some(v));
        assert!((s - (0.2848 * v.spawn_rules.hunter_chance_scale).clamp(0.0, 1.0)).abs() < 1e-12);
        let mut w = v.clone();
        w.spawn_rules.hunter_chance_scale = 100.0;
        assert_eq!(scaled_chance(0.2848, Some(&w)), 1.0);
        w.spawn_rules.hunter_chance_scale = 0.0;
        assert_eq!(scaled_chance(0.2848, Some(&w)), 0.0);
        // a chunk without a variant keeps the level's base chance
        assert_eq!(hunter_chance_at(0.2848, "5", 0, 0, -2), 0.2848);
    }

    #[test]
    fn keeps_thins_below_one_and_is_deterministic() {
        assert!((0..50).all(|n| keeps(1.0, "w", "0,0", n)));
        assert!((0..50).all(|n| !keeps(0.0, "w", "0,0", n)));
        let kept = (0..400).filter(|n| keeps(0.5, "w", "0,0", *n)).count();
        assert!((120..280).contains(&kept), "kept {kept}");
        assert_eq!(keeps(0.5, "w", "1,2", 7), keeps(0.5, "w", "1,2", 7));
    }
}
