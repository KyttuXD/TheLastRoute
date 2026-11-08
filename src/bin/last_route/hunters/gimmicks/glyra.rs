//! Glyra and the music box. While the box plays she is calm, slow (x0.6) and cannot grab;
//! winding down she gets restless; once it stops she hunts fast (x1.3) with sharper hearing.
//! Winding the box (hold interact 1.5 s) settles her (glyra_gimmick.gd, music_boxes.gd).
//! The port ticks the box clock every frame, which the original's finite floor intended.

use super::Ctx;
use crate::config::Difficulty;
use crate::hunters::brain::{dist, Brain, State, Target};

#[derive(Clone, Copy, PartialEq)]
pub enum Mood {
    Playing,
    WindingDown,
    Stopped,
}

/// Her tunables (data/gimmicks/glyra.json; game_config.gd GLYRA_*). A missing or bad key keeps the Godot value.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub calm_speed_scale: f32,
    pub hunt_speed_scale: Vec<(String, f32)>,
    pub hunt_hearing_scale: f32,
    pub calm_sight: f32,
    pub calm_leash: f32,
    pub calm_after_wind: f32,
    pub home_radius: f32,
    pub peek_ambush: bool,
    pub peek_chance_scale: f32,
}

impl Tuning {
    pub fn parse(text: &str) -> Tuning {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let num = |k: &str, d: f32| match v.get(k).and_then(|x| x.as_f64()) {
            Some(n) if n > 0.0 && n.is_finite() => n as f32,
            _ => d,
        };
        let mut hunt: Vec<(String, f32)> = v
            .get("hunt_speed_scale")
            .and_then(|x| x.as_object())
            .map(|o| o.iter().filter_map(|(k, x)| x.as_f64().filter(|n| *n > 0.0).map(|n| (k.clone(), n as f32))).collect())
            .unwrap_or_default();
        if !hunt.iter().any(|(k, _)| k == "standard") {
            hunt.push(("standard".to_string(), 1.3));
        }
        Tuning {
            calm_speed_scale: num("calm_speed_scale", 0.6),
            hunt_speed_scale: hunt,
            hunt_hearing_scale: num("hunt_hearing_scale", 1.5),
            calm_sight: num("calm_sight", 4.0),
            calm_leash: num("calm_leash", 8.0),
            calm_after_wind: num("calm_after_wind", 3.0),
            home_radius: num("home_radius", 2.5),
            peek_ambush: v.get("peek_ambush").and_then(|x| x.as_bool()).unwrap_or(true),
            peek_chance_scale: v.get("peek_chance_scale").and_then(|x| x.as_f64()).filter(|n| *n >= 0.0 && n.is_finite()).map_or(1.0, |n| n as f32),
        }
    }

    /// The hunt speed scale on `diff` (an absent difficulty uses standard).
    pub fn hunt_speed(&self, diff: Difficulty) -> f32 {
        let find = |id: &str| self.hunt_speed_scale.iter().find(|(k, _)| k == id).map(|(_, n)| *n);
        find(diff.id()).or_else(|| find("standard")).unwrap_or(1.3)
    }

    /// The peek chance she gets from the hunter's base chance: none while her tune plays, else base x scale
    /// (zero if the data switches the ambush off).
    pub fn peek_chance(&self, mood: Mood, base: f32) -> f32 {
        if mood == Mood::Playing || !self.peek_ambush {
            0.0
        } else {
            base * self.peek_chance_scale
        }
    }
}

fn tuning() -> &'static Tuning {
    static T: std::sync::OnceLock<Tuning> = std::sync::OnceLock::new();
    T.get_or_init(|| Tuning::parse(include_str!("../../data/gimmicks/glyra.json")))
}

pub struct Glyra {
    diff: Difficulty,
    pub mood: Mood,
    settle_left: f32,
    base_hearing: f32,
    was_mood: Mood,
    was_hunting: bool,
}

pub fn mood_of(left: f32) -> Mood {
    if left <= 0.0 {
        Mood::Stopped
    } else if left <= 15.0 {
        Mood::WindingDown
    } else {
        Mood::Playing
    }
}

impl Glyra {
    pub fn new() -> Glyra {
        Glyra::with_difficulty(Difficulty::Standard)
    }

    /// Glyra on `diff`: the hunt speed once the tune stops is per difficulty (Nightmare x1.5).
    pub fn with_difficulty(diff: Difficulty) -> Glyra {
        Glyra {
            diff,
            mood: Mood::Playing,
            settle_left: 0.0,
            base_hearing: -1.0,
            was_mood: Mood::Playing,
            was_hunting: false,
        }
    }

    pub fn playing(&self) -> bool {
        self.mood == Mood::Playing
    }

    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        if self.base_hearing < 0.0 {
            self.base_hearing = b.hearing_scale;
        }
        // she guards the nearest music box still on the floor in this window (Ctx.music_boxes, the uncollected boxes of
        // game/music_box.rs). Read every tick, so a window shift moves her leash with it. With no box left there is no
        // guard point, and she does not drift toward any.
        let guard = c
            .music_boxes
            .iter()
            .copied()
            .min_by(|a, z| dist(*a, b.pos).total_cmp(&dist(*z, b.pos)));
        self.mood = mood_of(c.music_box_left);
        if self.mood == Mood::Playing && self.was_mood != Mood::Playing {
            self.settle_left = tuning().calm_after_wind;
            self.was_hunting = true;
            // the box was wound: she takes it up again and her shoulders drop
            b.raise_cue("music_box_settle");
        }
        if self.mood == Mood::Stopped && self.was_mood == Mood::WindingDown {
            // the tune ran out: she starts, head up, the box held out
            b.raise_cue("music_box_startle");
        }
        if self.mood == Mood::Stopped
            && self.was_mood != Mood::Stopped
            && b.state != State::Chase
            && !t.hidden
        {
            b.investigate(t.pos);
        }
        self.was_mood = self.mood;
        b.hearing_scale = self.base_hearing * if self.mood == Mood::Stopped { tuning().hunt_hearing_scale } else { 1.0 };
        if self.mood == Mood::WindingDown && b.state == State::Idle {
            b.idle_timer = b.idle_timer.min(0.15);
        }
        if self.mood != Mood::Playing {
            return;
        }
        self.settle_left = (self.settle_left - dt).max(0.0);
        if self.settle_left > 0.0 {
            return;
        }
        let close = !t.hidden && dist(b.pos, t.pos) <= tuning().calm_sight;
        if b.state == State::Chase && b.direct_visible && self.was_hunting {
            return;
        }
        self.was_hunting = false;
        if let Some(box_pos) = guard {
            let drift = match b.state {
                State::Chase | State::Investigate | State::Search | State::Peek => !close,
                State::Wander | State::Idle => dist(b.pos, box_pos) > tuning().calm_leash,
            };
            if drift {
                b.state = State::Wander;
                b.direct_visible = false;
                b.last_known = None;
                if b.wander_target
                    .map(|w| dist(w, box_pos) > tuning().home_radius)
                    .unwrap_or(true)
                {
                    b.wander_target = Some((box_pos.0 + 1.2, box_pos.1));
                    b.clear_path();
                }
            }
        }
    }

    /// The mechanic pose she is in: cradling the playing box while she is calm (a chase puts it away).
    pub fn mechanic(&self, b: &Brain) -> Option<&'static str> {
        (self.mood == Mood::Playing && b.state != State::Chase).then_some("music_box_hold")
    }

    /// Her peek chance from the hunter's `base` (GlyraGimmick.peek_chance): none while the tune plays.
    pub fn peek_chance(&self, base: f32) -> f32 {
        tuning().peek_chance(self.mood, base)
    }

    pub fn speed_scale(&self) -> f32 {
        match self.mood {
            Mood::Playing => tuning().calm_speed_scale,
            Mood::Stopped => tuning().hunt_speed(self.diff),
            Mood::WindingDown => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;
    use crate::world::Facility;

    /// A floor, and Glyra standing 30 cells out from the origin (well past the 8-cell leash of any box near it).
    fn setup() -> (Facility, Brain) {
        let fac = Facility::generate("GLYRA-1", &Options::default(), &Catalog::load());
        let def = Content::load().hunter("glyra").unwrap().clone();
        let mut b = Brain::new(def, (30.0, 30.0), "GLYRA", Difficulty::Standard);
        b.pos = (30.0, 30.0);
        (fac, b)
    }

    /// The context with the music box playing (45 s left) and the given uncollected boxes in the window.
    fn ctx<'a>(fac: &'a Facility, music_boxes: &'a [(f32, f32)]) -> Ctx<'a> {
        Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 45.0,
            exposure: 0.0,
            flashlight_dead: false,
            music_boxes,
        }
    }

    /// A runner hidden far away, so only the box leash decides where Glyra goes.
    fn hidden_runner() -> Target {
        Target { pos: (60.0, 60.0), hidden: true, ..Default::default() }
    }

    /// The shipped data keeps the Godot numbers; Nightmare hunts at x1.5.
    #[test]
    fn shipped_data_matches_godot_and_nightmare_hunts_faster() {
        let g = Glyra::new();
        assert_eq!(g.speed_scale(), 0.6, "playing is the calm scale");
        let mut s = Glyra::new();
        s.mood = Mood::Stopped;
        assert_eq!(s.speed_scale(), 1.3);
        let mut n = Glyra::with_difficulty(Difficulty::Nightmare);
        n.mood = Mood::Stopped;
        assert_eq!(n.speed_scale(), 1.5);
        let mut d = Glyra::with_difficulty(Difficulty::Relaxed);
        d.mood = Mood::Stopped;
        assert_eq!(d.speed_scale(), 1.3, "a difficulty with no entry uses standard");
        n.mood = Mood::WindingDown;
        assert_eq!(n.speed_scale(), 1.0);
    }

    /// Peek ambush: none while the tune plays, the base chance once it winds down or stops, and a data switch.
    #[test]
    fn she_peeks_only_when_the_tune_is_not_playing() {
        let mut g = Glyra::new();
        assert_eq!(g.peek_chance(0.4), 0.0);
        g.mood = Mood::WindingDown;
        assert_eq!(g.peek_chance(0.4), 0.4);
        g.mood = Mood::Stopped;
        assert_eq!(g.peek_chance(0.4), 0.4);
        let off = Tuning::parse(r#"{"peek_ambush": false}"#);
        assert_eq!(off.peek_chance(Mood::Stopped, 0.4), 0.0);
        let half = Tuning::parse(r#"{"peek_chance_scale": 0.5}"#);
        assert_eq!(half.peek_chance(Mood::Stopped, 0.4), 0.2);
    }

    /// A data edit moves the calm numbers; a bad value keeps the Godot one.
    #[test]
    fn data_edits_change_the_calm_numbers() {
        let t = Tuning::parse(r#"{"calm_leash": 3, "calm_sight": -2, "hunt_speed_scale": {"hard": 2.0}}"#);
        assert_eq!(t.calm_leash, 3.0);
        assert_eq!(t.calm_sight, 4.0);
        assert_eq!(t.hunt_speed(Difficulty::Hard), 2.0);
        assert_eq!(t.hunt_speed(Difficulty::Nightmare), 1.3);
    }

    #[test]
    fn with_no_music_box_left_she_does_not_drift() {
        let (fac, mut b) = setup();
        // Investigate is one of the drift states: with a box in reach it would turn into a wander toward the box
        b.state = State::Investigate;
        let c = ctx(&fac, &[]);
        let mut g = Glyra::new();
        g.after(&mut b, 1.0 / 30.0, &c, &hidden_runner());
        assert!(matches!(b.state, State::Investigate), "no box, no guard point: she keeps her state");
        assert_eq!(b.wander_target, None, "no box, no drift");
    }

    #[test]
    fn with_one_music_box_left_she_guards_it() {
        let (fac, mut b) = setup();
        b.state = State::Wander;
        let boxes = [(10.0, 10.0)];
        let c = ctx(&fac, &boxes);
        let mut g = Glyra::new();
        g.after(&mut b, 1.0 / 30.0, &c, &hidden_runner());
        assert!(matches!(b.state, State::Wander));
        let (x, y) = b.wander_target.expect("one box in reach: she drifts to guard it");
        assert!((x - 11.2).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "guards the box at (10, 10), got ({x}, {y})");
    }

    #[test]
    fn she_guards_the_nearest_of_the_uncollected_boxes() {
        let (fac, mut b) = setup();
        b.state = State::Wander;
        // the far box is listed first; the near one at (20, 30) is 10 cells from her, the other 28
        let boxes = [(2.0, 30.0), (20.0, 30.0)];
        let c = ctx(&fac, &boxes);
        let mut g = Glyra::new();
        g.after(&mut b, 1.0 / 30.0, &c, &hidden_runner());
        let (x, y) = b.wander_target.expect("the near box is out of her leash: she drifts");
        assert!((x - 21.2).abs() < 1e-3 && (y - 30.0).abs() < 1e-3, "guards (20, 30), got ({x}, {y})");
    }
}
