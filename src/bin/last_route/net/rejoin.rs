//! Reconnect within a reserved window (TODO 194) and the room-closed notice (TODO 195, client part).
//!
//! A guest whose room link drops keeps the last room code and its peer token ([`Rejoin::remember`]). While the window of
//! `data/net/rejoin.json` is open ([`Rejoin::dropped`]) the app asks every `retry_seconds` ([`Rejoin::due`]) to join the
//! same room again with [`Rejoin::attempt`]; after `window_seconds` or `max_tries` it gives up. A drop whose reason says
//! the host left, the room closed or the runner was removed never opens a window, and the notice for those reads
//! `HOST LEFT - ROOM CLOSED` ([`closed_notice`]).
//!
//! Time is passed in as seconds (`now`), so the rules are tested without a clock. The token is a secret: it is never
//! printed, and `Debug` hides it.

use serde_json::Value;
use std::fmt;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.net_rejoin/1";

/// The parsed `data/net/rejoin.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub window_seconds: f64,
    pub retry_seconds: f64,
    pub max_tries: u32,
    pub no_rejoin_reasons: Vec<String>,
    pub host_left_reasons: Vec<String>,
    pub wait_for_world_seconds: f64,
    pub refresh_min_seconds: f64,
    strings: Value,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            window_seconds: 45.0,
            retry_seconds: 2.0,
            max_tries: 20,
            no_rejoin_reasons: ["REMOVED", "HOST LEFT", "ROOM CLOSED"].iter().map(|s| s.to_string()).collect(),
            host_left_reasons: ["HOST LEFT", "ROOM CLOSED"].iter().map(|s| s.to_string()).collect(),
            wait_for_world_seconds: 20.0,
            refresh_min_seconds: 2.0,
            strings: Value::Null,
        }
    }
}

fn list(v: &Value, key: &str, fallback: Vec<String>) -> Vec<String> {
    match v.get(key).and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(|s| s.to_uppercase()).collect(),
        None => fallback,
    }
}

/// Parses the file; a missing key keeps its default, a wrong schema or a non-positive time is an error.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("rejoin.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("rejoin.json: schema must be {SCHEMA}"));
    }
    let d = Rules::default();
    let num = |k: &str, fallback: f64| -> Result<f64, String> {
        match v.get(k) {
            None => Ok(fallback),
            Some(x) => x.as_f64().filter(|n| n.is_finite() && *n > 0.0).ok_or_else(|| format!("rejoin.json: {k} must be a positive number")),
        }
    };
    Ok(Rules {
        window_seconds: num("window_seconds", d.window_seconds)?,
        retry_seconds: num("retry_seconds", d.retry_seconds)?,
        max_tries: v.get("max_tries").and_then(Value::as_u64).map(|n| n.min(1000) as u32).unwrap_or(d.max_tries),
        no_rejoin_reasons: list(&v, "no_rejoin_reasons", d.no_rejoin_reasons),
        host_left_reasons: list(&v, "host_left_reasons", d.host_left_reasons),
        wait_for_world_seconds: num("wait_for_world_seconds", d.wait_for_world_seconds)?,
        refresh_min_seconds: num("refresh_min_seconds", d.refresh_min_seconds)?,
        strings: v.get("strings").cloned().unwrap_or(Value::Null),
    })
}

/// The embedded rules, read once; the defaults when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("net/rejoin.json").next().and_then(|(_, text)| parse(text).ok()).unwrap_or_default()
    })
}

/// A player-facing line of the file with its `{name}` holes filled, upper case; `fallback` when the key is missing.
pub fn text(rules: &Rules, key: &str, fallback: &str, fill: &[(&str, &str)]) -> String {
    let mut line = rules.strings.get(key).and_then(Value::as_str).unwrap_or(fallback).to_string();
    for (name, value) in fill {
        line = line.replace(&format!("{{{name}}}"), value);
    }
    line.to_uppercase()
}

/// [`text`] with the embedded rules.
pub fn line(key: &str, fallback: &str, fill: &[(&str, &str)]) -> String {
    text(rules(), key, fallback, fill)
}

fn matches_any(reason: &str, list: &[String]) -> bool {
    let up = reason.to_uppercase();
    list.iter().any(|r| !r.is_empty() && up.contains(r.as_str()))
}

/// True when a link that ended for `reason` may be rejoined.
pub fn may_rejoin(rules: &Rules, reason: &str) -> bool {
    !matches_any(reason, &rules.no_rejoin_reasons)
}

/// The notice a guest sees when its link ended: `HOST LEFT - ROOM CLOSED` when the host left or the room closed (the
/// server's close reason), else the reason itself.
pub fn closed_notice(rules: &Rules, reason: &str) -> String {
    if matches_any(reason, &rules.host_left_reasons) {
        text(rules, "host_left", "HOST LEFT - ROOM CLOSED", &[])
    } else {
        reason.to_uppercase()
    }
}

/// What a rejoin attempt needs: the room and the token that proves this runner was in it.
#[derive(Clone, PartialEq, Eq)]
pub struct Attempt {
    pub code: String,
    pub token: String,
}

impl fmt::Debug for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attempt").field("code", &self.code).field("token", &"<hidden>").finish()
    }
}

/// The reconnect state of one guest.
#[derive(Clone, Debug)]
pub struct Rejoin {
    rules: Rules,
    last: Option<Attempt>,
    dropped_at: Option<f64>,
    last_try: Option<f64>,
    tries: u32,
}

impl Rejoin {
    pub fn new(rules: Rules) -> Rejoin {
        Rejoin { rules, last: None, dropped_at: None, last_try: None, tries: 0 }
    }

    /// The guest is linked (or linked again): keep this room and token, close any open window.
    pub fn remember(&mut self, code: &str, token: &str) {
        self.last = Some(Attempt { code: code.trim().to_uppercase(), token: token.to_string() });
        self.dropped_at = None;
        self.last_try = None;
        self.tries = 0;
    }

    /// The link ended for `reason` at `now`. True when a window opened (the runner may rejoin); false when the reason
    /// forbids it or nothing was remembered, and then the memory is cleared.
    pub fn dropped(&mut self, reason: &str, now: f64) -> bool {
        if self.last.is_none() || !may_rejoin(&self.rules, reason) {
            self.forget();
            return false;
        }
        if self.dropped_at.is_none() {
            self.dropped_at = Some(now);
            self.last_try = None;
            self.tries = 0;
        }
        true
    }

    /// Seconds left in the window; None when no window is open.
    pub fn seconds_left(&self, now: f64) -> Option<f64> {
        self.dropped_at.map(|t| (self.rules.window_seconds - (now - t)).max(0.0))
    }

    /// True while a window is open and neither the time nor the tries are used up.
    pub fn within_window(&self, now: f64) -> bool {
        self.dropped_at.is_some() && self.seconds_left(now).map_or(false, |s| s > 0.0) && self.tries < self.rules.max_tries
    }

    /// True while a window is open, even when its time or tries are used up (the last try may still be joining).
    pub fn awaiting(&self) -> bool {
        self.dropped_at.is_some()
    }

    /// True when a window was open and has run out.
    pub fn expired(&self, now: f64) -> bool {
        self.dropped_at.is_some() && !self.within_window(now)
    }

    /// True when it is time to try again: inside the window and `retry_seconds` after the last try (the first at once).
    pub fn due(&self, now: f64) -> bool {
        self.within_window(now) && self.last_try.map_or(true, |t| now - t >= self.rules.retry_seconds)
    }

    /// Records that a try started at `now`.
    pub fn tried(&mut self, now: f64) {
        self.last_try = Some(now);
        self.tries += 1;
    }

    /// The room and token to join with, while the window is open.
    pub fn attempt(&self, now: f64) -> Option<&Attempt> {
        if self.within_window(now) {
            self.last.as_ref()
        } else {
            None
        }
    }

    /// The line shown while trying: RECONNECTING TO ROOM X // N S LEFT; None outside a window.
    pub fn status_line(&self, now: f64) -> Option<String> {
        let left = self.seconds_left(now)?;
        let code = self.last.as_ref().map(|a| a.code.as_str()).unwrap_or("");
        if left <= 0.0 {
            return Some(text(&self.rules, "reconnect_failed", "COULD NOT REJOIN ROOM {code}", &[("code", code)]));
        }
        let secs = format!("{}", left.ceil() as u64);
        Some(text(&self.rules, "reconnecting", "RECONNECTING TO ROOM {code} // {seconds}S LEFT", &[("code", code), ("seconds", &secs)]))
    }

    /// Leaves the room for good (the player quit, was removed, or the window ran out).
    pub fn forget(&mut self) {
        self.last = None;
        self.dropped_at = None;
        self.last_try = None;
        self.tries = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rj() -> Rejoin {
        Rejoin::new(rules().clone())
    }

    #[test]
    fn the_embedded_file_parses_and_has_a_window() {
        let r = rules();
        assert!(r.window_seconds > 0.0 && r.retry_seconds > 0.0 && r.max_tries > 0);
        assert!(parse(include_str!("../data/net/rejoin.json")).is_ok());
        assert!(parse("{\"schema\":\"x\"}").is_err());
        assert!(parse(&format!("{{\"schema\":\"{SCHEMA}\",\"window_seconds\":-1}}")).is_err());
    }

    #[test]
    fn a_dropped_guest_rejoins_within_the_window_and_not_after() {
        let mut j = rj();
        let w = j.rules.window_seconds;
        j.remember("abcde", "TOKEN");
        assert!(j.dropped("LOST THE CONNECTION TO THE ROOM", 100.0));
        assert!(j.within_window(100.0 + w * 0.5));
        let a = j.attempt(100.0 + w * 0.5).expect("an attempt inside the window");
        assert_eq!((a.code.as_str(), a.token.as_str()), ("ABCDE", "TOKEN"));
        assert!(!j.within_window(100.0 + w + 1.0));
        assert!(j.attempt(100.0 + w + 1.0).is_none());
        assert!(j.expired(100.0 + w + 1.0));
    }

    #[test]
    fn tries_are_spaced_and_capped() {
        let mut j = rj();
        j.remember("ABCDE", "T");
        j.dropped("LOST THE CONNECTION", 0.0);
        assert!(j.due(0.0), "the first try is at once");
        j.tried(0.0);
        assert!(!j.due(j.rules.retry_seconds * 0.5));
        assert!(j.due(j.rules.retry_seconds + 0.01));
        j.rules.max_tries = 1;
        assert!(!j.due(j.rules.retry_seconds + 0.01), "the cap stops further tries");
    }

    #[test]
    fn a_removal_or_a_closed_room_never_rejoins() {
        for reason in ["YOU WERE REMOVED FROM THE ROOM", "THE HOST LEFT THE ROOM", "THE ROOM CLOSED"] {
            let mut j = rj();
            j.remember("ABCDE", "T");
            assert!(!j.dropped(reason, 0.0), "{reason}");
            assert!(j.attempt(0.0).is_none());
        }
        let mut nothing = rj();
        assert!(!nothing.dropped("LOST THE CONNECTION", 0.0), "nothing remembered, nothing to rejoin");
    }

    #[test]
    fn linking_again_closes_the_window() {
        let mut j = rj();
        j.remember("ABCDE", "T");
        j.dropped("LOST THE CONNECTION", 0.0);
        j.remember("ABCDE", "T2");
        assert!(j.seconds_left(1.0).is_none() && !j.expired(1.0));
    }

    #[test]
    fn the_host_left_notice_reads_room_closed() {
        let r = rules();
        assert_eq!(closed_notice(r, "THE HOST LEFT THE ROOM"), "HOST LEFT - ROOM CLOSED");
        assert_eq!(closed_notice(r, "the room closed"), "HOST LEFT - ROOM CLOSED");
        assert_eq!(closed_notice(r, "lost the connection to the room"), "LOST THE CONNECTION TO THE ROOM");
    }

    #[test]
    fn the_status_line_counts_down_and_the_token_stays_hidden() {
        let mut j = rj();
        j.remember("ABCDE", "SECRETTOKEN");
        j.dropped("LOST THE CONNECTION", 0.0);
        let line = j.status_line(1.0).unwrap();
        assert!(line.contains("ABCDE") && line.contains("S LEFT"), "{line}");
        assert!(!format!("{:?}", j).contains("SECRETTOKEN"));
        let over = j.status_line(j.rules.window_seconds + 1.0).unwrap();
        assert!(over.contains("COULD NOT"), "{over}");
    }

    #[test]
    fn the_last_try_is_still_awaited_until_remembered() {
        let mut j = rj();
        j.remember("ABCDE", "T");
        j.dropped("LOST THE CONNECTION", 0.0);
        for _ in 0..j.rules.max_tries {
            j.tried(0.0);
        }
        assert!(!j.within_window(0.0), "the tries are used up");
        assert!(j.awaiting(), "the last try may still land");
        j.remember("ABCDE", "T2");
        assert!(!j.awaiting());
        j.forget();
        assert!(!j.awaiting());
    }

    #[test]
    fn a_changed_window_changes_behaviour() {
        let mut r = Rules::default();
        r.window_seconds = 5.0;
        let mut j = Rejoin::new(r);
        j.remember("ABCDE", "T");
        j.dropped("LOST", 0.0);
        assert!(j.within_window(4.0) && !j.within_window(6.0));
    }
}
