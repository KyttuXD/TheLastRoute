//! Anti-AFK void pressure (port of anti_idle.gd; TODO "Anti-AFK void pressure"). A party that stands still builds
//! pressure; past a threshold a void hunter is steered to it, and at the top one is spawned near it. Numbers are data
//! (`data/anti_idle.json`, `lr.anti_idle/1`). Only the host simulates hunters, so only the host's [`AntiIdle`] ever
//! changes: a guest passes `is_host = false` and gets [`Action::None`] with no state change.
//!
//! Hooks (the integrate agent): `mod anti_idle;` in game/mod.rs; one [`AntiIdle`] per party in the host's hunting
//! step; feed it the party's fastest member speed, the void hunters alive, and act on the returned [`Action`] (spawn a
//! void hunter `spawn_distance` from the party, or point an existing one at it).

use crate::config::Difficulty;
use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.anti_idle/1";
const TEXT: &str = include_str!("../data/anti_idle.json");

#[derive(Clone, Debug, PartialEq)]
pub struct Cfg {
    pub idle_speed: f32,
    /// Seconds of standing still before pressure rises: relaxed, standard, hard, nightmare.
    pub grace: [f32; 4],
    pub rise_per_s: f32,
    pub fall_per_s: f32,
    pub steer_pressure: f32,
    pub spawn_pressure: f32,
    pub spawn_cooldown_s: f32,
    pub max_void_hunters: usize,
    pub spawn_distance: f32,
}

fn n(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as f32)
        .ok_or_else(|| format!("anti_idle.json: '{key}' missing or not a number of at least 0"))
}

pub fn parse(text: &str) -> Result<Cfg, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("anti_idle.json: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("anti_idle.json: 'schema' must be {SCHEMA}"));
    }
    let g = &v["grace_seconds"];
    let c = Cfg {
        idle_speed: n(&v, "idle_speed")?,
        grace: [n(g, "relaxed")?, n(g, "standard")?, n(g, "hard")?, n(g, "nightmare")?],
        rise_per_s: n(&v, "rise_per_s")?,
        fall_per_s: n(&v, "fall_per_s")?,
        steer_pressure: n(&v, "steer_pressure")?,
        spawn_pressure: n(&v, "spawn_pressure")?,
        spawn_cooldown_s: n(&v, "spawn_cooldown_s")?,
        max_void_hunters: n(&v, "max_void_hunters")? as usize,
        spawn_distance: n(&v, "spawn_distance")?,
    };
    if c.steer_pressure > c.spawn_pressure || c.spawn_pressure > 1.0 {
        return Err("anti_idle.json: steer_pressure <= spawn_pressure <= 1 is required".into());
    }
    Ok(c)
}

pub fn cfg() -> &'static Cfg {
    static C: OnceLock<Cfg> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

impl Cfg {
    pub fn grace_for(&self, d: Difficulty) -> f32 {
        match d {
            Difficulty::Relaxed => self.grace[0],
            Difficulty::Standard => self.grace[1],
            Difficulty::Hard => self.grace[2],
            Difficulty::Nightmare => self.grace[3],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Point an existing void hunter at the party.
    Steer,
    /// Spawn a void hunter near the party.
    Spawn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AntiIdle {
    idle_s: f32,
    pressure: f32,
    since_spawn: f32,
}

impl Default for AntiIdle {
    fn default() -> Self {
        AntiIdle { idle_s: 0.0, pressure: 0.0, since_spawn: f32::MAX }
    }
}

impl AntiIdle {
    pub fn pressure(&self) -> f32 {
        self.pressure
    }

    pub fn idle_seconds(&self) -> f32 {
        self.idle_s
    }

    /// One host step. `party_speed` is the fastest member speed (m/s); `void_hunters` how many void hunters live.
    pub fn step(&mut self, dt: f32, is_host: bool, party_speed: f32, void_hunters: usize, d: Difficulty, c: &Cfg) -> Action {
        if !is_host {
            return Action::None;
        }
        let dt = dt.max(0.0);
        self.since_spawn = (self.since_spawn + dt).min(1.0e9);
        if party_speed > c.idle_speed {
            self.idle_s = 0.0;
            self.pressure = (self.pressure - c.fall_per_s * dt).max(0.0);
            return Action::None;
        }
        self.idle_s += dt;
        if self.idle_s > c.grace_for(d) {
            self.pressure = (self.pressure + c.rise_per_s * dt).min(1.0);
        }
        if self.pressure >= c.spawn_pressure && void_hunters < c.max_void_hunters && self.since_spawn >= c.spawn_cooldown_s {
            self.since_spawn = 0.0;
            return Action::Spawn;
        }
        if self.pressure >= c.steer_pressure && void_hunters > 0 {
            return Action::Steer;
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(a: &mut AntiIdle, seconds: f32, host: bool, speed: f32) -> Action {
        let mut last = Action::None;
        for _ in 0..(seconds * 10.0) as i32 {
            let r = a.step(0.1, host, speed, 0, Difficulty::Standard, cfg());
            if r != Action::None {
                last = r;
            }
        }
        last
    }

    #[test]
    fn shipped_data_parses_and_harder_means_shorter_grace() {
        assert_eq!(&parse(TEXT).unwrap(), cfg());
        assert!(cfg().grace_for(Difficulty::Nightmare) < cfg().grace_for(Difficulty::Relaxed));
        assert!(parse("{}").is_err());
    }

    #[test]
    fn pressure_rises_only_after_the_grace_while_idle() {
        let mut a = AntiIdle::default();
        run(&mut a, cfg().grace_for(Difficulty::Standard) - 5.0, true, 0.0);
        assert_eq!(a.pressure(), 0.0, "inside the grace nothing rises");
        run(&mut a, 20.0, true, 0.0);
        assert!(a.pressure() > 0.0);
    }

    #[test]
    fn moving_resets_idle_and_drains_pressure() {
        let mut a = AntiIdle::default();
        run(&mut a, cfg().grace_for(Difficulty::Standard) + 20.0, true, 0.0);
        let p = a.pressure();
        assert!(p > 0.0);
        run(&mut a, 5.0, true, 3.0);
        assert!(a.pressure() < p);
        assert_eq!(a.idle_seconds(), 0.0);
    }

    #[test]
    fn a_guest_never_changes_state_or_acts() {
        let mut a = AntiIdle::default();
        assert_eq!(run(&mut a, 600.0, false, 0.0), Action::None);
        assert_eq!(a, AntiIdle::default());
    }

    #[test]
    fn long_idle_spawns_once_then_steers_and_a_data_change_matters() {
        let mut a = AntiIdle::default();
        let g = cfg().grace_for(Difficulty::Standard);
        let need = g + cfg().spawn_pressure / cfg().rise_per_s + 5.0;
        assert_eq!(run(&mut a, need, true, 0.0), Action::Spawn);
        // with a void hunter alive it steers instead of spawning more
        let r = a.step(0.1, true, 0.0, 1, Difficulty::Standard, cfg());
        assert_eq!(r, Action::Steer);
        let fast = parse(&TEXT.replace("\"rise_per_s\": 0.02", "\"rise_per_s\": 0.5")).unwrap();
        let mut b = AntiIdle::default();
        for _ in 0..((g + 5.0) * 10.0) as i32 {
            b.step(0.1, true, 0.0, 0, Difficulty::Standard, &fast);
        }
        assert!(b.pressure() >= fast.spawn_pressure);
    }
}
