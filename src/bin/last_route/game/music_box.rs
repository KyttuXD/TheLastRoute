//! The music box as a collectable: a route pickup that goes into the pack and is wound from it (game/items.rs).
//! Its counts and times are data (data/music_box.json): the boxes per region, the wound time by difficulty, the
//! winddown stretch and the tune's gain. Sources: GameConfig.MUSIC_BOXES_PER_REGION and MUSIC_BOX_WOUND_SECONDS
//! (game_config.gd 543-546) and MusicBoxes.mood_for (music_boxes.gd 31).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const FILE: &str = "music_box.json";

/// The parsed data/music_box.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Boxes per route region (`MUSIC_BOXES_PER_REGION`).
    pub boxes_per_region: usize,
    /// Wound seconds by difficulty id. "standard" is the fallback for any other id.
    pub wound_seconds: HashMap<String, f32>,
    /// The last stretch of a wound box, in seconds: Glyra's winding-down mood and the HUD's warning colour. (The tune's
    /// slowing is `slowdown_from_fraction`, not this.)
    pub winddown_seconds: f32,
    /// The tune plays at normal speed while the wind left is at least this fraction of the wound time (0.70 = 70 %),
    /// then slows and sinks as it drains to nothing (audio/cues/music_box.rs `slowdown_factor`).
    pub slowdown_from_fraction: f32,
    /// How fast the tune still plays just before the box empties (a factor of normal speed, tempo and pitch).
    pub slowdown_floor: f32,
    /// The tune loop's gain against the master bus.
    pub tune_gain: f32,
    /// Playback rate of each placed tune id (index = the placement's tune id, wraps). Optional key; default 1.0.
    pub placed_tune_rates: Vec<f32>,
    /// A placed box quieter than this per channel is not started. Optional key.
    pub placed_min_gain: f32,
}

/// The rules as the data file gives them. A missing or malformed file is a build defect (the file is embedded), so
/// this panics rather than guessing.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(FILE)
            .next()
            .map(|x| x.1)
            .unwrap_or_else(|| panic!("data/{FILE} is missing from the build"));
        parse(text)
    })
}

/// Reads the data file text. Split out so the tests can check the shipped file parses.
pub fn parse(text: &str) -> Rules {
    let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("data/{FILE}: {e}"));
    let num = |k: &str| v[k].as_f64().unwrap_or_else(|| panic!("data/{FILE}: {k} is not a number")) as f32;
    let wound_seconds = v["wound_seconds"]
        .as_object()
        .unwrap_or_else(|| panic!("data/{FILE}: wound_seconds is not an object"))
        .iter()
        .map(|(id, s)| (id.clone(), s.as_f64().unwrap_or_else(|| panic!("data/{FILE}: wound_seconds.{id}")) as f32))
        .collect::<HashMap<_, _>>();
    assert!(wound_seconds.contains_key("standard"), "data/{FILE}: wound_seconds needs a standard entry");
    Rules {
        boxes_per_region: v["boxes_per_region"]
            .as_u64()
            .unwrap_or_else(|| panic!("data/{FILE}: boxes_per_region is not a count")) as usize,
        wound_seconds,
        winddown_seconds: num("winddown_seconds"),
        slowdown_from_fraction: num("slowdown_from_fraction"),
        slowdown_floor: num("slowdown_floor"),
        tune_gain: num("tune_gain"),
        placed_tune_rates: v["placed_tune_rates"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| (x as f32).clamp(0.25, 4.0)).collect())
            .filter(|a: &Vec<f32>| !a.is_empty())
            .unwrap_or_else(|| vec![1.0]),
        placed_min_gain: v["placed_min_gain"].as_f64().unwrap_or(0.002) as f32,
    }
}

/// The seconds a box winds to on `difficulty_id` ("standard" for any id without its own entry).
pub fn wound_seconds(difficulty_id: &str) -> f32 {
    let r = rules();
    r.wound_seconds
        .get(difficulty_id)
        .or_else(|| r.wound_seconds.get("standard"))
        .copied()
        .unwrap_or(45.0)
}

/// Playback rate for the placed tune `tune` (wraps over the data list).
pub fn tune_rate(tune: u32) -> f32 {
    let r = &rules().placed_tune_rates;
    r[tune as usize % r.len()]
}

/// One placed music box as the audio sees it: its tune id from placement, where it stands (feet) and the wound
/// seconds it has left (0 = silent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedBox {
    pub tune: u32,
    pub pos: [f32; 3],
    pub left: f32,
}

/// A tune that should be playing, with its stereo gains from the listener (before the bus).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TuneRequest {
    pub tune: u32,
    pub gains: (f32, f32),
}

/// The tunes the wound placed boxes ask for, heard from `eye` with the camera's right / up / forward. Boxes that
/// share a tune (a pair) are one request: the louder box's gains win per channel. Sorted by tune id.
pub fn tune_requests(
    boxes: &[PlacedBox],
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
) -> Vec<TuneRequest> {
    let r = rules();
    let mut out: Vec<TuneRequest> = Vec::new();
    for b in boxes.iter().filter(|b| b.left > 0.0) {
        let (l, rr) = crate::audio::cues::positional::TUNE.gains_from(b.pos, eye, right, up, forward);
        let (l, rr) = (l * r.tune_gain, rr * r.tune_gain);
        if l.max(rr) < r.placed_min_gain {
            continue;
        }
        match out.iter_mut().find(|t| t.tune == b.tune) {
            Some(t) => t.gains = (t.gains.0.max(l), t.gains.1.max(rr)),
            None => out.push(TuneRequest { tune: b.tune, gains: (l, rr) }),
        }
    }
    out.sort_by_key(|t| t.tune);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AXES: ([f32; 3], [f32; 3], [f32; 3]) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]);

    fn heard(boxes: &[PlacedBox]) -> Vec<TuneRequest> {
        tune_requests(boxes, [0.0, 1.6, 0.0], AXES.0, AXES.1, AXES.2)
    }

    #[test]
    fn a_wound_box_asks_for_its_tune_and_an_empty_one_does_not() {
        let wound = PlacedBox { tune: 2, pos: [0.0, 0.0, -3.0], left: 20.0 };
        let r = heard(&[wound]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].tune, 2);
        assert!(r[0].gains.0 > 0.0 && r[0].gains.1 > 0.0);
        assert!(heard(&[PlacedBox { left: 0.0, ..wound }]).is_empty());
    }

    #[test]
    fn a_pair_shares_one_tune_and_the_louder_box_sets_the_level() {
        let near = PlacedBox { tune: 1, pos: [0.0, 0.0, -2.0], left: 10.0 };
        let far = PlacedBox { tune: 1, pos: [0.0, 0.0, -9.0], left: 10.0 };
        let both = heard(&[far, near]);
        assert_eq!(both.len(), 1, "one tune for the pair");
        assert_eq!(both[0].gains, heard(&[near])[0].gains);
    }

    #[test]
    fn distance_and_side_shape_the_gains_and_out_of_range_is_silent() {
        let at = |x: f32, z: f32| heard(&[PlacedBox { tune: 0, pos: [x, 0.0, z], left: 9.0 }]);
        let close = at(0.0, -2.0)[0].gains;
        let far = at(0.0, -8.0)[0].gains;
        assert!(close.0 > far.0);
        let right_side = at(4.0, 0.0)[0].gains;
        assert!(right_side.1 > right_side.0, "a box on the right is louder in the right ear");
        assert!(at(0.0, -40.0).is_empty(), "past the emitter's range it is not started");
    }

    #[test]
    fn different_tunes_are_different_requests_with_their_own_rates() {
        let a = PlacedBox { tune: 0, pos: [0.0, 0.0, -3.0], left: 5.0 };
        let b = PlacedBox { tune: 3, ..a };
        assert_eq!(heard(&[b, a]).iter().map(|t| t.tune).collect::<Vec<_>>(), vec![0, 3]);
        assert!(tune_rate(0) > 0.0 && tune_rate(3) > 0.0);
        assert_eq!(tune_rate(0), tune_rate(rules().placed_tune_rates.len() as u32), "the rates wrap");
    }

    #[test]
    fn the_shipped_file_parses_with_the_godot_numbers() {
        let r = rules();
        assert_eq!(r.boxes_per_region, 2);
        assert_eq!(r.winddown_seconds, 15.0);
        assert_eq!(r.slowdown_from_fraction, 0.70, "the tune slows below 70 % of the wind");
        assert!(r.slowdown_floor > 0.0 && r.slowdown_floor < 1.0);
        assert!(r.tune_gain > 0.0 && r.tune_gain <= 0.3, "the tune stays quiet");
        assert_eq!(wound_seconds("standard"), 45.0);
        assert_eq!(wound_seconds("hard"), 35.0);
        assert_eq!(wound_seconds("nightmare"), 25.0);
        // any other id winds like the standard box (the Godot fallback)
        assert_eq!(wound_seconds("relaxed"), 45.0);
    }
}
