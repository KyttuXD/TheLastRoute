//! The sewer's water as numbers: where its surface sits and how deep it is over the channel bed.
//!
//! Godot (`SewerRouteGenerator`, sewer_route_generator.gd:91-96): the bed lies `CHANNEL_DEPTH` (1.0) below the
//! walkways and the water surface `WATER_SURFACE` (-0.3) below them, so the water over the bed is 0.7 deep.
//! Both numbers, the shore fraction and the tint live in `data/sewer_water.json` (embedded as `sewer_water.json`);
//! this file holds only the behaviour. `water_depth_at` is pure: the grid in, metres of water out.
//! The mesh is `render::water`; swim and plunge physics read `water_depth_at`.

use std::sync::OnceLock;

use serde_json::Value;

use super::core::{Grid, N, WATER};

/// The water rules from `sewer_water.json`. Offsets are metres relative to the storey floor (negative = down).
#[derive(Clone, Debug, PartialEq)]
pub struct WaterParams {
    /// Godot CHANNEL_DEPTH: how far the channel bed lies below the floor (a positive number).
    pub channel_depth: f32,
    /// Godot WATER_SURFACE: the surface offset from the floor (negative = below it).
    pub water_surface: f32,
    /// Fraction of the full depth a cell reads at the shore, for the tint (0..=1).
    pub shore_depth_fraction: f32,
    /// RGBA of the surface over the shallowest water and over the bed.
    pub shallow: [u8; 4],
    pub deep: [u8; 4],
    /// Shape of the shallow-to-deep blend (1 = linear, above 1 = stays pale longer).
    pub deepen_power: f32,
    /// How far the sewers' ambient light leans toward the water's shallow tint (0..=1; the optional `glow_mix`, 0 if absent).
    /// Shipped at 0: a positive mix washes the sewer walls with the water colour and reads as a glow in the dark.
    pub glow_mix: f32,
    /// Prop types that may occupy a water cell (pipes, bridges, grates, sluice gates). Anything else, and every
    /// collectable, must be placed on dry floor.
    pub may_stand_in_water: Vec<String>,
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key).and_then(Value::as_f64).map(|f| f as f32).ok_or_else(|| format!("sewer_water.json: \"{key}\" must be a number"))
}

fn rgba(v: &Value, key: &str) -> Result<[u8; 4], String> {
    let a = v.get(key).and_then(Value::as_array).filter(|a| a.len() == 4);
    let a = a.ok_or_else(|| format!("sewer_water.json: tint.{key} must be 4 numbers"))?;
    let mut out = [0u8; 4];
    for (slot, x) in out.iter_mut().zip(a) {
        *slot = x.as_u64().filter(|n| *n <= 255).ok_or_else(|| format!("sewer_water.json: tint.{key} holds 0..=255"))? as u8;
    }
    Ok(out)
}

impl WaterParams {
    pub fn from_text(text: &str) -> Result<WaterParams, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("sewer_water.json: {e}"))?;
        let tint = root.get("tint").ok_or_else(|| "sewer_water.json has no \"tint\" block".to_string())?;
        let p = WaterParams {
            channel_depth: num(&root, "channel_depth")?,
            water_surface: num(&root, "water_surface")?,
            shore_depth_fraction: num(&root, "shore_depth_fraction")?,
            shallow: rgba(tint, "shallow")?,
            deep: rgba(tint, "deep")?,
            deepen_power: num(tint, "deepen_power")?,
            glow_mix: if root.get("glow_mix").is_some() { num(&root, "glow_mix")? } else { 0.0 },
            may_stand_in_water: match root.get("may_stand_in_water") {
                None => Vec::new(),
                Some(v) => {
                    let list = v.as_array().ok_or("sewer_water.json: may_stand_in_water must be a list of prop types")?;
                    let mut names = Vec::with_capacity(list.len());
                    for (i, item) in list.iter().enumerate() {
                        let name = item.as_str().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
                            .ok_or_else(|| format!("sewer_water.json: may_stand_in_water[{i}] must be a prop type name"))?;
                        names.push(name.to_string());
                    }
                    names
                }
            },
        };
        if !(0.0..=1.0).contains(&p.glow_mix) {
            return Err("sewer_water.json: glow_mix is 0..=1".into());
        }
        if p.channel_depth <= 0.0 || p.water_surface >= 0.0 || p.water_surface <= -p.channel_depth {
            return Err("sewer_water.json: the surface must lie below the floor and above the bed".into());
        }
        if !(0.0..=1.0).contains(&p.shore_depth_fraction) || p.deepen_power <= 0.0 {
            return Err("sewer_water.json: shore_depth_fraction is 0..=1 and deepen_power above 0".into());
        }
        Ok(p)
    }

    /// Metres of water over the bed in a water cell (surface above bed): 1.0 + -0.3 = 0.7 in Standard.
    pub fn full_depth(&self) -> f32 {
        self.channel_depth + self.water_surface
    }

    /// `base` (an ambient colour, 0..1) moved `glow_mix` of the way toward the shallow tint's colour: the water's glow
    /// on the sewer walls.
    pub fn glow_ambient(&self, base: [f32; 3]) -> [f32; 3] {
        let mut out = base;
        for (o, s) in out.iter_mut().zip(self.shallow) {
            *o += (s as f32 / 255.0 - *o) * self.glow_mix;
        }
        out
    }

    /// The bed's height relative to the floor.
    pub fn bed_offset(&self) -> f32 {
        -self.channel_depth
    }
}

/// Whether a prop of type `kind` is allowed to occupy a channel-water cell. Collectables are never on this list.
pub fn may_stand_in_water(kind: &str) -> bool {
    params().may_stand_in_water.iter().any(|s| s == kind)
}

/// The built-in water rules, read once.
pub fn params() -> &'static WaterParams {
    static P: OnceLock<WaterParams> = OnceLock::new();
    P.get_or_init(|| {
        let text = crate::content::embedded::data_under("sewer_water.json")
            .next()
            .map(|(_, t)| t)
            .expect("sewer_water.json is embedded");
        WaterParams::from_text(text).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// Metres of water over the bed for a grid code: the full depth in a WATER cell, 0 elsewhere.
pub fn depth_of_code(code: u8, p: &WaterParams) -> f32 {
    if code == WATER {
        p.full_depth()
    } else {
        0.0
    }
}

/// How deep the water is in cell (x, y) of a sewer chunk grid; 0 for dry land, bridges and anything off the grid.
pub fn water_depth_at(grid: &Grid, x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || x >= N as i32 || y >= N as i32 {
        return 0.0;
    }
    depth_of_code(grid[y as usize][x as usize], params())
}

/// Metres of water over the bed on a storey: the same on every sewer storey (Godot has one `CHANNEL_DEPTH` and one
/// `WATER_SURFACE`), so the swim physics, the hunters and the landing agree with the drawn bed.
///
/// The `level` argument is kept on purpose: level -2 used to have its own 1.4 m depth, and a per-storey depth is one
/// match arm here if the owner wants it back (docs/SEWERS.md, "Behaviour change").
pub fn full_depth_for_level(_level: i32) -> f32 {
    params().full_depth()
}

/// `water_depth_at` for a chunk of the given storey.
pub fn water_depth_at_level(grid: &Grid, x: i32, y: i32, level: i32) -> f32 {
    if water_depth_at(grid, x, y) > 0.0 {
        full_depth_for_level(level)
    } else {
        0.0
    }
}

/// The water surface height for a storey whose floor is at `floor_y`.
pub fn surface_y(floor_y: f32) -> f32 {
    floor_y + params().water_surface
}

/// The channel bed height for a storey whose floor is at `floor_y`.
pub fn bed_y(floor_y: f32) -> f32 {
    floor_y + params().bed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::sewer::core::{DRY, SOLID};

    fn grid_with(cells: &[(usize, usize, u8)]) -> Grid {
        let mut g: Grid = [[SOLID; N]; N];
        for (x, y, c) in cells {
            g[*y][*x] = *c;
        }
        g
    }

    #[test]
    fn standard_values_match_godot() {
        let p = params();
        assert_eq!(p.channel_depth, 1.0);
        assert_eq!(p.water_surface, -0.3);
        assert!((p.full_depth() - 0.7).abs() < 1e-6);
        assert_eq!(p.bed_offset(), -1.0);
    }

    #[test]
    fn depth_is_the_full_depth_in_water_and_zero_elsewhere() {
        let g = grid_with(&[(5, 5, WATER), (6, 5, DRY)]);
        assert!(water_depth_at(&g, 5, 5) > 0.0);
        assert_eq!(water_depth_at(&g, 5, 5), params().full_depth());
        assert_eq!(water_depth_at(&g, 6, 5), 0.0);
        assert_eq!(water_depth_at(&g, 7, 5), 0.0);
        assert_eq!(water_depth_at(&g, -1, 0), 0.0);
        assert_eq!(water_depth_at(&g, 0, N as i32), 0.0);
    }

    #[test]
    fn surface_lies_between_the_bed_and_the_floor() {
        let floor = -3.0;
        assert!(bed_y(floor) < surface_y(floor) && surface_y(floor) < floor);
    }

    #[test]
    fn a_data_change_changes_the_depth() {
        let text = r#"{"channel_depth":2.0,"water_surface":-0.5,"shore_depth_fraction":0.5,
            "tint":{"shallow":[1,2,3,4],"deep":[5,6,7,8],"deepen_power":1.0}}"#;
        let p = WaterParams::from_text(text).unwrap();
        assert!((depth_of_code(WATER, &p) - 1.5).abs() < 1e-6);
        assert_eq!(depth_of_code(DRY, &p), 0.0);
    }

    #[test]
    fn the_shipped_water_does_not_glow_and_names_what_may_stand_in_it() {
        let p = params();
        assert_eq!(p.glow_mix, 0.0, "a glow mix washes the dark sewer with the water tint");
        assert_eq!(p.glow_ambient([0.12, 0.1, 0.14]), [0.12, 0.1, 0.14]);
        let glowing = WaterParams { glow_mix: 0.5, ..p.clone() };
        let out = glowing.glow_ambient([0.0, 0.0, 0.0]);
        for (o, s) in out.iter().zip(p.shallow) {
            assert!((o - s as f32 / 255.0 * 0.5).abs() < 1e-6);
        }
        assert!(may_stand_in_water("sewerPipeRun") && may_stand_in_water("sewerBridge") && may_stand_in_water("sewerGrateDeck"));
        assert!(may_stand_in_water("sewerSluiceGate") && may_stand_in_water("sewerOutflowPipe"));
        assert!(!may_stand_in_water("sewerLamp") && !may_stand_in_water("sewerPump") && !may_stand_in_water("barrel"));
    }

    #[test]
    fn bad_data_is_refused() {
        let bad = r#"{"channel_depth":1.0,"water_surface":-1.5,"shore_depth_fraction":0.5,
            "tint":{"shallow":[1,2,3,4],"deep":[5,6,7,8],"deepen_power":1.0}}"#;
        assert!(WaterParams::from_text(bad).is_err());
        assert!(WaterParams::from_text("{}").is_err());
    }

    #[test]
    fn every_sewer_storey_has_the_same_water_depth_as_the_drawn_bed() {
        use crate::world::route::sewer::core::{LEVEL, LOWER_LEVEL};
        assert_eq!(full_depth_for_level(LEVEL), params().full_depth());
        assert_eq!(full_depth_for_level(LOWER_LEVEL), params().full_depth());
        assert_eq!(full_depth_for_level(LOWER_LEVEL), params().channel_depth + params().water_surface, "render bed = physics");
        let g = grid_with(&[(3, 3, WATER)]);
        assert_eq!(water_depth_at_level(&g, 3, 3, LOWER_LEVEL), params().full_depth());
        assert_eq!(water_depth_at_level(&g, 4, 3, LOWER_LEVEL), 0.0);
        assert_eq!(water_depth_at_level(&g, 3, 3, LEVEL), params().full_depth());
    }

    #[test]
    fn the_flood_numbers_are_not_sewer_water_data() {
        // a pipe flood's depth and drain live in data/pipes.json (flow.max_depth, shut_drain_seconds)
        let shipped: Value = serde_json::from_str(
            crate::content::embedded::data_under("sewer_water.json").next().map(|(_, t)| t).expect("embedded"),
        )
        .unwrap();
        assert!(shipped.get("flood").is_none(), "sewer_water.json carries a dead flood block");
        // an old file that still has the block loads, the block is ignored
        let text = r#"{"channel_depth":1.0,"water_surface":-0.3,"shore_depth_fraction":0.5,
            "flood":{"depth":0.0,"valve_drain":0.0},
            "tint":{"shallow":[1,2,3,4],"deep":[5,6,7,8],"deepen_power":1.0}}"#;
        assert!(WaterParams::from_text(text).is_ok());
    }
}
