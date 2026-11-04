//! What the host's game does with what a guest (or the net layer) reports (Godot `_apply_hunter_event`, `_resolve_taser`,
//! `_apply_valve_event`, `_receive_snapshot`). The net session only validates and queues these (`net/world_sync.rs`);
//! the app hands them here each frame (`app/mod.rs`).
//!
//! * [`Game::apply_guest_event`]: a guest's taser intent, its escape from a capture, its caught report, a blocked
//!   capture, or the net layer's `capture_timeout`. The host alone resolves them; a guest game ignores them.
//! * [`Game::apply_relay_snapshot`]: the sabotaged relay count in a late joiner's snapshot.
//! * [`Game::apply_pipe_event`] and [`Game::host_turn_valve`]: the sewer valve and pipe burst events.
//!
//! Guests never change hunter state here: every method that touches a hunter returns at once on a display only game.
//! A remote runner is never the local runner, so nothing here feeds the danger readout, the heartbeat or the chase cue.
//! The tuning (stun time, taser range and angle, cooldowns) is the difficulty profile and `config`, not this file.

use super::{guest_gate, Game};
use crate::config::{TASER_HALF_ANGLE, TASER_RANGE};
use crate::hunters::brain::Brain;
use crate::net::valve_events::{self, ValveRequest};
use serde_json::{Map, Value};

/// A number field of an event, read as f32 when it is finite.
fn num(ev: &Map<String, Value>, key: &str) -> Option<f32> {
    ev.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32)
}

impl Game {
    /// Host: applies one accepted guest event (`NetSession::take_guest_events`). The app adds the guest's place to a
    /// taser or escape event as `x`, `y` (this window's cells) and `yaw` before it calls this, because the game does
    /// not track remote runners; an event without them cannot be placed and does nothing.
    ///
    /// * `taser`: the nearest active hunter in the guest's taser cone is stunned like the local taser does (a
    ///   taser-immune hunter is slowed instead).
    /// * `escaped`: the hunter that held the guest (`hunter_id`, else the nearest to `x`, `y`) is stunned for the profile's
    ///   `escape_stun`, doubled when the guest used the axe (`axe`), and searches from where the guest broke free.
    /// * `caught`: the guest was taken; the host is told.
    /// * `capture_blocked`, `capture_timeout`: the host stops waiting for that guest; nothing else changes.
    pub fn apply_guest_event(&mut self, ev: Map<String, Value>) {
        if guest_gate::is_display_only(self) {
            return;
        }
        let at = num(&ev, "x").zip(num(&ev, "y"));
        match ev.get("what").and_then(Value::as_str).unwrap_or_default() {
            "taser" => {
                if let (Some(from), Some(yaw)) = (at, num(&ev, "yaw")) {
                    self.guest_taser(from, yaw);
                }
            }
            "escaped" => self.guest_escaped(&ev, at),
            "caught" => self.announce("A RUNNER WAS CAUGHT"),
            _ => {}
        }
    }

    /// The taser fired from `from` facing `yaw` (the runner convention: forward is `(-sin, -cos)`).
    fn guest_taser(&mut self, from: (f32, f32), yaw: f32) {
        let facing = (-yaw.sin(), -yaw.cos());
        let mut best: Option<(f32, usize)> = None;
        for (i, h) in self.hunters.iter().enumerate() {
            if !h.active || h.level != self.fac.level || h.gimmick.hidden_from_view() {
                continue;
            }
            if Brain::beam_reaches(&self.fac, from, facing, h.pos, TASER_RANGE, TASER_HALF_ANGLE) {
                let d = super::pdist(h.pos, from);
                if best.map_or(true, |b| d < b.0) {
                    best = Some((d, i));
                }
            }
        }
        let Some((_, i)) = best else { return };
        let prof = self.prof();
        if self.hunters[i].def.taser_immune {
            let def = &self.hunters[i].def;
            let factor = def.taser_slow_factor.unwrap_or(prof.taser_slow_factor);
            let seconds = def.taser_slow_seconds.unwrap_or(prof.taser_slow_seconds);
            self.hunters[i].apply_slow(seconds, factor);
        } else {
            self.hunters[i].apply_stun(prof.escape_stun, false);
        }
    }

    fn guest_escaped(&mut self, ev: &Map<String, Value>, at: Option<(f32, f32)>) {
        let named = ev
            .get("hunter_id")
            .or_else(|| ev.get("entity_id"))
            .and_then(Value::as_str)
            .and_then(|id| self.hunters.iter().position(|h| h.def.id == id));
        let nearest = || {
            let at = at?;
            self.hunters
                .iter()
                .enumerate()
                .filter(|(_, h)| h.active && h.level == self.fac.level)
                .min_by(|a, b| super::pdist(a.1.pos, at).total_cmp(&super::pdist(b.1.pos, at)))
                .map(|(i, _)| i)
        };
        let Some(i) = named.or_else(nearest) else { return };
        let prof = self.prof();
        let axe = ev.get("axe").and_then(Value::as_bool).unwrap_or(false);
        self.hunters[i].apply_stun(prof.escape_stun * if axe { 2.0 } else { 1.0 }, false);
        if let Some(p) = at.filter(|_| self.hunters[i].level == self.fac.level) {
            let fac = &self.fac;
            self.hunters[i].enter_search_from(p, 1.8, fac);
        }
    }

    /// Guest: a late joiner's snapshot (`NetSession::take_snapshot`) carries how many relays the host has sabotaged
    /// offline. The guest runs no sabotage clock, so it can only say so; it never changes hunter state.
    pub fn apply_relay_snapshot(&mut self, relays_sabotaged: u32) {
        if relays_sabotaged > 0 {
            self.announce(&format!("{relays_sabotaged} RELAYS OFFLINE"));
        }
    }

    /// The sabotaged relay count the host puts in a late joiner's snapshot: the relays its sabotage tracker holds
    /// offline right now (`NetSession::set_relays_sabotaged`).
    pub fn relays_sabotaged_count(&self) -> u32 {
        self.sabotage.as_ref().map_or(0, |t| t.blocked().len() as u32)
    }

    /// Every machine: applies a `stream_valve` or `stream_pipe_burst` world event to the pipe network. Returns whether
    /// the event was a pipe event. Replays are harmless, and a bad event changes nothing.
    pub fn apply_pipe_event(&mut self, ev: &Map<String, Value>) -> bool {
        let now = self.time;
        if let Some(turn) = valve_events::parse_turn(ev) {
            self.pipes.apply(&turn, now);
            return true;
        }
        if let Some(flow) = valve_events::parse_burst(ev) {
            self.pipes.start_flow(&flow, now);
            return true;
        }
        false
    }

    /// Host: decides a guest's valve request (`NetSession::take_valve_requests`). `at` and `depth` are the guest's last
    /// sample (`NetSession::guest_position`, absolute cells). The turn runs with `by_player = false`, so it carries no
    /// noise. On success the host's network has the turn and the wire fields come back for `Outbox::emit`.
    pub fn host_turn_valve(&mut self, req: &ValveRequest, at: (f32, f32), depth: i32) -> Option<Vec<(&'static str, Value)>> {
        if guest_gate::is_display_only(self) {
            return None;
        }
        // the valve is the host's own prop: its place and keys are never the request's claim (`valve_at`, `effect`
        // and `gate` of the request are ignored), so a guest cannot turn a valve it is not standing at
        let rules = super::pipes::rules();
        let storey = super::pipes::parse_valve_id(&req.valve_id, rules.valve.max_id_length)?.1;
        let prop = self.fac.props.iter().find(|p| p.id == req.valve_id && p.ptype == rules.valve.prop_type)?;
        let valve_at = prop.cell.center();
        let (effect, gate) = self.pipes.valve(&req.valve_id).map(|v| (v.effect.clone(), v.gate.clone())).unwrap_or_default();
        let q = super::pipes::TurnAt {
            valve_id: &req.valve_id,
            effect: if effect == "flood" { "" } else { &effect },
            gate: &gate,
            valve_at,
            valve_storey: storey,
            at,
            storey: -depth,
            by_player: false,
        };
        let ev = self.pipes.turn(q).ok()?;
        self.pipes.apply(&ev, self.time);
        // the turn is a sound the hunters can hear, whoever made it: it sounds at the valve, on the valve's storey. The
        // runner's meter noise is the turner's own (none here), so only the positioned event is pushed.
        self.push_valve_events(&ev, valve_at, storey);
        self.start_authored_floods(&req.valve_id, super::hunter_pipes::authored_floods());
        Some(valve_events::turn_fields(&ev))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use serde_json::json;

    fn run() -> Game {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-APPLY-7", 0, &content, &catalog);
        // Standard admits route hunters after one relay, so the first relay gives the reports a hunter to act on
        g.relays_collected = 1;
        g.ensure_hunters(&content);
        g
    }

    fn event(what: &str, fields: Vec<(&str, Value)>) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("what".into(), json!(what));
        for (k, v) in fields {
            m.insert(k.into(), v);
        }
        m
    }

    #[test]
    fn an_escape_report_stuns_the_named_hunter_for_the_profile_time_and_the_axe_doubles_it() {
        let mut g = run();
        assert!(!g.hunters.is_empty());
        let id = g.hunters[0].def.id.clone();
        let stun = g.prof().escape_stun;
        let before = g.hunters[0].stun;
        g.apply_guest_event(event("escaped", vec![("hunter_id", json!(id))]));
        assert!(g.hunters[0].stun >= stun.max(before), "stunned for at least the profile's escape stun");
        g.hunters[0].stun = 0.0;
        g.apply_guest_event(event("escaped", vec![("hunter_id", json!(id)), ("axe", json!(true))]));
        assert!(g.hunters[0].stun >= stun * 2.0 - 1e-4, "an axe escape stuns twice as long");
    }

    #[test]
    fn an_escape_with_no_hunter_or_place_does_nothing() {
        let mut g = run();
        let stuns: Vec<f32> = g.hunters.iter().map(|h| h.stun).collect();
        g.apply_guest_event(event("escaped", vec![("hunter_id", json!("nobody"))]));
        g.apply_guest_event(event("capture_blocked", vec![]));
        g.apply_guest_event(event("capture_timeout", vec![("peer_id", json!(2))]));
        g.apply_guest_event(event("unknown", vec![]));
        let after: Vec<f32> = g.hunters.iter().map(|h| h.stun).collect();
        assert_eq!(stuns, after);
    }

    #[test]
    fn a_guest_taser_ignores_a_hunter_on_another_level() {
        let mut g = run();
        let Some(target) = g.hunters.iter().position(|h| h.active && !h.def.taser_immune && !h.gimmick.hidden_from_view()) else {
            return;
        };
        let hp = g.hunters[target].pos;
        for (dx, dz) in [(0.8f32, 0.0f32), (-0.8, 0.0), (0.0, 0.8), (0.0, -0.8)] {
            let from = (hp.0 + dx, hp.1 + dz);
            if !g.fac.has_line_of_sight(from, hp) {
                continue;
            }
            let yaw = (-(hp.0 - from.0)).atan2(-(hp.1 - from.1));
            g.hunters[target].level = g.fac.level + 1;
            g.hunters[target].stun = 0.0;
            g.apply_guest_event(event("taser", vec![("x", json!(from.0)), ("y", json!(from.1)), ("yaw", json!(yaw))]));
            assert_eq!(g.hunters[target].stun, 0.0, "a hunter one floor away is not tasered");
            return;
        }
    }

    #[test]
    fn a_guest_taser_stuns_the_hunter_in_front_of_it_and_misses_one_behind() {
        let mut g = run();
        let Some(target) = g.hunters.iter().position(|h| h.active && !h.def.taser_immune && !h.gimmick.hidden_from_view()) else {
            return;
        };
        let hp = g.hunters[target].pos;
        for (dx, dz) in [(0.8f32, 0.0f32), (-0.8, 0.0), (0.0, 0.8), (0.0, -0.8)] {
            let from = (hp.0 + dx, hp.1 + dz);
            if !g.fac.has_line_of_sight(from, hp) {
                continue;
            }
            // face the hunter: forward is (-sin yaw, -cos yaw)
            let yaw = (-(hp.0 - from.0)).atan2(-(hp.1 - from.1));
            g.hunters[target].stun = 0.0;
            g.apply_guest_event(event("taser", vec![("x", json!(from.0)), ("y", json!(from.1)), ("yaw", json!(yaw))]));
            assert!(g.hunters[target].stun > 0.0, "the hunter in the cone is stunned");
            g.hunters[target].stun = 0.0;
            let away = yaw + std::f32::consts::PI;
            g.apply_guest_event(event("taser", vec![("x", json!(from.0)), ("y", json!(from.1)), ("yaw", json!(away))]));
            assert_eq!(g.hunters[target].stun, 0.0, "facing away hits nothing");
            return;
        }
    }

    #[test]
    fn a_guest_game_never_changes_a_hunter_from_a_report() {
        let mut g = run();
        g.remote_hunters = true;
        let id = g.hunters[0].def.id.clone();
        g.apply_guest_event(event("escaped", vec![("hunter_id", json!(id))]));
        assert_eq!(g.hunters[0].stun, 0.0, "only the host resolves reports");
    }

    #[test]
    fn an_old_guest_ignores_the_new_keys() {
        let mut g = run();
        g.remote_hunters = true;
        let id = g.hunters[0].def.id.clone();
        let level = g.runner_level();
        let frame = json!({"index": 0, "id": id, "position": [4.5, 6.5], "level": level, "presentation_y": g.p.y()});
        g.apply_hunter_frame(&[frame]);
        assert_eq!(g.hunters[0].pos, (4.5, 6.5), "position is still applied when level and presentation_y ride along");
        // a level of another floor is read too: the hunter leaves the viewer's list and the position still lands
        let frame = json!({"index": 0, "id": id, "position": [5.5, 7.5], "level": level - 1, "presentation_y": -2.0});
        g.apply_hunter_frame(&[frame]);
        assert!(g.hunters.is_empty(), "it is shown on the floor below");
        let below = g.resident_hunters.list(level - 1).iter().find(|s| s.brain.def.id == id).expect("on the other floor's list");
        assert_eq!((below.brain.pos, below.brain.level), ((5.5, 7.5), level - 1));
    }

    #[test]
    fn the_relay_snapshot_is_said_and_the_sabotage_count_is_zero_without_a_tracker() {
        let mut g = run();
        g.apply_relay_snapshot(0);
        assert!(g.toast.is_empty(), "no sabotaged relay, nothing to say");
        g.apply_relay_snapshot(2);
        assert!(g.toast.contains('2'));
        g.sabotage = None;
        assert_eq!(g.relays_sabotaged_count(), 0);
    }

    #[test]
    fn a_published_valve_turn_and_a_burst_change_the_pipe_network_on_every_machine() {
        let mut g = run();
        let ev = event(
            valve_events::VALVE,
            vec![("valve_id", json!("SEWER:0:-1:0:PROP:3")), ("effect", json!("flow")), ("shut", json!(false)), ("turns", json!(1)), ("gate", json!(""))],
        );
        assert!(g.apply_pipe_event(&ev));
        assert!(g.pipes.is_open("SEWER:0:-1:0:PROP:3"), "an odd flow turn opens the valve");
        let burst = event(valve_events::BURST, vec![("flow_id", json!("BRK:0:-1:0:9"))]);
        assert!(g.apply_pipe_event(&burst));
        assert!(g.pipes.run("BRK:0:-1:0:9").is_some());
        assert!(!g.apply_pipe_event(&event("hunter_door", vec![])), "other events are not pipe events");
    }

    fn add_valve(g: &mut Game, id: &str, at: (i32, i32)) {
        g.fac.props.push(crate::world::facility::Prop {
            id: id.to_string(),
            cell: crate::world::Cell::new(at.0, at.1),
            ptype: crate::game::pipes::rules().valve.prop_type.clone(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.3, 0.3),
            offset: (0.0, 0.0),
            door: None,
        });
    }

    #[test]
    fn a_guest_cannot_turn_a_valve_it_names_at_a_place_it_picks_or_one_the_host_does_not_have() {
        let mut g = run();
        add_valve(&mut g, "SEWER:0:-1:0:PROP:3", (10, 10));
        // the guest stands far from the real valve and claims the valve is at its own feet
        let lie = ValveRequest {
            peer: 2,
            valve_id: "SEWER:0:-1:0:PROP:3".into(),
            effect: "flow".into(),
            gate: String::new(),
            valve_at: (60.5, 60.5),
        };
        assert!(g.host_turn_valve(&lie, (60.5, 60.5), 1).is_none(), "the claimed valve place is ignored");
        assert_eq!(g.pipes.turns("SEWER:0:-1:0:PROP:3"), 0);
        // a valve id the host has no prop for is refused even when the guest stands on the claimed place
        let unknown = ValveRequest { valve_id: "SEWER:0:-1:0:PROP:9".into(), valve_at: (10.5, 10.5), ..lie };
        assert!(g.host_turn_valve(&unknown, (10.5, 10.5), 1).is_none(), "no such prop on the host");
    }

    #[test]
    fn the_host_turns_a_guests_valve_only_when_the_guest_stands_at_it() {
        let mut g = run();
        add_valve(&mut g, "SEWER:0:-1:0:PROP:3", (10, 10));
        let req = ValveRequest {
            peer: 2,
            valve_id: "SEWER:0:-1:0:PROP:3".into(),
            effect: "flow".into(),
            gate: String::new(),
            valve_at: (10.5, 10.5),
        };
        let reach = crate::game::pipes::rules().valve.reach;
        assert!(g.host_turn_valve(&req, (10.5 + reach + 5.0, 10.5), 1).is_none(), "too far");
        assert!(g.host_turn_valve(&req, (10.5, 10.5), 2).is_none(), "wrong storey");
        g.noise_events.clear();
        let fields = g.host_turn_valve(&req, (10.5, 10.5), 1).expect("at the valve on its storey");
        assert!(fields.iter().any(|(k, v)| *k == "turns" && v == &json!(1)));
        let heard = g.noise_events.iter().find(|e| e.kind == crate::hunters::noise_events::NoiseKind::ValveTurn);
        let heard = heard.expect("a guest's valve turn is a sound the hunters can hear");
        assert_eq!((heard.pos, heard.level), ((10.5, 10.5), -1), "at the valve, on the valve's storey");
        assert_eq!(g.p.noise_burst, 0.0, "and the runner's own meter noise is not touched");
        assert!(g.pipes.is_open("SEWER:0:-1:0:PROP:3"), "the host applied the turn too");
        g.remote_hunters = true;
        assert!(g.host_turn_valve(&req, (10.5, 10.5), 1).is_none(), "a guest never decides a turn");
    }
}
