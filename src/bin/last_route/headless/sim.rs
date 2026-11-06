//! The headless run: the same `Game` the window plays, stepped at a fixed timestep with
//! scripted `Controls`, plus JSON views of its state and an event log derived frame by frame.
//! Needs no window, GPU or audio device (the sounds are the silent `Sounds::default()`).

use crate::audio::Sounds;
use crate::config::*;
use crate::content::Content;
use crate::game::{Controls, Focus, Game};
use crate::hunters::brain::State;
use crate::world::catalog::Catalog;
use crate::world::Cell;
use macroquad::math::vec2;
use serde_json::{json, Value};

pub const DEFAULT_DT: f32 = 1.0 / 60.0;
const EVENT_LIMIT: usize = 2000;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Finite,
    Infinite,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Finite => "finite",
            Mode::Infinite => "infinite",
        }
    }
}

pub fn parse_difficulty(s: &str) -> Option<Difficulty> {
    match s.to_ascii_lowercase().as_str() {
        "relaxed" | "easy" => Some(Difficulty::Relaxed),
        "standard" | "normal" | "default" => Some(Difficulty::Standard),
        "hard" => Some(Difficulty::Hard),
        "nightmare" => Some(Difficulty::Nightmare),
        _ => None,
    }
}

pub fn parse_mode(s: &str) -> Option<Mode> {
    match s.to_ascii_lowercase().as_str() {
        "finite" | "route" | "default" => Some(Mode::Finite),
        "infinite" | "endless" | "streamed" => Some(Mode::Infinite),
        _ => None,
    }
}

/// Inputs an agent holds down until it changes them (movement, sprint, crouch, hold-[E], a
/// steady look rate), and one-frame presses queued for the next frame.
#[derive(Default, Clone)]
pub struct Held {
    pub fwd: f32,
    pub strafe: f32,
    pub sprint: bool,
    pub crouch: bool,
    pub interact_held: bool,
    /// Jump held: hops again each time the feet land.
    pub jump: bool,
    /// Radians per second of turn (x = yaw right, y = pitch down).
    pub look_rate: (f32, f32),
}

/// Snapshot of the fields the event log watches.
#[derive(Clone, Default)]
struct Watch {
    depth: i32,
    level: Option<i32>,
    relays_collected: i32,
    coins: i32,
    score: i32,
    hidden: Option<usize>,
    in_vent: bool,
    escape: bool,
    report: bool,
    hunters: Vec<(String, bool, State)>,
    message: String,
    message_t: f32,
    toast: String,
    toast_t: f32,
    slots: Vec<Option<(&'static str, i32)>>,
    flashlight: bool,
    stairs_center: Option<(i32, i32)>,
}

pub struct Sim {
    pub content: Content,
    pub catalog: Catalog,
    pub sounds: Sounds,
    pub game: Option<Game>,
    pub mode: Mode,
    pub dt: f32,
    /// Frames stepped since the run started.
    pub frame: u64,
    pub held: Held,
    pub pending: Controls,
    pub events: Vec<Value>,
    pub event_seq: u64,
    /// Events up to this sequence number were already returned by `events` (since = cursor).
    pub event_cursor: u64,
    watch: Watch,
    /// The run's statistics (the original's _run_statistics keys the port can fill).
    pub chases_started: u32,
    pub encountered: std::collections::BTreeSet<String>,
    pub visited_levels: std::collections::BTreeSet<i32>,
    pub items_collected: u32,
}

fn state_id(s: State) -> &'static str {
    match s {
        State::Idle => "idle",
        State::Wander => "wander",
        State::Investigate => "investigate",
        State::Chase => "chase",
        State::Search => "search",
        State::Peek => "peek",
    }
}

pub fn r2(v: f32) -> f64 {
    ((v as f64) * 1000.0).round() / 1000.0
}

fn pt(p: (f32, f32)) -> Value {
    json!([r2(p.0), r2(p.1)])
}

fn cell(c: Cell) -> Value {
    json!([c.x, c.y])
}

pub fn focus_json(g: &Game, f: Focus) -> Value {
    let (kind, index, at): (&str, Option<usize>, (f32, f32)) = match f {
        Focus::Relay(i) => ("relay", Some(i), g.fac.relays[i].center()),
        Focus::Battery(i) => ("battery", Some(i), g.fac.batteries[i].center()),
        Focus::Pickup(i) => ("pickup", Some(i), g.pickups[i].pos),
        Focus::Coin(i) => ("coin", Some(i), g.fix.coins[i].center()),
        Focus::Locker(i) => ("locker", Some(i), g.fac.hiding[i].cell.center()),
        Focus::Safe => (
            "safe_zone",
            None,
            g.fac.safe_zone.map(|c| c.center()).unwrap_or(g.p.pos),
        ),
        Focus::Lift => ("lift", None, g.fac.exit.center()),
        Focus::Ladder(i) => ("ladder", Some(i), g.fac.stairs[i].cell.center()),
        Focus::LightSwitch(i) => (
            "light_switch",
            Some(i),
            g.fac.room_lights[i].switch_cell.center(),
        ),
        Focus::Vent(i) => ("vent", Some(i), g.fix.vents[i].center()),
        Focus::Shop(i) => ("shop", Some(i), g.hunters[i].pos),
        Focus::Door(i) => (
            "door",
            Some(i),
            g.fac.doors().find(|d| d.0 == i).map_or(g.p.pos, |d| d.1),
        ),
        Focus::Valve(i) => ("valve", Some(i), g.fac.props[i].cell.center()),
    };
    json!({ "kind": kind, "index": index, "pos": pt(at), "distance": r2(crate::game::pdist(at, g.p.pos)) })
}

impl Sim {
    pub fn new() -> Sim {
        // never touch the player's real checkpoint from an agent or a test
        crate::app::checkpoint::ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        Sim::with(Content::load(), Catalog::load(), Sounds::default())
    }

    pub fn with(content: Content, catalog: Catalog, sounds: Sounds) -> Sim {
        crate::app::checkpoint::ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        Sim {
            content,
            catalog,
            sounds,
            game: None,
            // Keep newly-created simulators aligned with the player-facing default.
            mode: Mode::Infinite,
            dt: DEFAULT_DT,
            frame: 0,
            held: Held::default(),
            pending: Controls::default(),
            events: vec![],
            event_seq: 0,
            event_cursor: 0,
            watch: Watch::default(),
            chases_started: 0,
            encountered: Default::default(),
            visited_levels: Default::default(),
            items_collected: 0,
        }
    }

    /// Starts a run. `depth` (finite) or `level` (infinite, 0 = top) picks where.
    pub fn new_run(&mut self, diff: Difficulty, seed: u32, mode: Mode, depth: i32, level: i32) {
        let g = match mode {
            Mode::Infinite => {
                // a numeric seed is the room text it has always stood for (route::room_text)
                let text = crate::world::route::room_text(seed);
                Game::new_infinite_sim(diff, seed, &text, level, &self.content, &self.catalog)
            }
            Mode::Finite => {
                let mut g = Game::new_sim(diff, seed, &self.content, &self.catalog);
                if depth > 1 {
                    g.depth = depth;
                    g.start_floor_sim(&self.content, &self.catalog);
                }
                g
            }
        };
        self.install(g, diff, seed, mode);
    }

    /// A run on the Infinite route for a room seed text (the text a host's `Start` message carries), the way a client
    /// enters it: `route::run_seed_of_text` is the run seed and the text the world seed. `no_local_runner` makes it a
    /// dedicated room's game (game/remote_targets.rs): no runner plays, the guests are the targets.
    pub fn new_room_run(&mut self, diff: Difficulty, room_text: &str, no_local_runner: bool) {
        let seed = crate::world::route::run_seed_of_text(room_text);
        let mut g = Game::new_infinite_sim(diff, seed, room_text, 0, &self.content, &self.catalog);
        g.no_local_runner = no_local_runner;
        self.install(g, diff, seed, Mode::Infinite);
    }

    fn install(&mut self, mut g: Game, diff: Difficulty, seed: u32, mode: Mode) {
        g.fade = 0.0;
        self.game = Some(g);
        self.mode = mode;
        self.frame = 0;
        self.held = Held::default();
        self.pending = Controls::default();
        self.events.clear();
        self.event_cursor = self.event_seq;
        self.chases_started = 0;
        self.encountered.clear();
        self.visited_levels.clear();
        self.items_collected = 0;
        let lv = self
            .game
            .as_ref()
            .map(|g| g.inf.as_ref().map(|i| i.level).unwrap_or(1 - g.depth))
            .unwrap_or(0);
        self.visited_levels.insert(lv);
        self.watch = self.capture();
        self.push("run_started", json!({ "seed": seed, "difficulty": diff.id(), "mode": mode.id(), "depth": self.game.as_ref().unwrap().depth }));
    }

    pub fn push(&mut self, kind: &str, data: Value) {
        self.event_seq += 1;
        let mut e = json!({ "seq": self.event_seq, "frame": self.frame, "type": kind });
        if let (Some(o), Value::Object(d)) = (e.as_object_mut(), data) {
            for (k, v) in d {
                o.insert(k, v);
            }
        }
        self.events.push(e);
        if self.events.len() > EVENT_LIMIT {
            let cut = self.events.len() - EVENT_LIMIT;
            self.events.drain(..cut);
        }
    }

    /// The controls for the next frame: the held inputs plus any queued presses.
    fn controls(&mut self) -> Controls {
        let mut c = std::mem::take(&mut self.pending);
        c.fwd = self.held.fwd.clamp(-1.0, 1.0);
        c.strafe = self.held.strafe.clamp(-1.0, 1.0);
        c.sprint |= self.held.sprint;
        c.crouch |= self.held.crouch;
        c.jump |= self.held.jump;
        c.interact_held |= self.held.interact_held || c.interact;
        c.look += vec2(self.held.look_rate.0, self.held.look_rate.1) * self.dt;
        c
    }

    /// Steps one frame with explicit controls (bots and tests) or the held ones (`None`).
    pub fn step_with(&mut self, c: Option<Controls>) {
        if self.game.is_none() {
            return;
        }
        let c = match c {
            Some(c) => c,
            None => self.controls(),
        };
        let dt = self.dt;
        let g = self.game.as_mut().unwrap();
        g.step(dt, &c, &self.content, &self.catalog, &self.sounds);
        self.frame += 1;
        self.diff_events();
    }

    /// One frame of a dedicated room (`Game::no_local_runner`): no controls, and none of the per-frame event log, which
    /// watches the local runner. The room's own outputs are read from the game (catches, the hunter frame).
    pub fn step_room(&mut self) {
        let Some(g) = self.game.as_mut() else { return };
        g.step(self.dt, &Controls::default(), &self.content, &self.catalog, &self.sounds);
        self.frame += 1;
    }

    /// Steps `n` frames with the held inputs; stops early when the run ends (if asked).
    pub fn step(&mut self, n: u64, stop_on_report: bool) -> u64 {
        let mut done = 0;
        for _ in 0..n {
            if stop_on_report
                && self
                    .game
                    .as_ref()
                    .map(|g| g.report.is_some())
                    .unwrap_or(true)
            {
                break;
            }
            self.step_with(None);
            done += 1;
        }
        done
    }

    fn capture(&self) -> Watch {
        let Some(g) = self.game.as_ref() else {
            return Watch::default();
        };
        Watch {
            depth: g.depth,
            level: g.inf.as_ref().map(|i| i.level),
            relays_collected: g.relays_collected,
            coins: g.coins,
            score: g.score,
            hidden: g.p.hidden,
            in_vent: g.p.in_vent,
            escape: g.escape.is_some(),
            report: g.report.is_some(),
            hunters: g
                .hunters
                .iter()
                .map(|h| (h.def.id.clone(), h.active, h.state))
                .collect(),
            message: g.message.clone(),
            message_t: g.message_t,
            toast: g.toast.clone(),
            toast_t: g.toast_t,
            slots: g
                .slots
                .iter()
                .map(|s| s.map(|s| (s.item.label(), s.n)))
                .collect(),
            flashlight: g.p.flashlight_on,
            stairs_center: g.inf.as_ref().map(|i| i.center),
        }
    }

    pub(crate) fn diff_events(&mut self) {
        let now = self.capture();
        let was = std::mem::replace(&mut self.watch, now.clone());
        let g = self.game.as_ref().unwrap();
        let mut out: Vec<(&str, Value)> = vec![];
        let floor_changed = now.depth != was.depth || now.level != was.level;
        if floor_changed {
            out.push(("floor_changed", json!({ "depth": now.depth, "level": now.level, "level_seed": g.level_seed(), "relays_total": g.fac.relays.len() })));
        } else if now.stairs_center != was.stairs_center {
            out.push(("window_recentred", json!({ "center": now.stairs_center.map(|c| json!([c.0, c.1])), "chunks_visited": g.chunks_visited() })));
        }
        if now.relays_collected > was.relays_collected {
            out.push(("relay_collected", json!({ "relays_collected": now.relays_collected, "relays_total": g.fac.relays.len(), "score": now.score })));
        }
        if now.coins != was.coins {
            out.push((
                "coins_changed",
                json!({ "coins": now.coins, "delta": now.coins - was.coins }),
            ));
        }
        if now.slots != was.slots && !floor_changed {
            out.push(("hotbar_changed", json!({ "slots": now.slots.iter().map(|s| s.map(|s| json!({ "item": s.0, "count": s.1 }))).collect::<Vec<_>>() })));
        }
        if now.flashlight != was.flashlight {
            out.push(("flashlight", json!({ "on": now.flashlight })));
        }
        if now.hidden != was.hidden {
            match now.hidden {
                Some(i) => out.push(("hid", json!({ "locker": i }))),
                None => out.push(("left_hiding", json!({ "locker": was.hidden }))),
            }
        }
        if now.in_vent != was.in_vent {
            out.push((
                if now.in_vent {
                    "entered_vent"
                } else {
                    "left_vent"
                },
                json!({}),
            ));
        }
        if !floor_changed && now.hunters.len() == was.hunters.len() {
            for (i, (a, b)) in was.hunters.iter().zip(now.hunters.iter()).enumerate() {
                if !a.1 && b.1 {
                    out.push(("hunter_activated", json!({ "index": i, "id": b.0 })));
                }
                if a.2 != b.2 {
                    out.push(("hunter_state", json!({ "index": i, "id": b.0, "from": state_id(a.2), "to": state_id(b.2) })));
                }
            }
        } else if now.hunters.len() > was.hunters.len() && !floor_changed {
            for (i, h) in now.hunters.iter().enumerate().skip(was.hunters.len()) {
                out.push((
                    "hunter_spawned",
                    json!({ "index": i, "id": h.0, "active": h.1 }),
                ));
            }
        }
        if now.escape && !was.escape {
            let h = g
                .escape
                .as_ref()
                .map(|e| g.hunters[e.hunter].def.id.clone());
            out.push(("captured", json!({ "hunter": h, "presses_needed": ESCAPE_PRESSES, "seconds": ESCAPE_DURATION })));
        }
        if was.escape && !now.escape && !now.report {
            out.push(("escaped", json!({})));
        }
        if now.message_t > was.message_t || (now.message != was.message && !now.message.is_empty())
        {
            out.push(("message", json!({ "text": now.message })));
        }
        if now.toast_t > was.toast_t || (now.toast != was.toast && !now.toast.is_empty()) {
            out.push(("toast", json!({ "text": now.toast })));
        }
        if now.report && !was.report {
            out.push(("run_ended", self.report_json()));
        }
        // statistics
        self.visited_levels
            .insert(now.level.unwrap_or(1 - now.depth));
        let filled = |v: &Vec<Option<(&'static str, i32)>>| {
            v.iter()
                .filter(|s| s.is_some())
                .map(|s| s.unwrap().1.max(1) as u32)
                .sum::<u32>()
        };
        if !floor_changed
            && now.slots.len() == was.slots.len()
            && filled(&now.slots) > filled(&was.slots)
            && now.slots.iter().filter(|s| s.is_some()).count()
                >= was.slots.iter().filter(|s| s.is_some()).count()
        {
            self.items_collected += 1;
        }
        for (k, v) in out.iter() {
            if *k == "hunter_state" && v["to"] == "chase" {
                self.chases_started += 1;
            }
        }
        for h in g
            .hunters
            .iter()
            .filter(|h| h.active && !h.gimmick.hidden_from_view())
        {
            if !self.encountered.contains(&h.def.id)
                && crate::game::pdist(h.pos, g.p.pos) < 20.0
                && g.fac.has_line_of_sight(h.pos, g.p.pos)
            {
                self.encountered.insert(h.def.id.clone());
                out.push(("hunter_encounter", json!({ "hunter": h.def.id, "state": state_id(h.state), "dist": r2(crate::game::pdist(h.pos, g.p.pos)) })));
            }
        }
        for (k, v) in out {
            self.push(k, v);
        }
    }

    // -- views --------------------------------------------------------------------------------

    pub fn run_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return json!({ "active": false });
        };
        json!({
            "active": g.report.is_none(),
            "ended": g.report.is_some(),
            "seed": g.run_seed,
            "difficulty": g.difficulty.id(),
            "mode": self.mode.id(),
            "depth": g.depth,
            "level": g.inf.as_ref().map(|i| i.level),
            "level_seed": g.level_seed(),
            "frame": self.frame,
            "time": r2(g.time),
            "dt": r2(self.dt),
            "score": g.score,
            "coins": g.coins,
            "relays_collected": g.relays_collected,
            "relays_total": g.fac.relays.len(),
            "lift_ready": g.fac.has_lift && g.relays_collected >= g.fac.relays.len() as i32,
            "escape": g.escape.as_ref().map(|e| json!({ "remaining": r2(e.remaining), "progress": e.progress, "needed": ESCAPE_PRESSES, "hunter": g.hunters[e.hunter].def.id })),
            "shop_open": g.shop_open(),
            "message": if g.message_t > 0.0 { Value::from(g.message.clone()) } else { Value::Null },
            "toast": if g.toast_t > 0.0 { Value::from(g.toast.clone()) } else { Value::Null },
            "danger": r2(g.danger()),
            "music_box_left": r2(g.music_box_left),
            "chunks_visited": g.chunks_visited(),
            "window_center": g.inf.as_ref().map(|i| json!([i.center.0, i.center.1])),
        })
    }

    pub fn player_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        let p = &g.p;
        let (fx, fy) = p.forward2();
        let cellv = (p.pos.0.floor() as i32, p.pos.1.floor() as i32);
        let room = g.fac.room_at(p.pos.0 as f64, p.pos.1 as f64).map(|i| {
            let r = &g.fac.rooms[i];
            json!({ "index": i, "type": r.rtype, "label": r.label, "region": r.region })
        });
        json!({
            "pos": pt(p.pos),
            "cell": [cellv.0, cellv.1],
            "yaw": r2(p.yaw),
            "pitch": r2(p.pitch),
            "forward": pt((fx, fy)),
            "feet_y": r2(p.feet_y),
            "y": r2(p.y() + p.feet_y),
            "climbing": g.ladder.is_some(),
            "stamina": r2(p.stamina),
            "exhausted": p.exhausted,
            "lamp": r2(p.lamp),
            "flashlight_on": p.flashlight_on,
            "crouching": p.crouching,
            "sprinting": p.sprinting,
            "moving": p.moving,
            "noise": r2(p.noise),
            "hidden": p.hidden,
            "hide_lock": r2(p.hide_lock),
            "in_vent": p.in_vent,
            "stun": r2(p.stun),
            "pesticide": r2(p.pesticide),
            "stim": { "inject": r2(p.stim_inject.max(0.0)), "boost": r2(p.stim_boost.max(0.0)), "crash": r2(p.stim_crash.max(0.0)) },
            "walked": r2(p.walked),
            "in_safe_zone": g.in_safe_zone(),
            "room": room,
            "local_climate": r2(g.local_climate()),
        })
    }

    pub fn meters_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        let m = &g.meters;
        json!({
            "stamina": r2(g.p.stamina),
            "lamp": r2(g.p.lamp),
            "exposure": r2(m.exposure),
            "noise": r2(m.noise_meter),
            "temperature": r2(m.temperature),
            "ambient": r2(m.ambient),
            "heat_locked": m.heat_locked,
            "overheating": m.overheating(),
            "freezing": m.freezing(),
        })
    }

    pub fn hotbar_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        json!({
            "selected": g.selected,
            "slots": g.slots.iter().map(|s| s.map(|s| json!({ "item": item_id(s.item), "label": s.item.label(), "count": s.n }))).collect::<Vec<_>>(),
            "taser_cooldown": r2(g.taser_cd),
            "melee_cooldown": r2(g.melee_cd),
            // the hunter specialty bar, slots 6 to 9 (game/specialty.rs): shown while the owning hunter is on the floor
            "specialty": {
                "selected": g.specialty.selected.map(|i| i + crate::game::specialty::FIRST_NUMBER as usize),
                "slots": crate::game::specialty::rules().slots.iter().enumerate().map(|(i, d)| json!({
                    "number": d.number,
                    "id": d.id,
                    "visible": g.specialty.visible[i],
                    "item": g.specialty.items[i].map(|it| match it {
                        crate::game::specialty::SpecItem::MusicBox => "music_box",
                        crate::game::specialty::SpecItem::Temperature(crate::game::specialty::Mode::Heat) => "heat",
                        crate::game::specialty::SpecItem::Temperature(crate::game::specialty::Mode::Cold) => "cold",
                    }),
                })).collect::<Vec<_>>(),
            },
        })
    }

    pub fn npcs_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        Value::Array(g.npcs.iter().map(|n| {
            json!({
                "id": n.id,
                "name": n.name,
                "slot": n.slot,
                "level": n.level,
                "pos": pt(n.at),
                "distance": r2(crate::game::pdist(n.at, g.p.pos)),
                "smart": n.smart,
                "joined": n.joined,
                "walking": n.walking,
            })
        }).collect())
    }

    pub fn hunters_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        Value::Array(g.hunters.iter().enumerate().map(|(i, h)| {
            let d = crate::game::pdist(h.pos, g.p.pos);
            json!({
                "index": i,
                "id": h.def.id,
                "name": h.def.name,
                "tier": h.def.tier,
                "active": h.active,
                "state": state_id(h.state),
                "pos": pt(h.pos),
                "facing": pt(h.facing),
                "distance": r2(d),
                "visible": g.fac.has_line_of_sight(h.pos, g.p.pos) && !h.gimmick.hidden_from_view(),
                "stun": r2(h.stun),
                "slow": r2(h.slow_timer),
                "speed": r2(h.moving_speed),
                "last_known": h.last_known.map(pt),
                "taser_immune": h.def.taser_immune,
                "melee_immune": h.def.melee_immune,
            })
        }).collect())
    }

    pub fn relays_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        Value::Array(g.fac.relays.iter().enumerate().map(|(i, c)| json!({
            "index": i, "cell": cell(*c), "pos": pt(c.center()), "taken": g.relays_taken[i],
            "distance": r2(crate::game::pdist(c.center(), g.p.pos)),
        })).collect())
    }

    pub fn focus_value(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        match g.focus {
            Some(f) => {
                let mut v = focus_json(g, f);
                v["prompt"] = g.focus_prompt().map(Value::from).unwrap_or(Value::Null);
                v
            }
            None => Value::Null,
        }
    }

    /// Everything interactable on the floor (positions in cells; 1 cell = 1 m).
    pub fn objects_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        let d = |p: (f32, f32)| r2(crate::game::pdist(p, g.p.pos));
        json!({
            "start": cell(g.fac.start),
            "exit": cell(g.fac.exit),
            "has_lift": g.fac.has_lift,
            "safe_zone": g.fac.safe_zone.map(cell),
            "hunter_spawn": cell(g.fac.hunter_spawn),
            "relays": self.relays_json(),
            "batteries": g.fac.batteries.iter().enumerate().map(|(i, c)| json!({ "index": i, "cell": cell(*c), "taken": g.batteries_taken[i], "distance": d(c.center()) })).collect::<Vec<_>>(),
            "pickups": g.pickups.iter().enumerate().map(|(i, p)| json!({ "index": i, "item": item_id(p.item), "pos": pt(p.pos), "count": p.n, "taken": p.taken, "distance": d(p.pos) })).collect::<Vec<_>>(),
            "coins": g.fix.coins.iter().enumerate().map(|(i, c)| json!({ "index": i, "cell": cell(*c), "taken": g.coins_taken[i], "distance": d(c.center()) })).collect::<Vec<_>>(),
            "lockers": g.fac.hiding.iter().enumerate().map(|(i, h)| json!({ "index": i, "cell": cell(h.cell), "facing": [h.facing.0, h.facing.1], "disabled": g.hiding_disabled[i], "distance": d(h.cell.center()) })).collect::<Vec<_>>(),
            "stairs": g.fac.stairs.iter().enumerate().map(|(i, s)| json!({ "index": i, "cell": cell(s.cell), "dir": s.dir, "run": [s.run.0, s.run.1], "shape": format!("{:?}", s.shape), "kind": format!("{:?}", s.kind), "clockwise": s.clockwise, "distance": d(s.cell.center()) })).collect::<Vec<_>>(),
            "catwalk_links": g.fac.catwalk_links.iter().enumerate().map(|(i, l)| json!({ "index": i, "cell": cell(l.cell), "run": [l.run.0, l.run.1], "deck_height": l.deck_height, "distance": d(l.cell.center()) })).collect::<Vec<_>>(),
            "light_switches": g.fac.room_lights.iter().enumerate().map(|(i, l)| json!({ "index": i, "room": l.room, "cell": cell(l.switch_cell), "on": g.lights_on[i] })).collect::<Vec<_>>(),
            "music_box": g
                .pickups
                .iter()
                .find(|p| !p.taken && p.item == crate::game::Item::MusicBox)
                .map(|p| cell(Cell::new(p.pos.0.floor() as i32, p.pos.1.floor() as i32))),
            "vents": g.fix.vents.iter().map(|c| cell(*c)).collect::<Vec<_>>(),
            "doors": g.fac.doors().enumerate().map(|(i, (prop, at))| {
                let c = g.fac.props[prop].cell;
                json!({ "index": i, "prop": prop, "cell": cell(c), "open": r2(g.fac.door_open_amount(c)), "want_open": g.fac.door_is_open(c), "distance": d(at), "type": g.fac.props[prop].ptype, "span": g.fac.props[prop].door.map(|d| d.span), "pos": pt(at) })
            }).collect::<Vec<_>>(),
        })
    }

    pub fn rooms_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        Value::Array(g.fac.rooms.iter().enumerate().map(|(i, r)| json!({
            "index": i, "id": r.id, "type": r.rtype, "label": r.label, "region": r.region,
            "rect": [r.x, r.y, r.w, r.h], "center": cell(r.center), "landmark": r.landmark,
            "climate": g.climate.get(i).map(|c| r2(*c)),
        })).collect())
    }

    /// The floor as text rows: '#' wall, '.' floor, '?' undiscovered (when `discovered_only`),
    /// with markers P player, H active hunter, R relay, B battery, L locker, X exit/lift,
    /// S safe zone, < > stairs up/down. Optional window around the player.
    pub fn map_json(&self, discovered_only: bool, radius: Option<i32>) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        let f = &g.fac;
        let (px, py) = (g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        let (x0, y0, x1, y1) = match radius {
            Some(r) => (
                (px - r).max(0),
                (py - r).max(0),
                (px + r).min(f.cols - 1),
                (py + r).min(f.rows - 1),
            ),
            None => (0, 0, f.cols - 1, f.rows - 1),
        };
        let w = (x1 - x0 + 1) as usize;
        let mut grid: Vec<Vec<char>> = (y0..=y1)
            .map(|y| {
                (x0..=x1)
                    .map(|x| {
                        if discovered_only && !g.discovered[y as usize][x as usize] {
                            '?'
                        } else if f.is_wall_cell(x, y) {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect();
        let mut mark = |c: (i32, i32), ch: char, need_seen: bool| {
            if c.0 < x0 || c.1 < y0 || c.0 > x1 || c.1 > y1 {
                return;
            }
            if need_seen && discovered_only && !g.discovered[c.1 as usize][c.0 as usize] {
                return;
            }
            grid[(c.1 - y0) as usize][(c.0 - x0) as usize] = ch;
        };
        for (i, h) in f.hiding.iter().enumerate() {
            mark(
                (h.cell.x, h.cell.y),
                if g.hiding_disabled[i] { 'l' } else { 'L' },
                true,
            );
        }
        for (i, c) in f.batteries.iter().enumerate() {
            if !g.batteries_taken[i] {
                mark((c.x, c.y), 'B', true);
            }
        }
        for (i, c) in f.relays.iter().enumerate() {
            if !g.relays_taken[i] {
                mark((c.x, c.y), 'R', true);
            }
        }
        for s in &f.stairs {
            mark(
                (s.cell.x, s.cell.y),
                if s.dir < 0 { '>' } else { '<' },
                true,
            );
        }
        if let Some(s) = f.safe_zone {
            mark((s.x, s.y), 'S', true);
        }
        if f.has_lift {
            mark((f.exit.x, f.exit.y), 'X', true);
        }
        for h in g
            .hunters
            .iter()
            .filter(|h| h.active && !h.gimmick.hidden_from_view())
        {
            mark((h.pos.0.floor() as i32, h.pos.1.floor() as i32), 'H', false);
        }
        mark((px, py), 'P', false);
        let seen = g
            .discovered
            .iter()
            .map(|r| r.iter().filter(|d| **d).count())
            .sum::<usize>();
        json!({
            "origin": [x0, y0],
            "size": [w, grid.len()],
            "cols": f.cols,
            "rows": f.rows,
            "discovered_cells": seen,
            "legend": "# wall . floor ? unseen P you H hunter R relay B battery L locker (l burnt out) X lift S safe zone > stairs down < stairs up",
            "rows_text": grid.into_iter().map(|r| r.into_iter().collect::<String>()).collect::<Vec<_>>(),
        })
    }

    /// Discovered cells as [x, y] pairs (or a count only when `full` is false).
    pub fn discovered_json(&self, full: bool) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        let mut cells = vec![];
        let mut n = 0;
        for (y, row) in g.discovered.iter().enumerate() {
            for (x, d) in row.iter().enumerate() {
                if *d {
                    n += 1;
                    if full {
                        cells.push(json!([x, y]));
                    }
                }
            }
        }
        if full {
            json!({ "count": n, "cells": cells })
        } else {
            json!({ "count": n })
        }
    }

    pub fn report_json(&self) -> Value {
        let Some(g) = self.game.as_ref() else {
            return Value::Null;
        };
        match &g.report {
            Some(r) => json!({
                "ended": true,
                "hunter": r.hunter,
                "hunter_name": r.hunter_name,
                "cause": r.cause,
                "depth": r.depth,
                "score": r.score,
                "relays": r.relays,
                "coins": r.coins,
                "seed": r.seed,
                "difficulty": r.difficulty.id(),
                "frames": self.frame,
                "time": r2(g.time),
                "walked": r2(g.p.walked),
                "shots": g.shots,
                "hits": g.hits,
            }),
            None => json!({
                "ended": false,
                "depth": g.depth,
                "score": g.score,
                "relays": g.relays_collected,
                "coins": g.coins,
                "seed": g.run_seed,
                "difficulty": g.difficulty.id(),
                "frames": self.frame,
                "time": r2(g.time),
                "walked": r2(g.p.walked),
                "shots": g.shots,
                "hits": g.hits,
            }),
        }
    }

    pub fn state_json(&self) -> Value {
        if self.game.is_none() {
            return json!({ "run": { "active": false } });
        }
        json!({
            "run": self.run_json(),
            "player": self.player_json(),
            "meters": self.meters_json(),
            "hotbar": self.hotbar_json(),
            "hunters": self.hunters_json(),
            "relays": self.relays_json(),
            "focus": self.focus_value(),
        })
    }

    /// Events after `since` (a `seq`), or the ones not yet returned.
    pub fn take_events(&mut self, since: Option<u64>, peek: bool) -> Value {
        let from = since.unwrap_or(self.event_cursor);
        let out: Vec<Value> = self
            .events
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap_or(0) > from)
            .cloned()
            .collect();
        if !peek {
            self.event_cursor = self.event_seq;
        }
        Value::Array(out)
    }
}

pub fn item_id(i: crate::game::Item) -> &'static str {
    use crate::game::Item::*;
    match i {
        Flashlight => "flashlight",
        Taser => "taser",
        Crowbar => "crowbar",
        Axe => "axe",
        Battery => "battery",
        Stim => "stim",
        MusicBox => "music_box",
    }
}
