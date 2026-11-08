//! NPC teammates over the room. The room's host runs the NPCs (its game simulates them, `Game::update_npcs`); every
//! guest only draws them. The host sends each NPC's place on the STATE channel (`_xnpc_place`, 20 Hz while any NPC is
//! there) and a remove on EVENTS (`_xnpc_remove`) when one goes. A guest keeps the latest place of each NPC and stops
//! drawing one that has gone quiet for [`NPC_STALE`], so a dropped remove or a lost link cannot leave a ghost behind.
//!
//! Positions cross the wire as absolute cells (the window origin added, as runner samples do); each viewer turns them
//! back into its own window with its own origin, so two players in different windows agree on where an NPC stands.

use crate::game::npc::Npc;
use crate::net::protocol::Message;
use serde_json::{Map, Value};
use crate::shared::clock::Instant;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// How often the host sends the NPCs' places (the runner samples go at the same rate).
pub const NPC_SEND_EVERY: Duration = Duration::from_millis(50);
/// How long a guest keeps an NPC it has not heard of again.
pub const NPC_STALE: Duration = Duration::from_millis(1500);

/// One NPC as the host last described it: absolute cells and feet height, facing, and its walk.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteNpc {
    pub name: String,
    /// Absolute cells.
    pub x: f32,
    pub y: f32,
    /// Absolute feet height and the level that owns it.
    pub h: f32,
    pub level: i32,
    pub yaw: f32,
    pub walking: bool,
    pub stride: f32,
    seen: Instant,
}

/// An NPC as a guest draws it: its cell in this viewer's window.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteNpcView {
    pub name: String,
    pub at: (f32, f32),
    pub level: i32,
    pub facing: f32,
    pub stride: f32,
    pub walking: bool,
}

#[derive(Default)]
pub struct NpcSync {
    /// Host: when the places last went out, and the NPC ids the last places named (to send their removes).
    last_send: Option<Instant>,
    sent: BTreeSet<u32>,
    /// Guest: the NPCs the host describes, by id.
    remote: BTreeMap<i64, RemoteNpc>,
}

/// The state map one NPC is sent as: its name, absolute place, height, level, facing and walk.
pub fn wire_state(npc: &Npc, origin: (i32, i32)) -> Map<String, Value> {
    let (x, y) = super::world_sync::to_absolute(npc.at, origin);
    let mut m = Map::new();
    m.insert("name".into(), Value::String(npc.name.clone()));
    m.insert("x".into(), serde_json::json!(x));
    m.insert("y".into(), serde_json::json!(y));
    m.insert("h".into(), serde_json::json!(npc.y));
    m.insert("level".into(), serde_json::json!(npc.level));
    m.insert("yaw".into(), serde_json::json!(npc.facing));
    m.insert("walking".into(), Value::Bool(npc.walking));
    m.insert("stride".into(), serde_json::json!(npc.stride));
    m
}

fn finite(state: &Map<String, Value>, key: &str) -> Option<f32> {
    state.get(key).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite())
}

impl NpcSync {
    /// Host: the messages for this tick, from each NPC's id and wire state ([`wire_state`]). Places go out when a send is
    /// due (every [`NPC_SEND_EVERY`]), for every NPC; a remove goes out at once for each NPC the last places named that
    /// is gone now.
    pub fn host_messages(&mut self, npcs: &[(u32, Map<String, Value>)], now: Instant) -> Vec<Message> {
        let mut out = Vec::new();
        let now_ids: BTreeSet<u32> = npcs.iter().map(|(id, _)| *id).collect();
        for gone in self.sent.difference(&now_ids).copied().collect::<Vec<u32>>() {
            out.push(Message::XnpcRemove { id: i64::from(gone) });
        }
        self.sent = now_ids;
        let due = self.last_send.is_none_or(|t| now.duration_since(t) >= NPC_SEND_EVERY);
        if due && !npcs.is_empty() {
            self.last_send = Some(now);
            for (id, state) in npcs {
                out.push(Message::XnpcPlace { id: i64::from(*id), state: state.clone() });
            }
        }
        out
    }

    /// Guest: the host describes an NPC.
    pub fn receive_place(&mut self, id: i64, state: &Map<String, Value>, now: Instant) {
        let (Some(x), Some(y), Some(h)) = (finite(state, "x"), finite(state, "y"), finite(state, "h")) else { return };
        let level = state.get("level").and_then(Value::as_i64).unwrap_or(0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.remote.insert(id, RemoteNpc {
            name: state.get("name").and_then(Value::as_str).unwrap_or("").chars().take(24).collect(),
            x,
            y,
            h,
            level,
            yaw: finite(state, "yaw").unwrap_or(0.0),
            walking: state.get("walking").and_then(Value::as_bool).unwrap_or(false),
            stride: finite(state, "stride").unwrap_or(0.0),
            seen: now,
        });
    }

    /// Guest: the host says an NPC is gone.
    pub fn receive_remove(&mut self, id: i64) {
        self.remote.remove(&id);
    }

    /// Guest: the NPCs heard of in the last [`NPC_STALE`], in this viewer's window (`origin`), for the figure code.
    pub fn views(&self, origin: (i32, i32), now: Instant) -> Vec<RemoteNpcView> {
        self.remote
            .values()
            .filter(|n| now.duration_since(n.seen) <= NPC_STALE)
            .map(|n| {
                let (x, y) = super::world_sync::to_window((n.x, n.y), origin);
                RemoteNpcView { name: n.name.clone(), at: (x, y), level: n.level, facing: n.yaw, stride: n.stride, walking: n.walking }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f64, y: f64, level: i64) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("name".into(), Value::String("R-04821".into()));
        m.insert("x".into(), serde_json::json!(x));
        m.insert("y".into(), serde_json::json!(y));
        m.insert("h".into(), serde_json::json!(0.0));
        m.insert("level".into(), serde_json::json!(level));
        m.insert("yaw".into(), serde_json::json!(0.5));
        m.insert("walking".into(), Value::Bool(true));
        m.insert("stride".into(), serde_json::json!(1.2));
        m
    }

    #[test]
    fn a_place_is_turned_into_each_viewers_own_window() {
        let t0 = Instant::now();
        let mut guest = NpcSync::default();
        guest.receive_place(7, &state(120.0, 80.0, 0), t0);
        // a viewer whose window starts at (100, 50) sees the NPC at (20, 30) in its window
        let views = guest.views((100, 50), t0);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].at, (20.0, 30.0));
        assert_eq!(views[0].name, "R-04821");
        assert!(views[0].walking);
    }

    #[test]
    fn a_remove_and_a_quiet_npc_are_both_forgotten() {
        let t0 = Instant::now();
        let mut guest = NpcSync::default();
        guest.receive_place(1, &state(1.0, 1.0, 0), t0);
        guest.receive_place(2, &state(2.0, 2.0, 0), t0);
        guest.receive_remove(1);
        assert_eq!(guest.views((0, 0), t0).len(), 1);
        let later = t0 + NPC_STALE + Duration::from_millis(10);
        assert!(guest.views((0, 0), later).is_empty(), "a quiet NPC stops being drawn");
    }

    #[test]
    fn a_place_without_a_position_is_refused() {
        let t0 = Instant::now();
        let mut guest = NpcSync::default();
        let mut bad = state(1.0, 1.0, 0);
        bad.insert("x".into(), Value::String("here".into()));
        guest.receive_place(3, &bad, t0);
        assert!(guest.views((0, 0), t0).is_empty());
    }

    #[test]
    fn a_host_sends_places_at_its_rate_and_a_remove_when_an_npc_goes() {
        let mut host = NpcSync::default();
        let t0 = Instant::now();
        let one = (1u32, state(1.0, 1.0, 0));
        let two = (2u32, state(2.0, 2.0, 0));
        let first = host.host_messages(&[one.clone(), two.clone()], t0);
        assert_eq!(first.len(), 2, "one place per NPC");
        assert!(host.host_messages(&[one.clone(), two.clone()], t0 + Duration::from_millis(10)).is_empty(), "not yet due");
        let gone = host.host_messages(&[one.clone()], t0 + NPC_SEND_EVERY);
        assert!(matches!(gone.first(), Some(Message::XnpcRemove { id: 2 })), "the vanished NPC is removed");
        assert_eq!(gone.iter().filter(|m| matches!(m, Message::XnpcPlace { .. })).count(), 1);
    }
}
