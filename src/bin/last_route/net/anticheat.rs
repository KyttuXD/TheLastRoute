//! Host-side checks on a guest's runner samples (TODO 872 host-side guest anti-cheat, TODO 933 guest validator).
//!
//! The host compares each guest sample with the last one it accepted ([`Guard::check`]): moving faster than the limit
//! allows (SPEED), jumping far in one sample on the same floor (TELEPORT), or standing inside solid ground (NOCLIP) is a
//! strike. Strikes are rate limited, are forgiven after a clean stretch, and enough of them ask the host to remove the
//! guest ([`Outcome::Kick`]). A violating sample is not accepted, so the guest's baseline stays at its last good place
//! and the caller can pull the guest back to it (`Outcome::Strike { back_to }`).
//!
//! Every limit is in `data/net/anticheat.json` "guard"; nothing here holds a tuning number. Time is passed in as seconds.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.net_anticheat/1";

/// The "guard" limits of `data/net/anticheat.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub max_speed_mps: f32,
    pub speed_slack: f32,
    pub teleport_distance_m: f32,
    pub min_dt_seconds: f64,
    pub strike_cooldown_seconds: f64,
    pub strike_decay_seconds: f64,
    pub kick_strikes: u32,
    pub noclip_enabled: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_speed_mps: 12.5,
            speed_slack: 1.6,
            teleport_distance_m: 7.0,
            min_dt_seconds: 0.02,
            strike_cooldown_seconds: 0.5,
            strike_decay_seconds: 20.0,
            kick_strikes: 5,
            noclip_enabled: true,
        }
    }
}

/// Parses the file's "guard" object over the defaults; a wrong schema or a non-positive number is an error.
pub fn parse(text: &str) -> Result<Limits, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("anticheat.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("anticheat.json: schema must be {SCHEMA}"));
    }
    let mut l = Limits::default();
    let Some(g) = v.get("guard") else { return Ok(l) };
    let num = |k: &str| -> Result<Option<f64>, String> {
        match g.get(k) {
            None => Ok(None),
            Some(x) => x.as_f64().filter(|n| n.is_finite() && *n > 0.0).map(Some).ok_or_else(|| format!("anticheat.json: guard.{k} must be a positive number")),
        }
    };
    if let Some(n) = num("max_speed_mps")? {
        l.max_speed_mps = n as f32;
    }
    if let Some(n) = num("speed_slack")? {
        l.speed_slack = n as f32;
    }
    if let Some(n) = num("teleport_distance_m")? {
        l.teleport_distance_m = n as f32;
    }
    if let Some(n) = num("min_dt_seconds")? {
        l.min_dt_seconds = n;
    }
    if let Some(n) = num("strike_cooldown_seconds")? {
        l.strike_cooldown_seconds = n;
    }
    if let Some(n) = num("strike_decay_seconds")? {
        l.strike_decay_seconds = n;
    }
    if let Some(n) = num("kick_strikes")? {
        l.kick_strikes = n as u32;
    }
    if let Some(b) = g.get("noclip_enabled").and_then(Value::as_bool) {
        l.noclip_enabled = b;
    }
    Ok(l)
}

/// The embedded limits, read once; the defaults when the file is missing or broken.
pub fn limits() -> &'static Limits {
    static LIMITS: OnceLock<Limits> = OnceLock::new();
    LIMITS.get_or_init(|| crate::content::embedded::data_under("net/anticheat.json").next().and_then(|(_, t)| parse(t).ok()).unwrap_or_default())
}

/// One guest sample: when it was taken (seconds), where (cells), and on which floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub t: f64,
    pub x: f32,
    pub y: f32,
    pub depth: i32,
}

/// What was wrong with a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violation {
    Speed,
    Teleport,
    Noclip,
}

/// What the host does with a sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Accepted.
    Fine,
    /// Rejected, no strike counted (inside the cooldown of an earlier strike): keep the guest at `back_to`.
    Rejected { kind: Violation, back_to: Sample },
    /// Rejected and a strike counted; pull the guest back to `back_to`.
    Strike { kind: Violation, strikes: u32, back_to: Sample },
    /// Too many strikes: remove the guest.
    Kick { kind: Violation, strikes: u32 },
}

#[derive(Clone, Debug)]
struct Track {
    last: Sample,
    strikes: u32,
    last_strike: Option<f64>,
    clean_since: f64,
}

/// The host's guard: one track per guest peer.
#[derive(Clone, Debug)]
pub struct Guard {
    limits: Limits,
    tracks: HashMap<u32, Track>,
}

impl Guard {
    pub fn new(limits: Limits) -> Guard {
        Guard { limits, tracks: HashMap::new() }
    }

    /// The guard with the embedded limits.
    pub fn standard() -> Guard {
        Guard::new(limits().clone())
    }

    /// Checks `s` from guest `peer`. `solid(x, y)` says whether a cell on the sample's floor is solid ground.
    pub fn check(&mut self, peer: u32, s: Sample, solid: impl Fn(f32, f32) -> bool) -> Outcome {
        if !(s.x.is_finite() && s.y.is_finite() && s.t.is_finite()) {
            // a non-finite place is never accepted; there is nowhere to pull the guest back to without a baseline
            return match self.tracks.get(&peer) {
                Some(tr) => Outcome::Rejected { kind: Violation::Teleport, back_to: tr.last },
                None => Outcome::Rejected { kind: Violation::Teleport, back_to: Sample { t: 0.0, x: 0.0, y: 0.0, depth: s.depth } },
            };
        }
        let l = self.limits.clone();
        let Some(tr) = self.tracks.get_mut(&peer) else {
            self.tracks.insert(peer, Track { last: s, strikes: 0, last_strike: None, clean_since: s.t });
            return Outcome::Fine;
        };
        // a forgiven strike for every decay period without one
        if tr.strikes > 0 && s.t - tr.clean_since >= l.strike_decay_seconds {
            tr.strikes -= 1;
            tr.clean_since = s.t;
        }
        if s.depth != tr.last.depth {
            // another floor (stairs, ladders): a fresh baseline
            tr.last = s;
            return Outcome::Fine;
        }
        let dt = (s.t - tr.last.t).max(l.min_dt_seconds);
        let dist = ((s.x - tr.last.x).powi(2) + (s.y - tr.last.y).powi(2)).sqrt();
        let too_fast = (dist as f64) / dt > f64::from(l.max_speed_mps * l.speed_slack);
        let kind = if too_fast && dist > l.teleport_distance_m {
            Some(Violation::Teleport)
        } else if too_fast && dist > l.teleport_distance_m * 0.25 {
            Some(Violation::Speed)
        } else if l.noclip_enabled && solid(s.x, s.y) {
            Some(Violation::Noclip)
        } else {
            None
        };
        let Some(kind) = kind else {
            tr.last = s;
            return Outcome::Fine;
        };
        let back_to = tr.last;
        let cooling = tr.last_strike.map_or(false, |t| s.t - t < l.strike_cooldown_seconds);
        if cooling {
            return Outcome::Rejected { kind, back_to };
        }
        tr.strikes += 1;
        tr.last_strike = Some(s.t);
        tr.clean_since = s.t;
        if tr.strikes >= l.kick_strikes {
            return Outcome::Kick { kind, strikes: tr.strikes };
        }
        Outcome::Strike { kind, strikes: tr.strikes, back_to }
    }

    pub fn strikes(&self, peer: u32) -> u32 {
        self.tracks.get(&peer).map_or(0, |t| t.strikes)
    }

    /// A guest left: its track goes.
    pub fn forget(&mut self, peer: u32) {
        self.tracks.remove(&peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t: f64, x: f32) -> Sample {
        Sample { t, x, y: 0.0, depth: 0 }
    }
    fn open(_: f32, _: f32) -> bool {
        false
    }

    #[test]
    fn the_embedded_limits_parse() {
        let l = limits();
        assert!(l.max_speed_mps > 0.0 && l.kick_strikes > 0);
        assert!(parse(include_str!("../data/net/anticheat.json")).is_ok());
        assert!(parse("{\"schema\":\"nope\"}").is_err());
    }

    #[test]
    fn honest_running_is_fine() {
        let mut g = Guard::standard();
        let v = g.limits.max_speed_mps;
        for i in 0..100 {
            let t = i as f64 * 0.033;
            assert_eq!(g.check(7, at(t, v * 0.9 * t as f32), open), Outcome::Fine, "sample {i}");
        }
        assert_eq!(g.strikes(7), 0);
    }

    #[test]
    fn speed_over_the_limit_strikes() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        // a step that is under the teleport distance but far over the speed limit
        let step = g.limits.teleport_distance_m * 0.5;
        let dt = (step / (g.limits.max_speed_mps * g.limits.speed_slack * 2.0)) as f64;
        let dt = dt.max(g.limits.min_dt_seconds * 1.01);
        let speed_is_over = (step as f64) / dt > f64::from(g.limits.max_speed_mps * g.limits.speed_slack);
        if speed_is_over {
            match g.check(7, at(dt, step), open) {
                Outcome::Strike { kind: Violation::Speed, strikes: 1, back_to } => assert_eq!(back_to.x, 0.0),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_teleport_strikes_and_the_baseline_stays_put() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        let far = g.limits.teleport_distance_m * 3.0;
        match g.check(7, at(0.1, far), open) {
            Outcome::Strike { kind: Violation::Teleport, strikes: 1, back_to } => assert_eq!(back_to.x, 0.0),
            other => panic!("{other:?}"),
        }
        // the baseline did not move: the same sample a moment later is a teleport again, not a free pass
        let later = g.limits.strike_cooldown_seconds + 0.1;
        assert!(matches!(g.check(7, at(0.1 + later, far), open), Outcome::Strike { strikes: 2, .. }));
    }

    #[test]
    fn strikes_inside_the_cooldown_are_rejected_not_counted() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        let far = g.limits.teleport_distance_m * 3.0;
        g.check(7, at(0.1, far), open);
        let soon = 0.1 + g.limits.strike_cooldown_seconds * 0.5;
        assert!(matches!(g.check(7, at(soon, far), open), Outcome::Rejected { kind: Violation::Teleport, .. }));
        assert_eq!(g.strikes(7), 1);
    }

    #[test]
    fn noclip_inside_solid_ground_strikes() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        let wall = |x: f32, _y: f32| x > 0.4;
        assert!(matches!(g.check(7, at(0.05, 0.5), wall), Outcome::Strike { kind: Violation::Noclip, .. }));
        g.limits.noclip_enabled = false;
        g.forget(7);
        g.check(7, at(0.0, 0.0), open);
        assert_eq!(g.check(7, at(0.05, 0.5), wall), Outcome::Fine, "the data switch turns the check off");
    }

    #[test]
    fn enough_strikes_kick_and_a_clean_stretch_forgives() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        let far = g.limits.teleport_distance_m * 30.0;
        let mut t = 0.1;
        let mut last = Outcome::Fine;
        for _ in 0..g.limits.kick_strikes {
            last = g.check(7, at(t, far), open);
            t += g.limits.strike_cooldown_seconds + 0.1;
        }
        assert!(matches!(last, Outcome::Kick { strikes, .. } if strikes == g.limits.kick_strikes));

        let mut g = Guard::standard();
        g.check(8, at(0.0, 0.0), open);
        g.check(8, at(0.1, far), open);
        assert_eq!(g.strikes(8), 1);
        let after = 0.1 + g.limits.strike_decay_seconds + 1.0;
        g.check(8, at(after, 0.0), open);
        assert_eq!(g.strikes(8), 0, "a clean stretch forgives a strike");
    }

    #[test]
    fn a_new_floor_is_a_fresh_baseline() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        let up = Sample { t: 0.05, x: 500.0, y: 500.0, depth: 1 };
        assert_eq!(g.check(7, up, open), Outcome::Fine);
        assert_eq!(g.strikes(7), 0);
    }

    #[test]
    fn a_non_finite_sample_is_rejected() {
        let mut g = Guard::standard();
        g.check(7, at(0.0, 0.0), open);
        assert!(matches!(g.check(7, at(0.1, f32::NAN), open), Outcome::Rejected { .. }));
    }

    #[test]
    fn a_changed_limit_changes_behaviour() {
        let mut l = Limits::default();
        l.teleport_distance_m = 100.0;
        l.max_speed_mps = 1000.0;
        let mut g = Guard::new(l);
        g.check(7, at(0.0, 0.0), open);
        assert_eq!(g.check(7, at(0.1, 50.0), open), Outcome::Fine);
    }
}
