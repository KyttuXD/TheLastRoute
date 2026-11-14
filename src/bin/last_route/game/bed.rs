//! Beds: lying down and sleeping (Godot `player_controller_3d.gd` `use_furniture` "lay", `toggle_sleep`,
//! `sleep_disturb`, `_update_sleep`; docs/SLEEP.md). This file wires the two modules that were written ahead of it:
//! game/seating.rs holds the pose on the furniture and game/sleep.rs the sleeper and its drowsiness.
//!
//! Interact on a bed-type prop lies down, jump sleeps, jump or a move key wakes, interact gets up. A sleeper is not
//! hidden: the body stays where it stood beside the bed (only the eye moves onto the pillow), so a save, the net sample
//! and the hunters all read the standing spot, and a hunter catches a sleeper like anyone else.
//! The numbers and the texts are the `bed` section of data/sleep.json.
//!
//! Seats (Godot `use_furniture` "sit") share the same rest: interact on a chair, couch, bench or stool sits, interact
//! or a move key stands up. A seated runner is as visible and as catchable as a lying one, cannot sleep, and gets no
//! stamina beyond the normal recovery. The seat rules are the `sit` section of data/seating.json.

use super::seating::{seating, Pose, SeatKind, SeatState};
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
    pub lying_rested: String,
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
            lying_rested: text(t, "lying_rested")?,
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

/// The kind of the first rule that matches a prop type (compared in lower case).
fn kind_in<'a>(prop_rules: &'a [PropRule], ptype: &str) -> Option<&'a str> {
    let lowered = ptype.to_lowercase();
    prop_rules
        .iter()
        .find(|r| {
            r.contains.as_deref().is_some_and(|w| lowered.contains(w))
                || r.ends_with.as_deref().is_some_and(|w| lowered.ends_with(w))
        })
        .map(|r| r.kind.as_str())
}

impl BedRules {
    /// The seating kind a prop type lies on, None for a prop that is not a bed (Godot `interaction_for`: types ending
    /// in "Bed", plus bunks; "bedroomNightstand" and "bedroomBench" are not beds).
    pub fn kind_of(&self, ptype: &str) -> Option<&str> {
        kind_in(&self.prop_rules, ptype)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SitText {
    pub seated: String,
    pub stood: String,
}

/// The `sit` section of data/seating.json.
#[derive(Clone, Debug, PartialEq)]
pub struct SitRules {
    pub prop_rules: Vec<PropRule>,
    /// Prop types (lower case) that match a rule and are still no seat (the work benches).
    pub not_seats: Vec<String>,
    pub eye_above_seat: f32,
    pub look_pitch: f32,
    pub look_yaw_limit: f32,
    pub look_pitch_down: f32,
    pub look_pitch_up: f32,
    pub stand_on_move: bool,
    pub text: SitText,
}

pub fn parse_sit(src: &str) -> Result<SitRules, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("seating.json: {e}"))?;
    let s = &v["sit"];
    let err = |what: &str| format!("seating.json: sit {what}");
    let num = |key: &str| s.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| err(&format!("'{key}' missing or not a number")));
    let limit = |key: &str| num(key).and_then(|x| if x >= 0.0 { Ok(x.to_radians()) } else { Err(err(&format!("'{key}' must be at least 0"))) });
    let line = |key: &str| s["text"].get(key).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string).ok_or_else(|| err(&format!("text '{key}' missing or empty")));
    let word = |r: &Value, key: &str| r.get(key).and_then(Value::as_str).filter(|w| !w.is_empty()).map(str::to_lowercase);
    let mut prop_rules = vec![];
    for r in s["prop_rules"].as_array().ok_or_else(|| err("'prop_rules' missing"))? {
        let rule = PropRule { contains: word(r, "contains"), ends_with: word(r, "ends_with"), kind: word(r, "kind").ok_or_else(|| err("prop rule 'kind' missing"))? };
        if rule.contains.is_none() && rule.ends_with.is_none() {
            return Err(err("prop rule needs 'contains' or 'ends_with'"));
        }
        if seating().kind(&rule.kind).map(|k| k.pose) != Some(Pose::Sit) {
            return Err(err(&format!("prop rule kind '{}' is not a sit kind of seating.json", rule.kind)));
        }
        prop_rules.push(rule);
    }
    Ok(SitRules {
        prop_rules,
        not_seats: s["not_seats"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_lowercase).collect(),
        eye_above_seat: num("eye_above_seat")?,
        look_pitch: num("look_pitch")?,
        look_yaw_limit: limit("look_yaw_limit_deg")?,
        look_pitch_down: limit("look_pitch_down_deg")?,
        look_pitch_up: limit("look_pitch_up_deg")?,
        stand_on_move: s["stand_on_move"].as_bool().unwrap_or(false),
        text: SitText { seated: line("seated")?, stood: line("stood")? },
    })
}

pub fn sit_rules() -> &'static SitRules {
    static C: OnceLock<SitRules> = OnceLock::new();
    C.get_or_init(|| parse_sit(super::seating::TEXT).unwrap_or_else(|e| panic!("{e}")))
}

impl SitRules {
    /// The seating kind a prop type is sat on, None for a prop that is no seat (Godot `interaction_for` "sit": a type
    /// containing chair, couch, bench, stool or sofa, and the diner booth; the work benches are left out).
    pub fn kind_of(&self, ptype: &str) -> Option<&str> {
        let lowered = ptype.to_lowercase();
        if self.not_seats.iter().any(|t| *t == lowered) {
            return None;
        }
        kind_in(&self.prop_rules, ptype)
    }
}

/// The seat (kind name and its data/seating.json entry) of a prop type that is sat on. A bed is never a seat.
pub fn sit_seat_of(ptype: &str) -> Option<(&'static str, &'static SeatKind)> {
    if rules().kind_of(ptype).is_some() {
        return None;
    }
    let kind = sit_rules().kind_of(ptype)?;
    seating().kind(kind).map(|k| (kind, k))
}

/// The interact prompt of a seat prop type ("[E] SIT": the label is the seat kind's).
pub fn sit_prompt(ptype: &str) -> String {
    match sit_seat_of(ptype) {
        Some((_, k)) => format!("[E] {}", k.label),
        None => String::new(),
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

/// The runner's rest on a bed or a seat: the pose (seating.rs), the sleeper (sleep.rs) and the pillow or seat view.
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
    /// Seated: the move keys have been let go since sitting down, so the next press stands up (a runner that walked
    /// into the seat with a key held is not stood up again at once).
    move_armed: bool,
}

impl Rest {
    /// On a bed or a seat: the pose holds the runner.
    pub fn resting(&self) -> bool {
        self.seat.locked()
    }

    pub fn lying(&self) -> bool {
        self.seat.seated().is_some_and(|s| s.pose == Pose::Lie)
    }

    pub fn sitting(&self) -> bool {
        self.seat.seated().is_some_and(|s| s.pose == Pose::Sit)
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
        if self.rest.resting() {
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
            move_armed: false,
        };
        self.p.yaw = yaw;
        self.p.pitch = r.look_pitch;
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.flashlight_on = false;
        self.focus = None;
        true
    }

    /// Interact on seat prop `i`: the runner sits, looking out of the seat's front (the prop's own facing: its local
    /// -z turned by its quarter turns, as Godot's furniture point). The body stays where it stood, like on a bed; only
    /// the eye goes to the seat. False when the prop is no seat, is out of the seating reach, or the runner already rests.
    pub(super) fn sit_down(&mut self, i: usize) -> bool {
        let Some((ptype, rotation)) = self.fac.props.get(i).map(|p| (p.ptype.clone(), p.rotation)) else {
            return false;
        };
        let Some((kind, _)) = sit_seat_of(&ptype) else {
            return false;
        };
        if self.rest.resting() {
            return false;
        }
        let r = sit_rules();
        let centre = self.bed_centre(i);
        let turn = rotation as f32 * std::f32::consts::FRAC_PI_2;
        let facing = (-turn.sin(), -turn.cos());
        let body = [self.p.pos.0, self.p.height, self.p.pos.1];
        let mut seat = SeatState::default();
        let (anchor, yaw) = match seat.sit(seating(), kind, centre, facing, body, self.p.height) {
            Some(s) => (s.anchor, s.yaw),
            None => return false,
        };
        let stand = self.p.eye();
        self.rest = Rest {
            seat,
            sleep: Sleep::default(),
            from_off: (stand.x - self.p.pos.0, stand.y, stand.z - self.p.pos.1),
            eye_off: (anchor[0] - self.p.pos.0, anchor[1] + r.eye_above_seat, anchor[2] - self.p.pos.1),
            base_yaw: yaw,
            ease: 0.0,
            dark: 0.0,
            stir_cd: 0.0,
            move_armed: false,
        };
        self.p.yaw = yaw;
        self.p.pitch = r.look_pitch;
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.flashlight_on = false;
        self.focus = None;
        true
    }

    /// What this runner's sample says about its rest, for the other runners (`Sample::with_rest`): lying or sitting,
    /// where the body is drawn in absolute cells (the mattress or the seat, not the spot beside it where the body
    /// stands), how far that is above the runner's feet, and the way it faces. `None` when the runner is on its feet.
    /// The anchor is taken relative to the body, which does not move while it rests, so a window recentre keeps it right.
    pub fn rest_wire(&self) -> Option<crate::net::world_sync::RestPose> {
        let s = self.rest.seat.seated()?;
        let o = self.window_origin();
        Some(crate::net::world_sync::RestPose {
            lying: s.pose == Pose::Lie,
            x: self.p.pos.0 + (s.anchor[0] - s.from[0]) + o.0 as f32,
            y: self.p.pos.1 + (s.anchor[2] - s.from[2]) + o.1 as f32,
            up: s.anchor[1] - s.floor_y,
            yaw: s.yaw,
        })
    }

    /// Gets up (waking first): the body never left the spot beside the bed, so only the view returns.
    pub(super) fn get_up(&mut self) {
        self.rest = Rest::default();
    }

    /// The eye while the runner lies on a bed or sits (view-relative, like `Player::eye`), None when it does not.
    pub(super) fn rest_eye(&self) -> Option<Vec3> {
        if !self.rest.resting() {
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

    /// The prompt while the runner sits, lies or sleeps, None when it does not.
    pub(crate) fn rest_prompt(&self) -> Option<String> {
        if self.rest.sitting() {
            return Some(sit_rules().text.seated.clone());
        }
        if !self.rest.lying() {
            return None;
        }
        let t = &rules().text;
        // too awake to fall asleep (the sleep key answers NOT TIRED): say so instead of offering SLEEP
        let line = if self.rest.asleep() {
            &t.asleep
        } else if self.meters.drowsiness <= sleep_cfg().wake_at {
            &t.lying_rested
        } else {
            &t.lying
        };
        let max = sleep_cfg().max.max(1e-3);
        let fatigue = (self.meters.drowsiness / max * 100.0).clamp(0.0, 100.0).round() as i32;
        Some(line.replace("{fatigue}", &fatigue.to_string()))
    }

    /// Holds the look of a lying runner around the bed's length and of a seated one around the seat's facing (Godot
    /// `_clamp_rest_look`).
    pub(super) fn rest_clamp_look(&mut self) {
        if !self.rest.resting() {
            return;
        }
        let (yaw_limit, down, up) = if self.rest.lying() {
            let r = rules();
            (r.look_yaw_limit, r.look_pitch_down, r.look_pitch_up)
        } else {
            let r = sit_rules();
            (r.look_yaw_limit, r.look_pitch_down, r.look_pitch_up)
        };
        let base = self.rest.base_yaw;
        self.p.yaw = base + wrap_angle(self.p.yaw - base).clamp(-yaw_limit, yaw_limit);
        self.p.pitch = self.p.pitch.clamp(-down, up);
    }

    /// Every frame, before the look: a grab, a down, a hiding spot, a ladder or the run's end takes the runner off the
    /// bed, and the drowsiness steps (it rises awake and falls asleep: data/sleep.json).
    pub(super) fn rest_tick(&mut self, dt: f32) {
        if self.rest.resting()
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

    /// One frame of a runner on a seat or a bed: it does not move. Seated, interact stands up and so does a move key
    /// pressed after sitting (data/seating.json `stand_on_move`); there is no sleep and no stamina beyond the recovery.
    /// Lying, interact gets up; jump sleeps or wakes; a move key wakes.
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
            let seated = self.rest.sitting();
            self.get_up();
            if seated {
                self.say(&sit_rules().text.stood);
            }
            return;
        }
        if self.rest.sitting() {
            self.rest.dark = 0.0;
            if c.fwd == 0.0 && c.strafe == 0.0 {
                self.rest.move_armed = true;
            } else if self.rest.move_armed && sit_rules().stand_on_move && !stunned {
                self.get_up();
                self.say(&sit_rules().text.stood);
            }
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
        if g.rest.resting() {
            g.update_rest(DT, c);
        }
    }

    /// A run with a single seat 0.8 m in front of the runner (along +z, its front toward the runner), the runner
    /// looking at its pan, and no hunter awake.
    fn seat_game(room: &str, ptype: &str) -> (Game, usize, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, room, 0, &content, &catalog);
        g.ensure_hunters(&content);
        for h in g.hunters.iter_mut() {
            h.active = false;
        }
        let cell = crate::world::Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.p.pos = cell.center();
        let mut seat = bed_prop(ptype, cell, (0.0, 0.8));
        seat.id = "TEST:SEAT".to_string();
        seat.half = (0.4, 0.4);
        g.fac.props.push(seat);
        let i = g.fac.props.len() - 1;
        let (_, k) = sit_seat_of(ptype).unwrap();
        g.p.yaw = std::f32::consts::PI;
        g.p.pitch = (k.height - g.p.eye().y).atan2(0.8);
        g.p.stun = 0.0;
        (g, i, content, catalog)
    }

    #[test]
    fn the_shipped_sit_rules_parse_and_name_the_seats_godot_names() {
        let r = parse_sit(crate::game::seating::TEXT).expect("the sit section parses");
        assert_eq!(&r, sit_rules());
        for (seat, kind) in [
            ("officeChair", "chair"),
            ("woodenChair", "chair"),
            ("waitingChairs", "chair"),
            ("loungeCouch", "couch"),
            ("loungeCouchWide", "couch"),
            ("mallBench", "bench"),
            ("gothicBedroomBench", "bench"),
            ("dinerBooth", "bench"),
            ("vanityStool", "stool"),
        ] {
            assert_eq!(sit_seat_of(seat).map(|s| s.0), Some(kind), "{seat}");
            assert_eq!(sit_prompt(seat), "[E] SIT", "{seat}");
        }
        // the work benches, the beds and plain furniture are no seats
        for other in ["labBench", "pottingBench", "singleBed", "prisonBunk", "officeDesk", "bedsideLamp", "micStand"] {
            assert!(sit_seat_of(other).is_none(), "{other} is not a seat");
            assert_eq!(sit_prompt(other), "");
        }
        // Godot: SIT_EYE_ABOVE_SEAT .83, SIT_YAW_LIMIT 75 degrees, SIT_PITCH -0.95..0.6
        assert!((r.eye_above_seat - 0.83).abs() < 1e-6);
        assert!((r.look_yaw_limit - 75f32.to_radians()).abs() < 1e-4);
        assert!((r.look_pitch_down - 0.95).abs() < 0.01 && (r.look_pitch_up - 0.6).abs() < 0.01);
        assert!(parse_sit("{}").is_err());
        assert!(parse_sit(&crate::game::seating::TEXT.replace("\"eye_above_seat\": 0.83,", "")).is_err(), "the numbers are required data");
    }

    #[test]
    fn a_resting_runner_tells_the_others_where_it_sits_or_lies_and_a_standing_one_says_nothing() {
        let (mut g, i, _, _) = seat_game("ROOM-BED-7", "officeChair");
        assert!(g.rest_wire().is_none(), "on its feet: no rest in the sample");
        let stood = g.p.pos;
        assert!(g.sit_down(i));
        let (_, k) = sit_seat_of("officeChair").unwrap();
        let o = g.window_origin();
        let seat = g.rest_wire().expect("seated");
        assert!(!seat.lying);
        assert!((seat.up - k.height).abs() < 1e-4, "the pan's height above the feet");
        // the anchor is on the chair (0.8 m ahead of where the body still stands), in absolute cells
        let at = (seat.x - o.0 as f32, seat.y - o.1 as f32);
        assert_eq!(g.p.pos, stood, "the body did not move");
        let centre = g.bed_centre(i);
        // rotation 0 faces -z, toward the runner: the anchor is the seat's centre moved by the kind's offset along it
        assert!(pdist(at, (centre.0, centre.1 - k.offset)) < 1e-3, "{at:?} for a seat at {centre:?}");
        assert!(pdist(centre, stood) > 0.5, "and the seat is not where the body stands");
        assert!((seat.yaw - g.p.yaw).abs() < 1e-4, "it faces the way the seat does");
        g.get_up();
        assert!(g.rest_wire().is_none());
        let (mut g, i, _, _) = bed_game("ROOM-BED-7");
        assert!(g.lie_down(i));
        assert!(g.rest_wire().expect("lying").lying);
        // the sample carries it, and an older peer's sample without the key reads as standing
        let sent = crate::net::world_sync::Sample::of(&g.p, g.depth, g.window_origin()).with_rest(g.rest_wire()).to_json();
        let back = crate::net::world_sync::Sample::from_json(&sent).expect("parses");
        assert_eq!(back.rest.map(|r| r.lying), Some(true));
        let mut old = sent.clone();
        old.as_object_mut().unwrap().remove("rest");
        assert_eq!(crate::net::world_sync::Sample::from_json(&old).expect("parses").rest, None);
    }

    #[test]
    fn a_seat_in_reach_offers_to_sit_and_a_seated_runner_is_offered_to_stand() {
        let (mut g, i, content, catalog) = seat_game("ROOM-BED-7", "officeChair");
        assert_eq!(g.find_focus(), Some(Focus::Seat(i)));
        g.focus = Some(Focus::Seat(i));
        assert_eq!(g.focus_prompt().as_deref(), Some("[E] SIT"));
        g.interact(Focus::Seat(i), &content, &catalog, &Sounds::default());
        assert!(g.rest.sitting() && g.rest.resting() && !g.rest.lying() && !g.rest.asleep());
        assert_eq!(g.focus_prompt().as_deref(), Some(sit_rules().text.seated.as_str()));
        // jump does not put a seated runner to sleep
        g.meters.drowsiness = 50.0;
        press_jump(&mut g);
        assert!(g.rest.sitting() && !g.rest.asleep());
        // a seated runner cannot sit or lie on something else
        assert!(!g.sit_down(i) && !g.lie_down(i));
        // interact stands up
        tick(&mut g, &Controls { interact: true, ..Controls::default() });
        assert!(!g.rest.resting());
        assert_eq!(g.message, sit_rules().text.stood);
        // a seat out of the seating reach offers nothing
        g.fac.props[i].offset = (0.0, seating().default_reach + 1.0);
        assert_ne!(g.find_focus(), Some(Focus::Seat(i)));
        assert!(!g.sit_down(i), "out of reach");
    }

    #[test]
    fn a_seated_runner_does_not_move_and_the_eye_goes_to_the_seat() {
        let (mut g, i, content, catalog) = seat_game("ROOM-BED-7", "officeChair");
        let stand = g.eye();
        assert!(g.sit_down(i));
        let at = g.p.pos;
        // the seat faces the runner (its front is -z), so the seated look is back the way the runner came
        assert!(wrap_angle(g.p.yaw).abs() < 1e-4, "yaw {}", g.p.yaw);
        for _ in 0..60 {
            g.step(DT, &Controls::default(), &content, &catalog, &Sounds::default());
        }
        assert!(g.rest.sitting());
        assert_eq!(g.p.pos, at, "the body stays put");
        let eye = g.eye();
        let (_, k) = sit_seat_of("officeChair").unwrap();
        assert!((eye.y - (g.p.height + k.height + sit_rules().eye_above_seat)).abs() < 1e-4, "seated eye height {}", eye.y);
        assert!(eye.y < stand.y);
        assert!(pdist((eye.x, eye.z), g.bed_centre(i)) <= 0.2, "the eye is over the seat");
        // the look is held around the seat's facing
        g.p.yaw += 3.0;
        g.p.pitch = -1.5;
        g.rest_clamp_look();
        assert!(wrap_angle(g.p.yaw - g.rest.base_yaw).abs() <= sit_rules().look_yaw_limit + 1e-4);
        assert!(g.p.pitch >= -sit_rules().look_pitch_down - 1e-4);
        // sitting gives no stamina beyond the normal recovery
        g.p.stamina = 10.0;
        g.p.stim_crash = 0.0;
        for _ in 0..60 {
            tick(&mut g, &Controls::default());
        }
        assert!((g.p.stamina - 10.0 - STAMINA_RECOVERY).abs() < 0.2, "stamina rose {}", g.p.stamina - 10.0);
        // a move key stands the runner up, on the spot it stood on
        g.step(DT, &Controls { fwd: 1.0, ..Controls::default() }, &content, &catalog, &Sounds::default());
        assert!(!g.rest.resting());
        assert_eq!(g.p.pos, at);
        assert_eq!(g.eye(), g.p.eye());
    }

    #[test]
    fn a_move_key_held_while_sitting_down_does_not_stand_the_runner_up_until_it_is_pressed_again() {
        let (mut g, i, _, _) = seat_game("ROOM-BED-7", "mallBench");
        assert!(g.sit_down(i));
        let walk = Controls { fwd: 1.0, ..Controls::default() };
        for _ in 0..30 {
            tick(&mut g, &walk);
        }
        assert!(g.rest.sitting(), "the key that walked the runner here does not stand it up");
        tick(&mut g, &Controls::default());
        tick(&mut g, &walk);
        assert!(!g.rest.resting(), "let go and pressed again, it does");
        assert_eq!(g.message, sit_rules().text.stood);
    }

    #[test]
    fn a_seated_runner_is_not_hidden_and_a_catch_takes_it_off_the_seat() {
        let (mut g, i, _, _) = seat_game("ROOM-IMMUNE-7", "officeChair");
        g.room_teammates = 1;
        assert!(g.sit_down(i));
        assert!(g.p.hidden.is_none(), "sitting is not hiding");
        // a hunter's grab (the escape struggle) ends the rest
        g.escape = Some(Escape { remaining: 1.0, progress: 0, hunter: 0 });
        g.rest_tick(DT);
        assert!(!g.rest.resting());
        g.escape = None;
        // so does being taken down
        assert!(g.sit_down(i));
        assert!(g.go_down(0), "a seated runner is caught like any runner");
        assert!(g.runner_downed());
        g.rest_tick(DT);
        assert!(!g.rest.resting());
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
    fn a_lying_runner_too_awake_to_sleep_is_told_so_and_a_tired_one_is_offered_sleep() {
        let (mut g, i, _content, _catalog) = bed_game("ROOM-BED-7");
        assert!(g.lie_down(i));
        g.meters.drowsiness = 0.0;
        let prompt = g.rest_prompt().expect("the lying prompt");
        assert!(prompt.contains("TOO AWAKE") && !prompt.contains("[SPACE] SLEEP"), "{prompt}");
        press_jump(&mut g);
        assert!(!g.rest.asleep(), "NOT TIRED keeps the runner awake");
        g.meters.drowsiness = sleep_cfg().max;
        let prompt = g.rest_prompt().expect("the lying prompt");
        assert!(prompt.contains("[SPACE] SLEEP"), "{prompt}");
        press_jump(&mut g);
        assert!(g.rest.asleep());
        // the HUD draws this prompt over the sleep fade (ui/hud.rs), so it has to be there while asleep
        assert_eq!(g.rest_prompt().as_deref(), Some(rules().text.asleep.as_str()));
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

    /// A bed the generator placed on a real floor, not one the test put down: from a floor cell a metre from it, facing
    /// its mattress, the bed is the focus and the lying prompt is offered.
    #[test]
    fn a_generated_bed_is_in_focus_from_the_floor_beside_it() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut checked = 0;
        for (seed, room) in [(7u32, "ROOM-BED-7"), (8, "ROOM-BED-8"), (9, "ROOM-BED-9")] {
            let mut g = Game::new_infinite_sim(Difficulty::Standard, seed, room, 0, &content, &catalog);
            g.ensure_hunters(&content);
            for h in g.hunters.iter_mut() {
                h.active = false;
            }
            let beds: Vec<usize> = (0..g.fac.props.len()).filter(|&i| seat_of(&g.fac.props[i].ptype).is_some()).collect();
            for i in beds {
                let (cx, cz) = g.bed_centre(i);
                let (_, k) = seat_of(&g.fac.props[i].ptype).unwrap();
                let spot = (0..8).map(|n| {
                    let a = n as f32 * std::f32::consts::FRAC_PI_4;
                    (cx + a.cos(), cz + a.sin())
                });
                let Some(at) = spot.clone().find(|p| g.fac.is_floor(p.0 as f64, p.1 as f64) && g.fac.has_line_of_sight(*p, (cx, cz))) else {
                    continue;
                };
                g.p.pos = at;
                // the yaw that looks at the bed most directly
                let (dx, dz) = (cx - at.0, cz - at.1);
                let n = (dx * dx + dz * dz).sqrt().max(1e-6);
                let best = (0..64)
                    .map(|s| s as f32 * std::f32::consts::TAU / 64.0)
                    .max_by(|a, b| {
                        g.p.yaw = *a;
                        let fa = g.p.forward2();
                        g.p.yaw = *b;
                        let fb = g.p.forward2();
                        ((fa.0 * dx + fa.1 * dz) / n).total_cmp(&((fb.0 * dx + fb.1 * dz) / n))
                    })
                    .unwrap();
                g.p.yaw = best;
                let horizontal = (dx * dx + dz * dz).sqrt();
                g.p.pitch = (k.height - g.p.eye().y).atan2(horizontal);
                g.p.stun = 0.0;
                g.focus = g.find_focus();
                let ptype = g.fac.props[i].ptype.clone();
                assert_eq!(g.focus, Some(Focus::Bed(i)), "{ptype} (seed {seed}): focus {:?} from {at:?}, bed at ({cx}, {cz})", g.focus);
                assert_eq!(g.focus_prompt().as_deref(), Some("[E] LIE DOWN"), "{ptype}");
                assert!(g.lie_down(i), "{ptype} (seed {seed}): the lie-down is refused from the floor");
                checked += 1;
                g.get_up();
            }
        }
        assert!(checked > 0, "no generated bed to test: the seeds need a bedroom");
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

