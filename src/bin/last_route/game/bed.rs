//! Beds: lying down and sleeping (Godot `player_controller_3d.gd` `use_furniture` "lay", `toggle_sleep`,
//! `sleep_disturb`, `_update_sleep`; docs/SLEEP.md). This file wires the two modules that were written ahead of it:
//! game/seating.rs holds the pose on the furniture and game/sleep.rs the sleeper and its drowsiness.
//!
//! Interact on a bed-type prop lies down, jump sleeps, jump or a move key wakes, interact gets up. A sleeper is not
//! hidden: the body stays where it stood beside the bed (only the eye moves onto the pillow), so a save, the net sample
//! and the hunters all read the standing spot, and a hunter catches a sleeper like anyone else.
//! The numbers and the texts are the `bed` section of data/sleep.json.

use super::seating::{seating, SeatKind, SeatState};
use super::sleep::{sleep_cfg, Sleep};
use super::{pdist, Controls, Game};
use crate::config::MAX_STAMINA;
use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../data/sleep.json");

/// One line of `bed.prop_rules`: a prop type (lower case) that contains or ends with the word is a bed of `kind`.
#[derive(Clone, Debug, PartialEq)]
pub struct PropRule {
    pub contains: Option<String>,
    pub ends_with: Option<String>,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BedText {
    pub lying: String,
    pub asleep: String,
    pub fell_asleep: String,
    pub awake: String,
    pub jolt: String,
    pub rested: String,
    pub not_tired: String,
}

/// The `bed` section of data/sleep.json.
#[derive(Clone, Debug, PartialEq)]
pub struct BedRules {
    pub prop_rules: Vec<PropRule>,
    pub eye_above_mattress: f32,
    pub head_inset: f32,
    pub ease_seconds: f32,
    pub look_pitch: f32,
    pub look_yaw_limit: f32,
    pub look_pitch_down: f32,
    pub look_pitch_up: f32,
    pub stamina_per_s: f32,
    pub dark_alpha: f32,
    pub dark_in_per_s: f32,
    pub stir_radius: f32,
    pub stir_cooldown_s: f32,
    pub text: BedText,
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as f32)
        .ok_or_else(|| format!("sleep.json: bed '{key}' missing or not a number of at least 0"))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("sleep.json: bed text '{key}' missing or empty"))
}

pub fn parse(src: &str) -> Result<BedRules, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("sleep.json: {e}"))?;
    let b = &v["bed"];
    let word = |r: &Value, key: &str| r.get(key).and_then(Value::as_str).filter(|w| !w.is_empty()).map(str::to_lowercase);
    let mut prop_rules = vec![];
    for r in b["prop_rules"].as_array().ok_or("sleep.json: bed 'prop_rules' missing")? {
        let rule = PropRule { contains: word(r, "contains"), ends_with: word(r, "ends_with"), kind: text(r, "kind")? };
        if rule.contains.is_none() && rule.ends_with.is_none() {
            return Err("sleep.json: a bed prop rule needs 'contains' or 'ends_with'".into());
        }
        if seating().kind(&rule.kind).is_none() {
            return Err(format!("sleep.json: bed prop rule kind '{}' is not in seating.json", rule.kind));
        }
        prop_rules.push(rule);
    }
    let t = &b["text"];
    let rules = BedRules {
        prop_rules,
        eye_above_mattress: num(b, "eye_above_mattress")?,
        head_inset: num(b, "head_inset")?,
        ease_seconds: num(b, "ease_seconds")?,
        look_pitch: num(b, "look_pitch")?,
        look_yaw_limit: num(b, "look_yaw_limit_deg")?.to_radians(),
        look_pitch_down: num(b, "look_pitch_down_deg")?.to_radians(),
        look_pitch_up: num(b, "look_pitch_up_deg")?.to_radians(),
        stamina_per_s: num(b, "stamina_per_s")?,
        dark_alpha: num(b, "dark_alpha")?,
        dark_in_per_s: num(b, "dark_in_per_s")?,
        stir_radius: num(b, "stir_radius")?,
        stir_cooldown_s: num(b, "stir_cooldown_s")?,
        text: BedText {
            lying: text(t, "lying")?,
            asleep: text(t, "asleep")?,
            fell_asleep: text(t, "fell_asleep")?,
            awake: text(t, "awake")?,
            jolt: text(t, "jolt")?,
            rested: text(t, "rested")?,
            not_tired: text(t, "not_tired")?,
        },
    };
    if rules.dark_alpha > 1.0 {
        return Err("sleep.json: bed 'dark_alpha' must be at most 1".into());
    }
    Ok(rules)
}

pub fn rules() -> &'static BedRules {
    static C: OnceLock<BedRules> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

impl BedRules {
    /// The seating kind a prop type lies on, None for a prop that is not a bed (Godot `interaction_for`: types ending
    /// in "Bed", plus bunks; "bedroomNightstand" and "bedroomBench" are not beds).
    pub fn kind_of(&self, ptype: &str) -> Option<&str> {
        let lowered = ptype.to_lowercase();
        self.prop_rules
            .iter()
            .find(|r| {
                r.contains.as_deref().is_some_and(|w| lowered.contains(w))
                    || r.ends_with.as_deref().is_some_and(|w| lowered.ends_with(w))
            })
            .map(|r| r.kind.as_str())
    }
}

/// The seat (kind name and its data/seating.json entry) of a bed prop type.
pub fn seat_of(ptype: &str) -> Option<(&'static str, &'static SeatKind)> {
    let kind = rules().kind_of(ptype)?;
    seating().kind(kind).map(|k| (kind, k))
}

/// The interact prompt of a bed prop type ("[E] LIE DOWN": the label is the seat kind's).
pub fn lie_prompt(ptype: &str) -> String {
    match seat_of(ptype) {
        Some((_, k)) => format!("[E] {}", k.label),
        None => String::new(),
    }
}

/// The runner's rest on a bed: the pose (seating.rs), the sleeper (sleep.rs) and the pillow view.
#[derive(Clone, Debug, Default)]
pub struct Rest {
    seat: SeatState,
    sleep: Sleep,
    /// The standing eye when the runner lay down and the pillow eye, both as (x, view y, z) with x and z relative to
    /// the body: the body does not move, so the view survives a window recentre.
    from_off: (f32, f32, f32),
    eye_off: (f32, f32, f32),
    /// The look along the bed that the view is held around.
    base_yaw: f32,
    /// 0..1 of the glide from the standing eye to the pillow.
    ease: f32,
    /// 0..1 of the dark screen while asleep.
    dark: f32,
    stir_cd: f32,
}

impl Rest {
    pub fn lying(&self) -> bool {
        self.seat.locked()
    }

    pub fn asleep(&self) -> bool {
        self.sleep.sleeping()
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

impl Game {
    /// The centre of bed prop `i` (the cell centre plus the prop's placement offset).
    pub(super) fn bed_centre(&self, i: usize) -> (f32, f32) {
        match self.fac.props.get(i) {
            Some(p) => {
                let (x, z) = p.cell.center();
                (x + p.offset.0, z + p.offset.1)
            }
            None => self.p.pos,
        }
    }

    /// Interact on bed prop `i`: the runner lies down looking along the bed, in the sense nearer to where it looks now.
    /// False when the prop is no bed, is out of the seating reach, or the runner already lies.
    pub(super) fn lie_down(&mut self, i: usize) -> bool {
        let Some((ptype, half)) = self.fac.props.get(i).map(|p| (p.ptype.clone(), p.half)) else {
            return false;
        };
        let Some((kind, _)) = seat_of(&ptype) else {
            return false;
        };
        if self.rest.lying() {
            return false;
        }
        let r = rules();
        let centre = self.bed_centre(i);
        let (fx, fz) = self.p.forward2();
        let facing = if half.0 >= half.1 {
            (if fx >= 0.0 { 1.0 } else { -1.0 }, 0.0)
        } else {
            (0.0, if fz >= 0.0 { 1.0 } else { -1.0 })
        };
        let body = [self.p.pos.0, self.p.height, self.p.pos.1];
        let mut seat = SeatState::default();
        let (anchor, yaw) = match seat.sit(seating(), kind, centre, facing, body, self.p.height) {
            Some(s) => (s.anchor, s.yaw),
            None => return false,
        };
        // the head lies at the pillow end, behind the look
        let back = (half.0.max(half.1) - r.head_inset).max(0.0);
        let stand = self.p.eye();
        self.rest = Rest {
            seat,
            sleep: Sleep::default(),
            from_off: (stand.x - self.p.pos.0, stand.y, stand.z - self.p.pos.1),
            eye_off: (
                anchor[0] - facing.0 * back - self.p.pos.0,
                anchor[1] + r.eye_above_mattress,
                anchor[2] - facing.1 * back - self.p.pos.1,
            ),
            base_yaw: yaw,
            ease: 0.0,
            dark: 0.0,
            stir_cd: 0.0,
        };
        self.p.yaw = yaw;
        self.p.pitch = r.look_pitch;
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.flashlight_on = false;
        self.focus = None;
        true
    }

    /// Gets up (waking first): the body never left the spot beside the bed, so only the view returns.
    pub(super) fn get_up(&mut self) {
        self.rest = Rest::default();
    }

    /// The eye while the runner lies on a bed (view-relative, like `Player::eye`), None when it does not.
    pub(super) fn rest_eye(&self) -> Option<Vec3> {
        if !self.rest.lying() {
            return None;
        }
        let t = self.rest.ease.clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let (a, b) = (self.rest.from_off, self.rest.eye_off);
        Some(vec3(
            self.p.pos.0 + a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            self.p.pos.1 + a.2 + (b.2 - a.2) * t,
        ))
    }

    /// The prompt while the runner lies or sleeps, None when it does not.
    pub(super) fn rest_prompt(&self) -> Option<String> {
        if !self.rest.lying() {
            return None;
        }
        let t = &rules().text;
        let line = if self.rest.asleep() { &t.asleep } else { &t.lying };
        let max = sleep_cfg().max.max(1e-3);
        let fatigue = (self.meters.drowsiness / max * 100.0).clamp(0.0, 100.0).round() as i32;
        Some(line.replace("{fatigue}", &fatigue.to_string()))
    }

    /// Holds the look of a lying runner around the bed's length (Godot `_clamp_rest_look`).
    pub(super) fn rest_clamp_look(&mut self) {
        if !self.rest.lying() {
            return;
        }
        let r = rules();
        let base = self.rest.base_yaw;
        self.p.yaw = base + wrap_angle(self.p.yaw - base).clamp(-r.look_yaw_limit, r.look_yaw_limit);
        self.p.pitch = self.p.pitch.clamp(-r.look_pitch_down, r.look_pitch_up);
    }

    /// Every frame, before the look: a grab, a down, a hiding spot, a ladder or the run's end takes the runner off the
    /// bed, and the drowsiness steps (it rises awake and falls asleep: data/sleep.json).
    pub(super) fn rest_tick(&mut self, dt: f32) {
        if self.rest.lying()
            && (self.escape.is_some()
                || self.downed.is_some()
                || self.p.hidden.is_some()
                || self.ladder.is_some()
                || self.report.is_some())
        {
            self.get_up();
        }
        self.meters.update_drowsiness(dt, self.p.moving, self.rest.asleep(), sleep_cfg());
    }

    /// The nearest awake hunter of this floor and whether one chases the runner.
    fn sleep_threat(&self) -> (Option<f32>, bool) {
        let mut nearest: Option<f32> = None;
        let mut chased = false;
        for h in self.hunters.iter().filter(|h| h.active && !h.shadow) {
            let d = pdist(h.pos, self.p.pos);
            nearest = Some(nearest.map_or(d, |n| n.min(d)));
            chased |= h.state == crate::hunters::brain::State::Chase && !h.target_is_bot;
        }
        (nearest, chased)
    }

    fn wake(&mut self, line: &str) {
        self.rest.sleep = Sleep::default();
        self.say(line);
    }

    /// One frame of a runner lying on a bed: it does not move. Interact gets up; jump sleeps or wakes; a move key wakes.
    /// Asleep the screen darkens, the stamina refills faster and the drowsiness falls; the sleeper wakes rested, or with
    /// a jolt when a hunter comes near or chases it (game/sleep.rs `Sleep::step`), or when something stuns it.
    pub(super) fn update_rest(&mut self, dt: f32, c: &Controls) {
        let (cfg, r) = (sleep_cfg(), rules());
        let jump_edge = c.jump && !self.p.jump_prev;
        self.p.jump_prev = c.jump;
        let stunned = self.p.stun > 0.0;
        self.p.stun = (self.p.stun - dt).max(0.0);
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.noise = 0.0;
        self.p.move_vel = (0.0, 0.0);
        self.p.flashlight_on = false;
        self.p.recover_stamina(dt);
        self.focus = None;
        self.rest.ease = (self.rest.ease + dt / r.ease_seconds.max(1e-3)).min(1.0);
        let in_safe = self.in_safe_zone();
        let local = self.felt_climate();
        self.meters.update(
            dt,
            &super::meters::Situation {
                flashlight_on: false,
                noise: 0.0,
                crouching: false,
                hidden: false,
                in_safe_zone: in_safe,
                local,
            },
        );
        if c.interact && !stunned {
            self.get_up();
            return;
        }
        if !self.rest.asleep() {
            self.rest.dark = 0.0;
            if jump_edge && !stunned {
                if self.meters.drowsiness <= cfg.wake_at {
                    crate::audio::sound::interaction_fail();
                    self.say(&r.text.not_tired);
                } else if self.rest.sleep.toggle_sleep(self.meters.drowsiness, cfg) {
                    self.rest.stir_cd = 0.0;
                    crate::audio::sound::sleep_settle();
                    self.say(&r.text.fell_asleep);
                }
            }
            return;
        }
        if jump_edge || c.fwd != 0.0 || c.strafe != 0.0 {
            self.wake(&r.text.awake);
            return;
        }
        if stunned {
            crate::audio::sound::sleep_jolt();
            self.wake(&r.text.jolt);
            return;
        }
        self.p.stamina = (self.p.stamina + r.stamina_per_s * dt).min(MAX_STAMINA);
        let (nearest, chased) = self.sleep_threat();
        let mut drowsiness = self.meters.drowsiness;
        let jolt = self.rest.sleep.step(&mut drowsiness, nearest, false, chased, cfg);
        self.meters.drowsiness = drowsiness;
        if let Some(j) = jolt {
            self.meters.exposure = (self.meters.exposure + j.exposure).clamp(0.0, 100.0);
            self.meters.noise_meter = (self.meters.noise_meter + j.noise).clamp(0.0, 100.0);
            self.p.stun = self.p.stun.max(j.frozen_seconds);
            crate::audio::sound::sleep_jolt();
            self.say(&r.text.jolt);
            return;
        }
        if !self.rest.asleep() {
            self.say(&r.text.rested);
            return;
        }
        self.rest.stir_cd = (self.rest.stir_cd - dt).max(0.0);
        if let Some(d) = nearest.filter(|d| *d <= r.stir_radius) {
            if self.rest.stir_cd <= 0.0 {
                self.rest.stir_cd = r.stir_cooldown_s;
                crate::audio::sound::sleep_stir(d);
            }
        }
        self.rest.dark = (self.rest.dark + r.dark_in_per_s * dt).min(1.0);
        self.fade = self.fade.max(self.rest.dark * r.dark_alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::{Difficulty, STAMINA_RECOVERY};
    use crate::content::Content;
    use crate::game::{Escape, Focus};
    use crate::world::catalog::Catalog;

    const DT: f32 = 1.0 / 60.0;

    fn bed_prop(ptype: &str, cell: crate::world::Cell, offset: (f32, f32)) -> crate::world::facility::Prop {
        crate::world::facility::Prop {
            id: "TEST:BED".to_string(),
            cell,
            ptype: ptype.to_string(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.5, 1.0),
            offset,
            door: None,
            sewer: None,
            crawl_under: false,
        }
    }

    /// A run with a single bed 0.8 m in front of the runner (along +z), the runner looking at its mattress, tired,
    /// and no hunter awake.
    fn bed_game(room: &str) -> (Game, usize, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, room, 0, &content, &catalog);
        g.ensure_hunters(&content);
        for h in g.hunters.iter_mut() {
            h.active = false;
        }
        let cell = crate::world::Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.p.pos = cell.center();
        g.fac.props.push(bed_prop("singleBed", cell, (0.0, 0.8)));
        let i = g.fac.props.len() - 1;
        let (_, k) = seat_of("singleBed").unwrap();
        g.p.yaw = std::f32::consts::PI;
        g.p.pitch = (k.height - g.p.eye().y).atan2(0.8);
        g.p.stun = 0.0;
        g.meters.drowsiness = 50.0;
        (g, i, content, catalog)
    }

    fn tick(g: &mut Game, c: &Controls) {
        g.rest_tick(DT);
        if g.rest.lying() {
            g.update_rest(DT, c);
        }
    }

    fn press_jump(g: &mut Game) {
        tick(g, &Controls { jump: true, ..Controls::default() });
        tick(g, &Controls::default());
    }

    #[test]
    fn the_shipped_bed_rules_parse_and_name_the_beds_godot_names() {
        let r = parse(TEXT).expect("the bed section parses");
        assert_eq!(&r, rules());
        for bed in ["singleBed", "doubleBed", "hospitalBed", "gothicDoubleBed", "bed", "singleBedMansion"] {
            assert_eq!(r.kind_of(bed), Some("bed"), "{bed}");
        }
        for bunk in ["bunkBed", "prisonBunk", "bunk"] {
            assert_eq!(r.kind_of(bunk), Some("bunk"), "{bunk}");
        }
        for other in ["bedroomNightstand", "bedroomWardrobe", "bedroomBench", "bedsideLamp", "officeChair"] {
            assert_eq!(r.kind_of(other), None, "{other} is not a bed");
        }
        assert_eq!(lie_prompt("singleBed"), "[E] LIE DOWN");
        assert!(parse("{}").is_err());
        assert!(parse(&TEXT.replace("\"stamina_per_s\": 9.0,", "")).is_err(), "the numbers are required data");
    }

    #[test]
    fn a_bed_in_reach_offers_to_lie_down_and_a_lying_runner_is_offered_sleep() {
        let (mut g, i, content, catalog) = bed_game("ROOM-BED-7");
        assert_eq!(g.find_focus(), Some(Focus::Bed(i)));
        g.focus = Some(Focus::Bed(i));
        assert_eq!(g.focus_prompt().as_deref(), Some("[E] LIE DOWN"));
        g.interact(Focus::Bed(i), &content, &catalog, &Sounds::default());
        assert!(g.rest.lying() && !g.rest.asleep());
        let prompt = g.focus_prompt().expect("the lying prompt");
        assert!(prompt.contains("SLEEP") && prompt.contains("50%"), "{prompt}");
        press_jump(&mut g);
        assert!(g.rest.asleep());
        assert_eq!(g.focus_prompt().as_deref(), Some(rules().text.asleep.as_str()));
        // a bed out of the seating reach offers nothing
        g.get_up();
        g.fac.props[i].offset = (0.0, seating().default_reach + 1.0);
        assert_ne!(g.find_focus(), Some(Focus::Bed(i)));
        assert!(!g.lie_down(i), "out of reach");
    }

    #[test]
    fn a_lying_runner_does_not_move_and_the_eye_goes_to_the_pillow() {
        let (mut g, i, content, catalog) = bed_game("ROOM-BED-7");
        let stand = g.eye();
        assert!(g.lie_down(i));
        let at = g.p.pos;
        let walk = Controls { fwd: 1.0, strafe: 1.0, sprint: true, ..Controls::default() };
        for _ in 0..60 {
            g.step(DT, &walk, &content, &catalog, &Sounds::default());
        }
        assert!(g.rest.lying(), "the move keys do not get a waking runner up");
        assert_eq!(g.p.pos, at, "the body stays put");
        let eye = g.eye();
        let (_, k) = seat_of("singleBed").unwrap();
        assert!((eye.y - (g.p.height + k.height + rules().eye_above_mattress)).abs() < 1e-4, "pillow height {}", eye.y);
        assert!(eye.y < stand.y);
        assert!(pdist((eye.x, eye.z), g.bed_centre(i)) <= 1.0, "the eye is on the bed");
        // the look is held around the bed's length
        g.p.yaw += 3.0;
        g.p.pitch = -1.5;
        g.rest_clamp_look();
        assert!(wrap_angle(g.p.yaw - g.rest.base_yaw).abs() <= rules().look_yaw_limit + 1e-4);
        assert!(g.p.pitch >= -rules().look_pitch_down - 1e-4);
        // interact gets up, on the spot the runner stood on
        g.step(DT, &Controls { interact: true, ..Controls::default() }, &content, &catalog, &Sounds::default());
        assert!(!g.rest.lying());
        assert_eq!(g.p.pos, at);
        assert_eq!(g.eye(), g.p.eye());
    }

    #[test]
    fn sleep_restores_fatigue_and_stamina_by_the_data_rates_and_darkens_the_screen() {
        let (mut g, i, _, _) = bed_game("ROOM-BED-7");
        assert!(g.lie_down(i));
        press_jump(&mut g);
        assert!(g.rest.asleep());
        let (d0, s0) = (g.meters.drowsiness, 10.0);
        g.p.stamina = s0;
        g.p.stim_crash = 0.0;
        g.fade = 0.0;
        for _ in 0..60 {
            tick(&mut g, &Controls::default());
        }
        let fell = d0 - g.meters.drowsiness;
        assert!((fell - sleep_cfg().sleep_fall_per_s).abs() < 0.05, "fatigue fell {fell} in one second");
        let rose = g.p.stamina - s0;
        assert!((rose - (STAMINA_RECOVERY + rules().stamina_per_s)).abs() < 0.2, "stamina rose {rose}");
        assert!(g.fade > 0.5 && g.fade <= rules().dark_alpha, "dark screen {}", g.fade);
    }

    #[test]
    fn a_sleeper_wakes_on_jump_on_a_move_key_and_when_rested() {
        let (mut g, i, _, _) = bed_game("ROOM-BED-7");
        assert!(g.lie_down(i));
        press_jump(&mut g);
        assert!(g.rest.asleep());
        press_jump(&mut g);
        assert!(!g.rest.asleep() && g.rest.lying(), "jump wakes, the runner still lies");
        press_jump(&mut g);
        assert!(g.rest.asleep());
        tick(&mut g, &Controls { strafe: -1.0, ..Controls::default() });
        assert!(!g.rest.asleep() && g.rest.lying(), "a move key wakes");
        // rested: the drowsiness reaches `wake_at`
        g.meters.drowsiness = sleep_cfg().wake_at + 1.0;
        press_jump(&mut g);
        assert!(g.rest.asleep());
        for _ in 0..120 {
            tick(&mut g, &Controls::default());
            if !g.rest.asleep() {
                break;
            }
        }
        assert!(!g.rest.asleep() && g.rest.lying());
        assert!(g.meters.drowsiness <= sleep_cfg().wake_at + 1e-3);
        assert_eq!(g.message, rules().text.rested);
        // a rested runner is not tired enough to sleep again
        g.meters.drowsiness = 0.0;
        press_jump(&mut g);
        assert!(!g.rest.asleep());
        // interact gets a sleeper up in one press
        g.meters.drowsiness = 50.0;
        press_jump(&mut g);
        assert!(g.rest.asleep());
        tick(&mut g, &Controls { interact: true, ..Controls::default() });
        assert!(!g.rest.lying() && !g.rest.asleep());
    }

    #[test]
    fn a_hunter_coming_near_or_a_stun_jolts_the_sleeper_awake() {
        let (mut g, i, content, _) = bed_game("ROOM-IMMUNE-7");
        // the arrival floor may hold no hunter: this test brings its own
        if g.hunters.is_empty() {
            let def = content.hunters.iter().find(|h| !crate::hunters::secret_creatures::is_secret(&h.id)).expect("a hunter in the data").clone();
            g.hunters.push(crate::hunters::brain::Brain::new(def, g.p.pos, "BED-TEST", Difficulty::Standard));
        }
        assert!(g.lie_down(i));
        press_jump(&mut g);
        // far away: the sleeper sleeps on
        g.hunters[0].active = true;
        g.hunters[0].state = crate::hunters::brain::State::Wander;
        g.hunters[0].pos = (g.p.pos.0 + sleep_cfg().disturb_radius + 3.0, g.p.pos.1);
        tick(&mut g, &Controls::default());
        assert!(g.rest.asleep());
        // inside the disturb radius: a jolt
        let exposure = g.meters.exposure;
        g.hunters[0].pos = (g.p.pos.0 + sleep_cfg().disturb_radius - 1.0, g.p.pos.1);
        tick(&mut g, &Controls::default());
        assert!(!g.rest.asleep() && g.rest.lying());
        assert!(g.p.stun > 0.0, "the jolt freezes the runner for a moment");
        assert!(g.meters.exposure > exposure);
        assert_eq!(g.message, rules().text.jolt);
        // a stun (a pounce, a wire) wakes too
        g.hunters[0].active = false;
        g.p.stun = 0.0;
        press_jump(&mut g);
        assert!(g.rest.asleep());
        g.p.stun = 0.5;
        tick(&mut g, &Controls::default());
        assert!(!g.rest.asleep());
    }

    #[test]
    fn a_sleeper_is_not_hidden_and_a_catch_takes_it_off_the_bed() {
        let (mut g, i, _, _) = bed_game("ROOM-IMMUNE-7");
        g.room_teammates = 1;
        assert!(g.lie_down(i));
        press_jump(&mut g);
        assert!(g.rest.asleep());
        assert!(g.p.hidden.is_none(), "sleeping is not hiding");
        // a hunter's grab (the escape struggle) ends the rest
        g.escape = Some(Escape { remaining: 1.0, progress: 0, hunter: 0 });
        g.rest_tick(DT);
        assert!(!g.rest.lying() && !g.rest.asleep());
        g.escape = None;
        // so does being taken down
        assert!(g.lie_down(i));
        press_jump(&mut g);
        assert!(g.rest.asleep());
        assert!(g.go_down(0), "a sleeper is caught like any runner");
        assert!(g.runner_downed());
        g.rest_tick(DT);
        assert!(!g.rest.lying());
    }

    #[test]
    fn hiding_still_works_and_a_hidden_runner_cannot_lie_down() {
        let (mut g, i, content, catalog) = bed_game("ROOM-TEST-7");
        assert!(!g.fac.hiding.is_empty());
        let cell = g.fac.props[i].cell;
        g.interact(Focus::Locker(0), &content, &catalog, &Sounds::default());
        assert_eq!(g.p.hidden, Some(0));
        assert!(!g.rest.lying());
        g.rest_tick(DT);
        assert_eq!(g.p.hidden, Some(0), "the rest tick leaves a hidden runner alone");
        g.leave_locker(None);
        assert!(g.p.hidden.is_none());
        // a runner that lies down and is then put in a spot leaves the bed
        g.p.pos = cell.center();
        assert!(g.lie_down(i));
        g.p.hidden = Some(0);
        g.rest_tick(DT);
        assert!(!g.rest.lying());
        g.p.hidden = None;
    }
}
