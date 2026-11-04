//! The climb between levels (a stair flight or a sewer ladder). Input is frozen from the press: the runner walks the
//! flight or climbs the ladder over `CLIMB_SECONDS` while the eye follows the steps down or up a storey, the screen
//! fades to black, the level loads behind the black (`Game::take_flight`), and the screen fades back in at the landing.

use super::Game;
use crate::config::WALL_HEIGHT;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::{Cell, Stair, STAIR_FLIGHT};
use crate::world::infinite;
use std::f32::consts::{PI, TAU};

pub const CLIMB_SECONDS: f32 = 1.2;
pub const FADE_OUT_SECONDS: f32 = 0.35;
pub const FADE_IN_SECONDS: f32 = 0.35;
/// The share of the climb spent walking from where the runner stood to the foot of the flight.
const APPROACH_SHARE: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Climb,
    FadeOut,
    FadeIn,
}

impl Phase {
    fn length(self) -> f32 {
        match self {
            Phase::Climb => CLIMB_SECONDS,
            Phase::FadeOut => FADE_OUT_SECONDS,
            Phase::FadeIn => FADE_IN_SECONDS,
        }
    }
}

/// The runner's body during the climb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: (f32, f32),
    pub yaw: f32,
    pub lift: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Transition {
    pub phase: Phase,
    /// Seconds into `phase`.
    pub t: f32,
    /// The flight taken: an index into the stairs of the level being left.
    pub stair: usize,
    from: (f32, f32),
    via: (f32, f32),
    to: (f32, f32),
    yaw_from: f32,
    yaw_to: f32,
    lift: f32,
    /// A ladder down or up that changes storey (game/ladder.rs): after the load the runner is put beside this ladder.
    beside: Option<(Cell, (i32, i32))>,
}

impl Transition {
    /// The climb of `s` (index `stair`) from `from`, facing `yaw`. The far end is the landing one storey over, where
    /// `Game::take_stairs` puts the runner; a ladder's far end is its own shaft cell.
    pub fn new(stair: usize, s: Stair, from: (f32, f32), yaw: f32) -> Transition {
        let far = Cell::new(
            s.cell.x + s.run.0 * (STAIR_FLIGHT + 1),
            s.cell.y + s.run.1 * (STAIR_FLIGHT + 1),
        );
        let yaw_to = if s.is_ladder() {
            yaw
        } else {
            (-s.run.0 as f32).atan2(-s.run.1 as f32)
        };
        Transition {
            phase: Phase::Climb,
            t: 0.0,
            stair,
            from,
            via: s.cell.center(),
            to: far.center(),
            yaw_from: yaw,
            yaw_to,
            lift: s.dir as f32 * WALL_HEIGHT,
            beside: None,
        }
    }

    /// The fade alone, for a manual ladder climb that changes storey (game/ladder.rs): no climb, the screen goes black,
    /// the storey loads, and the runner is placed beside the ladder on the new storey (`beside`). The body stays where it
    /// stood until the load.
    pub fn ladder_fade(stair: usize, cell: Cell, side: (i32, i32), at: (f32, f32), yaw: f32) -> Transition {
        Transition {
            phase: Phase::FadeOut,
            t: 0.0,
            stair,
            from: at,
            via: at,
            to: at,
            yaw_from: yaw,
            yaw_to: yaw,
            lift: 0.0,
            beside: Some((cell, side)),
        }
    }

    /// Moves the clock on by `dt` seconds. Returns true when the screen has just gone fully black: the load is due.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.t += dt;
        let mut black = false;
        loop {
            let len = self.phase.length();
            if self.t < len {
                return black;
            }
            match self.phase {
                Phase::Climb => {
                    self.t -= len;
                    self.phase = Phase::FadeOut;
                }
                Phase::FadeOut => {
                    self.t -= len;
                    self.phase = Phase::FadeIn;
                    black = true;
                }
                Phase::FadeIn => {
                    self.t = len;
                    return black;
                }
            }
        }
    }

    pub fn finished(&self) -> bool {
        self.phase == Phase::FadeIn && self.t >= FADE_IN_SECONDS
    }

    /// The runner's body now: on the climb it moves along the flight, and once the climb is over it stays at the far end.
    pub fn pose(&self) -> Pose {
        let k = match self.phase {
            Phase::Climb => ease(self.t / CLIMB_SECONDS),
            Phase::FadeOut | Phase::FadeIn => 1.0,
        };
        let pos = if k < APPROACH_SHARE {
            lerp2(self.from, self.via, k / APPROACH_SHARE)
        } else {
            lerp2(self.via, self.to, (k - APPROACH_SHARE) / (1.0 - APPROACH_SHARE))
        };
        Pose {
            pos,
            yaw: self.yaw_from + turn(self.yaw_from, self.yaw_to) * k,
            lift: self.lift * k,
        }
    }

    /// The black over the screen: rising through the fade out, falling through the fade in, clear during the climb.
    pub fn fade_level(&self) -> f32 {
        match self.phase {
            Phase::Climb => 0.0,
            Phase::FadeOut => (self.t / FADE_OUT_SECONDS).clamp(0.0, 1.0),
            Phase::FadeIn => 1.0 - (self.t / FADE_IN_SECONDS).clamp(0.0, 1.0),
        }
    }
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// The signed turn from `from` to `to` along the shorter way round.
fn turn(from: f32, to: f32) -> f32 {
    (to - from + PI).rem_euclid(TAU) - PI
}

impl Game {
    /// Starts the climb of flight `i` (interact's `Focus::Stairs`). A level the run may not enter is refused here.
    pub(super) fn start_stairs(&mut self, i: usize) {
        let s = self.fac.stairs[i];
        let to_level = self.inf.as_ref().map_or(0, |inf| inf.level) + s.dir;
        if !infinite::level_allowed(to_level) {
            return;
        }
        self.focus = None;
        self.p.moving = false;
        self.transition = Some(Transition::new(i, s, self.p.pos, self.p.yaw));
    }

    /// One frame of the transition: the climb's pose, the fade, and the level load once the screen is black.
    pub(super) fn advance_transition(&mut self, dt: f32, content: &Content, catalog: &Catalog) {
        let Some(mut tr) = self.transition else {
            return;
        };
        if tr.tick(dt) {
            self.take_flight(tr.stair, content, catalog);
            self.p.climb_lift = 0.0;
            if let Some((cell, side)) = tr.beside {
                self.place_beside_ladder(cell, side);
            }
        }
        if tr.phase != Phase::FadeIn {
            let pose = tr.pose();
            self.p.pos = pose.pos;
            self.p.yaw = pose.yaw;
            self.p.climb_lift = pose.lift;
        }
        if tr.phase != Phase::Climb {
            self.fade = tr.fade_level();
        }
        self.transition = if tr.finished() { None } else { Some(tr) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::{Difficulty, CAMERA_HEIGHT};
    use crate::game::player::Player;
    use crate::game::Controls;
    use crate::world::facility::LinkKind;
    use macroquad::prelude::vec2;

    const DT: f32 = 0.05;

    fn stairs_down() -> Stair {
        Stair {
            cell: Cell::new(3, 4),
            dir: -1,
            run: (0, 1),
            kind: LinkKind::Stairs,
        }
    }

    fn ladder_down() -> Stair {
        Stair {
            cell: Cell::new(3, 4),
            dir: -1,
            run: (0, 0),
            kind: LinkKind::Ladder(crate::world::facility::Cover::Manhole),
        }
    }

    fn ladder_game(content: &Content, catalog: &Catalog) -> (Game, usize) {
        let g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, content, catalog);
        let i = g
            .fac
            .stairs
            .iter()
            .position(|s| s.is_ladder() && s.dir < 0)
            .expect("the start window has a ladder down");
        (g, i)
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    #[test]
    fn the_phases_run_climb_then_fade_out_then_fade_in_with_the_load_between() {
        let mut tr = Transition::new(0, stairs_down(), (3.5, 2.5), 0.0);
        let mut blacks = 0;
        for _ in 0..23 {
            blacks += tr.tick(DT) as usize;
        }
        assert_eq!((tr.phase, blacks), (Phase::Climb, 0), "1.15 s in: still climbing");
        for _ in 0..7 {
            blacks += tr.tick(DT) as usize;
        }
        assert_eq!((tr.phase, blacks), (Phase::FadeOut, 0), "1.50 s in: fading out, not yet black-loaded");
        for _ in 0..3 {
            blacks += tr.tick(DT) as usize;
        }
        assert_eq!((tr.phase, blacks), (Phase::FadeIn, 1), "1.65 s in: black once, the load is due");
        assert!(!tr.finished());
        for _ in 0..4 {
            blacks += tr.tick(DT) as usize;
        }
        assert_eq!((tr.phase, blacks), (Phase::FadeIn, 1), "1.85 s in: fading back in");
        assert!(!tr.finished());
        for _ in 0..3 {
            blacks += tr.tick(DT) as usize;
        }
        assert_eq!(blacks, 1, "the black comes once");
        assert!(tr.finished(), "2.00 s in: the fade back in is done");
    }

    #[test]
    fn a_long_frame_still_passes_the_black_once() {
        let mut tr = Transition::new(0, stairs_down(), (3.5, 2.5), 0.0);
        assert!(tr.tick(10.0), "a frame past the whole transition still loads the level");
        assert!(tr.finished());
    }

    #[test]
    fn the_fade_rises_to_black_at_the_load_and_clears_at_the_landing() {
        let mut tr = Transition::new(0, stairs_down(), (3.5, 2.5), 0.0);
        assert_eq!(tr.fade_level(), 0.0);
        for _ in 0..30 {
            tr.tick(DT);
        }
        let out = tr.fade_level();
        assert!(out > 0.8 && out < 1.0, "fading out: {out}");
        for _ in 0..3 {
            tr.tick(DT);
        }
        assert_eq!(tr.phase, Phase::FadeIn);
        let back = tr.fade_level();
        assert!(back > 0.5 && back < 1.0, "fading back in from black: {back}");
        for _ in 0..10 {
            tr.tick(DT);
        }
        assert_eq!(tr.fade_level(), 0.0);
    }

    #[test]
    fn the_stairs_climb_walks_the_flight_and_lowers_the_eye_a_storey() {
        let from = (3.5, 2.5);
        let mut tr = Transition::new(0, stairs_down(), from, 0.0);
        let start = tr.pose();
        assert!(near(start.pos, from) && start.lift == 0.0);
        tr.t = CLIMB_SECONDS;
        let end = tr.pose();
        assert!(near(end.pos, Cell::new(3, 9).center()), "far end {:?}", end.pos);
        assert!((end.lift + WALL_HEIGHT).abs() < 1e-4, "eye lowers a storey: {}", end.lift);
        let ahead = (-end.yaw.sin(), -end.yaw.cos());
        assert!(near(ahead, (0.0, 1.0)), "faces down the flight: {ahead:?}");
        tr.t = CLIMB_SECONDS * 0.5;
        let mid = tr.pose();
        assert!(mid.lift < 0.0 && mid.lift > end.lift, "the eye drops steadily: {}", mid.lift);
    }

    #[test]
    fn the_climb_lift_moves_the_eye_height() {
        let mut p = Player::new((0.0, 0.0));
        assert_eq!(p.eye_height(), CAMERA_HEIGHT);
        p.climb_lift = -WALL_HEIGHT;
        assert!((p.eye_height() - (CAMERA_HEIGHT - WALL_HEIGHT)).abs() < 1e-4);
    }

    #[test]
    fn input_is_frozen_from_the_press_until_the_fade_clears() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut idle, i) = ladder_game(&content, &catalog);
        let (mut busy, _) = ladder_game(&content, &catalog);
        for g in [&mut idle, &mut busy] {
            g.transition = Some(Transition::new(i, g.fac.stairs[i], g.p.pos, g.p.yaw));
        }
        assert!(idle.transition.is_some() && busy.transition.is_some());

        let quiet = Controls::default();
        let loud = Controls {
            fwd: 1.0,
            strafe: 1.0,
            sprint: true,
            interact: true,
            look: vec2(0.4, -0.2),
            ..Default::default()
        };
        let mut seen = vec![];
        let mut frames = 0;
        while let Some(tr) = idle.transition {
            if seen.last() != Some(&tr.phase) {
                seen.push(tr.phase);
            }
            idle.step(DT, &quiet, &content, &catalog, &Sounds::default());
            busy.step(DT, &loud, &content, &catalog, &Sounds::default());
            frames += 1;
            assert!(frames < 200, "the transition never ended");
            assert_eq!(idle.p.pos, busy.p.pos, "input moved the runner during the transition");
            assert_eq!(idle.p.yaw, busy.p.yaw, "input turned the runner during the transition");
            assert_eq!(idle.transition.map(|t| t.phase), busy.transition.map(|t| t.phase));
            let level = idle.inf.as_ref().unwrap().level;
            match idle.transition {
                Some(t) if t.phase != Phase::FadeIn => assert_eq!(level, 0, "loaded before the black"),
                _ => assert_eq!(level, -1, "the level loads at the black"),
            }
        }
        assert_eq!(seen, vec![Phase::Climb, Phase::FadeOut, Phase::FadeIn]);
        assert!((36..=40).contains(&frames), "a 1.9 s transition, took {frames} frames");
        assert!(busy.transition.is_none());
        assert_eq!(busy.fade, 0.0);
        assert_eq!(busy.inf.as_ref().unwrap().level, -1);
        assert_eq!(busy.p.climb_lift, 0.0);
    }
}
