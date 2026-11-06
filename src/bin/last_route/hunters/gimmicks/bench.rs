//! A test bench for the gimmicks: one hunter on a generated floor, stepped at 30 fps against a scripted runner. The
//! mechanic-animation tests (`mechanic_tests.rs`, and the ones beside the gimmicks) drive the real hooks through
//! `Brain::update` and read `Brain::mechanic`, which is what the host sends to guests and the view plays.

use super::{Ctx, Event};
use crate::config::Difficulty;
use crate::content::Content;
use crate::hunters::brain::{dist, Brain, State, Target};
use crate::world::catalog::Catalog;
use crate::world::facility::Options;
use crate::world::Facility;

pub const DT: f32 = 1.0 / 30.0;

pub struct Bench {
    pub fac: Facility,
    pub b: Brain,
    pub t: Target,
    pub diff: Difficulty,
    pub batteries_taken: Vec<bool>,
    pub relays_taken: Vec<bool>,
    pub hiding_disabled: Vec<bool>,
    pub music_box_left: f32,
    pub heater_on: bool,
    pub heater_thawed: bool,
    pub flashlight_dead: bool,
    /// Every event the hunter has raised since the bench was made.
    pub events: Vec<Event>,
}

impl Bench {
    pub fn new(id: &str) -> Bench {
        Bench::with_diff(id, Difficulty::Standard)
    }

    pub fn with_diff(id: &str, diff: Difficulty) -> Bench {
        let fac = Facility::generate("MECHANIC-1", &Options::default(), &Catalog::load());
        let def = Content::load().hunter(id).unwrap_or_else(|| panic!("no hunter {id}")).clone();
        let start = fac.start.center();
        let mut b = Brain::new(def, start, "MECHANIC", diff);
        b.active = true;
        b.state = State::Wander;
        let mut x = Bench {
            batteries_taken: vec![false; fac.batteries.len()],
            relays_taken: vec![false; fac.relays.len()],
            hiding_disabled: vec![false; fac.hiding.len()],
            fac,
            b,
            t: Target::default(),
            diff,
            music_box_left: 45.0,
            heater_on: false,
            heater_thawed: false,
            flashlight_dead: false,
            events: vec![],
        };
        x.runner_far();
        x
    }

    /// Puts the hunter at `pos`.
    pub fn place(&mut self, pos: (f32, f32)) {
        self.b.pos = pos;
    }

    /// The runner hidden in a locker on the floor cell farthest from the hunter: no hunter sees, hears or is seen by her.
    pub fn runner_far(&mut self) {
        let here = self.b.pos;
        let far = self
            .fac
            .floor_cells()
            .into_iter()
            .map(|c| c.center())
            .max_by(|a, z| dist(*a, here).total_cmp(&dist(*z, here)))
            .expect("a floor");
        self.t = Target { pos: far, facing: (1.0, 0.0), hidden: true, hiding_spot: None, ..Target::default() };
    }

    /// A floor point `gap` cells east of another with a clear line of sight between them: `(west, east)`.
    pub fn clear_pair(&self, gap: f32) -> ((f32, f32), (f32, f32)) {
        for c in self.fac.floor_cells() {
            let p = c.center();
            let q = (p.0 + gap, p.1);
            if self.fac.can_occupy(p.0 as f64, p.1 as f64, 0.4)
                && self.fac.can_occupy(q.0 as f64, q.1 as f64, 0.4)
                && self.fac.has_line_of_sight(p, q)
            {
                return (p, q);
            }
        }
        panic!("no clear {gap} cell run on the test floor");
    }

    /// The runner stands `gap` cells west of the hunter in the open, her lamp on and aimed at the hunter, in plain view.
    pub fn beam_on_hunter(&mut self, gap: f32) {
        let (west, east) = self.clear_pair(gap);
        self.b.pos = east;
        self.t = Target {
            pos: west,
            facing: (1.0, 0.0),
            hidden: false,
            flashlight_on: true,
            lamp_charge: 1.0,
            ..Target::default()
        };
    }

    /// One frame; the events it raised.
    pub fn frame(&mut self) -> Vec<Event> {
        let ctx = Ctx {
            fac: &self.fac,
            relays_taken: &self.relays_taken,
            batteries_taken: &self.batteries_taken,
            hiding_disabled: &self.hiding_disabled,
            vents: &[],
            heater_on: self.heater_on,
            heater_thawed: self.heater_thawed,
            diff: self.diff,
            music_box_left: self.music_box_left,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: self.flashlight_dead,
        };
        let (_, ev) = self.b.update(DT, &ctx, &self.t, 1, 0, &self.diff.profile());
        for e in &ev {
            match e {
                Event::TakeBattery(i) => self.batteries_taken[*i] = true,
                Event::SpotDisabled(i, v) => self.hiding_disabled[*i] = *v,
                _ => {}
            }
        }
        self.events.extend(ev.iter().cloned());
        ev
    }

    /// `secs` of frames; every event raised.
    pub fn step(&mut self, secs: f32) -> Vec<Event> {
        let mut all = vec![];
        for _ in 0..((secs / DT).round() as usize).max(1) {
            all.extend(self.frame());
        }
        all
    }

    /// Frames until `done` is true (checked after each), at most `max` seconds. True when it became true.
    pub fn until(&mut self, max: f32, mut done: impl FnMut(&Bench) -> bool) -> bool {
        for _ in 0..((max / DT).round() as usize) {
            self.frame();
            if done(self) {
                return true;
            }
        }
        false
    }

    /// The mechanic animation the hunter is in (what the view plays and the host sends).
    pub fn mech(&self) -> Option<&'static str> {
        self.b.mechanic()
    }

    /// Frames until the hunter is in mechanic `name`.
    pub fn until_mech(&mut self, max: f32, name: &'static str) -> bool {
        self.until(max, |x| x.mech() == Some(name))
    }

    /// True when `ev` holds a meter event (Aki's burst, Ferryn's flash).
    pub fn has_meter(ev: &[Event]) -> bool {
        ev.iter().any(|e| matches!(e, Event::Meter { .. }))
    }
}
