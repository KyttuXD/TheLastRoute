//! The white wolf spider (white_wolf_spider_gimmick.gd): a fast ambusher.
//! It lurks motionless at its lair, barks ONCE to mark a runner it has seen or heard (the telegraph), bursts after them for a
//! limited time, then gives up and goes home. Survivable on purpose: the bark gives a head start, the burst is limited by its own
//! stamina, a shelter ends the chase, and a flashlight beam held on it for a moment drives it off.
//!
//!   LURK -> BARK (holds, bark clip + cue) -> HUNT (<= hunt seconds) -> RETREAT (to the lair) -> LURK (cooldown)
//!
//! Host-simulated like every hunter; a guest only needs `phase` and `bark_seq` (the hunter frame) to play the bark. The numbers are
//! in data/hunters/secret_creatures.json. Specs: docs/SECRET_WOLF_SPIDER.md.

use crate::hunters::brain::{Brain, State, Target};
use crate::hunters::secret_creatures::{Knobs, Rules};
use crate::world::Facility;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Lurk,
    Bark,
    Hunt,
    Retreat,
}

impl Phase {
    /// The wire number (Godot's enum order).
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: i64) -> Phase {
        match i {
            1 => Phase::Bark,
            2 => Phase::Hunt,
            3 => Phase::Retreat,
            _ => Phase::Lurk,
        }
    }
}

pub struct WolfSpider {
    alert_radius: f32,
    bark_seconds: f32,
    hunt_seconds: f32,
    cooldown_seconds: f32,
    hunt_speed_scale: f32,
    /// Seconds of beam that drive it off; None = it ignores the light.
    flash_hold: Option<f32>,
    pub hiss: bool,
    pub phase: Phase,
    pub bark_seq: u32,
    lair: Option<(f32, f32)>,
    timer: f32,
    beam_time: f32,
    cooldown: f32,
    sense_left: f32,
    spawn_barks: bool,
    rest_scale: f32,
}

impl WolfSpider {
    pub fn new() -> WolfSpider {
        let v = &Rules::get().white;
        WolfSpider {
            alert_radius: v.alert_radius,
            bark_seconds: v.bark_seconds,
            hunt_seconds: v.hunt_seconds,
            cooldown_seconds: v.cooldown_seconds,
            hunt_speed_scale: v.hunt_speed_scale,
            flash_hold: v.flash_hold,
            hiss: v.hiss,
            phase: Phase::Lurk,
            bark_seq: 0,
            lair: None,
            timer: 0.0,
            beam_time: 0.0,
            cooldown: 0.0,
            sense_left: 0.0,
            spawn_barks: false,
            rest_scale: 1.0,
        }
    }

    pub fn lair(&self) -> Option<(f32, f32)> {
        self.lair
    }

    /// Seconds of cooldown left before it marks a runner again (tests read it).
    pub fn cooldown_left(&self) -> f32 {
        self.cooldown
    }

    /// A spawner may start it already marking its runner (set right after creating the brain).
    pub fn start_with_bark(&mut self) {
        self.spawn_barks = true;
    }

    /// The per-difficulty multipliers: notice = sight range, chase = chase speed, rest = rest time, alert = alert radius.
    pub fn apply_knobs(&mut self, b: &mut Brain, knobs: &Knobs) {
        b.def.sight *= knobs.notice;
        b.def.chase *= knobs.chase;
        self.rest_scale = knobs.rest;
        self.alert_radius *= knobs.alert;
    }

    pub fn holds_position(&self) -> bool {
        matches!(self.phase, Phase::Lurk | Phase::Bark)
    }

    pub fn speed_scale(&self) -> f32 {
        match self.phase {
            Phase::Hunt => self.hunt_speed_scale,
            Phase::Retreat => Rules::get().retreat_speed_scale,
            _ => 1.0,
        }
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &super::Ctx, t: &Target) {
        let lair = match self.lair {
            Some(l) => l,
            None => {
                self.lair = Some(b.pos);
                if self.spawn_barks {
                    self.spawn_barks = false;
                    self.begin_bark(b, t.pos);
                }
                b.pos
            }
        };
        self.cooldown = (self.cooldown - dt).max(0.0);
        let tp = t.pos;
        // a runner on another level is out of reach: the spider never leaves its own
        let exposed = !t.hidden && !t.in_safe_zone && t.cross.is_none();
        match self.phase {
            Phase::Lurk => {
                self.sense_left -= dt;
                if self.cooldown <= 0.0 && exposed && self.sense_left <= 0.0 {
                    self.sense_left = Rules::get().sense_every;
                    if self.notices(b, c.fac, t) {
                        self.begin_bark(b, tp);
                    }
                }
            }
            Phase::Bark => {
                self.timer -= dt;
                face(b, tp);
                if self.timer <= 0.0 {
                    self.phase = Phase::Hunt;
                    self.timer = self.hunt_seconds;
                    b.chase(tp);
                }
            }
            Phase::Hunt => {
                self.timer -= dt;
                self.beam_time = if b.beam_alerted { self.beam_time + dt } else { 0.0 };
                let driven_off = b.beam_alerted && self.flash_hold.is_some_and(|hold| self.beam_time > hold);
                if !exposed || b.winded || self.timer <= 0.0 || driven_off {
                    self.begin_retreat(b, lair);
                } else {
                    // it does not lose a runner it has marked: it follows sound and light
                    b.chase(tp);
                }
            }
            Phase::Retreat => {
                if dist(b.pos, lair) <= Rules::get().lair_radius {
                    self.phase = Phase::Lurk;
                    self.cooldown = self.cooldown_seconds * self.rest_scale;
                    b.state = State::Idle;
                    b.wander_target = None;
                } else {
                    b.state = State::Wander;
                    b.wander_target = Some(lair);
                }
            }
        }
    }

    /// Perception runs between the two hooks and may flip the state to CHASE (it sees the runner): a lurking or barking creature
    /// stands its ground instead, and a retreating one keeps walking home.
    pub fn after(&mut self, b: &mut Brain) {
        match self.phase {
            Phase::Lurk | Phase::Bark => {
                b.state = State::Idle;
                b.idle_timer = 5.0; // never tick over into WANDER (no wander-target search while it stands guard)
                b.direct_visible = false;
            }
            Phase::Retreat => {
                if let Some(lair) = self.lair {
                    b.state = State::Wander;
                    b.wander_target = Some(lair);
                }
                b.direct_visible = false;
            }
            Phase::Hunt => {}
        }
    }

    fn notices(&self, b: &Brain, fac: &Facility, t: &Target) -> bool {
        let d = dist(b.pos, t.pos);
        let noise = t.noise * b.hearing_scale * Rules::get().hear_radius_scale;
        if d <= self.alert_radius && fac.has_line_of_sight(b.pos, t.pos) {
            return true;
        }
        noise > 0.0 && d <= noise && fac.has_line_of_sight(b.pos, t.pos)
    }

    fn begin_bark(&mut self, b: &mut Brain, at: (f32, f32)) {
        self.phase = Phase::Bark;
        self.timer = self.bark_seconds;
        self.bark_seq += 1;
        b.state = State::Idle;
        face(b, at);
    }

    fn begin_retreat(&mut self, b: &mut Brain, lair: (f32, f32)) {
        self.phase = Phase::Retreat;
        b.state = State::Wander;
        b.wander_target = Some(lair);
        b.direct_visible = false;
    }

    /// The window moved under it: its lair moves with the cells.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.lair = self.lair.map(|l| (l.0 + d.0, l.1 + d.1));
    }

    /// The numbers a guest needs: phase and bark counter.
    pub fn network_state(&self) -> (u8, u32) {
        (self.phase.index(), self.bark_seq)
    }

    pub fn apply_network_state(&mut self, phase: i64, bark_seq: u32) {
        self.phase = Phase::from_index(phase);
        self.bark_seq = bark_seq;
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn face(b: &mut Brain, at: (f32, f32)) {
    let look = (at.0 - b.pos.0, at.1 - b.pos.1);
    let l2 = look.0 * look.0 + look.1 * look.1;
    if l2 > 0.0001 {
        let l = l2.sqrt();
        b.facing = (look.0 / l, look.1 / l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::hunters::brain::CrossSense;
    use crate::hunters::gimmicks::{Ctx, Gimmick};
    use crate::hunters::secret_creatures;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    const DT: f32 = 1.0 / 30.0;

    fn facility() -> Facility {
        Facility::generate("SPIDER-1", &Options::default(), &Catalog::load())
    }

    /// A lair and a runner spot 6 to 10 cells apart in clear line of sight.
    fn sight_pair(fac: &Facility) -> ((f32, f32), (f32, f32)) {
        let cells = fac.floor_cells();
        for a in cells.iter().step_by(3) {
            for b in cells.iter().step_by(7) {
                let (pa, pb) = (a.center(), b.center());
                let d = ((pa.0 - pb.0).powi(2) + (pa.1 - pb.1).powi(2)).sqrt();
                if (6.0..=10.0).contains(&d) && fac.has_line_of_sight(pa, pb) {
                    return (pa, pb);
                }
            }
        }
        panic!("no open pair in the test facility");
    }

    fn runner(pos: (f32, f32)) -> Target {
        Target { pos, facing: (1.0, 0.0), ..Default::default() }
    }

    /// A runner far outside the alert radius that makes no noise.
    fn nobody(from: (f32, f32)) -> Target {
        runner((from.0 + 500.0, from.1 + 500.0))
    }

    fn spider(id: &str, at: (f32, f32)) -> Brain {
        let content = Content::load();
        let mut b = Brain::new(content.hunter(id).unwrap().clone(), at, "SPIDER", Difficulty::Standard);
        b.active = true;
        b
    }

    fn gim(b: &Brain) -> &WolfSpider {
        match &b.gimmick {
            Gimmick::WolfSpider(s) => s,
            _ => panic!("not a wolf spider"),
        }
    }

    fn gim_mut(b: &mut Brain) -> &mut WolfSpider {
        match &mut b.gimmick {
            Gimmick::WolfSpider(s) => s,
            _ => panic!("not a wolf spider"),
        }
    }

    fn step(b: &mut Brain, fac: &Facility, t: &Target, dt: f32) {
        let ctx = Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: false,
        };
        b.update(dt, &ctx, t, 1, 0, &Difficulty::Standard.profile());
    }

    /// Steps until the spider is hunting (or `limit` frames pass).
    fn until_hunt(b: &mut Brain, fac: &Facility, t: &Target, limit: usize) {
        for _ in 0..limit {
            step(b, fac, t, DT);
            if gim(b).phase == Phase::Hunt {
                return;
            }
        }
    }

    #[test]
    fn it_is_the_gimmick_with_the_authored_numbers() {
        let b = spider(secret_creatures::WHITE, (5.5, 5.5));
        let w = gim(&b);
        assert_eq!(w.alert_radius, 15.0);
        assert!(w.flash_hold.is_some() && !w.hiss, "the white one is driven off by the light and barks");
    }

    #[test]
    fn it_lurks_barks_once_then_hunts_after_the_head_start() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        let mut b = spider(secret_creatures::WHITE, lair);
        // nobody marked: it stands at its lair and never wanders
        for _ in 0..90 {
            step(&mut b, &fac, &nobody(lair), DT);
        }
        assert_eq!(gim(&b).phase, Phase::Lurk);
        assert_eq!(b.pos, lair);
        // a runner in sight within the alert radius is marked: it barks once and holds still
        let mut barking = 0;
        for _ in 0..300 {
            step(&mut b, &fac, &runner(near), DT);
            match gim(&b).phase {
                Phase::Bark => {
                    barking += 1;
                    assert_eq!(b.pos, lair, "it stands still while it barks (the head start)");
                }
                Phase::Hunt => break,
                _ => {}
            }
        }
        assert_eq!(gim(&b).bark_seq, 1, "one bark");
        assert_eq!(gim(&b).phase, Phase::Hunt, "the bark ends in a hunt");
        assert!(barking >= (1.2 / DT) as usize, "the head start is about 1.25 s, got {barking} frames");
        assert_eq!(gim(&b).speed_scale(), 1.0);
        assert!(!gim(&b).holds_position(), "it runs once the bark is done");
    }

    #[test]
    fn a_shelter_ends_the_hunt_and_it_goes_home_and_rests() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        let mut b = spider(secret_creatures::WHITE, lair);
        until_hunt(&mut b, &fac, &runner(near), 300);
        assert_eq!(gim(&b).phase, Phase::Hunt);
        let sheltered = Target { in_safe_zone: true, ..runner(near) };
        step(&mut b, &fac, &sheltered, DT);
        assert_eq!(gim(&b).phase, Phase::Retreat, "a shelter ends the hunt at once");
        assert_eq!(gim(&b).speed_scale(), Rules::get().retreat_speed_scale);
        for _ in 0..4000 {
            step(&mut b, &fac, &sheltered, DT);
            if gim(&b).phase == Phase::Lurk {
                break;
            }
        }
        assert_eq!(gim(&b).phase, Phase::Lurk, "it hurries back to its lair");
        assert!(gim(&b).cooldown_left() > 25.0, "and rests (30 s)");
        // a runner in plain sight does not start another hunt while it rests
        for _ in 0..30 {
            step(&mut b, &fac, &runner(near), 0.2);
        }
        assert_eq!(gim(&b).phase, Phase::Lurk);
    }

    #[test]
    fn the_flashlight_drives_it_off() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        let mut b = spider(secret_creatures::WHITE, lair);
        until_hunt(&mut b, &fac, &runner(near), 300);
        assert_eq!(gim(&b).phase, Phase::Hunt);
        let mut left = false;
        for _ in 0..6 {
            b.beam_alerted = true;
            let mut g = std::mem::take(&mut b.gimmick);
            let c = Ctx {
                fac: &fac,
                relays_taken: &[],
                batteries_taken: &[],
                hiding_disabled: &[],
                vents: &[],
                heater_on: false,
                heater_thawed: true,
                diff: Difficulty::Standard,
                music_box_left: 0.0,
                music_boxes: &[],
                exposure: 0.0,
                flashlight_dead: false,
            };
            g.before_perception(&mut b, 0.1, &c, &runner(near), &mut vec![]);
            b.gimmick = g;
            left |= gim(&b).phase == Phase::Retreat;
        }
        assert!(left, "a beam held on it drives it off");
    }

    #[test]
    fn a_spawner_can_start_it_already_barking() {
        let fac = facility();
        let (lair, _) = sight_pair(&fac);
        let mut b = spider(secret_creatures::WHITE, lair);
        gim_mut(&mut b).start_with_bark();
        step(&mut b, &fac, &nobody(lair), DT);
        assert_eq!(gim(&b).phase, Phase::Bark, "it tells at once");
        assert_eq!(gim(&b).bark_seq, 1);
    }

    #[test]
    fn a_runner_on_another_level_is_not_hunted() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        let mut b = spider(secret_creatures::WHITE, lair);
        let up = Target { cross: Some(CrossSense { sees: true, hear_scale: 1.0 }), ..runner(near) };
        for _ in 0..120 {
            step(&mut b, &fac, &up, DT);
        }
        assert_eq!(gim(&b).phase, Phase::Lurk);
    }

    #[test]
    fn the_knobs_scale_the_creature_and_the_wire_numbers_round_trip() {
        let mut b = spider(secret_creatures::WHITE, (5.5, 5.5));
        let (sight, chase) = (b.def.sight, b.def.chase);
        let knobs = Knobs { notice: 1.2, chase: 1.1, rest: 0.7, alert: 1.2 };
        let mut g = std::mem::take(&mut b.gimmick);
        if let Gimmick::WolfSpider(s) = &mut g {
            s.apply_knobs(&mut b, &knobs);
            assert!((s.alert_radius - 15.0 * 1.2).abs() < 1e-4);
            s.apply_network_state(2, 7);
            assert_eq!(s.network_state(), (2, 7));
            s.apply_network_state(99, 1);
            assert_eq!(s.phase, Phase::Lurk, "an unknown phase reads as lurk");
        }
        b.gimmick = g;
        assert!((b.def.sight - sight * 1.2).abs() < 1e-4 && (b.def.chase - chase * 1.1).abs() < 1e-4);
    }
}
