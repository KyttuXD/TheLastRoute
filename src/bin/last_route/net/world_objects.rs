//! The world objects every runner of a room must agree on: relays, batteries, floor items and coins that are taken, room
//! doors that stand open and room lights that are switched off (issue 9 of docs/MULTIPLAYER_SYNC_AUDIT.md).
//!
//! The host decides. A guest's interaction is a request ([`REQUEST`], sent as `RequestInteraction`); the host checks it
//! against the guest's own sample, applies it and publishes one [`EVENT`] that every machine applies. A runner that joins
//! during the route is sent the whole table as [`BATCH`] events after its snapshot.
//!
//! An object is named by what it is and where it is: its kind, its level and its absolute cell ([`ObjKey`]). Every
//! machine builds the same world from the room seed, so the cell names the same object everywhere, and a machine that
//! has not streamed that chunk yet keeps the state in its [`ObjectTable`] and applies it when the chunk is built
//! (`Game::sync_world_objects`).
//!
//! Wire (all three kinds are new; an older build ignores them, and a new guest only asks a host that said it answers,
//! see [`SYNC_KEY`]):
//! * `{"what": "world_object", "k": "r", "l": 0, "x": 12, "y": -40, "on": true, "by": 3}`
//! * `{"what": "world_objects", "o": [["r", 0, 12, -40, 1], ...]}`
//! * `{"what": "world_object_request", "k": "d", "l": 0, "x": 12, "y": -40, "on": false}`
//!
//! `k` is the kind (`r` relay, `b` battery, `i` floor item, `c` coin, `d` door, `l` light). `on` means taken for the
//! four kinds that are taken, open for a door and lit for a light. `by` is the peer that took it (0 for nobody in
//! particular: a bot, or a replay), so that runner alone is given the item.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The `what` of one published change.
pub const EVENT: &str = "world_object";
/// The `what` of the table a late joiner is sent, in parts.
pub const BATCH: &str = "world_objects";
/// The `what` of a guest's request.
pub const REQUEST: &str = "world_object_request";
/// The key of the host's room message that says it answers [`REQUEST`] (an older host sends none, and its guests then
/// keep every object to themselves as before).
pub const SYNC_KEY: &str = "sync";
/// The value of [`SYNC_KEY`] from which the host answers object requests.
pub const SYNC_VERSION: u64 = 2;
/// Most objects a table keeps (a memory bound: about 0.5 MB at the very most; a route in play holds a few hundred).
pub const TABLE_MAX: usize = 16_384;
/// The farthest absolute cell and level an object may name: anything beyond is a broken or hostile message.
const CELL_LIMIT: i64 = 10_000_000;
const LEVEL_LIMIT: i64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Relay,
    Battery,
    Item,
    Coin,
    Door,
    Light,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Relay, Kind::Battery, Kind::Item, Kind::Coin, Kind::Door, Kind::Light];

    pub fn wire(self) -> &'static str {
        match self {
            Kind::Relay => "r",
            Kind::Battery => "b",
            Kind::Item => "i",
            Kind::Coin => "c",
            Kind::Door => "d",
            Kind::Light => "l",
        }
    }

    pub fn from_wire(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.wire() == name)
    }

    /// A relay, battery, floor item or coin: taken once and gone for good.
    pub fn is_take(self) -> bool {
        !matches!(self, Kind::Door | Kind::Light)
    }

    /// The state a world that nobody touched has: not taken, a shut door, a lit light. The table keeps only what differs.
    pub fn default_on(self) -> bool {
        self == Kind::Light
    }
}

/// Which object: its kind, its level and its absolute cell (a door's threshold cell, a light's switch cell).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjKey {
    pub kind: Kind,
    pub level: i32,
    pub x: i32,
    pub y: i32,
}

/// One change of one object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Op {
    pub key: ObjKey,
    /// Taken, open or lit (see the module text).
    pub on: bool,
    /// The peer that took it; 0 for nobody in particular.
    pub by: u32,
}

fn key_fields(o: &mut Map<String, Value>, key: ObjKey) {
    o.insert("k".into(), json!(key.kind.wire()));
    o.insert("l".into(), json!(key.level));
    o.insert("x".into(), json!(key.x));
    o.insert("y".into(), json!(key.y));
}

fn int(v: Option<&Value>, limit: i64) -> Option<i32> {
    let n = v?.as_i64().filter(|n| (-limit..=limit).contains(n))?;
    i32::try_from(n).ok()
}

fn key_of(kind: Option<&Value>, level: Option<&Value>, x: Option<&Value>, y: Option<&Value>) -> Option<ObjKey> {
    Some(ObjKey {
        kind: Kind::from_wire(kind?.as_str()?)?,
        level: int(level, LEVEL_LIMIT)?,
        x: int(x, CELL_LIMIT)?,
        y: int(y, CELL_LIMIT)?,
    })
}

/// The event the host publishes for one change.
pub fn event(op: Op) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("what".into(), json!(EVENT));
    key_fields(&mut o, op.key);
    o.insert("on".into(), json!(op.on));
    if op.by != 0 {
        o.insert("by".into(), json!(op.by));
    }
    o
}

/// The request a guest sends: make this object `on`.
pub fn request(key: ObjKey, on: bool) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("what".into(), json!(REQUEST));
    key_fields(&mut o, key);
    o.insert("on".into(), json!(on));
    o
}

/// Whether `event` is one of the two published kinds.
pub fn is_object_event(event: &Map<String, Value>) -> bool {
    matches!(event.get("what").and_then(Value::as_str), Some(EVENT | BATCH))
}

/// The changes a published event carries: one for [`EVENT`], the listed ones for [`BATCH`] (an entry that does not parse is
/// skipped). `None` for any other event, or an [`EVENT`] that does not parse.
pub fn parse_event(o: &Map<String, Value>) -> Option<Vec<Op>> {
    match o.get("what").and_then(Value::as_str)? {
        EVENT => {
            let key = key_of(o.get("k"), o.get("l"), o.get("x"), o.get("y"))?;
            let on = o.get("on").and_then(Value::as_bool)?;
            let by = o.get("by").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
            Some(vec![Op { key, on, by }])
        }
        BATCH => Some(
            o.get("o")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|e| {
                            let e = e.as_array()?;
                            let key = key_of(e.first(), e.get(1), e.get(2), e.get(3))?;
                            let on = e.get(4).and_then(Value::as_i64)? != 0;
                            Some(Op { key, on, by: 0 })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => None,
    }
}

/// A guest's request, read by the host: the object and the state asked for. `None` for another kind or a bad field.
pub fn parse_request(o: &Map<String, Value>) -> Option<(ObjKey, bool)> {
    if o.get("what").and_then(Value::as_str) != Some(REQUEST) {
        return None;
    }
    let key = key_of(o.get("k"), o.get("l"), o.get("x"), o.get("y"))?;
    Some((key, o.get("on").and_then(Value::as_bool)?))
}

/// The lasting state of the route's objects: what is taken, which doors stand open and which lights are off. Only what
/// differs from an untouched world is kept. The host's table is the truth; a guest's is its copy of it.
#[derive(Debug, Default)]
pub struct ObjectTable {
    state: HashMap<ObjKey, bool>,
    relays: u32,
}

impl ObjectTable {
    /// Applies one change. Returns whether the table changed (a replay, or a second take of one object, changes nothing).
    /// A take past [`TABLE_MAX`] is still a change, it is only not remembered.
    pub fn apply(&mut self, op: Op) -> bool {
        let key = op.key;
        if key.kind.is_take() {
            if !op.on || self.state.contains_key(&key) {
                return false;
            }
            if self.state.len() < TABLE_MAX {
                self.state.insert(key, true);
            }
            if key.kind == Kind::Relay {
                self.relays = self.relays.saturating_add(1);
            }
            return true;
        }
        if op.on == key.kind.default_on() {
            return self.state.remove(&key).is_some();
        }
        if self.state.get(&key) == Some(&op.on) {
            return false;
        }
        if self.state.len() < TABLE_MAX {
            self.state.insert(key, op.on);
        }
        true
    }

    /// The state the table holds for an object; `None` for one nobody touched (see [`Kind::default_on`]).
    pub fn get(&self, key: ObjKey) -> Option<bool> {
        self.state.get(&key).copied()
    }

    /// The state of an object, touched or not.
    pub fn is_on(&self, key: ObjKey) -> bool {
        self.get(key).unwrap_or_else(|| key.kind.default_on())
    }

    /// How many relays the room's runners have recovered on this route.
    pub fn relays(&self) -> u32 {
        self.relays
    }

    pub fn len(&self) -> usize {
        self.state.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// A new route (or none): nothing of the old one is kept.
    pub fn clear(&mut self) {
        self.state.clear();
        self.relays = 0;
    }

    /// The whole table as [`BATCH`] events whose JSON is each at most about `max_bytes` long, for a late joiner. An empty
    /// table gives none.
    pub fn batches(&self, max_bytes: usize) -> Vec<Map<String, Value>> {
        let mut out: Vec<Map<String, Value>> = Vec::new();
        let mut part: Vec<Value> = Vec::new();
        // the event's own keys and brackets
        const ENVELOPE: usize = 32;
        let mut used = ENVELOPE;
        let close = |part: &mut Vec<Value>, out: &mut Vec<Map<String, Value>>| {
            if !part.is_empty() {
                let mut o = Map::new();
                o.insert("what".into(), json!(BATCH));
                o.insert("o".into(), Value::Array(std::mem::take(part)));
                out.push(o);
            }
        };
        for (key, on) in &self.state {
            let entry = json!([key.kind.wire(), key.level, key.x, key.y, i32::from(*on)]);
            let size = serde_json::to_vec(&entry).map_or(0, |b| b.len()) + 1;
            if used + size > max_bytes && !part.is_empty() {
                close(&mut part, &mut out);
                used = ENVELOPE;
            }
            used += size;
            part.push(entry);
        }
        close(&mut part, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: Kind, x: i32) -> ObjKey {
        ObjKey { kind, level: -1, x, y: -40 }
    }

    #[test]
    fn a_published_change_and_a_request_read_back_as_they_were_written() {
        for kind in Kind::ALL {
            let op = Op { key: key(kind, 12), on: true, by: 3 };
            assert_eq!(parse_event(&event(op)), Some(vec![op]), "{kind:?}");
            assert_eq!(parse_request(&request(op.key, false)), Some((op.key, false)), "{kind:?}");
        }
        // nobody in particular took it: the key is left out and reads as 0
        let quiet = Op { key: key(Kind::Coin, 1), on: true, by: 0 };
        assert!(event(quiet).get("by").is_none());
        assert_eq!(parse_event(&event(quiet)), Some(vec![quiet]));
        // a request is not an event and an event is not a request
        assert!(parse_event(&request(quiet.key, true)).is_none());
        assert!(parse_request(&event(quiet)).is_none());
    }

    #[test]
    fn a_broken_or_hostile_object_message_changes_nothing() {
        let good = event(Op { key: key(Kind::Door, 5), on: true, by: 0 });
        for (field, bad) in [("k", json!("z")), ("k", json!(7)), ("l", json!(9000)), ("x", json!(1e12)), ("y", json!("far")), ("on", json!("yes"))] {
            let mut o = good.clone();
            o.insert(field.into(), bad);
            assert!(parse_event(&o).is_none(), "{field}");
        }
        let mut batch = Map::new();
        batch.insert("what".into(), json!(BATCH));
        batch.insert("o".into(), json!([["r", 0, 1, 2, 1], ["?", 0, 1, 2, 1], "junk", ["d", 0, 3], ["l", 0, 4, 5, 0]]));
        let ops = parse_event(&batch).expect("a batch");
        assert_eq!(ops.len(), 2, "the entries that parse are kept, the rest skipped");
        assert_eq!((ops[0].key.kind, ops[1].key.kind, ops[1].on), (Kind::Relay, Kind::Light, false));
    }

    #[test]
    fn a_pickup_is_taken_once_and_a_second_take_changes_nothing() {
        let mut t = ObjectTable::default();
        let relay = Op { key: key(Kind::Relay, 1), on: true, by: 2 };
        assert!(t.apply(relay));
        assert!(!t.apply(Op { by: 3, ..relay }), "the second runner is too late");
        assert!(!t.apply(Op { on: false, ..relay }), "nothing is ever put back");
        assert_eq!((t.relays(), t.is_on(relay.key)), (1, true));
        assert!(!t.is_on(key(Kind::Relay, 2)), "an untouched relay is there");
    }

    #[test]
    fn a_door_and_a_light_keep_their_latest_state_and_forget_the_untouched_one() {
        let mut t = ObjectTable::default();
        let (door, light) = (key(Kind::Door, 1), key(Kind::Light, 1));
        assert!(!t.is_on(door) && t.is_on(light), "a door starts shut and a light lit");
        assert!(t.apply(Op { key: door, on: true, by: 0 }));
        assert!(!t.apply(Op { key: door, on: true, by: 0 }), "a replay changes nothing");
        assert!(t.apply(Op { key: light, on: false, by: 0 }));
        assert_eq!(t.len(), 2);
        assert!(t.apply(Op { key: door, on: false, by: 0 }));
        assert!(t.apply(Op { key: light, on: true, by: 0 }));
        assert!(t.is_empty(), "back as the world was built: nothing to tell a late joiner");
    }

    #[test]
    fn the_table_reaches_a_late_joiner_in_parts_that_each_fit() {
        let mut host = ObjectTable::default();
        for x in 0..200 {
            host.apply(Op { key: key(Kind::Item, x), on: true, by: 1 });
        }
        host.apply(Op { key: key(Kind::Door, 7), on: true, by: 0 });
        host.apply(Op { key: key(Kind::Light, 7), on: false, by: 0 });
        host.apply(Op { key: key(Kind::Relay, 7), on: true, by: 0 });
        let parts = host.batches(600);
        assert!(parts.len() > 1);
        let mut late = ObjectTable::default();
        for part in &parts {
            assert!(serde_json::to_vec(part).unwrap().len() <= 600, "each part fits");
            for op in parse_event(part).expect("a batch") {
                late.apply(op);
            }
        }
        assert_eq!((late.len(), late.relays()), (host.len(), 1));
        assert!(late.is_on(key(Kind::Door, 7)) && !late.is_on(key(Kind::Light, 7)) && late.is_on(key(Kind::Item, 199)));
        assert!(ObjectTable::default().batches(600).is_empty());
    }

    #[test]
    fn an_older_build_ignores_every_object_message() {
        // what an older guest does with a world event it does not know: no lasting-state key (so it is not held), no pipe
        // event, no notice
        let op = Op { key: key(Kind::Relay, 1), on: true, by: 2 };
        let mut batch = Map::new();
        batch.insert("what".into(), json!(BATCH));
        batch.insert("o".into(), json!([["r", 0, 1, 2, 1]]));
        for ev in [event(op), batch] {
            assert!(crate::net::hunter_events::state_key(&ev).is_none());
            assert!(crate::net::hunter_events::notice(&ev).is_none());
            assert!(crate::net::valve_events::parse_turn(&ev).is_none() && crate::net::valve_events::parse_burst(&ev).is_none());
        }
        // what an older host does with the request: it is no valve or breach request, so nothing is queued
        let ask = request(op.key, true);
        assert!(crate::net::valve_events::parse_request(2, &ask).is_none());
        assert!(crate::net::valve_events::parse_burst_request(2, &ask).is_none());
    }
}
