//! The startup gate and the update check around it (TODO 912): the splash stays up for a minimum time, the update check runs
//! once and is retried a few times when it fails, and the game moves on when both are done. All numbers are
//! `data/startup.json` (`lr.startup/1`); this file holds the behaviour. [`Gate`] is a pure state machine (no clock, no
//! threads): the caller feeds it frame time and reports each check's result.
//!
//! `data()` also carries the update badge and account panel texts (read by `ui/update_badge.rs` and `ui/account_panel.rs`).

use crate::net::api::{self, UpdateManifest};
use crate::net::NetError;
use serde_json::Value;
use std::sync::OnceLock;

#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub const SCHEMA: &str = "lr.startup/1";
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
const TEXT: &str = include_str!("../data/startup.json");

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub struct Rules {
    pub min_splash_s: f32,
    /// How many times the check is tried before the game gives up and starts anyway.
    pub attempts: u32,
    pub retry_delay_s: f32,
    pub timeout_s: f32,
    /// TODO 1023: a verified admin goes past the hub.
    pub admin_skip_hub: bool,
}

#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("startup.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("startup.json: key 'schema' must be {SCHEMA}"));
    }
    let check = v.get("update_check").ok_or("startup.json: key 'update_check' missing")?;
    let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_f64).ok_or_else(|| format!("startup.json: key '{k}' missing or not a number"));
    let rules = Rules {
        min_splash_s: num(&v, "min_splash_s")? as f32,
        attempts: num(check, "attempts")? as u32,
        retry_delay_s: num(check, "retry_delay_s")? as f32,
        timeout_s: num(check, "timeout_s")? as f32,
        admin_skip_hub: v.get("admin_skip_hub").and_then(Value::as_bool).unwrap_or(false),
    };
    if rules.min_splash_s < 0.0 || rules.retry_delay_s < 0.0 || rules.timeout_s <= 0.0 {
        return Err("startup.json: times must not be negative and timeout_s must be above 0".into());
    }
    if rules.attempts < 1 {
        return Err("startup.json: key 'attempts' must be at least 1".into());
    }
    Ok(rules)
}

#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// The whole file, for the badge and panel texts.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn data() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(TEXT).unwrap_or_else(|e| panic!("startup.json: {e}")))
}

/// TODO 1023: whether the hub is skipped for this account (the flag is data, the admin bit comes from the server's answer).
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn skip_hub(reply: &api::AccountReply) -> bool {
    rules().admin_skip_hub && reply.grants_admin_ui()
}

/// One update check on the worker thread (the manifest request; the updater's own timeout applies).
#[allow(dead_code)] // not wired into the game yet
pub fn check_once(platform: &str) -> Result<UpdateManifest, NetError> {
    api::update_manifest(platform)
}

/// What the caller must do after a [`Gate::tick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub enum Action {
    Nothing,
    /// Start an update check now, then call [`Gate::report`] with how it ended.
    StartCheck,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
enum State {
    Idle,
    InFlight,
    Waiting(f32),
    /// `true`: a check answered. `false`: every attempt failed.
    Finished(bool),
}

#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub struct Gate {
    rules: Rules,
    elapsed: f32,
    started: u32,
    state: State,
}

impl Gate {
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn new(rules: Rules) -> Gate {
        Gate { rules, elapsed: 0.0, started: 0, state: State::Idle }
    }

    /// Advances the splash clock and the retry wait.
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn tick(&mut self, dt: f32) -> Action {
        let dt = dt.max(0.0);
        self.elapsed += dt;
        match self.state {
            State::Idle => {
                self.state = State::InFlight;
                self.started += 1;
                Action::StartCheck
            }
            State::Waiting(left) => {
                let left = left - dt;
                if left <= 0.0 {
                    self.state = State::InFlight;
                    self.started += 1;
                    Action::StartCheck
                } else {
                    self.state = State::Waiting(left);
                    Action::Nothing
                }
            }
            _ => Action::Nothing,
        }
    }

    /// The result of the check [`Action::StartCheck`] asked for.
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn report(&mut self, answered: bool) {
        if self.state != State::InFlight {
            return;
        }
        self.state = if answered {
            State::Finished(true)
        } else if self.started >= self.rules.attempts {
            State::Finished(false)
        } else {
            State::Waiting(self.rules.retry_delay_s)
        };
    }

    /// The check is over (answered, or out of attempts).
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn check_done(&self) -> bool {
        matches!(self.state, State::Finished(_))
    }

    /// The check answered (false while pending and after giving up).
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn check_answered(&self) -> bool {
        self.state == State::Finished(true)
    }

    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn attempts_started(&self) -> u32 {
        self.started
    }

    /// The splash has been up long enough AND the check is over: the game may start.
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn ready(&self) -> bool {
        self.elapsed >= self.rules.min_splash_s && self.check_done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(min: f32, attempts: u32, delay: f32) -> Rules {
        Rules { min_splash_s: min, attempts, retry_delay_s: delay, timeout_s: 5.0, admin_skip_hub: true }
    }

    #[test]
    fn data_file_parses_and_has_sane_values() {
        let r = parse(TEXT).unwrap();
        assert!(r.min_splash_s > 0.0 && r.attempts >= 1 && r.timeout_s > 0.0);
        assert!(data().get("update_badge").is_some() && data().get("account_panel").is_some());
    }

    #[test]
    fn the_splash_holds_for_its_minimum_time_even_when_the_check_is_instant() {
        let mut g = Gate::new(rules(2.0, 3, 1.0));
        assert_eq!(g.tick(0.1), Action::StartCheck);
        g.report(true);
        assert!(g.check_done() && !g.ready(), "answered early but the splash is still showing");
        g.tick(1.0);
        assert!(!g.ready());
        g.tick(1.0);
        assert!(g.ready());
    }

    #[test]
    fn a_failed_check_retries_after_the_delay_then_succeeds() {
        let mut g = Gate::new(rules(0.0, 3, 2.0));
        assert_eq!(g.tick(0.0), Action::StartCheck);
        g.report(false);
        assert!(!g.check_done());
        assert_eq!(g.tick(1.0), Action::Nothing, "still waiting");
        assert_eq!(g.tick(1.5), Action::StartCheck, "the delay is over");
        g.report(true);
        assert!(g.check_answered() && g.ready());
        assert_eq!(g.attempts_started(), 2);
    }

    #[test]
    fn it_gives_up_after_the_attempts_and_the_game_still_starts() {
        let mut g = Gate::new(rules(0.0, 2, 0.5));
        g.tick(0.0);
        g.report(false);
        g.tick(0.6);
        g.report(false);
        assert!(g.check_done() && !g.check_answered());
        assert!(g.ready());
        assert_eq!(g.tick(5.0), Action::Nothing, "no third attempt");
        assert_eq!(g.attempts_started(), 2);
    }

    #[test]
    fn hub_skip_needs_the_flag_and_the_server_admin_bit() {
        let reply = |admin: bool| api::AccountReply { session_token: "t".into(), admin, ..Default::default() };
        assert!(skip_hub(&reply(true)));
        assert!(!skip_hub(&reply(false)));
    }
}
