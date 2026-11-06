//! Glyra and the music box. While the box plays she is calm, slow (x0.6) and cannot grab;
//! winding down she gets restless; once it stops she hunts fast (x1.3) with sharper hearing.
//! Winding the box (hold interact 1.5 s) settles her (glyra_gimmick.gd, music_boxes.gd).
//! The port ticks the box clock every frame, which the original's finite floor intended.

use super::Ctx;
use crate::hunters::brain::{dist, Brain, State, Target};

#[derive(Clone, Copy, PartialEq)]
pub enum Mood {
    Playing,
    WindingDown,
    Stopped,
}

pub struct Glyra {
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
        Glyra {
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
            self.settle_left = 3.0;
            self.was_hunting = true;
        }
        if self.mood == Mood::Stopped
            && self.was_mood != Mood::Stopped
            && b.state != State::Chase
            && !t.hidden
        {
            b.investigate(t.pos);
        }
        self.was_mood = self.mood;
        b.hearing_scale = self.base_hearing * if self.mood == Mood::Stopped { 1.5 } else { 1.0 };
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
        let close = !t.hidden && dist(b.pos, t.pos) <= 4.0;
        if b.state == State::Chase && b.direct_visible && self.was_hunting {
            return;
        }
        self.was_hunting = false;
        if let Some(box_pos) = guard {
            let drift = match b.state {
                State::Chase | State::Investigate | State::Search | State::Peek => !close,
                State::Wander | State::Idle => dist(b.pos, box_pos) > 8.0,
            };
            if drift {
                b.state = State::Wander;
                b.direct_visible = false;
                b.last_known = None;
                if b.wander_target
                    .map(|w| dist(w, box_pos) > 2.5)
                    .unwrap_or(true)
                {
                    b.wander_target = Some((box_pos.0 + 1.2, box_pos.1));
                    b.clear_path();
                }
            }
        }
    }

    pub fn speed_scale(&self) -> f32 {
        match self.mood {
            Mood::Playing => 0.6,
            Mood::Stopped => 1.3,
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
            music_boxes,
        }
    }

    /// A runner hidden far away, so only the box leash decides where Glyra goes.
    fn hidden_runner() -> Target {
        Target { pos: (60.0, 60.0), hidden: true, ..Default::default() }
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
