//! The connection between levels (a stair flight or a sewer ladder): the data behind it. Nothing here fades or holds.
//! A stairwell taken by interact changes the level on the frame it is taken (`start_stairs`, which lands the runner on
//! the far landing through `take_flight`). A walked flight and a ladder's shaft change storey on the frame the runner
//! reaches the connection (game/endless.rs `walked_level_change`, game/ladder.rs `advance_ladder`), through the same
//! `change_storey` load, with the momentum and the cell kept.
//!
//! The landing offset and the far end's distance are data (data/stairs_links.json, `links()`). The far end must be the
//! landing the world generator lays out one storey over (`STAIR_FLIGHT + 1` steps), so `parse` refuses any other
//! `flight_steps`. Godot reference: game/infinite_route/infinite_route_test.gd (`_update_player_ladder`,
//! `_on_player_ladder_finished`).

use super::Game;
use crate::content::embedded;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::STAIR_FLIGHT;
use crate::world::infinite;
use std::sync::OnceLock;

/// The data file (under the embedded data root, see build.rs).
pub const STAIRS_LINKS_PATH: &str = "stairs_links.json";

/// The level-link numbers read from data/stairs_links.json.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StairsLinks {
    /// How far in from the far landing's cell, along the flight, the runner stands on arrival (cells).
    pub landing_offset: f32,
    /// Cells from the near landing to the far one (the flight's length plus one).
    pub flight_steps: i32,
    /// How close (cells) the runner gets to a stair or ladder landing before the storey it leads to is built ahead
    /// (`Game::prefetch_connections`), so the change on the connection's frame builds no window.
    pub prefetch_distance: f32,
}

/// Reads a `StairsLinks` from the JSON text. A value out of range or a `flight_steps` other than the world's is an error.
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
        flight_steps: num("flight_steps")? as i32,
        prefetch_distance: num("prefetch_distance")?,
    };
    if links.flight_steps != STAIR_FLIGHT + 1 {
        return Err(format!("flight_steps must be {} (the world's flight plus one)", STAIR_FLIGHT + 1));
    }
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
            flight_steps: STAIR_FLIGHT + 1,
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
        assert_eq!(got.flight_steps, want.flight_steps);
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
    fn the_flight_length_in_the_data_must_match_the_world() {
        assert!(parse(&with_json("flight_steps", "6")).is_err(), "a far end the world does not lay out is refused");
        assert!(parse(&with_json("landing_offset", "-1")).is_err(), "a negative offset is refused");
        assert!(parse(&with_json("prefetch_distance", "-1")).is_err(), "a negative prefetch distance is refused");
    }
}
