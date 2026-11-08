//! Hard abilities: pounce leap, sprint burst, runner intercept and door slam-through
//! (Godot game/hunters/ai/hard_abilities.gd, tables lines 15-65, logic 90-340). Host-only: only the side that simulates
//! the hunters ticks a `HardAbilities`; a guest reads the visible state (`leaping`, `leap_dir`, `leap_seq`,
//! `burst_active`, `stumble_left`) from the host's frame.
//!
//! Every number is in data/hard_abilities.json (`lr.hard_abilities/1`); the nightmare pounce of the special hunters
//! (Nyxie, Solyth) also lives in data/hunters/nightmare_pounce.json and a test keeps the two equal. The hunter speed
//! model (`speed_multiplier`) reads the `speed` block of data/hunters/pathing.json.
//!
//! This module is behaviour only and never touches a brain: the caller (hunters/brain.rs, game/hunting.rs) feeds it
//! positions, the visible/chasing flags and a random roll, and applies what it returns. Capture and locker rules are
//! NOT changed here (Godot's `blocks_capture` while stumbling is left to the integration, see the hooks).

use crate::config::Difficulty;
use crate::content::embedded;
use crate::hunters::brain::{cell_of, dist};
use crate::world::Cell;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const PATH: &str = "hard_abilities.json";
pub const SCHEMA: &str = "lr.hard_abilities/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ability {
    Pounce,
    Slam,
    Burst,
    Intercept,
}

impl Ability {
    pub const ALL: [Ability; 4] = [Ability::Pounce, Ability::Slam, Ability::Burst, Ability::Intercept];
    fn key(self) -> &'static str {
        match self {
            Ability::Pounce => "pounce",
            Ability::Slam => "slam",
            Ability::Burst => "burst",
            Ability::Intercept => "intercept",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PounceRow {
    pub min: f32,
    pub max: f32,
    /// Chance per eligible second (rolled as rate * dt).
    pub rate: f32,
    pub cooldown: f32,
    pub per_chase: i32,
    pub stumble: f32,
    pub seconds: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurstRow {
    pub chance: f32,
    pub scale: f32,
    pub seconds: f32,
    pub cooldown: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterceptRow {
    pub chance: f32,
    pub lead: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlamRow {
    pub chance: f32,
    pub seconds: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leap {
    pub speed: f32,
    pub lead_seconds: f32,
    pub bonk_stun: f32,
    pub stumble_speed: f32,
    /// A leap never starts within this many cells of a link (ladder, stairs, grate, lift, drop), either end.
    pub link_clearance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gates {
    pub nyxie_requires_dark: bool,
    pub solyth_min_temperature: f32,
    pub void_black_unwatched: f32,
}

#[derive(Clone, Debug)]
pub struct Rules {
    frequency: HashMap<String, f32>,
    budget: HashMap<String, f32>,
    pub runner_guard: f32,
    roster: HashMap<(Ability, String), Vec<String>>,
    pounce: HashMap<String, PounceRow>,
    burst: HashMap<String, BurstRow>,
    intercept: HashMap<String, InterceptRow>,
    slam: HashMap<String, SlamRow>,
    pub leap: Leap,
    pub gates: Gates,
    pub velocity_smoothing: f32,
}

fn num(v: &Value, what: &str) -> Result<f32, String> {
    let n = v.as_f64().ok_or_else(|| format!("{what}: must be a number"))? as f32;
    if !n.is_finite() || n < 0.0 {
        return Err(format!("{what}: must be zero or more"));
    }
    Ok(n)
}

fn rows<T>(v: &Value, key: &str, make: impl Fn(&Value, &str) -> Result<T, String>) -> Result<HashMap<String, T>, String> {
    let mut out = HashMap::new();
    for (mode, row) in v[key].as_object().ok_or_else(|| format!("missing {key}"))? {
        out.insert(mode.clone(), make(row, &format!("{key}.{mode}"))?);
    }
    Ok(out)
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("schema must be {SCHEMA}"));
        }
        let num_map = |key: &str| -> Result<HashMap<String, f32>, String> {
            let mut m = HashMap::new();
            for (k, x) in v[key].as_object().ok_or_else(|| format!("missing {key}"))? {
                m.insert(k.clone(), num(x, &format!("{key}.{k}"))?);
            }
            Ok(m)
        };
        let mut roster = HashMap::new();
        for a in Ability::ALL {
            for (mode, list) in v["roster"][a.key()].as_object().ok_or_else(|| format!("missing roster.{}", a.key()))? {
                let ids = list.as_array().ok_or("roster entries are lists")?;
                roster.insert((a, mode.clone()), ids.iter().filter_map(|x| x.as_str().map(String::from)).collect());
            }
        }
        let leap = &v["leap"];
        let gates = &v["gates"];
        Ok(Rules {
            frequency: num_map("frequency")?,
            budget: num_map("budget")?,
            runner_guard: num(&v["pounce_runner_guard"], "pounce_runner_guard")?,
            roster,
            pounce: rows(&v, "pounce", |r, w| {
                let p = PounceRow {
                    min: num(&r["min"], w)?,
                    max: num(&r["max"], w)?,
                    rate: num(&r["rate"], w)?,
                    cooldown: num(&r["cooldown"], w)?,
                    per_chase: num(&r["per_chase"], w)? as i32,
                    stumble: num(&r["stumble"], w)?,
                    seconds: num(&r["seconds"], w)?,
                };
                if p.min > p.max {
                    return Err(format!("{w}: min is above max"));
                }
                Ok(p)
            })?,
            burst: rows(&v, "burst", |r, w| {
                Ok(BurstRow { chance: num(&r["chance"], w)?, scale: num(&r["scale"], w)?, seconds: num(&r["seconds"], w)?, cooldown: num(&r["cooldown"], w)? })
            })?,
            intercept: rows(&v, "intercept", |r, w| Ok(InterceptRow { chance: num(&r["chance"], w)?, lead: num(&r["lead"], w)? }))?,
            slam: rows(&v, "slam", |r, w| Ok(SlamRow { chance: num(&r["chance"], w)?, seconds: num(&r["seconds"], w)? }))?,
            leap: Leap {
                speed: num(&leap["speed"], "leap.speed")?,
                lead_seconds: num(&leap["lead_seconds"], "leap.lead_seconds")?,
                bonk_stun: num(&leap["bonk_stun"], "leap.bonk_stun")?,
                stumble_speed: num(&leap["stumble_speed"], "leap.stumble_speed")?,
                link_clearance: num(&leap["link_clearance"], "leap.link_clearance")?,
            },
            gates: Gates {
                nyxie_requires_dark: gates["nyxie_requires_dark"].as_bool().unwrap_or(true),
                solyth_min_temperature: num(&gates["solyth_min_temperature"], "gates.solyth_min_temperature")?,
                void_black_unwatched: num(&gates["void_black_unwatched_seconds"], "gates.void_black_unwatched_seconds")?,
            },
            velocity_smoothing: num(&v["velocity_smoothing"], "velocity_smoothing")?,
        })
    }

    /// The Godot FREQUENCY gate: abilities run only while this is above zero (Relaxed is 0).
    pub fn frequency(&self, diff: Difficulty) -> f32 {
        self.frequency.get(diff.id()).copied().unwrap_or(0.0)
    }

    /// Seconds between two uses of one ability anywhere in the party.
    pub fn budget(&self, diff: Difficulty) -> f32 {
        self.budget.get(diff.id()).copied().unwrap_or(f32::INFINITY)
    }

    /// Whether this hunter has the ability on this difficulty.
    pub fn has(&self, ability: Ability, diff: Difficulty, hunter_id: &str) -> bool {
        self.frequency(diff) > 0.0
            && self.roster.get(&(ability, diff.id().to_string())).map_or(false, |l| l.iter().any(|h| h == hunter_id))
    }

    pub fn pounce(&self, diff: Difficulty) -> Option<PounceRow> {
        self.pounce.get(diff.id()).copied()
    }
    pub fn burst(&self, diff: Difficulty) -> Option<BurstRow> {
        self.burst.get(diff.id()).copied()
    }
    pub fn intercept(&self, diff: Difficulty) -> Option<InterceptRow> {
        self.intercept.get(diff.id()).copied()
    }
    pub fn slam(&self, diff: Difficulty) -> Option<SlamRow> {
        self.slam.get(diff.id()).copied()
    }
}

/// The embedded rules, read once. A broken file is reported on stderr and every ability is off.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Rules::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Rules {
                frequency: HashMap::new(),
                budget: HashMap::new(),
                runner_guard: 10.0,
                roster: HashMap::new(),
                pounce: HashMap::new(),
                burst: HashMap::new(),
                intercept: HashMap::new(),
                slam: HashMap::new(),
                leap: Leap { speed: 7.2, lead_seconds: 0.15, bonk_stun: 0.8, stumble_speed: 0.3, link_clearance: 1.5 },
                gates: Gates { nyxie_requires_dark: true, solyth_min_temperature: 60.0, void_black_unwatched: 3.0 },
                velocity_smoothing: 8.0,
            }
        })
    })
}

/// The party budget and the per-runner pounce guard. One per floor, on the host, shared by every hunter's component.
#[derive(Clone, Debug, Default)]
pub struct Budget {
    last: HashMap<Ability, f32>,
    runner: HashMap<i32, f32>,
}

impl Budget {
    /// True when the ability may fire at `now`; with `commit` the use is recorded.
    pub fn allow(&mut self, ability: Ability, now: f32, gap: f32, commit: bool) -> bool {
        if now - self.last.get(&ability).copied().unwrap_or(-1.0e9) < gap {
            return false;
        }
        if commit {
            self.last.insert(ability, now);
        }
        true
    }

    /// One pounce per runner within the guard window, whichever hunter.
    pub fn runner_free(&self, runner: i32, now: f32, guard: f32) -> bool {
        now - self.runner.get(&runner).copied().unwrap_or(-1.0e9) >= guard
    }

    pub fn mark_runner(&mut self, runner: i32, now: f32) {
        self.runner.insert(runner, now);
    }
}

/// What the caller knows about the target and the hunter this frame, for a pounce.
#[derive(Clone, Copy, Debug)]
pub struct PounceCtx {
    pub hunter: (f32, f32),
    pub target: (f32, f32),
    pub runner: i32,
    pub chasing: bool,
    pub visible: bool,
    pub stunned: bool,
    /// The target is hidden, in a safe zone, in a vent, untargetable or on a link.
    pub target_protected: bool,
    /// A link lies within `leap.link_clearance` of the hunter or the target.
    pub near_link: bool,
    /// The straight line to the target is clear of walls.
    pub sweep_clear: bool,
    pub flashlight_on: bool,
    pub temperature: f32,
    pub hunter_in_room: bool,
    /// A fair random draw in 0..1.
    pub roll: f32,
    pub dt: f32,
    pub now: f32,
}

/// One hunter's ability state (Godot HardAbilities, one per brain).
#[derive(Clone, Debug)]
pub struct HardAbilities {
    pub diff: Difficulty,
    pub hunter_id: String,
    rules: &'static Rules,
    pub leaping: bool,
    pub leap_dir: (f32, f32),
    /// Increments on every launch; the view plays the single footfall on it.
    pub leap_seq: u32,
    pub stumble_left: f32,
    pub burst_left: f32,
    leap_left: f32,
    pounce_cooldown: f32,
    pounces_this_chase: i32,
    burst_cooldown: f32,
    was_visible: bool,
    last_target: Option<(f32, f32)>,
    velocity: (f32, f32),
    unwatched: f32,
}

/// How a leap step ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeapEnd {
    Flying,
    /// A wall or prop stopped it: the hunter is stunned this long.
    Bonk(f32),
    /// It ran its course: the hunter stumbles (see `stumble_left`).
    Landed,
}

/// What losing sight did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SightLost {
    /// Where the hunter now heads (ahead of the runner), if the intercept fired.
    pub intercept: Option<(f32, f32)>,
    pub burst: bool,
}

impl HardAbilities {
    /// The component for a hunter on a difficulty, or None when it has no ability there (Relaxed never).
    pub fn new(diff: Difficulty, hunter_id: &str) -> Option<HardAbilities> {
        HardAbilities::with_rules(rules(), diff, hunter_id)
    }

    pub fn with_rules(rules: &'static Rules, diff: Difficulty, hunter_id: &str) -> Option<HardAbilities> {
        if !Ability::ALL.iter().any(|a| rules.has(*a, diff, hunter_id)) {
            return None;
        }
        Some(HardAbilities {
            diff,
            hunter_id: hunter_id.to_string(),
            rules,
            leaping: false,
            leap_dir: (0.0, 0.0),
            leap_seq: 0,
            stumble_left: 0.0,
            burst_left: 0.0,
            leap_left: 0.0,
            pounce_cooldown: 0.0,
            pounces_this_chase: 0,
            burst_cooldown: 0.0,
            was_visible: false,
            last_target: None,
            velocity: (0.0, 0.0),
            unwatched: 0.0,
        })
    }

    pub fn has(&self, a: Ability) -> bool {
        self.rules.has(a, self.diff, &self.hunter_id)
    }

    pub fn burst_active(&self) -> bool {
        self.burst_left > 0.0
    }

    /// The runner's smoothed velocity (cells per second), used to aim ahead of it.
    pub fn target_velocity(&self) -> (f32, f32) {
        self.velocity
    }

    pub fn cancel(&mut self) {
        self.leaping = false;
        self.leap_left = 0.0;
        self.stumble_left = 0.0;
    }

    /// Advance the timers and the runner's velocity estimate. Call once per brain update, before movement.
    pub fn tick(&mut self, dt: f32, target: Option<(f32, f32)>) {
        self.pounce_cooldown = (self.pounce_cooldown - dt).max(0.0);
        self.burst_cooldown = (self.burst_cooldown - dt).max(0.0);
        self.burst_left = (self.burst_left - dt).max(0.0);
        self.stumble_left = (self.stumble_left - dt).max(0.0);
        if let Some(p) = target {
            if let Some(last) = self.last_target {
                if dt > 0.0 {
                    let k = (dt * self.rules.velocity_smoothing).clamp(0.0, 1.0);
                    let raw = ((p.0 - last.0) / dt, (p.1 - last.1) / dt);
                    self.velocity = (self.velocity.0 + (raw.0 - self.velocity.0) * k, self.velocity.1 + (raw.1 - self.velocity.1) * k);
                }
            }
            self.last_target = Some(p);
        }
    }

    /// Speed multiplier from the abilities: the burst scale, times the stumble speed while stumbling.
    pub fn speed_scale(&self) -> f32 {
        let mut s = 1.0;
        if self.burst_left > 0.0 {
            s *= self.rules.burst(self.diff).map_or(1.0, |b| b.scale);
        }
        if self.stumble_left > 0.0 {
            s *= self.rules.leap.stumble_speed;
        }
        s
    }

    // ------------------------------------------------------------------------------------------ pounce

    fn pounce_allowed_for_hunter(&self, c: &PounceCtx) -> bool {
        let g = self.rules.gates;
        match self.hunter_id.as_str() {
            "solyth" => c.temperature >= g.solyth_min_temperature,
            "nyxie" => !g.nyxie_requires_dark || !c.flashlight_on,
            "void_red" => !c.hunter_in_room,
            "void_black" => self.unwatched >= g.void_black_unwatched,
            _ => true,
        }
    }

    /// Tell the component whether the Void Black is being watched (its unwatched clock gates its pounce).
    pub fn note_watched(&mut self, watched: bool, dt: f32) {
        self.unwatched = if watched { 0.0 } else { self.unwatched + dt };
    }

    /// Gate and roll a pounce. Returns true and starts the leap when it fires. Gates: the difficulty row (range,
    /// per-chase count, cooldown), the roster, the hunter's own gate, the runner guard and the party budget.
    pub fn try_pounce(&mut self, c: &PounceCtx, budget: &mut Budget) -> bool {
        if !self.has(Ability::Pounce) {
            return false;
        }
        let Some(row) = self.rules.pounce(self.diff) else { return false };
        if !c.chasing {
            self.pounces_this_chase = 0;
        }
        if self.leaping || self.pounce_cooldown > 0.0 || self.stumble_left > 0.0 || self.pounces_this_chase >= row.per_chase {
            return false;
        }
        if !c.chasing || !c.visible || c.stunned || c.target_protected || c.near_link || !c.sweep_clear {
            return false;
        }
        let d = dist(c.hunter, c.target);
        if d < row.min || d > row.max || !self.pounce_allowed_for_hunter(c) {
            return false;
        }
        if !budget.runner_free(c.runner, c.now, self.rules.runner_guard) {
            return false;
        }
        if c.roll >= row.rate * c.dt {
            return false;
        }
        let gap = self.rules.budget(self.diff);
        if !budget.allow(Ability::Pounce, c.now, gap, true) {
            return false;
        }
        budget.mark_runner(c.runner, c.now);
        let lead = self.rules.leap.lead_seconds;
        let mut dir = (c.target.0 + self.velocity.0 * lead - c.hunter.0, c.target.1 + self.velocity.1 * lead - c.hunter.1);
        if dir.0 * dir.0 + dir.1 * dir.1 < 0.0001 {
            dir = (c.target.0 - c.hunter.0, c.target.1 - c.hunter.1);
        }
        let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(1e-6);
        self.leap_dir = (dir.0 / len, dir.1 / len);
        self.leaping = true;
        self.leap_left = row.seconds;
        self.leap_seq += 1;
        self.pounces_this_chase += 1;
        true
    }

    /// Distance the leap wants to cover this frame (the caller moves with collision and passes what it moved).
    pub fn leap_wanted(&self, dt: f32) -> f32 {
        self.rules.leap.speed * dt.min(self.leap_left).max(0.0)
    }

    /// Finish a leap step. `wanted` is `leap_wanted(dt)`, `moved` what collision allowed.
    pub fn finish_leap_step(&mut self, dt: f32, wanted: f32, moved: f32) -> LeapEnd {
        let Some(row) = self.rules.pounce(self.diff) else {
            self.cancel();
            return LeapEnd::Landed;
        };
        self.leap_left -= dt;
        if wanted > 0.001 && moved < wanted * 0.5 {
            self.leaping = false;
            self.pounce_cooldown = row.cooldown;
            return LeapEnd::Bonk(self.rules.leap.bonk_stun);
        }
        if self.leap_left <= 0.0 {
            self.leaping = false;
            self.pounce_cooldown = row.cooldown;
            self.stumble_left = row.stumble;
            return LeapEnd::Landed;
        }
        LeapEnd::Flying
    }

    // ----------------------------------------------------------------------- sight lost: burst + intercept

    /// Call every frame with whether the hunter is chasing and sees the runner directly. On the frame sight is lost
    /// (visible to not visible while still chasing) the intercept and burst roll; `rolls` are two fair draws in 0..1.
    pub fn update_sight(&mut self, chasing: bool, visible_now: bool, last_known: (f32, f32), rolls: [f32; 2], budget: &mut Budget, now: f32) -> Option<SightLost> {
        let visible = chasing && visible_now;
        let lost = self.was_visible && !visible && chasing;
        self.was_visible = visible;
        if !lost {
            return None;
        }
        let gap = self.rules.budget(self.diff);
        let mut out = SightLost { intercept: None, burst: false };
        if self.has(Ability::Intercept) {
            if let Some(row) = self.rules.intercept(self.diff) {
                if rolls[0] < row.chance && budget.allow(Ability::Intercept, now, gap, true) {
                    out.intercept = Some(intercept_point(last_known, self.velocity, row.lead));
                }
            }
        }
        if self.has(Ability::Burst) && self.burst_cooldown <= 0.0 {
            if let Some(row) = self.rules.burst(self.diff) {
                if rolls[1] < row.chance && budget.allow(Ability::Burst, now, gap, true) {
                    self.burst_left = row.seconds;
                    self.burst_cooldown = row.cooldown;
                    out.burst = true;
                }
            }
        }
        Some(out)
    }

    // --------------------------------------------------------------------------------- door slam-through

    /// Seconds a closed room door stays slammed open when this chasing hunter reaches it; 0 = it opens it normally.
    pub fn door_slam_seconds(&self, chasing: bool, roll: f32, budget: &mut Budget, now: f32) -> f32 {
        if !chasing || !self.has(Ability::Slam) {
            return 0.0;
        }
        let Some(row) = self.rules.slam(self.diff) else { return 0.0 };
        if roll >= row.chance || !budget.allow(Ability::Slam, now, self.rules.budget(self.diff), true) {
            return 0.0;
        }
        row.seconds
    }
}

/// The point ahead of the runner: where they were last known plus their velocity times the lead seconds.
pub fn intercept_point(last_known: (f32, f32), velocity: (f32, f32), lead: f32) -> (f32, f32) {
    (last_known.0 + velocity.0 * lead, last_known.1 + velocity.1 * lead)
}

/// The cell the hunter heads for to cut the runner off.
pub fn intercept_cell(last_known: (f32, f32), velocity: (f32, f32), lead: f32) -> Cell {
    cell_of(intercept_point(last_known, velocity, lead))
}

// ------------------------------------------------------------------------------------------- speed model

/// The `speed` block of data/hunters/pathing.json.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedModel {
    pub ability_scale_max: f32,
    pub default_hunter_scale: f32,
    hunter_scale: HashMap<String, f32>,
}

impl SpeedModel {
    pub fn parse(text: &str) -> Result<SpeedModel, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = &v["speed"];
        let mut hunter_scale = HashMap::new();
        let mut default_hunter_scale = 1.0;
        for (k, x) in s["hunter_scale"].as_object().ok_or("speed.hunter_scale missing")? {
            let n = num(x, &format!("speed.hunter_scale.{k}"))?;
            if k == "default" {
                default_hunter_scale = n;
            } else {
                hunter_scale.insert(k.clone(), n);
            }
        }
        Ok(SpeedModel { ability_scale_max: num(&s["ability_scale_max"], "speed.ability_scale_max")?, default_hunter_scale, hunter_scale })
    }

    /// The multiplier on a hunter's walking speed: its ability scale (burst, stumble) capped, times its own scale.
    pub fn multiplier(&self, hunter_id: &str, ability_scale: f32) -> f32 {
        ability_scale.clamp(0.0, self.ability_scale_max) * self.hunter_scale.get(hunter_id).copied().unwrap_or(self.default_hunter_scale)
    }
}

pub fn speed_model() -> &'static SpeedModel {
    static M: OnceLock<SpeedModel> = OnceLock::new();
    M.get_or_init(|| {
        let text = embedded::data_under("hunters/pathing.json").next().map(|(_, t)| t).unwrap_or("");
        SpeedModel::parse(text).unwrap_or_else(|e| {
            eprintln!("hunters/pathing.json: {e}");
            SpeedModel { ability_scale_max: 2.0, default_hunter_scale: 1.0, hunter_scale: HashMap::new() }
        })
    })
}

/// The speed multiplier for a hunter this frame: `abilities` is its component (None = no hard ability).
pub fn speed_multiplier(hunter_id: &str, abilities: Option<&HardAbilities>) -> f32 {
    speed_model().multiplier(hunter_id, abilities.map_or(1.0, |a| a.speed_scale()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PounceCtx {
        PounceCtx {
            hunter: (10.0, 10.0),
            target: (13.0, 10.0),
            runner: 1,
            chasing: true,
            visible: true,
            stunned: false,
            target_protected: false,
            near_link: false,
            sweep_clear: true,
            flashlight_on: true,
            temperature: 70.0,
            hunter_in_room: false,
            roll: 0.0,
            dt: 1.0,
            now: 1000.0,
        }
    }

    fn comp(diff: Difficulty, id: &str) -> HardAbilities {
        HardAbilities::new(diff, id).expect("has an ability")
    }

    #[test]
    fn the_shipped_file_parses_and_standard_matches_godot() {
        let r = rules();
        assert!(r.has(Ability::Pounce, Difficulty::Nightmare, "nyxie"));
        assert!(!r.has(Ability::Pounce, Difficulty::Standard, "nyxie"));
        let p = r.pounce(Difficulty::Standard).unwrap();
        assert_eq!((p.min, p.max, p.cooldown, p.per_chase), (2.2, 3.5, 120.0, 1));
        assert_eq!(r.frequency(Difficulty::Relaxed), 0.0);
        assert_eq!(r.budget(Difficulty::Nightmare), 25.0);
        assert_eq!(r.slam(Difficulty::Standard).unwrap().chance, 0.10);
        assert_eq!(r.slam(Difficulty::Hard).unwrap().chance, 1.0);
        assert_eq!(r.leap.speed, 7.2);
    }

    /// Nyxie's and Solyth's nightmare pounce file equals the shared table (Godot lines 17, 26, 49-55).
    #[test]
    fn nightmare_pounce_file_matches_the_shared_table() {
        let text = embedded::data_under("hunters/nightmare_pounce.json").next().map(|(_, t)| t).expect("embedded");
        let v: Value = serde_json::from_str(text).unwrap();
        let p = rules().pounce(Difficulty::Nightmare).unwrap();
        let n = &v["numbers"];
        let close = |a: f32, b: &Value| (a as f64 - b.as_f64().unwrap()).abs() < 1e-6;
        assert!(close(p.min, &n["min"]) && close(p.max, &n["max"]) && close(p.rate, &n["rate"]) && close(p.cooldown, &n["cooldown"]));
        assert!(close(p.stumble, &n["stumble"]) && close(p.seconds, &n["seconds"]));
        assert_eq!(p.per_chase as i64, n["per_chase"].as_i64().unwrap());
        let l = rules().leap;
        assert!(close(l.speed, &v["leap"]["leap_speed"]) && close(l.bonk_stun, &v["leap"]["bonk_stun"]));
        assert!(close(l.stumble_speed, &v["leap"]["stumble_speed"]) && close(l.lead_seconds, &v["leap"]["lead_seconds"]));
        assert!(close(rules().budget(Difficulty::Nightmare), &v["budget"]["seconds"]));
        assert!(close(rules().gates.solyth_min_temperature, &v["gates"]["solyth_min_temperature"]));
    }

    #[test]
    fn relaxed_has_no_abilities_and_standard_pounce_is_roster_gated() {
        for id in ["haelith", "nyxie", "elise", "quenelle", "mavrine"] {
            assert!(HardAbilities::new(Difficulty::Relaxed, id).is_none(), "{id}");
        }
        assert!(HardAbilities::new(Difficulty::Standard, "nyxie").is_none());
        assert!(HardAbilities::new(Difficulty::Nightmare, "nyxie").is_some());
    }

    #[test]
    fn pounce_is_gated_by_difficulty_range_and_visibility() {
        let mut b = Budget::default();
        let mut hard = comp(Difficulty::Hard, "haelith");
        assert!(hard.try_pounce(&ctx(), &mut b), "3 cells, visible, roll 0 fires on Hard");
        assert!(hard.leaping && hard.leap_seq == 1);
        assert!((hard.leap_dir.0 - 1.0).abs() < 1e-5, "aims at the target");

        let mut far = ctx();
        far.target = (20.0, 10.0);
        assert!(!comp(Difficulty::Hard, "haelith").try_pounce(&far, &mut Budget::default()), "out of range");
        let mut hidden = ctx();
        hidden.target_protected = true;
        assert!(!comp(Difficulty::Hard, "haelith").try_pounce(&hidden, &mut Budget::default()));
        let mut unseen = ctx();
        unseen.visible = false;
        assert!(!comp(Difficulty::Hard, "haelith").try_pounce(&unseen, &mut Budget::default()));
        let mut roll = ctx();
        roll.roll = 0.99;
        assert!(!comp(Difficulty::Hard, "haelith").try_pounce(&roll, &mut Budget::default()), "roll above rate * dt");

        let mut mid = ctx();
        mid.target = (14.5, 10.0); // 4.5 cells: in range on Hard (max 4.5), out on Standard (max 3.5)
        assert!(comp(Difficulty::Hard, "haelith").try_pounce(&mid, &mut Budget::default()));
        assert!(!comp(Difficulty::Standard, "haelith").try_pounce(&mid, &mut Budget::default()));
    }

    #[test]
    fn hunter_specific_pounce_gates_hold() {
        let lit = ctx();
        assert!(!comp(Difficulty::Nightmare, "nyxie").try_pounce(&lit, &mut Budget::default()), "Nyxie needs the runner's torch off");
        let mut dark = ctx();
        dark.flashlight_on = false;
        assert!(comp(Difficulty::Nightmare, "nyxie").try_pounce(&dark, &mut Budget::default()));
        let mut cold = ctx();
        cold.temperature = 40.0;
        assert!(!comp(Difficulty::Nightmare, "solyth").try_pounce(&cold, &mut Budget::default()));
        assert!(comp(Difficulty::Nightmare, "solyth").try_pounce(&ctx(), &mut Budget::default()));
    }

    #[test]
    fn party_budget_and_runner_guard_limit_pounces() {
        let mut b = Budget::default();
        assert!(comp(Difficulty::Hard, "haelith").try_pounce(&ctx(), &mut b));
        let mut second = ctx();
        second.runner = 2;
        second.now = 1010.0;
        assert!(!comp(Difficulty::Hard, "xivvy").try_pounce(&second, &mut b), "inside the 45 s party budget");
        second.now = 1050.0;
        assert!(comp(Difficulty::Hard, "xivvy").try_pounce(&second, &mut b), "budget over, other runner");
        let mut same = ctx();
        same.now = 2000.0;
        let mut b2 = Budget::default();
        b2.mark_runner(1, same.now - 5.0);
        assert!(!comp(Difficulty::Hard, "haelith").try_pounce(&same, &mut b2), "same runner inside the guard");
    }

    #[test]
    fn leap_bonks_on_a_wall_and_stumbles_on_landing() {
        let mut a = comp(Difficulty::Hard, "haelith");
        assert!(a.try_pounce(&ctx(), &mut Budget::default()));
        let wanted = a.leap_wanted(0.1);
        assert!(wanted > 0.0);
        assert_eq!(a.finish_leap_step(0.1, wanted, 0.0), LeapEnd::Bonk(rules().leap.bonk_stun));
        assert!(!a.leaping);

        let mut a = comp(Difficulty::Hard, "haelith");
        assert!(a.try_pounce(&ctx(), &mut Budget::default()));
        let mut end = LeapEnd::Flying;
        for _ in 0..20 {
            let w = a.leap_wanted(0.1);
            end = a.finish_leap_step(0.1, w, w);
            if end != LeapEnd::Flying {
                break;
            }
        }
        assert_eq!(end, LeapEnd::Landed);
        assert!(a.stumble_left > 0.0);
        assert!((a.speed_scale() - rules().leap.stumble_speed).abs() < 1e-6, "slowed while stumbling");
        a.tick(5.0, None);
        assert_eq!(a.stumble_left, 0.0);
    }

    #[test]
    fn burst_starts_on_sight_lost_and_expires() {
        let mut a = comp(Difficulty::Nightmare, "quenelle");
        let mut b = Budget::default();
        assert!(a.update_sight(true, true, (0.0, 0.0), [0.0, 0.0], &mut b, 100.0).is_none(), "still in sight");
        let lost = a.update_sight(true, false, (0.0, 0.0), [0.0, 0.0], &mut b, 101.0).expect("sight lost");
        assert!(lost.burst && a.burst_active());
        assert!((a.speed_scale() - 1.40).abs() < 1e-6);
        let secs = rules().burst(Difficulty::Nightmare).unwrap().seconds;
        a.tick(secs + 0.1, None);
        assert!(!a.burst_active(), "burst timer expired");
        assert_eq!(a.speed_scale(), 1.0);
    }

    #[test]
    fn burst_is_gated_by_chance_and_roster() {
        let mut a = comp(Difficulty::Hard, "quenelle");
        a.update_sight(true, true, (0.0, 0.0), [0.0, 0.0], &mut Budget::default(), 0.0);
        let r = a.update_sight(true, false, (0.0, 0.0), [1.0, 0.99], &mut Budget::default(), 1.0).unwrap();
        assert!(!r.burst && r.intercept.is_none(), "rolls at or above the Hard intercept chance 1.0 and burst chance 0.6");
        let mut other = comp(Difficulty::Hard, "elise");
        other.update_sight(true, true, (0.0, 0.0), [0.0; 2], &mut Budget::default(), 0.0);
        let r = other.update_sight(true, false, (0.0, 0.0), [0.0; 2], &mut Budget::default(), 1.0).unwrap();
        assert!(!r.burst && r.intercept.is_none(), "Elise has neither");
    }

    #[test]
    fn intercept_cell_lies_ahead_of_the_runner() {
        let mut a = comp(Difficulty::Nightmare, "mavrine");
        // the runner moves +x at 4 cells per second
        for i in 0..40 {
            a.tick(0.1, Some((10.0 + i as f32 * 0.4, 10.0)));
        }
        let v = a.target_velocity();
        assert!(v.0 > 3.0 && v.1.abs() < 0.1, "velocity estimate {v:?}");
        a.update_sight(true, true, (20.0, 10.0), [0.0; 2], &mut Budget::default(), 0.0);
        let lost = a.update_sight(true, false, (20.0, 10.0), [0.0, 0.0], &mut Budget::default(), 1.0).unwrap();
        let ahead = lost.intercept.expect("fires on Nightmare");
        assert!(ahead.0 > 23.0, "well ahead along the runner's heading: {ahead:?}");
        assert!(intercept_cell((20.0, 10.0), v, 1.8).x > 20);
        assert_eq!(intercept_cell((20.0, 10.0), (0.0, 0.0), 1.8), cell_of((20.0, 10.0)), "a still runner is not led");
    }

    #[test]
    fn door_slam_is_gated_by_difficulty_and_chance() {
        let hard = comp(Difficulty::Hard, "elise");
        assert_eq!(hard.door_slam_seconds(true, 0.5, &mut Budget::default(), 100.0), 20.0);
        assert_eq!(hard.door_slam_seconds(false, 0.0, &mut Budget::default(), 100.0), 0.0, "only while chasing");
        let std = comp(Difficulty::Standard, "elise");
        assert_eq!(std.door_slam_seconds(true, 0.05, &mut Budget::default(), 100.0), 8.0, "10 percent on Standard");
        assert_eq!(std.door_slam_seconds(true, 0.5, &mut Budget::default(), 100.0), 0.0);
        let mut shared = Budget::default();
        assert_eq!(hard.door_slam_seconds(true, 0.0, &mut shared, 100.0), 20.0);
        assert_eq!(hard.door_slam_seconds(true, 0.0, &mut shared, 110.0), 0.0, "party budget");
    }

    #[test]
    fn speed_multipliers_apply() {
        let m = speed_model();
        assert_eq!(m.multiplier("anyone", 1.0), 1.0);
        assert!((m.multiplier("anyone", 1.25) - 1.25).abs() < 1e-6);
        assert_eq!(m.multiplier("anyone", 99.0), m.ability_scale_max);
        let mut a = comp(Difficulty::Nightmare, "ferryn");
        a.update_sight(true, true, (0.0, 0.0), [0.0; 2], &mut Budget::default(), 0.0);
        a.update_sight(true, false, (0.0, 0.0), [0.0; 2], &mut Budget::default(), 1.0);
        assert!(speed_multiplier("ferryn", Some(&a)) > 1.3);
        assert_eq!(speed_multiplier("ferryn", None), 1.0);
        let custom = SpeedModel::parse(r#"{"speed":{"ability_scale_max":1.5,"hunter_scale":{"default":1.0,"slowpoke":0.5}}}"#).unwrap();
        assert_eq!(custom.multiplier("slowpoke", 1.0), 0.5);
        assert_eq!(custom.multiplier("slowpoke", 4.0), 0.75);
    }

    #[test]
    fn bad_data_is_refused() {
        assert!(Rules::parse(r#"{"schema":"x"}"#).is_err());
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap();
        assert!(Rules::parse(&text.replace("\"min\": 2.2, \"max\": 3.5", "\"min\": 9.0, \"max\": 3.5")).is_err());
    }

    #[test]
    fn roster_hunters_exist_in_the_content() {
        let c = crate::content::Content::load();
        let ids: Vec<&str> = c.hunters.iter().map(|h| h.id.as_str()).collect();
        for ((ab, mode), list) in &rules().roster {
            for h in list {
                assert!(ids.contains(&h.as_str()), "{h} on the {mode} {} roster is not a hunter", ab.key());
            }
        }
    }
}
