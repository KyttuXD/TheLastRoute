//! The PUBLIC route's settings, from the ACTION list. SHOW reads them for anyone; the change rows are for full admin
//! owners only, and the server checks the owner's session again. A change row is queued by the menu, sent by the app
//! with the owner's session (`Admin::run_public`), and its answer goes to the game's message strip (`Admin::poll_public`),
//! the same way RESET PASSWORD does it. A change reads the current value first, so a step never works from a stale copy.
//! The server holds the values and refuses an out-of-range one (`backend/public_settings.mjs`).

use super::Admin;
use crate::game::Game;
use crate::net::api::{self, PublicSettings};
use crate::net::{BackendConfig, NetError, Pending};
use serde_json::json;

/// The steps of the two numeric settings the ACTION list changes.
const JOIN_STEP: i64 = 5;
const JOIN_MIN: i64 = 1;
const JOIN_MAX: i64 = 120;
const TIMEOUT_STEP: i64 = 5;
const TIMEOUT_MIN: i64 = 5;
const TIMEOUT_MAX: i64 = 120;

/// What an ACTION row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicOp {
    /// Moves the join limit (`true`) or the player timeout (`false`) by this many steps (+1 or -1).
    JoinLimit(i64),
    PlayerTimeout(i64),
    /// Reads the settings and shows them.
    Show,
}

#[derive(Default)]
pub struct PublicState {
    request: Option<PublicOp>,
    pending: Option<(PublicOp, Pending<PublicSettings>)>,
}

/// The one-line summary of the values, for the message strip.
pub fn summary(s: &PublicSettings) -> String {
    format!(
        "ADMIN // PUBLIC ROUTE // JOIN {} A MINUTE, TIMEOUT {:.0} S, HEARTBEAT {:.0} S, FALLBACK {}",
        s.join_per_minute,
        s.player_ttl_seconds,
        s.heartbeat_seconds,
        if s.client_host_fallback { "ON" } else { "OFF" }
    )
}

/// The new value of a setting that moves by `step` from `current`, kept within `min..=max`.
fn stepped(current: i64, step: i64, min: i64, max: i64) -> i64 {
    (current + step).clamp(min, max)
}

/// What the answer for a change says: the new value of the one setting it moved.
fn changed_line(op: PublicOp, s: &PublicSettings) -> String {
    match op {
        PublicOp::JoinLimit(_) => format!("ADMIN // PUBLIC JOIN LIMIT IS NOW {} A MINUTE", s.join_per_minute),
        PublicOp::PlayerTimeout(_) => format!("ADMIN // PUBLIC PLAYER TIMEOUT IS NOW {:.0} S", s.player_ttl_seconds),
        PublicOp::Show => summary(s),
    }
}

impl Admin {
    /// An ACTION row for the public route was run. A change needs a full admin owner; SHOW does not.
    pub(super) fn ask_public(&mut self, op: PublicOp) -> String {
        if op != PublicOp::Show && !self.is_owner() {
            return "ADMIN // THE PUBLIC ROUTE NEEDS A FULL ADMIN ACCOUNT".into();
        }
        if self.public.pending.is_some() || self.public.request.is_some() {
            return "ADMIN // A PUBLIC ROUTE REQUEST IS ALREADY RUNNING".into();
        }
        self.public.request = Some(op);
        match op {
            PublicOp::Show => "ADMIN // READING THE PUBLIC ROUTE SETTINGS...".into(),
            _ => "ADMIN // SENDING THE PUBLIC ROUTE CHANGE...".into(),
        }
    }

    /// Sends the queued request. The app calls this once per frame; it does nothing unless a row asked for one.
    pub fn run_public(&mut self, config: &BackendConfig, session_token: &str) {
        if self.public.pending.is_some() {
            return;
        }
        let Some(op) = self.public.request.take() else { return };
        let (cfg, token) = (config.clone(), session_token.to_string());
        let pending = match op {
            PublicOp::Show => Pending::spawn(move || api::public_settings(&cfg)),
            PublicOp::JoinLimit(_) | PublicOp::PlayerTimeout(_) if token.is_empty() => {
                Pending::done(Err(NetError::Refused("Sign in with a full admin account first".into())))
            }
            PublicOp::JoinLimit(dir) => Pending::spawn(move || {
                // read the current value, move it one step, and send only that one setting
                let now = api::public_settings(&cfg)?;
                let value = stepped(i64::from(now.join_per_minute), dir * JOIN_STEP, JOIN_MIN, JOIN_MAX);
                api::admin_set_public_settings(&cfg, &token, &json!({ "join_per_minute": value }))
            }),
            PublicOp::PlayerTimeout(dir) => Pending::spawn(move || {
                let now = api::public_settings(&cfg)?;
                let value = stepped(now.player_ttl_seconds.round() as i64, dir * TIMEOUT_STEP, TIMEOUT_MIN, TIMEOUT_MAX);
                api::admin_set_public_settings(&cfg, &token, &json!({ "player_ttl_seconds": value }))
            }),
        };
        self.public.pending = Some((op, pending));
    }

    /// Takes in the answer when it has arrived and puts it on the message strip.
    pub(super) fn poll_public(&mut self, g: &mut Game) {
        let Some((_, pending)) = self.public.pending.as_mut() else { return };
        let Some(result) = pending.poll() else { return };
        let (op, _) = self.public.pending.take().expect("a pending public request was just polled");
        let line = match result {
            Ok(settings) => changed_line(op, &settings),
            Err(error) => format!("ADMIN // PUBLIC ROUTE FAILED // {error}"),
        };
        g.say(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PublicSettings {
        PublicSettings { join_per_minute: 20, player_ttl_seconds: 20.0, heartbeat_seconds: 5.0, client_host_fallback: true }
    }

    #[test]
    fn a_step_stays_inside_the_server_limits() {
        assert_eq!(stepped(20, JOIN_STEP, JOIN_MIN, JOIN_MAX), 25);
        assert_eq!(stepped(118, JOIN_STEP, JOIN_MIN, JOIN_MAX), JOIN_MAX);
        assert_eq!(stepped(3, -JOIN_STEP, JOIN_MIN, JOIN_MAX), JOIN_MIN);
        assert_eq!(stepped(5, -TIMEOUT_STEP, TIMEOUT_MIN, TIMEOUT_MAX), TIMEOUT_MIN);
    }

    #[test]
    fn the_summary_names_every_setting_the_server_holds() {
        assert_eq!(summary(&sample()), "ADMIN // PUBLIC ROUTE // JOIN 20 A MINUTE, TIMEOUT 20 S, HEARTBEAT 5 S, FALLBACK ON");
        let off = PublicSettings { client_host_fallback: false, ..sample() };
        assert!(summary(&off).ends_with("FALLBACK OFF"));
    }

    #[test]
    fn a_change_answer_names_the_value_it_moved() {
        let moved = PublicSettings { join_per_minute: 25, ..sample() };
        assert_eq!(changed_line(PublicOp::JoinLimit(1), &moved), "ADMIN // PUBLIC JOIN LIMIT IS NOW 25 A MINUTE");
        assert_eq!(changed_line(PublicOp::PlayerTimeout(-1), &sample()), "ADMIN // PUBLIC PLAYER TIMEOUT IS NOW 20 S");
    }

    #[test]
    fn a_change_needs_an_owner_but_showing_does_not() {
        let mut a = Admin::new();
        assert!(a.ask_public(PublicOp::JoinLimit(1)).contains("FULL ADMIN"));
        assert!(a.public.request.is_none(), "a refused change is not queued");
        assert!(a.ask_public(PublicOp::Show).contains("READING"));
        assert_eq!(a.public.request, Some(PublicOp::Show));
    }
}
