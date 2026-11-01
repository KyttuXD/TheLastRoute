//! Nyxie (special): darkness stalker. 120 s without light wakes her 10 cells away and she hunts
//! you until the beam catches her (instantly dispelled within 7 cells). A dead lamp for 10 s
//! wakes her on top of you: an inescapable catch unless you are hidden or safe
//! (nyxie_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{Brain, State, Target};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Wake {
    #[default]
    None,
    Timer,
    Empty,
}

#[derive(Default)]
pub struct Nyxie {
    pub wake: Wake,
    darkness: f32,
    empty: f32,
    /// Set the frame she wakes (for the sound cue).
    pub just_woke: bool,
}

/// The floor cell whose centre distance to `p` is closest to `d` (first match).
pub fn spawn_near(c: &Ctx, p: (f32, f32), d: f32) -> (f32, f32) {
    let mut best = (f32::MAX, p);
    for cell in c.fac.floor_cells() {
        let cc = cell.center();
        let err = (crate::hunters::brain::dist(cc, p) - d).abs();
        if err < best.0 {
            best = (err, cc);
        }
    }
    best.1
}

impl Nyxie {
    pub fn inescapable(&self) -> bool {
        self.wake == Wake::Empty
    }

    pub fn speed_scale(&self) -> f32 {
        1.0
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        self.just_woke = false;
        let lit = t.flashlight_on && t.lamp_charge > 0.0;
        if lit {
            self.darkness = 0.0;
            self.empty = 0.0;
        } else {
            self.darkness += dt;
            self.empty = if t.lamp_charge <= 0.0 {
                self.empty + dt
            } else {
                0.0
            };
        }
        if self.wake == Wake::None {
            if self.empty >= 10.0 {
                self.wake = Wake::Empty;
                self.just_woke = true;
            } else if self.darkness >= 120.0 {
                self.wake = Wake::Timer;
                self.just_woke = true;
            }
        }
        match self.wake {
            Wake::None => {
                b.active = false;
                b.state = State::Idle;
                return;
            }
            Wake::Empty => {
                b.active = true;
                b.pos = t.pos;
            }
            Wake::Timer => {
                if !b.active {
                    b.active = true;
                    b.pos = spawn_near(c, t.pos, 10.0);
                } else if lit && Brain::beam_reaches(c.fac, t.pos, t.facing, b.pos, 7.0, 0.33) {
                    self.wake = Wake::None;
                    b.active = false;
                    b.state = State::Idle;
                    self.darkness = 0.0;
                    return;
                }
            }
        }
        b.chase(t.pos);
    }
}
