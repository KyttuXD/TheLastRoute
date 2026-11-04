//! The connection between levels (a stair flight or a sewer ladder): the data behind it. Nothing here fades or holds.
//! A stairwell taken by interact changes the level on the frame it is taken (`start_stairs`, which lands the runner on
//! the far landing through `take_flight`). A walked flight and a ladder's shaft change storey on the frame the runner
//! reaches the connection (game/endless.rs `walked_level_change`, game/ladder.rs `advance_ladder`), through the same
//! `change_storey` load, with the momentum and the cell kept.
//!
//! The landing offset and the prefetch distance are data (data/stairs_links.json, `links()`). Where the runner arrives
//! is `arrival`: the far end of the flight (foot B going down, landing A going up), put `landing_offset` in along the
//! direction the flight leaves that end by, facing along it. There is no far-cell arithmetic: both levels derive the
//! flight from the same connection (world/flight_links.rs), so the far end is the stair's own `far_end()`.
//! Godot reference: game/infinite_route/infinite_route_test.gd (`_update_player_ladder`, `_on_player_ladder_finished`).

use super::Game;
use crate::content::embedded;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::Stair;
use crate::world::infinite;
use std::sync::OnceLock;

/// The data file (under the embedded data root, see build.rs).
pub const STAIRS_LINKS_PATH: &str = "stairs_links.json";

/// The level-link numbers read from data/stairs_links.json.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StairsLinks {
    /// How far in from the far landing's cell, along the flight, the runner stands on arrival (cells).
    pub landing_offset: f32,
    /// How close (cells) the runner gets to a stair or ladder landing before the storey it leads to is built ahead
    /// (`Game::prefetch_connections`), so the change on the connection's frame builds no window.
    pub prefetch_distance: f32,
}

/// Reads a `StairsLinks` from the JSON text. A negative value is an error.
pub fn parse(text: &str) -> Result<StairsLinks, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let num = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_f64())
            .map(|x| x as f32)
            .ok_or_else(|| format!("{k}: a number is needed"))
    };
    let links = StairsLinks {
        landing_offset: num("landing_offset")?,
        prefetch_distance: num("prefetch_distance")?,
    };
    if !(links.prefetch_distance >= 0.0) {
        return Err("prefetch_distance must not be negative".into());
    }
    if !(links.landing_offset >= 0.0) {
        return Err("landing_offset must not be negative".into());
    }
    Ok(links)
}

static LINKS: OnceLock<StairsLinks> = OnceLock::new();

/// The level-link data, loaded once from the embedded data/stairs_links.json.
pub fn links() -> StairsLinks {
    *LINKS.get_or_init(|| {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == STAIRS_LINKS_PATH)
            .map(|(_, t)| t)
            .expect("data/stairs_links.json is embedded by build.rs");
        parse(text).expect("data/stairs_links.json is valid")
    })
}

/// The offset (cells) the runner is put in from a landing on arrival, for `Game::take_stairs`.
pub fn landing_offset() -> f32 {
    links().landing_offset
}

/// Where the runner stands, and which way it faces, after taking stair `s` on `from_level` (the level that holds `s`):
/// on the flight's far end (foot B going down, landing A going up), `landing_offset` in along the direction the flight
/// leaves that end by, facing along it. At the landing A that direction is `-run` (away from the flight, into the
/// room); at the foot B it is the unit vector of the last segment of the walk path (a switchback gives `-run`, a
/// straight flight `+run`, a spiral sideways). The yaw faces along that direction (Player forward is (-sin, -cos)).
pub fn arrival(s: &Stair, from_level: i32) -> ((f32, f32), f32) {
    let target = s.far_end();
    let landing = s.dir > 0;
    let e = if landing {
        (-(s.run.0 as f32), -(s.run.1 as f32))
    } else {
        let path = s.walk_path(from_level, 0.25);
        let last = *path.last().unwrap_or(&target.center());
        // the earliest sample from the end that is a real step back, so a short final segment cannot skew the direction
        let back = path
            .iter()
            .rev()
            .find(|p| (last.0 - p.0).hypot(last.1 - p.1) >= 0.3)
            .copied()
            .unwrap_or((last.0 + s.run.0 as f32, last.1 + s.run.1 as f32));
        let (dx, dz) = (last.0 - back.0, last.1 - back.1);
        let len = dx.hypot(dz).max(1e-6);
        (dx / len, dz / len)
    };
    let (cx, cz) = target.center();
    let off = landing_offset();
    ((cx + e.0 * off, cz + e.1 * off), (-e.0).atan2(-e.1))
}

impl Game {
    /// Takes stairwell `i` (interact's `Focus::Stairs`) on this frame: the level changes and the runner is on the far
    /// landing at once, with no fade and no climb. A level the run may not enter is refused here.
    pub(super) fn start_stairs(&mut self, i: usize, content: &Content, catalog: &Catalog) {
        let s = self.fac.stairs[i];
        let to_level = self.inf.as_ref().map_or(0, |inf| inf.level) + s.dir;
        if !infinite::level_allowed(to_level) {
            return;
        }
        self.focus = None;
        self.take_flight(i, content, catalog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Standard values, as the game had them before they moved into data/stairs_links.json.
    fn standard() -> StairsLinks {
        StairsLinks {
            landing_offset: 0.6,
            prefetch_distance: 8.0,
        }
    }

    fn with_json(key: &str, value: &str) -> String {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == STAIRS_LINKS_PATH)
            .map(|(_, t)| t)
            .expect("embedded");
        let mut v: serde_json::Value = serde_json::from_str(text).expect("json");
        v[key] = serde_json::from_str(value).expect("value");
        v.to_string()
    }

    #[test]
    fn the_embedded_data_file_is_the_standard_values() {
        let got = links();
        let want = standard();
        assert!((got.landing_offset - want.landing_offset).abs() < 1e-6);
        assert!((got.prefetch_distance - want.prefetch_distance).abs() < 1e-6);
        assert_eq!(landing_offset(), got.landing_offset);
    }

    #[test]
    fn a_changed_landing_offset_in_the_data_is_read() {
        let text = with_json("landing_offset", "0.3");
        assert!((parse(&text).expect("valid").landing_offset - 0.3).abs() < 1e-6);
        assert!((standard().landing_offset - 0.6).abs() < 1e-6, "the standard value is unchanged");
    }

    #[test]
    fn a_negative_value_in_the_data_is_refused() {
        assert!(parse(&with_json("landing_offset", "-1")).is_err(), "a negative offset is refused");
        assert!(parse(&with_json("prefetch_distance", "-1")).is_err(), "a negative prefetch distance is refused");
    }

    /// A real window with a Stairs flight of `shape` on `level` (ROOM-TEST-n, n < 300).
    fn find(shape: crate::world::flight::FlightShape, level: i32) -> Option<(Game, Stair)> {
        let (content, catalog) = (Content::load(), Catalog::load());
        for n in 0..300 {
            let text = format!("ROOM-TEST-{n}");
            let g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &text, level, &content, &catalog);
            let s = g
                .fac
                .stairs
                .iter()
                .copied()
                .find(|s| matches!(s.kind, crate::world::facility::LinkKind::Stairs) && s.shape == shape);
            if let Some(s) = s {
                return Some((g, s));
            }
        }
        None
    }

    #[test]
    fn arrival_is_offset_in_from_the_far_end_and_faces_along_the_exit() {
        use crate::world::flight::FlightShape::*;
        let mut checked = 0;
        for (shape, level) in [(Switchback, 0), (Spiral, 0), (Straight, -1)] {
            let Some((g, s)) = find(shape, level) else { continue };
            let (pos, yaw) = arrival(&s, g.fac.level);
            let (cx, cz) = s.far_end().center();
            let d = (pos.0 - cx).hypot(pos.1 - cz);
            assert!((d - landing_offset()).abs() < 1e-4, "{shape:?}: {d} m from the far end's centre");
            // the facing is the exit direction: forward is (-sin yaw, -cos yaw)
            let fwd = (-yaw.sin(), -yaw.cos());
            let e = ((pos.0 - cx) / d, (pos.1 - cz) / d);
            assert!((fwd.0 - e.0).abs() < 1e-3 && (fwd.1 - e.1).abs() < 1e-3, "{shape:?}: faces along the exit {e:?}, not {fwd:?}");
            if s.dir > 0 {
                assert!((e.0 + s.run.0 as f32).abs() < 1e-4 && (e.1 + s.run.1 as f32).abs() < 1e-4, "landing A exits by -run");
            }
            checked += 1;
        }
        assert!(checked >= 1, "a flight was found in the scanned windows");
    }
}
