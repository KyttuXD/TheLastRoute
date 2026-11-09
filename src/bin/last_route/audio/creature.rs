//! The Inferno creature's cues (game_3d/creature_view.gd): the bark or hiss tell when a new bark number arrives, the dry skitter of
//! its legs while it walks or runs, and now and then something cracking in the dark of its lair. All positional from the creature:
//! the pan is its side of the listener, the loudness falls with distance. The sounds are the existing cues (`spider_bark`,
//! `spider_hiss`, `spider_skitter`, `inferno_crackle`); `update` is called once per game frame beside `world::update`.

use super::cues::rules::{self, Play};
use super::engine::with;
use super::hunter_ambient::HunterAmbient;
use super::world::Listener;
use crate::hunters::secret_creatures::{Rules, ViewRules};
use crate::world::rng::Rng;
use std::cell::RefCell;
use std::collections::HashMap;

/// One creature as the sound sees it.
#[derive(Clone, Copy, Debug)]
pub struct CreatureAudio {
    /// A key that stays with the creature (its place in the hunter list).
    pub slot: usize,
    pub pos: (f32, f32),
    pub active: bool,
    /// The gimmick is in its hunt phase (the legs run instead of walk).
    pub hunting: bool,
    pub bark_seq: u32,
    /// The tell is a hiss (the black one), not a bark.
    pub hiss: bool,
}

/// The listener as the creature cues need it: where it stands and which way its right ear points on the floor.
#[derive(Clone, Copy, Debug)]
pub struct Ears {
    pub pos: (f32, f32),
    pub right: (f32, f32),
}

impl From<&Listener> for Ears {
    fn from(l: &Listener) -> Ears {
        Ears { pos: l.pos, right: (l.right[0], l.right[2]) }
    }
}

/// One creature's timers.
pub struct CreatureClock {
    last: Option<(f32, f32)>,
    bark_seen: Option<u32>,
    skitter_left: f32,
    crackle_left: f32,
    rng: Rng,
}

impl CreatureClock {
    pub fn new(slot: usize) -> CreatureClock {
        CreatureClock { last: None, bark_seen: None, skitter_left: 0.0, crackle_left: 3.0, rng: Rng::new(&format!("CREATURE-CRACKLE:{slot}")) }
    }

    /// The cues this frame: pan from the creature's side of the listener, loudness from its distance.
    pub fn tick(&mut self, dt: f32, c: &CreatureAudio, ears: &Ears, view: &ViewRules) -> Vec<Play> {
        let mut out = vec![];
        let speed = match self.last {
            Some(last) if dt > 1.0e-4 => ((c.pos.0 - last.0).powi(2) + (c.pos.1 - last.1).powi(2)).sqrt() / dt,
            _ => 0.0,
        };
        self.last = Some(c.pos);
        let off = (c.pos.0 - ears.pos.0, c.pos.1 - ears.pos.1);
        let distance = (off.0 * off.0 + off.1 * off.1).sqrt();
        let pan = ((off.0 * ears.right.0 + off.1 * ears.right.1) / distance.max(1.0)).clamp(-1.0, 1.0) as f64;
        // the tell: a new bark number (a spider first seen already barking plays it too), never while it is dormant
        let fresh = match self.bark_seen {
            Some(seen) => seen != c.bark_seq,
            None => c.bark_seq > 0,
        };
        self.bark_seen = Some(c.bark_seq);
        if fresh && c.active && distance < view.bark_cue_radius {
            let loudness = (1.4 - distance / view.bark_cue_radius).clamp(0.35, 1.4) as f64;
            out.push(if c.hiss { rules::spider_hiss(pan, loudness) } else { rules::spider_bark(pan, loudness) });
        }
        self.skitter_left -= dt;
        self.crackle_left -= dt;
        let moving = c.active && speed > view.moving_speed;
        if self.skitter_left <= 0.0 && moving {
            self.skitter_left = if c.hunting { view.skitter_every_run } else { view.skitter_every_walk };
            // a chasing spider keeps its full skitter and range; only the walking (ambient) one is toned down
            let radius = if c.hunting { view.skitter_radius_run } else { view.skitter_radius };
            if distance < radius {
                let mut p = rules::spider_skitter(pan, (1.3 - distance / radius).clamp(0.15, 1.3) as f64);
                if !c.hunting {
                    p.gain *= HunterAmbient::get().skitter_gain;
                }
                out.push(p);
            }
        }
        if self.crackle_left <= 0.0 {
            let (lo, hi) = view.crackle_every;
            self.crackle_left = lo + (hi - lo) * self.rng.next_f32();
            if distance < view.lair_radius {
                let mut p = rules::inferno_crackle(pan, (1.2 - distance / view.lair_radius).clamp(0.15, 1.2) as f64);
                p.gain *= HunterAmbient::get().crackle_gain;
                out.push(p);
            }
        }
        out
    }
}

thread_local! {
    static CLOCKS: RefCell<HashMap<usize, CreatureClock>> = RefCell::new(HashMap::new());
}

/// Forget every creature's timers (a floor starts or a run ends).
pub fn reset() {
    CLOCKS.with(|c| c.borrow_mut().clear());
}

/// The frame's creature cues. A creature that is gone loses its timers.
pub fn update(dt: f32, l: &Listener, creatures: &[CreatureAudio]) {
    let view = &Rules::get().view;
    let ears = Ears::from(l);
    let plays: Vec<Play> = CLOCKS.with(|clocks| {
        let mut clocks = clocks.borrow_mut();
        clocks.retain(|slot, _| creatures.iter().any(|c| c.slot == *slot));
        creatures.iter().flat_map(|c| clocks.entry(c.slot).or_insert_with(|| CreatureClock::new(c.slot)).tick(dt, c, &ears, view)).collect()
    });
    if plays.is_empty() {
        return;
    }
    with(|e| {
        if !e.ready {
            return;
        }
        for p in &plays {
            e.play(p);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewRules {
        Rules::get().view.clone()
    }

    fn creature(pos: (f32, f32), bark_seq: u32, hunting: bool, hiss: bool) -> CreatureAudio {
        CreatureAudio { slot: 0, pos, active: true, hunting, bark_seq, hiss }
    }

    fn ears() -> Ears {
        Ears { pos: (0.0, 0.0), right: (1.0, 0.0) }
    }

    fn cues(plays: &[Play]) -> Vec<&str> {
        plays.iter().map(|p| p.cue.as_str()).collect()
    }

    #[test]
    fn a_new_bark_number_plays_the_white_bark_or_the_black_hiss_once() {
        let v = view();
        let mut white = CreatureClock::new(0);
        assert!(white.tick(0.02, &creature((10.0, 0.0), 0, false, false), &ears(), &v).is_empty());
        assert_eq!(cues(&white.tick(0.02, &creature((10.0, 0.0), 1, false, false), &ears(), &v)), vec!["spider_bark"]);
        assert!(white.tick(0.02, &creature((10.0, 0.0), 1, false, false), &ears(), &v).is_empty(), "the same number is silent");
        let mut black = CreatureClock::new(1);
        black.tick(0.02, &creature((10.0, 0.0), 0, false, true), &ears(), &v);
        assert_eq!(cues(&black.tick(0.02, &creature((10.0, 0.0), 1, false, true), &ears(), &v)), vec!["spider_hiss"]);
    }

    #[test]
    fn the_tell_is_positional_and_out_of_range_it_is_silent() {
        let v = view();
        let mut right = CreatureClock::new(0);
        right.tick(0.02, &creature((10.0, 0.0), 0, false, false), &ears(), &v);
        let near = right.tick(0.02, &creature((10.0, 0.0), 1, false, false), &ears(), &v);
        assert!(near[0].pan > 0.9, "on the right: pan {}", near[0].pan);
        let mut left = CreatureClock::new(1);
        left.tick(0.02, &creature((-10.0, 0.0), 0, false, false), &ears(), &v);
        assert!(left.tick(0.02, &creature((-10.0, 0.0), 1, false, false), &ears(), &v)[0].pan < -0.9);
        let mut far = CreatureClock::new(2);
        far.tick(0.02, &creature((500.0, 0.0), 0, false, false), &ears(), &v);
        assert!(far.tick(0.02, &creature((500.0, 0.0), 1, false, false), &ears(), &v).is_empty());
        let mut dormant = CreatureClock::new(3);
        dormant.tick(0.02, &CreatureAudio { active: false, ..creature((10.0, 0.0), 0, false, false) }, &ears(), &v);
        assert!(dormant.tick(0.02, &CreatureAudio { active: false, ..creature((10.0, 0.0), 1, false, false) }, &ears(), &v).is_empty());
    }

    #[test]
    fn the_legs_skitter_faster_when_it_runs_and_not_at_all_standing() {
        let v = view();
        let count = |hunting: bool, speed: f32| -> usize {
            let mut clock = CreatureClock::new(0);
            let mut x = 10.0;
            let mut n = 0;
            for _ in 0..300 {
                x += speed * 0.02;
                n += clock.tick(0.02, &creature((x, 0.0), 0, hunting, false), &ears(), &v).iter().filter(|p| p.cue == "spider_skitter").count();
            }
            n
        };
        assert_eq!(count(false, 0.0), 0, "standing still makes no skitter");
        let (walk, run) = (count(false, 5.0), count(true, 5.0));
        assert!(walk > 0 && run > walk, "walk {walk}, run {run}");
    }

    #[test]
    fn a_chasing_skitter_is_full_gain_and_only_the_walking_one_is_toned_down() {
        let v = view();
        let first_skitter = |hunting: bool| -> Play {
            let mut clock = CreatureClock::new(0);
            clock.tick(0.02, &creature((10.0, 0.0), 0, hunting, false), &ears(), &v);
            let plays = clock.tick(0.02, &creature((10.1, 0.0), 0, hunting, false), &ears(), &v);
            plays.into_iter().find(|p| p.cue == "spider_skitter").expect("a skitter in range")
        };
        let d = 10.1_f32;
        let full = |radius: f32| rules::spider_skitter(1.0, (1.3 - d / radius).clamp(0.15, 1.3) as f64).gain;
        let (chase, walk) = (first_skitter(true), first_skitter(false));
        let tone = HunterAmbient::get().skitter_gain;
        assert!(tone < 1.0, "the ambient skitter is toned down: {tone}");
        assert!((chase.gain - full(v.skitter_radius_run)).abs() < 1.0e-4, "chasing is not scaled by skitter_gain: {}", chase.gain);
        assert!((walk.gain - full(v.skitter_radius) * tone).abs() < 1.0e-4, "walking is scaled by skitter_gain: {}", walk.gain);
    }

    #[test]
    fn something_cracks_in_the_dark_now_and_then_only_near_the_lair() {
        let v = view();
        let crackles = |at: (f32, f32)| -> usize {
            let mut clock = CreatureClock::new(0);
            (0..3000).map(|_| clock.tick(0.02, &creature(at, 0, false, false), &ears(), &v).iter().filter(|p| p.cue == "inferno_crackle").count()).sum()
        };
        let near = crackles((20.0, 0.0));
        assert!((2..=8).contains(&near), "60 s near the lair: {near} crackles");
        assert_eq!(crackles((200.0, 0.0)), 0, "out of earshot");
    }
}
