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
    box_pos: Option<(f32, f32)>,
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
            box_pos: None,
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
        // the floor's music box sits near the start (see Game); she guards it
        let box_pos = *self.box_pos.get_or_insert_with(|| {
            let (sx, sy) = c.fac.start.center();
            (sx + 2.0, sy - 2.0)
        });
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

    pub fn speed_scale(&self) -> f32 {
        match self.mood {
            Mood::Playing => 0.6,
            Mood::Stopped => 1.3,
            Mood::WindingDown => 1.0,
        }
    }
}
