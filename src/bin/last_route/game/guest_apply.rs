//! What the host's game does with what a guest (or the net layer) reports (Godot `_apply_hunter_event`, `_resolve_taser`,
//! `_apply_valve_event`, `_receive_snapshot`). The net session only validates and queues these (`net/world_sync.rs`);
//! the app hands them here each frame (`app/mod.rs`).
//!
//! * [`Game::apply_guest_event`]: a guest's taser intent, its escape from a capture, its caught report, a blocked
//!   capture, or the net layer's `capture_timeout`. The host alone resolves them; a guest game ignores them.
//! * [`Game::apply_relay_snapshot`]: the sabotaged relay count in a late joiner's snapshot.
//! * [`Game::apply_pipe_event`] and [`Game::host_turn_valve`]: the sewer valve and pipe burst events.
//! * [`Game::sync_world_objects`]: the world objects of a room (relays, batteries, floor items and coins taken, room
//!   doors, room lights; `net/world_objects.rs`). The host decides each guest's request and publishes it; a guest
//!   applies what the host says, to its window now and to every window it builds later.
//!
//! Guests never change hunter state here: every method that touches a hunter returns at once on a display only game.
//! A remote runner is never the local runner, so nothing here feeds the danger readout, the heartbeat or the chase cue.
//! The tuning (stun time, taser range and angle, cooldowns) is the difficulty profile and `config`, not this file.

use super::interact::Actor;
use super::{guest_gate, Game};
use crate::config::{INTERACT_REACH, TASER_HALF_ANGLE, TASER_RANGE};
use crate::content::Content;
use crate::hunters::brain::Brain;
use crate::net::valve_events::{self, ValveRequest};
use crate::net::world_objects::{self, Kind, ObjKey, ObjectTable, Op};
use crate::net::world_sync::NetSession;
use crate::world::facility::DoorToggle;
use crate::world::Cell;
use serde_json::{Map, Value};

/// How far past the interact reach a guest may stand from an object it asks for (cells): the host reads the guest's
/// place from its last sample, which is a moment old, and a door's leaf is wider than its threshold cell.
const OBJECT_REACH_SLACK: f32 = 3.0;

/// What the host does with a guest's object request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ObjectDecision {
    /// Accepted: the change is the room's, every runner is told.
    Publish(Op),
    /// Refused, and the guest already shows it (a door it swung ahead of the answer): it is told how the object stands.
    Tell(Op),
    /// Refused; the guest shows nothing it should not.
    Drop,
}

/// A number field of an event, read as f32 when it is finite.
fn num(ev: &Map<String, Value>, key: &str) -> Option<f32> {
    ev.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32)
}

impl Game {
    /// A window cell position as an absolute one (what a guest sends the host).
    pub fn window_to_abs(origin: (i32, i32), at: (f32, f32)) -> (f32, f32) {
        (at.0 + origin.0 as f32, at.1 + origin.1 as f32)
    }

    /// An absolute cell position (a guest's sample) in this window's cells.
    pub fn abs_to_window(origin: (i32, i32), at: (f32, f32)) -> (f32, f32) {
        (at.0 - origin.0 as f32, at.1 - origin.1 as f32)
    }

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
        // a dedicated room: a report that closes a capture frees the guest for the hunters again (after the grace)
        if matches!(ev.get("what").and_then(Value::as_str), Some("escaped" | "caught" | "capture_blocked" | "capture_timeout")) {
            if let Some(peer) = ev.get("peer_id").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()) {
                self.guest_released(peer);
            }
        }
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
        let prof = self.prof();
        let axe = ev.get("axe").and_then(Value::as_bool).unwrap_or(false);
        let Some(i) = named.or_else(nearest) else {
            // the hunter that held the guest is on another floor than this host's runner (a resident floor): it is
            // stunned all the same, where it stands. Only a hunter the host named itself at the capture is looked up
            // there, never one found by a place.
            let id = ev.get("hunter_id").or_else(|| ev.get("entity_id")).and_then(Value::as_str);
            if let Some(h) = id.and_then(|id| self.resident_hunters.iter_all_mut().find(|h| h.def.id == id)) {
                h.apply_stun(prof.escape_stun * if axe { 2.0 } else { 1.0 }, false);
            }
            return;
        };
        self.hunters[i].apply_stun(prof.escape_stun * if axe { 2.0 } else { 1.0 }, false);
        if let Some(p) = at.filter(|_| self.hunters[i].level == self.fac.level) {
            let fac = &self.fac;
            self.hunters[i].enter_search_from(p, 1.8, fac);
        }
    }

    /// The level a world object of this window is keyed by: the floor label every runner on this floor sends as its
    /// sample's depth, turned back into a level.
    pub(super) fn objects_level(&self) -> i32 {
        1i32.saturating_sub(self.depth)
    }

    /// The room-wide name of the object of `kind` at window cell `cell` (`net/world_objects.rs`).
    pub(super) fn object_key(&self, kind: Kind, cell: Cell) -> ObjKey {
        let o = self.window_origin();
        ObjKey { kind, level: self.objects_level(), x: cell.x.saturating_add(o.0), y: cell.y.saturating_add(o.1) }
    }

    /// The window cell an object key names, when it is on this level and inside this window.
    fn object_cell(&self, key: ObjKey) -> Option<Cell> {
        if key.level != self.objects_level() {
            return None;
        }
        let o = self.window_origin();
        let (x, y) = (key.x.checked_sub(o.0)?, key.y.checked_sub(o.1)?);
        (x >= 0 && y >= 0 && x < self.fac.cols && y < self.fac.rows).then(|| Cell::new(x, y))
    }

    /// Guest of a host that decides the world objects: a pickup, a door or a light switch is asked for, not changed
    /// here alone. False for a host, a solo run, and a guest of an older host (which keeps its objects to itself).
    pub(super) fn asks_host_for_objects(&self) -> bool {
        self.remote_hunters && self.hunter_out.host_decides_objects
    }

    /// Guest: queues the request to the host (sent by [`Game::sync_world_objects`]).
    pub(super) fn ask_object(&mut self, kind: Kind, cell: Cell, on: bool) {
        let key = self.object_key(kind, cell);
        self.hunter_out.ask_object(key, on);
    }

    /// Host (or a solo run, where nobody takes the queue's word): an object of this window changed. `by_player` names
    /// this runner as the one who took it. A guest publishes nothing.
    pub(super) fn publish_object(&mut self, kind: Kind, cell: Cell, on: bool, by_player: bool) {
        if guest_gate::is_display_only(self) {
            return;
        }
        let by = if by_player { self.revive_net.me } else { 0 };
        let key = self.object_key(kind, cell);
        self.hunter_out.push_state(world_objects::event(Op { key, on, by }));
    }

    fn pickup_at(&self, c: Cell) -> Option<usize> {
        self.pickups.iter().position(|p| !p.taken && p.pos.0.floor() as i32 == c.x && p.pos.1.floor() as i32 == c.y)
    }

    /// Sets the registered room door at `cell` open or shut, whoever stands near (a leaf still never swings shut through
    /// a body: `Facility::update_doors`). Nothing for a cell that is no door of this window.
    fn set_door(&mut self, cell: Cell, open: bool) {
        if self.fac.door_centre(cell).is_some() && self.fac.door_is_open(cell) != open {
            self.fac.toggle_door(cell, &[]);
        }
    }

    /// Once per frame while a room is open, after the net tick, host and guest alike: the whole exchange of the world
    /// objects between this game and the room.
    ///
    /// * Guest: sends the requests the runner's interactions queued, applies what the host published since the last
    ///   frame (the item is this runner's when the host names it), and takes the team's relay count.
    /// * Host: decides the guests' requests from each guest's own sample and publishes what it accepts.
    /// * Both: when the window is another one than last time (a chunk border, a floor, the first frame of a route), the
    ///   whole table is applied to it, so a chunk streamed after the fact shows what happened there.
    /// * Host with a guest in the room, twice a second: the doors a hunter pulled open are published.
    ///
    /// Costs nothing per frame beyond four empty queue takes; the passes over the window run on a window change and on
    /// the door check only.
    pub fn sync_world_objects(&mut self, n: &mut NetSession, content: &Content) {
        let host = n.is_host();
        self.hunter_out.host_decides_objects = !host && n.host_decides_objects();
        if host {
            for (peer, key, on) in n.take_object_requests() {
                let Some((x, y, depth)) = n.guest_position(peer) else { continue };
                let known = n.objects().get(key);
                match self.host_object_request(key, on, peer, (x, y), depth, known, content) {
                    ObjectDecision::Publish(op) => {
                        n.publish_object(op);
                    }
                    ObjectDecision::Tell(op) => {
                        n.send_object_to(peer, op);
                    }
                    ObjectDecision::Drop => {}
                }
            }
        } else {
            for (key, on) in self.hunter_out.take_object_asks() {
                n.request_object(key, on);
            }
            for op in n.take_object_ops() {
                self.apply_object_op(op, content);
            }
            // every relay a runner of the room recovered counts for this runner too (the lift and the readouts read it)
            let team = i32::try_from(n.objects().relays()).unwrap_or(i32::MAX);
            if self.relays_collected < team {
                self.relays_collected = team;
            }
        }
        let o = self.window_origin();
        let moved = n.objects_window_changed((o.0, o.1, self.depth));
        if moved {
            for op in self.apply_object_table(n.objects(), host, content) {
                n.publish_object(op);
            }
        }
        if n.door_check_due() || (moved && host) {
            for op in self.doors_unlike_table(n.objects()) {
                n.publish_object(op);
            }
        }
    }

    /// Host: decides a guest's request to make object `key` `on`. `at` and `depth` are the guest's last sample
    /// (`NetSession::guest_position`, absolute cells), never a place the request carries; `known` is what the room's
    /// table holds for the object. A guest must stand at the object, on its level. While this window holds the object
    /// it must exist here and be free; an object in a chunk this host has not streamed is taken on the guest's word (the
    /// table still refuses a second take). A relay counts for the team and releases its hunters either way.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn host_object_request(&mut self, key: ObjKey, on: bool, peer: u32, at: (f32, f32), depth: i32, known: Option<bool>, content: &Content) -> ObjectDecision {
        if guest_gate::is_display_only(self) {
            return ObjectDecision::Drop;
        }
        let guest_level = 1i64 - i64::from(depth);
        let centre = (key.x as f32 + 0.5, key.y as f32 + 0.5);
        if (i64::from(key.level) - guest_level).abs() > 1 || (at.0 - centre.0).hypot(at.1 - centre.1) > INTERACT_REACH + OBJECT_REACH_SLACK {
            return ObjectDecision::Drop;
        }
        let here = self.object_cell(key);
        if key.kind.is_take() {
            // already taken: the first runner has it, and the asker was told so when it went
            if !on || known.is_some() {
                return ObjectDecision::Drop;
            }
            let stood = here.map(|_| Self::abs_to_window(self.window_origin(), at));
            if let Some(c) = here {
                let free = match key.kind {
                    Kind::Relay => match self.fac.relays.iter().position(|r| *r == c).filter(|i| self.relays_taken.get(*i) == Some(&false)) {
                        Some(i) => {
                            self.relay_recovered_by_guest(Some(i), stood, content);
                            true
                        }
                        None => false,
                    },
                    Kind::Battery => match self.fac.batteries.iter().position(|b| *b == c).filter(|i| self.batteries_taken.get(*i) == Some(&false)) {
                        Some(i) => {
                            self.batteries_taken[i] = true;
                            true
                        }
                        None => false,
                    },
                    Kind::Coin => match self.fix.coins.iter().position(|k| *k == c).filter(|i| self.coins_taken.get(*i) == Some(&false)) {
                        Some(i) => {
                            self.coins_taken[i] = true;
                            true
                        }
                        None => false,
                    },
                    _ => match self.pickup_at(c) {
                        Some(i) => {
                            self.pickups[i].taken = true;
                            true
                        }
                        None => false,
                    },
                };
                if !free {
                    return ObjectDecision::Drop;
                }
            } else if key.kind == Kind::Relay {
                self.relay_recovered_by_guest(None, None, content);
            }
            return ObjectDecision::Publish(Op { key, on: true, by: peer });
        }
        if key.kind == Kind::Door {
            if let Some(c) = here {
                if self.fac.door_centre(c).is_none() {
                    return ObjectDecision::Drop;
                }
                if self.fac.door_is_open(c) != on && !matches!(self.swing_door(c), Some(DoorToggle::Opened | DoorToggle::Closed)) {
                    // a body stands in the leaf's sweep: the door stays as it is, and the guest that swung it is told
                    return ObjectDecision::Tell(Op { key, on: self.fac.door_is_open(c), by: 0 });
                }
            }
            return ObjectDecision::Publish(Op { key, on, by: 0 });
        }
        if let Some(c) = here {
            match self.fac.room_lights.iter().position(|l| l.switch_cell == c) {
                Some(i) => self.set_room_light(i, on, content),
                None => return ObjectDecision::Drop,
            }
        }
        ObjectDecision::Publish(Op { key, on, by: 0 })
    }

    /// Guest: applies one change the host published. An object outside this window changes nothing now (the table keeps
    /// it for when its chunk is built). A take the host gave to this runner (`by` is its own peer id) puts the item in
    /// its pack; anyone else's only removes it from the floor.
    pub(super) fn apply_object_op(&mut self, op: Op, content: &Content) {
        let Some(c) = self.object_cell(op.key) else { return };
        let mine = op.by != 0 && op.by == self.revive_net.me;
        if mine {
            // the host's answer to this guest's own request (pack_ask_busy): the pack is up to date again
            self.pack_ask = None;
        }
        match op.key.kind {
            Kind::Relay => {
                let Some(i) = self.fac.relays.iter().position(|r| *r == c) else { return };
                if self.relays_taken.get(i) != Some(&false) {
                    return;
                }
                if mine {
                    self.claim_relay_now(i, Actor::Player, content);
                } else {
                    self.relays_taken[i] = true;
                    // a replay (nobody named) is old news: only a relay recovered just now is announced
                    if op.by != 0 {
                        self.announce("A RUNNER RECOVERED A RELAY");
                    }
                }
            }
            Kind::Battery => {
                let Some(i) = self.fac.batteries.iter().position(|b| *b == c) else { return };
                if self.batteries_taken.get(i) != Some(&false) {
                    return;
                }
                if mine {
                    self.take_battery_now(i, Actor::Player);
                }
                self.batteries_taken[i] = true;
            }
            Kind::Coin => {
                let Some(i) = self.fix.coins.iter().position(|k| *k == c) else { return };
                if self.coins_taken.get(i) != Some(&false) {
                    return;
                }
                if mine {
                    self.take_coin_now(i);
                }
                self.coins_taken[i] = true;
            }
            Kind::Item => {
                let Some(i) = self.pickup_at(c) else { return };
                if mine {
                    self.take_pickup_now(i, Actor::Player);
                }
                self.pickups[i].taken = true;
            }
            Kind::Door => self.set_door(c, op.on),
            Kind::Light => {
                if let Some(i) = self.fac.room_lights.iter().position(|l| l.switch_cell == c) {
                    self.set_room_light(i, op.on, content);
                }
            }
        }
    }

    /// Applies the room's whole table to this window (called when the window is a new one): what is taken is gone, the
    /// doors the table holds open stand open, the lights it holds dark are dark. A guest's doors follow the table both
    /// ways. The host is the source of the table, so what its own window knows and the table does not (a pickup taken
    /// before this session held the room, a door a hunter just pulled) is not undone: the takes are returned for the
    /// caller to publish, and the doors are published by the door check that follows.
    pub(super) fn apply_object_table(&mut self, table: &ObjectTable, host: bool, content: &Content) -> Vec<Op> {
        let mut publish: Vec<Op> = Vec::new();
        if table.is_empty() && !host {
            return publish;
        }
        let (level, o) = (self.objects_level(), self.window_origin());
        let key = |kind: Kind, c: Cell| ObjKey { kind, level, x: c.x.saturating_add(o.0), y: c.y.saturating_add(o.1) };
        let mut merge = |kind: Kind, c: Cell, taken: &mut bool| {
            let k = key(kind, c);
            match (table.get(k).is_some(), *taken) {
                (true, false) => *taken = true,
                (false, true) if host => publish.push(Op { key: k, on: true, by: 0 }),
                _ => {}
            }
        };
        for (c, taken) in self.fac.relays.iter().zip(self.relays_taken.iter_mut()) {
            merge(Kind::Relay, *c, taken);
        }
        for (c, taken) in self.fac.batteries.iter().zip(self.batteries_taken.iter_mut()) {
            merge(Kind::Battery, *c, taken);
        }
        for (c, taken) in self.fix.coins.iter().zip(self.coins_taken.iter_mut()) {
            merge(Kind::Coin, *c, taken);
        }
        for p in self.pickups.iter_mut() {
            let c = Cell::new(p.pos.0.floor() as i32, p.pos.1.floor() as i32);
            merge(Kind::Item, c, &mut p.taken);
        }
        let mut swing: Vec<(Cell, bool)> = Vec::new();
        for (i, _) in self.fac.doors() {
            let c = self.fac.props[i].cell;
            if self.fac.door_centre(c).is_none() {
                continue;
            }
            let (want, is) = (table.is_on(key(Kind::Door, c)), self.fac.door_is_open(c));
            if want != is && (want || !host) {
                swing.push((c, want));
            }
        }
        for (c, open) in swing {
            self.set_door(c, open);
        }
        let mut lights = false;
        for (l, lit) in self.fac.room_lights.iter().zip(self.lights_on.iter_mut()) {
            let want = table.is_on(key(Kind::Light, l.switch_cell));
            if *lit != want {
                *lit = want;
                lights = true;
            }
        }
        if lights {
            self.world.lamps = crate::render::world_mesh::lamps_near(&self.fac, &self.lights_on, content, Some(self.p.pos));
        }
        publish
    }

    /// Host: the registered room doors of this window whose state is not what the table says (a hunter pulled one open,
    /// a bot pressed one), as the changes to publish. One pass over the props, no allocation when nothing differs.
    pub(super) fn doors_unlike_table(&self, table: &ObjectTable) -> Vec<Op> {
        let mut out: Vec<Op> = Vec::new();
        for (i, _) in self.fac.doors() {
            let c = self.fac.props[i].cell;
            if self.fac.door_centre(c).is_none() {
                continue;
            }
            let (key, open) = (self.object_key(Kind::Door, c), self.fac.door_is_open(c));
            if table.is_on(key) != open {
                out.push(Op { key, on: open, by: 0 });
            }
        }
        out
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
        // a late joiner's replay says how long ago the turn or the burst happened (optional `age`): the water then stands
        // where the host's does, instead of starting to fill when the replay arrives
        let now = self.time - crate::net::hunter_events::event_age(ev);
        if let Some(turn) = valve_events::parse_turn(ev) {
            self.pipes.apply(&turn, now);
            self.sync_sewer_gates();
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
        let storey_here = depth.checked_neg()?;
        let rules = super::pipes::rules();
        let storey = super::pipes::parse_valve_id(&req.valve_id, rules.valve.max_id_length)?.1;
        let prop = self.fac.props.iter().find(|p| p.id == req.valve_id && p.ptype == rules.valve.prop_type)?;
        let valve_at = prop.cell.center();
        let (effect, gate) = super::interact::valve_keys(prop);
        let q = super::pipes::TurnAt {
            valve_id: &req.valve_id,
            effect: &effect,
            gate: &gate,
            valve_at,
            valve_storey: storey,
            at,
            storey: storey_here,
            by_player: false,
        };
        let ev = self.pipes.turn(q).ok()?;
        self.pipes.apply(&ev, self.time);
        self.sync_sewer_gates();
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

    #[test]
    fn window_and_absolute_cells_convert_both_ways() {
        let origin = (-96, 64);
        assert_eq!(Game::window_to_abs(origin, (10.5, 3.5)), (-85.5, 67.5));
        assert_eq!(Game::abs_to_window(origin, (-85.5, 67.5)), (10.5, 3.5));
        let at = (12.25, 40.75);
        assert_eq!(Game::abs_to_window(origin, Game::window_to_abs(origin, at)), at);
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

    #[test]
    fn a_replayed_burst_starts_as_long_ago_as_the_host_says_and_one_with_no_age_starts_now() {
        let mut g = run();
        g.time = 100.0;
        let old = event(valve_events::BURST, vec![("flow_id", json!("BRK:0:-1:0:9")), ("age", json!(30.0))]);
        assert!(g.apply_pipe_event(&old));
        assert_eq!(g.pipes.run("BRK:0:-1:0:9").map(|r| r.opened_at), Some(70.0), "the water has run for half a minute already");
        let fresh = event(valve_events::BURST, vec![("flow_id", json!("BRK:0:-1:0:10"))]);
        assert!(g.apply_pipe_event(&fresh));
        assert_eq!(g.pipes.run("BRK:0:-1:0:10").map(|r| r.opened_at), Some(100.0), "an older host sends no age: it starts now");
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
            sewer: None,
            crawl_under: false,
        });
    }

    #[test]
    fn a_guest_cannot_turn_a_valve_it_names_at_a_place_it_picks_or_one_the_host_does_not_have() {
        let mut g = run();
        add_valve(&mut g, "SEWER:0:-1:0:PROP:3", (10, 10));
        let min = ValveRequest { peer: 2, valve_id: "SEWER:0:-1:0:PROP:3".into(), effect: String::new(), gate: String::new(), valve_at: (0.0, 0.0) };
        assert!(g.host_turn_valve(&min, (10.5, 10.5), i32::MIN).is_none(), "i32::MIN depth is refused, not a panic");
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

/// The world objects of a room (issue 9 of docs/MULTIPLAYER_SYNC_AUDIT.md): the host decides, every runner applies.
#[cfg(test)]
mod world_object_tests {
    use super::*;
    use crate::game::items::Item;
    use crate::game::remote_targets::RemoteTarget;
    use crate::game::FloorPickup;
    use crate::world::catalog::Catalog;

    fn game(room: &str) -> (Game, Content) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, room, 0, &content, &catalog);
        (g, content)
    }

    /// A guest of a host that decides the objects, with peer id 5.
    fn guest(room: &str) -> (Game, Content) {
        let (mut g, content) = game(room);
        g.remote_hunters = true;
        g.hunter_out.host_decides_objects = true;
        g.revive_net.me = 5;
        (g, content)
    }

    /// Puts a stim pack on the floor where the runner stands; returns its index, its key and that place in absolute cells.
    fn drop_stim(g: &mut Game) -> (usize, ObjKey, (f32, f32)) {
        let cell = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.pickups.push(FloorPickup { pos: cell.center(), item: Item::Stim, n: 1, taken: false });
        let key = g.object_key(Kind::Item, cell);
        (g.pickups.len() - 1, key, Game::window_to_abs(g.window_origin(), cell.center()))
    }

    fn stims(g: &Game) -> i32 {
        g.slots.iter().flatten().filter(|s| s.item == Item::Stim).map(|s| s.n).sum()
    }

    #[test]
    fn the_host_gives_a_pickup_to_the_first_guest_that_asks_and_nobody_can_take_it_twice() {
        let (mut host, content) = game("ROOM-OBJ-1");
        let (i, key, at) = drop_stim(&mut host);
        let depth = host.depth;
        let before = stims(&host);
        // a guest that does not stand at it, or stands on another floor, is refused
        assert_eq!(host.host_object_request(key, true, 2, (at.0 + 40.0, at.1), depth, None, &content), ObjectDecision::Drop);
        assert_eq!(host.host_object_request(key, true, 2, at, depth + 3, None, &content), ObjectDecision::Drop);
        assert!(!host.pickups[i].taken);
        // the first guest at it gets it: the room is told who has it, and the host's own pack is not touched
        let got = host.host_object_request(key, true, 2, at, depth, None, &content);
        assert_eq!(got, ObjectDecision::Publish(Op { key, on: true, by: 2 }));
        assert!(host.pickups[i].taken, "it is gone from the host's floor");
        assert_eq!(stims(&host), before, "the item is the guest's");
        // the second guest, a moment later, finds nothing to take: by the host's floor, and by the room's table
        assert_eq!(host.host_object_request(key, true, 3, at, depth, None, &content), ObjectDecision::Drop);
        assert_eq!(host.host_object_request(key, true, 3, at, depth, Some(true), &content), ObjectDecision::Drop);
        // a pickup that is not in the host's world at all is not handed out on a guest's word while its chunk is here
        let nothing = ObjKey { x: key.x + 1, ..key };
        assert_eq!(host.host_object_request(nothing, true, 3, at, depth, None, &content), ObjectDecision::Drop);
        // a guest game never decides anything
        let (mut g, content) = guest("ROOM-OBJ-1");
        let (_, key, at) = drop_stim(&mut g);
        assert_eq!(g.host_object_request(key, true, 2, at, g.depth, None, &content), ObjectDecision::Drop);
    }

    #[test]
    fn a_guest_asks_for_a_pickup_and_has_it_only_when_the_host_names_it() {
        let (mut g, content) = guest("ROOM-OBJ-2");
        let (i, key, _) = drop_stim(&mut g);
        let before = stims(&g);
        assert!(!g.take_pickup(i, Actor::Player), "nothing is taken on the guest's own word");
        assert!(!g.pickups[i].taken && stims(&g) == before);
        assert_eq!(g.hunter_out.take_object_asks(), vec![(key, true)], "the host is asked");
        // the host gave it to someone else: it is gone from this floor and not in this pack
        g.apply_object_op(Op { key, on: true, by: 9 }, &content);
        assert!(g.pickups[i].taken && stims(&g) == before);
        // the same request answered in this runner's favour puts it in the pack
        let (mut g, content) = guest("ROOM-OBJ-2");
        let (i, key, _) = drop_stim(&mut g);
        g.apply_object_op(Op { key, on: true, by: 5 }, &content);
        assert!(g.pickups[i].taken);
        assert_eq!(stims(&g), before + 1, "the host named this runner");
        // a replay of it changes nothing more
        g.apply_object_op(Op { key, on: true, by: 5 }, &content);
        assert_eq!(stims(&g), before + 1);
    }

    #[test]
    fn a_guest_waits_for_the_host_before_it_asks_again_for_the_pack() {
        let (mut g, content) = guest("ROOM-OBJ-6");
        let (i, key, _) = drop_stim(&mut g);
        assert!(!g.pack_ask_busy(), "nothing is waiting at first");
        assert!(!g.take_pickup(i, Actor::Player));
        assert_eq!(g.hunter_out.take_object_asks(), vec![(key, true)]);
        // the answer is still on its way: another press asks for nothing, so the host cannot grant two takes into one slot
        assert!(g.pack_ask_busy());
        assert!(!g.take_pickup(i, Actor::Player));
        assert!(g.hunter_out.take_object_asks().is_empty(), "no second ask while one waits");
        // the host's answer (to this runner) ends the wait
        let me = g.revive_net.me;
        g.apply_object_op(Op { key, on: true, by: me.max(1) }, &content);
        assert!(!g.pack_ask_busy(), "the answer ends the wait");
    }

    #[test]
    fn a_new_guest_in_an_older_hosts_room_keeps_its_pickups_to_itself_as_before() {
        let (mut g, _) = guest("ROOM-OBJ-3");
        // the older host's room message carries no sync key
        g.hunter_out.host_decides_objects = false;
        let (i, _, _) = drop_stim(&mut g);
        let before = stims(&g);
        assert!(g.take_pickup(i, Actor::Player), "nobody answers a request there, so the runner takes it itself");
        assert!(g.pickups[i].taken);
        assert_eq!(stims(&g), before + 1);
        assert!(g.hunter_out.take_object_asks().is_empty() && g.hunter_out.take().is_empty(), "and nothing is sent");
    }

    #[test]
    fn a_relay_a_guest_recovers_counts_for_the_host_and_is_this_runners_only_when_the_host_says_so() {
        let (mut host, content) = game("ROOM-OBJ-4");
        let Some(cell) = host.fac.relays.first().copied() else { return };
        let key = host.object_key(Kind::Relay, cell);
        let at = Game::window_to_abs(host.window_origin(), cell.center());
        let count = host.relays_collected;
        let got = host.host_object_request(key, true, 2, at, host.depth, None, &content);
        assert_eq!(got, ObjectDecision::Publish(Op { key, on: true, by: 2 }));
        assert!(host.relays_taken[0]);
        assert_eq!(host.relays_collected, count + 1, "the team's relay releases the host's hunters like its own");
        assert_eq!(host.host_object_request(key, true, 3, at, host.depth, None, &content), ObjectDecision::Drop, "taken once");
        // the host's own relay is published with the host as the one who has it
        let (mut host, content) = game("ROOM-OBJ-4");
        host.revive_net.me = 1;
        host.hunter_out.take();
        assert!(host.claim_relay(0, Actor::Player, &content));
        let sent = host.hunter_out.take();
        let ops: Vec<Op> = sent.iter().filter_map(world_objects::parse_event).flatten().collect();
        assert_eq!(ops, vec![Op { key, on: true, by: 1 }]);
        // the guest who asked
        let (mut g, content) = guest("ROOM-OBJ-4");
        let (count, score) = (g.relays_collected, g.score);
        assert!(!g.claim_relay(0, Actor::Player, &content), "a guest's press is a request");
        assert!(!g.relays_taken[0] && g.relays_collected == count);
        assert_eq!(g.hunter_out.take_object_asks(), vec![(key, true)]);
        g.apply_object_op(Op { key, on: true, by: 5 }, &content);
        assert!(g.relays_taken[0]);
        assert_eq!(g.relays_collected, count + 1);
        assert!(g.score > score, "the relay is this runner's");
        // and a guest who only watched: the relay is gone, the score is not its own
        let (mut g, content) = guest("ROOM-OBJ-4");
        let score = g.score;
        g.apply_object_op(Op { key, on: true, by: 2 }, &content);
        assert!(g.relays_taken[0]);
        assert_eq!(g.score, score);
    }

    #[test]
    fn a_door_a_guest_asks_for_swings_on_the_host_and_a_blocked_one_is_answered_as_it_stands() {
        let (mut host, content) = game("ROOM-OBJ-5");
        host.fac.update_doors(&[], &[], 0.0);
        let shut: Vec<Cell> = host.fac.doors().map(|(i, _)| host.fac.props[i].cell).filter(|c| !host.fac.door_is_open(*c)).collect();
        // a door away from the host's own body, so only the guest decides whether its leaf is blocked
        let Some(cell) = shut.into_iter().find(|c| crate::game::pdist(host.fac.door_centre(*c).unwrap_or(host.p.pos), host.p.pos) > 3.0) else { return };
        let key = host.object_key(Kind::Door, cell);
        let centre = host.fac.door_centre(cell).expect("registered");
        let at = Game::window_to_abs(host.window_origin(), centre);
        let depth = host.depth;
        assert!(host.doors_unlike_table(&ObjectTable::default()).iter().all(|op| op.key != key), "shut, as the table says");
        assert_eq!(host.host_object_request(key, true, 2, at, depth, None, &content), ObjectDecision::Publish(Op { key, on: true, by: 0 }));
        assert!(host.fac.door_is_open(cell), "the host's door swung open");
        // a door a hunter pulled open is found by the door check the same way
        assert!(host.doors_unlike_table(&ObjectTable::default()).contains(&Op { key, on: true, by: 0 }));
        let mut table = ObjectTable::default();
        table.apply(Op { key, on: true, by: 0 });
        assert!(host.doors_unlike_table(&table).iter().all(|op| op.key != key), "published once, not again");
        // the guest stands in the doorway and asks to shut it: the leaf would swing through it, so the door stays open
        // and the guest (which swung its own copy shut already) is told how it stands
        let level = host.runner_level();
        host.remote_targets = vec![RemoteTarget::new(2, at, host.p.y(), level, 0.0, false, false, false, false, true)];
        assert_eq!(host.host_object_request(key, false, 2, at, depth, Some(true), &content), ObjectDecision::Tell(Op { key, on: true, by: 0 }));
        assert!(host.fac.door_is_open(cell));
        // it steps out of the sweep: now it shuts
        host.remote_targets.clear();
        assert_eq!(host.host_object_request(key, false, 2, at, depth, Some(true), &content), ObjectDecision::Publish(Op { key, on: false, by: 0 }));
        assert!(!host.fac.door_is_open(cell));
        // a guest shows what the host says, whatever it showed before
        let (mut g, content) = guest("ROOM-OBJ-5");
        g.fac.update_doors(&[], &[], 0.0);
        g.apply_object_op(Op { key, on: true, by: 0 }, &content);
        assert!(g.fac.door_is_open(cell));
        g.apply_object_op(Op { key, on: false, by: 0 }, &content);
        assert!(!g.fac.door_is_open(cell));
        // a guest's own press swings its door at once and asks the host
        assert!(g.press_door(cell).is_some());
        let asks = g.hunter_out.take_object_asks();
        assert_eq!(asks, vec![(key, g.fac.door_is_open(cell))]);
    }

    #[test]
    fn a_chunk_built_after_the_fact_shows_what_happened_there() {
        let (mut g, content) = guest("ROOM-OBJ-6");
        let (i, item, _) = drop_stim(&mut g);
        let mut table = ObjectTable::default();
        table.apply(Op { key: item, on: true, by: 0 });
        let relay = g.fac.relays.first().map(|c| g.object_key(Kind::Relay, *c));
        if let Some(relay) = relay {
            table.apply(Op { key: relay, on: true, by: 0 });
        }
        let light = g.fac.room_lights.first().map(|l| g.object_key(Kind::Light, l.switch_cell));
        if let Some(light) = light {
            table.apply(Op { key: light, on: false, by: 0 });
        }
        // an object in a chunk this runner has not streamed (another floor here) changes nothing now, and breaks nothing
        let elsewhere = ObjKey { level: item.level - 2, ..item };
        g.apply_object_op(Op { key: elsewhere, on: true, by: 0 }, &content);
        assert!(!g.pickups[i].taken);
        // the window is built (or is a new one): the table is applied to it
        let before = stims(&g);
        assert!(g.apply_object_table(&table, false, &content).is_empty(), "a guest publishes nothing");
        assert!(g.pickups[i].taken && stims(&g) == before, "gone, and nobody is given it again");
        if relay.is_some() {
            assert!(g.relays_taken[0]);
        }
        if light.is_some() {
            assert!(!g.lights_on[0], "the light the room switched off is dark here too");
        }
        // the host is the source: what its window knows and the table does not is published, not undone
        let (mut host, content) = game("ROOM-OBJ-6");
        let (i, item, _) = drop_stim(&mut host);
        host.pickups[i].taken = true;
        let told = host.apply_object_table(&ObjectTable::default(), true, &content);
        assert!(told.contains(&Op { key: item, on: true, by: 0 }), "{told:?}");
        assert!(host.pickups[i].taken);
    }
}
