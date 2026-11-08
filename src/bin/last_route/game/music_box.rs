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
    /// Hold seconds to wind from empty to 100 %.
    pub wind_seconds: f32,
    /// Seconds held past 100 % before the box jams.
    pub overwind_seconds: f32,
    /// Seconds a jammed box refuses winding.
    pub jam_cooldown_seconds: f32,
    /// The jam's noise burst strength and length (what hunters hear).
    pub jam_noise: f32,
    pub jam_noise_seconds: f32,
    pub prompt_wind: String,
    pub prompt_jammed: String,
    pub message_jammed: String,
    pub message_left_click: String,
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
    let text = |k: &str| v[k].as_str().unwrap_or_else(|| panic!("data/{FILE}: {k} is not text")).to_string();
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
        wind_seconds: num("wind_seconds").max(0.1),
        overwind_seconds: num("overwind_seconds").max(0.0),
        jam_cooldown_seconds: num("jam_cooldown_seconds").max(0.0),
        jam_noise: num("jam_noise"),
        jam_noise_seconds: num("jam_noise_seconds"),
        prompt_wind: text("prompt_wind"),
        prompt_jammed: text("prompt_jammed"),
        message_jammed: text("message_jammed"),
        message_left_click: text("message_left_click"),
    }
}

/// What one frame of winding did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindEvent {
    Idle,
    Winding,
    /// Held while the box is jammed: refused.
    Refused,
    /// The overwind ran out this frame: the box stopped and jammed.
    Jammed,
}

/// The winding state of the box in hand: the overwind timer and the jam cooldown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoxWind {
    pub over: f32,
    pub jammed_left: f32,
}

impl BoxWind {
    pub fn jammed(&self) -> bool {
        self.jammed_left > 0.0
    }

    /// One frame. `left` is the box's wound seconds, `full` its 100 % (the difficulty's wound time). Holding winds
    /// `full / wind_seconds` per second up to `full`; holding on past it for `overwind_seconds` jams the box (the wind is
    /// lost and it refuses winding for `jam_cooldown_seconds`). The cooldown runs whether held or not.
    pub fn step(&mut self, dt: f32, held: bool, left: &mut f32, full: f32) -> WindEvent {
        let r = rules();
        if self.jammed_left > 0.0 {
            self.jammed_left = (self.jammed_left - dt).max(0.0);
            self.over = 0.0;
            return if held { WindEvent::Refused } else { WindEvent::Idle };
        }
        if !held {
            self.over = 0.0;
            return WindEvent::Idle;
        }
        // a wound box plays down one second per second (Game::run_music_box), so a held box sits a frame's drain under
        // 100 % each frame: within that drain (one frame, at most 0.1 s) it counts as full and the hold is overwind (else
        // it could never jam)
        if *left < full - dt.min(0.1) * 1.01 {
            *left = (*left + full / r.wind_seconds * dt).min(full);
            return WindEvent::Winding;
        }
        *left = full;
        self.over += dt;
        if self.over >= r.overwind_seconds {
            self.over = 0.0;
            *left = 0.0;
            self.jammed_left = r.jam_cooldown_seconds;
            return WindEvent::Jammed;
        }
        WindEvent::Winding
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
    fn holding_winds_to_full_over_the_data_time() {
        let (mut w, mut left, full) = (BoxWind::default(), 0.0, wound_seconds("standard"));
        let r = rules();
        assert_eq!(w.step(r.wind_seconds * 0.5, true, &mut left, full), WindEvent::Winding);
        assert!(left > 0.0 && left < full, "half the time winds part way");
        assert_eq!(w.step(r.wind_seconds * 0.6, true, &mut left, full), WindEvent::Winding);
        assert_eq!(left, full, "wound to 100 %");
        let mut idle = 0.0;
        assert_eq!(w.step(1.0, false, &mut idle, full), WindEvent::Idle);
        assert_eq!(idle, 0.0, "not held: nothing winds");
    }

    #[test]
    fn overwinding_jams_and_a_jammed_box_refuses_until_the_cooldown_ends() {
        let (mut w, full) = (BoxWind::default(), wound_seconds("standard"));
        let mut left = full;
        let r = rules();
        assert_eq!(w.step(r.overwind_seconds * 0.5, true, &mut left, full), WindEvent::Winding);
        assert!(!w.jammed());
        assert_eq!(w.step(r.overwind_seconds * 0.6, true, &mut left, full), WindEvent::Jammed);
        assert!(w.jammed());
        assert_eq!(left, 0.0, "a jammed box stops playing");
        assert_eq!(w.step(0.1, true, &mut left, full), WindEvent::Refused);
        assert_eq!(left, 0.0, "refused: no wind");
        assert_eq!(w.step(r.jam_cooldown_seconds + 1.0, false, &mut left, full), WindEvent::Idle);
        assert!(!w.jammed(), "the cooldown ran out");
        assert_eq!(w.step(0.5, true, &mut left, full), WindEvent::Winding);
        assert!(left > 0.0);
    }

    #[test]
    fn a_box_that_plays_down_every_frame_still_jams_when_held_past_full() {
        let (mut w, full) = (BoxWind::default(), wound_seconds("standard"));
        let r = rules();
        let (mut left, dt) = (full, 1.0 / 60.0);
        let mut jammed_at = None;
        for f in 0..((r.overwind_seconds * 2.0 / dt) as usize) {
            // the tune drains the wind each frame, as run_music_box does
            left -= dt;
            if w.step(dt, true, &mut left, full) == WindEvent::Jammed {
                jammed_at = Some(f as f32 * dt);
                break;
            }
        }
        let t = jammed_at.expect("held past full, the box jams");
        assert!(t < r.overwind_seconds * 1.2, "it jams near the overwind time, not never ({t} s)");
    }

    #[test]
    fn releasing_before_the_overwind_ends_resets_it() {
        let (mut w, full) = (BoxWind::default(), wound_seconds("standard"));
        let mut left = full;
        let r = rules();
        w.step(r.overwind_seconds * 0.9, true, &mut left, full);
        w.step(0.1, false, &mut left, full);
        w.step(r.overwind_seconds * 0.9, true, &mut left, full);
        assert!(!w.jammed());
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
        assert!(r.wind_seconds > 0.0 && r.overwind_seconds > 0.0 && r.jam_cooldown_seconds > 0.0);
        assert_eq!(r.prompt_wind, "[E] WIND (HOLD)");
        assert_eq!(r.prompt_jammed, "JAMMED");
    }
}
