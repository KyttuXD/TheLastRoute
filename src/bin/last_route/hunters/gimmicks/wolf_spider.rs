//! The white and black wolf spider (white_wolf_spider_gimmick.gd, black_wolf_spider_gimmick.gd, spider_pack.gd): a fast ambusher.
//! It lurks motionless at its lair, barks ONCE to mark a runner it has seen or heard (the telegraph), bursts after them for a
//! limited time, then gives up and goes home. Survivable on purpose: the bark gives a head start, the burst is limited by its own
//! stamina, a shelter ends the chase, and a flashlight beam held on it for a moment drives it off (the black one ignores the light).
//!
//!   LURK -> BARK (holds, bark clip + cue) -> HUNT (<= hunt seconds) -> RETREAT (to the lair) -> LURK (cooldown)
//!
//! Host-simulated like every hunter; a guest only needs `phase` and `bark_seq` (the hunter frame) to play the bark. The black
//! variant is the same machine with harsher numbers (data/hunters/secret_creatures.json). Specs: docs/SECRET_WOLF_SPIDER.md.

use crate::hunters::brain::{Brain, State, Target};
use crate::hunters::secret_creatures::{self, Knobs, Rules};
use crate::world::Facility;
use std::sync::{Arc, Mutex};

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

/// The shared state of a pair (host only, nothing here is replicated): one rest timer so the two never chain-hunt a runner one
/// after the other, and a recall so when one gives up the other goes home with it. Member 0 is the clock: only it advances
/// `elapsed`, so a pair does not tick twice per frame.
#[derive(Default, Debug)]
pub struct PackState {
    members: usize,
    elapsed: f32,
    rest_until: f32,
    pub recall_seq: u32,
}

impl PackState {
    pub fn resting(&self) -> bool {
        self.elapsed < self.rest_until
    }

    pub fn start_rest(&mut self, seconds: f32) {
        self.rest_until = self.rest_until.max(self.elapsed + seconds);
    }

    pub fn recall(&mut self) {
        self.recall_seq += 1;
    }
}

pub type Pack = Arc<Mutex<PackState>>;

pub struct WolfSpider {
    black: bool,
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
    spawn_bark_delay: f32,
    delay_left: f32,
    delay_target: (f32, f32),
    rest_scale: f32,
    pack: Option<(Pack, usize)>,
    seen_recall: u32,
}

impl WolfSpider {
    pub fn new(id: &str) -> WolfSpider {
        let r = Rules::get();
        let v = r.variant_of(id);
        WolfSpider {
            black: id == secret_creatures::BLACK,
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
            spawn_bark_delay: 0.0,
            delay_left: 0.0,
            delay_target: (0.0, 0.0),
            rest_scale: 1.0,
            pack: None,
            seen_recall: 0,
        }
    }

    pub fn is_black(&self) -> bool {
        self.black
    }

    pub fn lair(&self) -> Option<(f32, f32)> {
        self.lair
    }

    /// Seconds of cooldown left before it marks a runner again (tests read it).
    pub fn cooldown_left(&self) -> f32 {
        self.cooldown
    }

    /// A spawner may start it already marking its runner (set right after creating the brain). `delay` staggers the tell of the
    /// second creature of a pair (the first barks at once, the other follows `delay` seconds later).
    pub fn start_with_bark(&mut self, delay: f32) {
        self.spawn_barks = true;
        self.spawn_bark_delay = delay;
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

    /// Two creatures of one pair: one shared rest timer and recall. Joins into either's pack when it already has one.
    pub fn link(a: &mut WolfSpider, b: &mut WolfSpider) {
        if a.pack.is_some() && b.pack.is_some() {
            return;
        }
        let pack: Pack = match (&a.pack, &b.pack) {
            (Some((p, _)), _) | (_, Some((p, _))) => p.clone(),
            _ => Arc::new(Mutex::new(PackState::default())),
        };
        for s in [a, b] {
            if s.pack.is_none() {
                let mut p = pack.lock().unwrap();
                s.pack = Some((pack.clone(), p.members));
                p.members += 1;
            }
        }
    }

    pub fn in_pack(&self) -> bool {
        self.pack.is_some()
    }

    fn tick_pack(&self, dt: f32) {
        if let Some((p, member)) = &self.pack {
            if *member == 0 {
                p.lock().unwrap().elapsed += dt;
            }
        }
    }

    fn pack_resting(&self) -> bool {
        self.pack.as_ref().is_some_and(|(p, _)| p.lock().unwrap().resting())
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &super::Ctx, t: &Target) {
        let lair = match self.lair {
            Some(l) => l,
            None => {
                self.lair = Some(b.pos);
                if self.spawn_barks {
                    self.spawn_barks = false;
                    if self.spawn_bark_delay > 0.0 {
                        self.delay_left = self.spawn_bark_delay;
                        self.delay_target = t.pos;
                    } else {
                        self.begin_bark(b, t.pos);
                    }
                }
                b.pos
            }
        };
        self.tick_pack(dt);
        if self.delay_left > 0.0 && self.phase == Phase::Lurk {
            self.delay_left -= dt;
            if self.delay_left <= 0.0 {
                let at = self.delay_target;
                self.begin_bark(b, at);
            }
        }
        self.cooldown = (self.cooldown - dt).max(0.0);
        let tp = t.pos;
        // a runner on another level is out of reach: the spider never leaves its own
        let exposed = !t.hidden && !t.in_safe_zone && t.cross.is_none();
        match self.phase {
            Phase::Lurk => {
                self.sense_left -= dt;
                if self.cooldown <= 0.0 && !self.pack_resting() && self.delay_left <= 0.0 && exposed && self.sense_left <= 0.0 {
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
                let recalled = self.pack.as_ref().is_some_and(|(p, _)| p.lock().unwrap().recall_seq != self.seen_recall);
                let driven_off = b.beam_alerted && self.flash_hold.is_some_and(|hold| self.beam_time > hold);
                if recalled {
                    // the other one gave up: both go home together
                    self.seen_recall = self.pack.as_ref().map_or(0, |(p, _)| p.lock().unwrap().recall_seq);
                    self.begin_retreat(b, lair, false);
                } else if !exposed || b.winded || self.timer <= 0.0 || driven_off {
                    self.begin_retreat(b, lair, true);
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

    fn begin_retreat(&mut self, b: &mut Brain, lair: (f32, f32), call_pack: bool) {
        if call_pack {
            if let Some((p, _)) = &self.pack {
                let mut p = p.lock().unwrap();
                p.start_rest(self.cooldown_seconds * self.rest_scale);
                p.recall();
                self.seen_recall = p.recall_seq;
            }
        }
        self.phase = Phase::Retreat;
        b.state = State::Wander;
        b.wander_target = Some(lair);
        b.direct_visible = false;
    }

    /// The window moved under it: its lair and a pending tell's aim move with the cells.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.lair = self.lair.map(|l| (l.0 + d.0, l.1 + d.1));
        self.delay_target = (self.delay_target.0 + d.0, self.delay_target.1 + d.1);
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
    fn both_ids_get_the_gimmick_and_the_black_one_is_harsher() {
        let w = spider(secret_creatures::WHITE, (5.5, 5.5));
        let k = spider(secret_creatures::BLACK, (5.5, 5.5));
        let (w, k) = (gim(&w), gim(&k));
        assert!(!w.is_black() && k.is_black());
        assert!(k.alert_radius > w.alert_radius && k.hunt_seconds > w.hunt_seconds && k.cooldown_seconds > w.cooldown_seconds);
        assert!(w.flash_hold.is_some() && k.flash_hold.is_none());
        assert!(!w.hiss && k.hiss);
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
        assert!(gim(&b).cooldown_left() > 25.0, "and rests (30 s for the white one)");
        // a runner in plain sight does not start another hunt while it rests
        for _ in 0..30 {
            step(&mut b, &fac, &runner(near), 0.2);
        }
        assert_eq!(gim(&b).phase, Phase::Lurk);
    }

    #[test]
    fn the_flashlight_drives_the_white_one_off_but_not_the_black_one() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        for (id, driven_off) in [(secret_creatures::WHITE, true), (secret_creatures::BLACK, false)] {
            let mut b = spider(id, lair);
            until_hunt(&mut b, &fac, &runner(near), 300);
            assert_eq!(gim(&b).phase, Phase::Hunt, "{id}");
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
                };
                g.before_perception(&mut b, 0.1, &c, &runner(near), &mut vec![]);
                b.gimmick = g;
                left |= gim(&b).phase == Phase::Retreat;
            }
            assert_eq!(left, driven_off, "{id}");
        }
    }

    #[test]
    fn the_pair_shares_one_rest_and_a_recall() {
        let fac = facility();
        let (lair, near) = sight_pair(&fac);
        let mut a = spider(secret_creatures::WHITE, lair);
        let mut k = spider(secret_creatures::BLACK, lair);
        {
            let (Gimmick::WolfSpider(x), Gimmick::WolfSpider(y)) = (&mut a.gimmick, &mut k.gimmick) else { unreachable!() };
            WolfSpider::link(x, y);
            assert!(x.in_pack() && y.in_pack());
        }
        for _ in 0..300 {
            step(&mut a, &fac, &runner(near), DT);
            step(&mut k, &fac, &runner(near), DT);
            if gim(&a).phase == Phase::Hunt && gim(&k).phase == Phase::Hunt {
                break;
            }
        }
        assert_eq!((gim(&a).phase, gim(&k).phase), (Phase::Hunt, Phase::Hunt));
        let sheltered = Target { in_safe_zone: true, ..runner(near) };
        step(&mut a, &fac, &sheltered, DT);
        assert_eq!(gim(&a).phase, Phase::Retreat);
        // the other one is recalled with it, though its own runner is still in the open
        step(&mut k, &fac, &runner(near), DT);
        assert_eq!(gim(&k).phase, Phase::Retreat, "when one gives up both go home");
    }

    #[test]
    fn a_spawner_can_start_it_barking_and_stagger_the_second_tell() {
        let fac = facility();
        let (lair, _) = sight_pair(&fac);
        let mut first = spider(secret_creatures::WHITE, lair);
        let mut second = spider(secret_creatures::BLACK, lair);
        gim_mut(&mut first).start_with_bark(0.0);
        gim_mut(&mut second).start_with_bark(1.8);
        let far = nobody(lair);
        step(&mut first, &fac, &far, DT);
        step(&mut second, &fac, &far, DT);
        assert_eq!(gim(&first).phase, Phase::Bark, "the first tells at once");
        assert_eq!(gim(&second).phase, Phase::Lurk, "the second waits");
        for _ in 0..70 {
            step(&mut second, &fac, &far, DT);
        }
        assert_ne!(gim(&second).phase, Phase::Lurk, "and tells 1.8 s later");
        assert_eq!(gim(&second).bark_seq, 1);
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
