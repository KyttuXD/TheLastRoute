//! How the maps tint the sewer's water (data/ui/map.json, `lr.map_look/1`), validated on load. The water cells themselves are
//! the sewer chunk grid's WATER cells (`Facility::is_water`, set in `world/infinite.rs`), the same ones the swim, slip and
//! landing code read, so the map shows exactly the water the runner can wade or swim in.

use crate::ui::style::*;
use macroquad::prelude::*;

/// The embedded look table.
const FILE: &str = include_str!("../../data/ui/map.json");
const FORMAT: &str = "lr.map_look/1";

/// The water's tint: the colour a discovered water cell moves toward, how far, and the legend word.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterLook {
    pub fill: Color,
    pub strength: f32,
    pub legend: String,
}

/// The validated table.
#[derive(Clone, Debug, PartialEq)]
pub struct MapLook {
    pub water: WaterLook,
    /// Cells the runner sees around itself for the discovery, in line of sight.
    pub reveal_radius: i32,
}

impl WaterLook {
    /// `floor` moved `strength` of the way toward the water fill (alpha kept at the floor's).
    pub fn over(&self, floor: Color) -> Color {
        let k = self.strength;
        Color::new(
            floor.r + (self.fill.r - floor.r) * k,
            floor.g + (self.fill.g - floor.g) * k,
            floor.b + (self.fill.b - floor.b) * k,
            floor.a,
        )
    }
}

fn hex6(v: &serde_json::Value, key: &str) -> Result<Color, String> {
    let s = v.as_str().ok_or_else(|| format!("{key}: expected a \"#rrggbb\" string, found {v}"))?;
    let digits = s.strip_prefix('#').filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_hexdigit()));
    let digits = digits.ok_or_else(|| format!("{key}: {s:?} is not a #rrggbb colour"))?;
    let n = u32::from_str_radix(digits, 16).map_err(|e| format!("{key}: {s:?}: {e}"))?;
    Ok(hex(n))
}

impl MapLook {
    /// Reads and validates a table. The error names the key at fault.
    pub fn parse(text: &str) -> Result<MapLook, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("map.json is not JSON: {e}"))?;
        match v.get("format").and_then(|f| f.as_str()) {
            Some(FORMAT) => {}
            other => return Err(format!("format: expected {FORMAT:?}, found {other:?}")),
        }
        let water = v.get("water").ok_or("water: missing")?;
        let fill = hex6(water.get("fill").ok_or("water.fill: missing")?, "water.fill")?;
        let strength = water
            .get("strength")
            .and_then(|s| s.as_f64())
            .filter(|s| (0.0..=1.0).contains(s))
            .ok_or("water.strength: expected a number from 0 to 1")? as f32;
        let legend = water
            .get("legend")
            .and_then(|s| s.as_str())
            .filter(|s| !s.trim().is_empty() && s.len() <= 24)
            .ok_or("water.legend: expected a short non-empty string")?
            .to_string();
        let reveal_radius = match v.get("reveal").and_then(|r| r.get("radius_cells")) {
            None => 7,
            Some(r) => r
                .as_i64()
                .filter(|r| (1..=20).contains(r))
                .ok_or("reveal.radius_cells: expected a whole number from 1 to 20")? as i32,
        };
        Ok(MapLook { water: WaterLook { fill, strength, legend }, reveal_radius })
    }

    /// The embedded table. A broken file is reported once on stderr and the shipped numbers are used.
    pub fn load() -> &'static MapLook {
        static LOOK: std::sync::OnceLock<MapLook> = std::sync::OnceLock::new();
        LOOK.get_or_init(|| {
            Self::parse(FILE).unwrap_or_else(|e| {
                eprintln!("data/ui/map.json: {e}; the maps use the built-in water tint");
                MapLook {
                    water: WaterLook { fill: hex(0x2f78c8), strength: 0.72, legend: "BLUE water".to_string() },
                    reveal_radius: 7,
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_table_parses() {
        let look = MapLook::parse(FILE).expect("data/ui/map.json is valid");
        assert!(look.water.legend.contains("water"));
        assert_eq!(look.reveal_radius, 7);
        assert_eq!(&MapLook::load().water, &look.water);
    }

    #[test]
    fn a_changed_value_changes_the_tint_and_the_radius() {
        let edited = FILE.replace("\"#2f78c8\"", "\"#ff0000\"").replace("\"radius_cells\": 7", "\"radius_cells\": 9");
        let look = MapLook::parse(&edited).expect("edited table");
        assert_eq!(look.water.fill, hex(0xff0000));
        assert_eq!(look.reveal_radius, 9);
        // strength 1 is the flat fill, 0 the floor as it was
        let floor = hex(0x101010);
        let flat = WaterLook { strength: 1.0, ..look.water.clone() };
        assert_eq!(flat.over(floor), hex(0xff0000));
        let none = WaterLook { strength: 0.0, ..look.water };
        assert_eq!(none.over(floor), floor);
    }

    #[test]
    fn bad_values_are_refused() {
        assert!(MapLook::parse(&FILE.replace("lr.map_look/1", "lr.other/1")).unwrap_err().contains("format"));
        assert!(MapLook::parse(&FILE.replace("\"#2f78c8\"", "\"blue\"")).unwrap_err().contains("water.fill"));
        assert!(MapLook::parse(&FILE.replace("0.72", "1.5")).unwrap_err().contains("water.strength"));
        assert!(MapLook::parse(&FILE.replace("\"radius_cells\": 7", "\"radius_cells\": 0")).unwrap_err().contains("radius"));
    }
}
