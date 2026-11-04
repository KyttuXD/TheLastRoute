//! The agent protocol: the Godot playtest bridge's commands (dev/playtest/playtest_bridge.gd,
//! PROTOCOL 1) with the same names and JSON shapes, plus port commands for direct state
//! queries. One JSON object per line in, one per line out; `id` is echoed; errors are
//! `{"ok": false, "error": "..."}`. See docs/AGENT_CLI.md.

use super::bot::{self, Bot};
use super::expect;
use super::sim::{focus_json, item_id, parse_difficulty, parse_mode, r2, Mode, Sim};
use crate::config::*;
use crate::game::{pdist, Focus, Item};
use crate::hunters::brain::State;
use macroquad::math::vec2;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const PROTOCOL: i64 = 1;
pub const MAX_STEP_FRAMES: u64 = 7200;
const MAX_UNTIL_FRAMES: u64 = 108000;

pub const COMMANDS: &[&str] = &[
    "hello",
    "observe",
    "act",
    "step",
    "start_run",
    "leave_run",
    "ui",
    "press",
    "action",
    "admin",
    "time_scale",
    "screenshot",
    "log",
    "quit",
    "state",
    "player",
    "meters",
    "hotbar",
    "hunters",
    "npcs",
    "relays",
    "focus",
    "objects",
    "rooms",
    "map",
    "discovered",
    "events",
    "report",
    "director",
    "nav",
    "look",
    "interact",
    "teleport",
    "goto",
    "step_until",
    "walk",
    "bot",
    "hash",
    "help",
];

pub const PULSES: &[&str] = &[
    "interact",
    "flashlight",
    "taser",
    "tool_use",
    "drop_item",
    "hotbar_previous",
    "hotbar_next",
    "jump",
    "slot_1",
    "slot_2",
    "slot_3",
    "slot_4",
    "slot_5",
    "shop_1",
    "shop_2",
    "shop_3",
    "shop_4",
    // accepted and ignored (menus the headless port has no screen for)
    "inventory",
    "toggle_map",
    "pause",
    "chat",
    "ui_accept",
    "ui_cancel",
    "ui_up",
    "ui_down",
    "ui_left",
    "ui_right",
];

/// Turns a seed given as a number or text (the original's seeds are words like ALPHA) into the
/// port's u32 run seed: numbers are used as-is, text is hashed (FNV-1a 32).
pub fn seed_from(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|u| u as u32)
            .or_else(|| n.as_i64().map(|i| i as u32)),
        Value::String(s) => Some(seed_text(s)),
        _ => None,
    }
}

pub fn seed_text(s: &str) -> u32 {
    if let Ok(n) = s.trim().parse::<u64>() {
        return n as u32;
    }
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h.max(1)
}

fn ok() -> Value {
    json!({ "ok": true })
}

pub fn err(msg: impl Into<String>) -> Value {
    json!({ "ok": false, "error": msg.into() })
}

fn f(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32)
}

fn pair_of(v: &Value) -> Option<(f32, f32)> {
    let a = v.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

fn pair(v: &Value, k: &str) -> Option<(f32, f32)> {
    pair_of(v.get(k)?)
}

fn state_upper(s: State) -> &'static str {
    match s {
        State::Idle => "IDLE",
        State::Wander => "WANDER",
        State::Investigate => "INVESTIGATE",
        State::Chase => "CHASE",
        State::Search => "SEARCH",
        State::Peek => "PEEK",
    }
}

/// RouteInteractable3D.Kind names for the port's focus kinds.
pub fn kind_name(f: Focus) -> &'static str {
    match f {
        Focus::Relay(_) => "RELAY",
        Focus::Battery(_) => "BATTERY",
        Focus::Pickup(_) => "TOOL_PICKUP",
        Focus::Coin(_) => "COIN",
        Focus::Locker(_) => "HIDING_SPOT",
        Focus::Safe => "SAFE_ZONE",
        Focus::Lift => "LIFT",
        Focus::Stairs(_) => "STAIRS",
        Focus::LightSwitch(_) => "LIGHT_SWITCH",
        Focus::Heater => "HEATER_STATION",
        Focus::MusicBox => "MUSIC_BOX",
        Focus::Vent(_) => "VENT",
        Focus::Shop(_) => "SHOP",
        Focus::Door(_) => "ROOM_DOOR",
        Focus::Valve(_) => "VALVE",
    }
}

fn parse_item(s: &str) -> Option<Item> {
    Some(match s.to_ascii_lowercase().as_str() {
        "flashlight" => Item::Flashlight,
        "taser" => Item::Taser,
        "crowbar" => Item::Crowbar,
        "axe" | "rescue_axe" => Item::Axe,
        "battery" => Item::Battery,
        "stim" | "stim_pack" => Item::Stim,
        _ => return None,
    })
}

pub struct Session {
    pub sim: Sim,
    pub bot: Option<Bot>,
    pub bot_seed: u32,
    pub seed_label: String,
    pub log: Vec<Value>,
    pub log_seq: u64,
    pub errors: u32,
    pub warnings: u32,
    pub quit: Option<i32>,
    /// A window draws the run (`--agent-window`): screenshots work.
    pub windowed: bool,
    /// A screenshot the window must take before replying (path).
    pub pending_shot: Option<String>,
    pub time_scale: f32,
    /// Window mode: advance the run in real time with the held inputs between commands.
    pub realtime: bool,
    pub last: Value,
}

impl Session {
    pub fn new(sim: Sim) -> Session {
        Session {
            sim,
            bot: None,
            bot_seed: 1,
            seed_label: String::new(),
            log: vec![],
            log_seq: 0,
            errors: 0,
            warnings: 0,
            quit: None,
            windowed: false,
            pending_shot: None,
            time_scale: 1.0,
            realtime: false,
            last: Value::Null,
        }
    }

    pub fn log_line(&mut self, kind: &str, text: &str) {
        self.log_seq += 1;
        match kind {
            "error" => self.errors += 1,
            "warning" => self.warnings += 1,
            _ => {}
        }
        let t: String = text.chars().take(400).collect();
        self.log
            .push(json!({ "seq": self.log_seq, "kind": kind, "text": t, "frame": self.sim.frame }));
        if self.log.len() > 500 {
            self.log.drain(..100);
        }
    }

    /// One request line -> one reply (always a JSON object); None when the reply is deferred
    /// (a window screenshot, answered after the next frame is drawn).
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let parsed: Value = match serde_json::from_str(line.trim().trim_start_matches('\u{feff}')) {
            Ok(v @ Value::Object(_)) => v,
            _ => return Some(err("not a JSON object")),
        };
        let id = parsed.get("id").cloned();
        let mut reply = self.handle_safe(&parsed);
        if reply.is_null() {
            return None;
        }
        if let (Some(id), Some(o)) = (id, reply.as_object_mut()) {
            o.insert("id".into(), id);
        }
        Some(reply)
    }

    /// `handle` with panics turned into an error reply and an "error" log line.
    pub fn handle_safe(&mut self, c: &Value) -> Value {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(c))) {
            Ok(r) => r,
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                self.log_line(
                    "error",
                    &format!("panic in '{}': {msg}", c["cmd"].as_str().unwrap_or("?")),
                );
                err(format!("internal error: {msg}"))
            }
        }
    }

    fn need_game(&self) -> Result<(), Value> {
        if self.sim.game.is_none() {
            Err(err("no active game"))
        } else {
            Ok(())
        }
    }

    pub fn handle(&mut self, c: &Value) -> Value {
        let name = c.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let r = self.dispatch(name, c);
        self.last = r.clone();
        r
    }

    fn section(&self, key: &str, v: Value) -> Value {
        if self.sim.game.is_none() {
            return err("no active game");
        }
        let mut o = Map::new();
        o.insert("ok".into(), Value::Bool(true));
        o.insert(key.into(), v);
        Value::Object(o)
    }

    fn dispatch(&mut self, name: &str, c: &Value) -> Value {
        match name {
            "hello" => json!({
                "ok": true, "protocol": PROTOCOL, "games": ["last_route"], "game": "last_route",
                "version": env!("CARGO_PKG_VERSION"), "revision": "rust-port", "debug_run": true,
                "fixed_fps": (1.0 / self.sim.dt).round() as i64, "physics_hz": (1.0 / self.sim.dt).round() as i64,
                "port": "rust", "headless": !self.windowed, "commands": COMMANDS, "pulses": PULSES,
            }),
            "help" => {
                json!({ "ok": true, "commands": COMMANDS, "pulses": PULSES, "docs": "docs/AGENT_CLI.md" })
            }
            "start_run" => self.start_run(c),
            "leave_run" => {
                self.sim.game = None;
                self.bot = None;
                ok()
            }
            "observe" => json!({ "ok": true, "obs": self.observe() }),
            "act" => {
                let act = c.get("act").unwrap_or(c);
                match self.apply_act(act) {
                    Ok(()) => ok(),
                    Err(e) => err(e),
                }
            }
            "action" => {
                let n = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                match self.apply_act(&json!({ "pulse": [n] })) {
                    Ok(()) => ok(),
                    Err(e) => err(e),
                }
            }
            "step" => self.step(c),
            "step_until" => self.step_until(c),
            "walk" => self.walk(c),
            "ui" => {
                json!({ "ok": true, "ui": { "buttons": [], "focus": "" }, "scene": self.scene() })
            }
            "press" => err(format!(
                "no visible button '{}'",
                c.get("name")
                    .or(c.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            )),
            "admin" => self.admin(c),
            "time_scale" => {
                self.time_scale = f(c, "scale").unwrap_or(1.0).clamp(0.05, 16.0);
                json!({ "ok": true, "scale": self.time_scale, "note": if self.windowed { "applies to realtime mode" } else { "headless steps are fixed; frames are what step counts" } })
            }
            "screenshot" => {
                if !self.windowed {
                    return err(
                        "no rendered image (headless run: use a windowed run on the test machine)",
                    );
                }
                let path = c
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("shots/playtest_{}.png", self.sim.frame));
                self.pending_shot = Some(path);
                Value::Null
            }
            "log" => {
                let since = c.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
                let lines: Vec<Value> = self
                    .log
                    .iter()
                    .filter(|l| l["seq"].as_u64().unwrap_or(0) > since)
                    .cloned()
                    .collect();
                json!({ "ok": true, "lines": lines, "seq": self.log_seq, "errors": self.errors, "warnings": self.warnings })
            }
            "quit" => {
                self.quit = Some(c.get("code").and_then(|v| v.as_i64()).unwrap_or(0) as i32);
                ok()
            }
            "state" => {
                if self.sim.game.is_none() {
                    return err("no active game");
                }
                let s = match c.get("sections") {
                    Some(Value::Array(want)) => {
                        let keep: HashSet<String> = want
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        self.probe(&keep)
                    }
                    _ => self.sim.state_json(),
                };
                json!({ "ok": true, "state": s })
            }
            "player" => self.section("player", self.sim.player_json()),
            "meters" => self.section("meters", self.sim.meters_json()),
            "hotbar" => self.section("hotbar", self.sim.hotbar_json()),
            "hunters" => self.section("hunters", self.sim.hunters_json()),
            "npcs" => self.section("npcs", self.sim.npcs_json()),
            "relays" => self.section("relays", self.sim.relays_json()),
            "focus" => self.section("focus", self.sim.focus_value()),
            "objects" => self.section("objects", self.sim.objects_json()),
            "rooms" => self.section("rooms", self.sim.rooms_json()),
            "report" => self.section("report", self.sim.report_json()),
            "director" => self.section("director", self.director_json()),
            "nav" => self.section("nav", self.nav_json()),
            "map" => {
                let disc = c
                    .get("discovered_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let radius = c
                    .get("radius")
                    .and_then(|v| v.as_i64())
                    .map(|r| r.clamp(1, 200) as i32);
                self.section("map", self.sim.map_json(disc, radius))
            }
            "discovered" => {
                let full = c.get("cells").and_then(|v| v.as_bool()).unwrap_or(false);
                self.section("discovered", self.sim.discovered_json(full))
            }
            "events" => {
                let since = c.get("since").and_then(|v| v.as_u64());
                let peek = c.get("peek").and_then(|v| v.as_bool()).unwrap_or(false);
                let ev = self.sim.take_events(since, peek);
                json!({ "ok": true, "events": ev, "seq": self.sim.event_seq })
            }
            "hash" => {
                if self.sim.game.is_none() {
                    return err("no active game");
                }
                json!({ "ok": true, "hash": self.state_hash(), "frame": self.sim.frame })
            }
            "look" => self.look(c),
            "interact" => self.interact(c),
            "teleport" => self.teleport(c),
            "goto" => self.goto(c),
            "bot" => self.bot_cmd(c),
            other => err(format!("unknown command '{other}'")),
        }
    }

    fn scene(&self) -> &'static str {
        match &self.sim.game {
            Some(g) if g.report.is_some() => "run_report",
            Some(g) if g.is_infinite() => "infinite_route",
            Some(_) => "main_3d",
            None => "main_menu",
        }
    }

    fn start_run(&mut self, c: &Value) -> Value {
        let game = c
            .get("game")
            .and_then(|v| v.as_str())
            .unwrap_or("last_route");
        if game != "last_route" {
            return err(format!("no playtest adapter for '{game}'"));
        }
        if c.get("online").and_then(|v| v.as_bool()).unwrap_or(false) {
            return err("the Rust port has no online play");
        }
        let diff = match c.get("difficulty").and_then(|v| v.as_str()) {
            Some(d) => match parse_difficulty(d) {
                Some(d) => d,
                None => {
                    return err(format!(
                        "unknown difficulty '{d}' (relaxed, standard, hard)"
                    ))
                }
            },
            None => Difficulty::Standard,
        };
        let mode = match c.get("mode").and_then(|v| v.as_str()) {
            Some(m) => match parse_mode(m) {
                Some(m) => m,
                None => return err(format!("unknown mode '{m}' (finite, infinite)")),
            },
            // The Infinite route is the primary map; finite remains opt-in for compatibility tooling.
            None => Mode::Infinite,
        };
        let seed = match c.get("seed") {
            Some(v) => match seed_from(v) {
                Some(s) => s,
                None => return err("seed must be a number or a string"),
            },
            None => 12345,
        };
        self.seed_label = match c.get("seed") {
            Some(Value::String(s)) => s.clone(),
            _ => seed.to_string(),
        };
        if let Some(fps) = c.get("fixed_fps").and_then(|v| v.as_f64()) {
            self.sim.dt = 1.0 / (fps.clamp(10.0, 240.0) as f32);
        }
        let depth = c.get("depth").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as i32;
        let level = c.get("level").and_then(|v| v.as_i64()).unwrap_or(0).min(0) as i32;
        self.sim.new_run(diff, seed, mode, depth, level);
        self.bot_seed = c.get("bot_seed").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        self.bot = None;
        if let Some(g) = self.sim.game.as_mut() {
            g.admin.god = c.get("god").and_then(|v| v.as_bool()).unwrap_or(false);
            g.admin.no_target = c
                .get("no_target")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        }
        json!({ "ok": true, "seed": seed, "seed_text": self.seed_label, "difficulty": diff.id(), "mode": mode.id(), "obs": self.observe() })
    }

    /// The bridge's act: move [strafe, forward], look [turn_right, look_down] (sticks, -1..1,
    /// held until changed), hold {sprint, crouch, jump, interact}, pulse [actions] (one frame).
    /// Port extras: look_delta [radians_right, radians_down] for the next frame, slot n (1-5).
    pub fn apply_act(&mut self, a: &Value) -> Result<(), String> {
        if !a.is_object() {
            return Err("act must be an object".into());
        }
        if let Some(m) = a.get("move") {
            let (x, y) = pair_of(m).ok_or("move must be [strafe_right, forward]")?;
            self.sim.held.strafe = x.clamp(-1.0, 1.0);
            self.sim.held.fwd = y.clamp(-1.0, 1.0);
        }
        if let Some(l) = a.get("look") {
            let (x, y) = pair_of(l).ok_or("look must be [turn_right, look_down]")?;
            self.sim.held.look_rate = (
                x.clamp(-1.0, 1.0) * KEY_LOOK_RADIANS_PER_SECOND,
                y.clamp(-1.0, 1.0) * KEY_LOOK_RADIANS_PER_SECOND,
            );
        }
        if let Some(l) = a.get("look_delta") {
            let (x, y) = pair_of(l).ok_or("look_delta must be [radians_right, radians_down]")?;
            self.sim.pending.look += vec2(x, y);
        }
        if let Some(h) = a.get("hold") {
            let Some(h) = h.as_object() else {
                return Err("hold must be an object".into());
            };
            for (k, v) in h {
                let on = v.as_bool().unwrap_or(false);
                match k.as_str() {
                    "sprint" => self.sim.held.sprint = on,
                    "crouch" => self.sim.held.crouch = on,
                    "jump" => self.sim.held.jump = on,
                    "interact" => self.sim.held.interact_held = on,
                    other => {
                        return Err(format!(
                            "unknown hold '{other}' (sprint, crouch, jump, interact)"
                        ))
                    }
                }
            }
        }
        if let Some(n) = a.get("slot").and_then(|v| v.as_i64()) {
            self.sim.pending.slot = Some((n.clamp(1, HOTBAR_SIZE as i64) - 1) as usize);
        }
        if let Some(p) = a.get("pulse") {
            let list: Vec<&str> = match p {
                Value::Array(v) => v.iter().filter_map(|x| x.as_str()).collect(),
                Value::String(s) => vec![s.as_str()],
                _ => return Err("pulse must be a list of action names".into()),
            };
            for name in list {
                let c = &mut self.sim.pending;
                match name {
                    "interact" => c.interact = true,
                    "flashlight" => c.flashlight = true,
                    "taser" => c.taser = true,
                    "tool_use" | "use" | "use_item" => c.use_item = true,
                    "drop_item" | "drop" => c.drop = true,
                    "hotbar_previous" => c.wheel = -1,
                    "hotbar_next" => c.wheel = 1,
                    "jump" => c.jump = true,
                    s if s.starts_with("slot_") => {
                        c.slot = s[5..]
                            .parse::<usize>()
                            .ok()
                            .filter(|n| (1..=HOTBAR_SIZE).contains(n))
                            .map(|n| n - 1)
                    }
                    s if s.starts_with("shop_") => {
                        c.shop_choice = s[5..]
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n >= 1)
                            .map(|n| n - 1)
                    }
                    s if PULSES.contains(&s) => {}
                    other => return Err(format!("unknown pulse '{other}'")),
                }
            }
        }
        Ok(())
    }

    /// Walks: holds `fwd` (-1..1, default 1: forward) and `strafe` for `frames` (default 60) frames, then lets go.
    /// The frames run as `step` does, so expectations and events see the walk.
    fn walk(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let fwd = f(c, "fwd").unwrap_or(1.0).clamp(-1.0, 1.0);
        let strafe = f(c, "strafe").unwrap_or(0.0).clamp(-1.0, 1.0);
        let n = c
            .get("frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(60)
            .clamp(1, MAX_STEP_FRAMES);
        self.sim.held.fwd = fwd;
        self.sim.held.strafe = strafe;
        let done = self.sim.step(n, true);
        self.sim.held.fwd = 0.0;
        self.sim.held.strafe = 0.0;
        let mut r = json!({ "ok": true, "frames": done, "frame": self.sim.frame });
        if c.get("observe").and_then(|v| v.as_bool()).unwrap_or(false) {
            r["obs"] = self.observe();
        }
        r
    }

    fn step(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        if let Some(a) = c.get("act") {
            if let Err(e) = self.apply_act(a) {
                return err(e);
            }
        }
        let n = c
            .get("frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, MAX_STEP_FRAMES);
        let stop = c
            .get("stop_on_end")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let done = self.sim.step(n, stop);
        let mut r = json!({ "ok": true, "frames": done, "frame": self.sim.frame });
        if c.get("observe").and_then(|v| v.as_bool()).unwrap_or(true) {
            r["obs"] = self.observe();
        }
        if c.get("events").and_then(|v| v.as_bool()).unwrap_or(false) {
            r["events"] = self.sim.take_events(None, false);
        }
        r
    }

    /// Steps until every expectation holds on the probe document (see expect.rs), or
    /// `max_frames`. `bot: true` lets the bot drive meanwhile.
    fn step_until(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        if let Some(a) = c.get("act") {
            if let Err(e) = self.apply_act(a) {
                return err(e);
            }
        }
        let list = c
            .get("expect")
            .or(c.get("until"))
            .cloned()
            .unwrap_or(Value::Null);
        if list.is_null() {
            return err("step_until needs 'expect' (a list of expectations)");
        }
        let max = c
            .get("max_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(600)
            .clamp(1, MAX_UNTIL_FRAMES);
        let use_bot = c.get("bot").and_then(|v| v.as_bool()).unwrap_or(false);
        let every = c
            .get("check_every")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1);
        let mut keys = HashSet::new();
        expect::sections(&list, &mut keys);
        let vars = Map::new();
        let mut frames = 0;
        let mut failures;
        loop {
            if frames % every == 0 || frames >= max {
                let doc = self.probe(&keys);
                failures = expect::check_all(&doc, &list, &vars);
                if failures.is_empty() {
                    break;
                }
            } else {
                failures = vec!["pending".into()];
            }
            let ended = self
                .sim
                .game
                .as_ref()
                .map(|g| g.report.is_some())
                .unwrap_or(true);
            if frames >= max || ended {
                if ended && frames % every != 0 {
                    let doc = self.probe(&keys);
                    failures = expect::check_all(&doc, &list, &vars);
                }
                break;
            }
            if use_bot {
                self.ensure_bot();
                let mut b = self.bot.take().unwrap();
                b.step(&mut self.sim);
                self.bot = Some(b);
            } else {
                self.sim.step_with(None);
            }
            frames += 1;
        }
        let mut r = json!({ "ok": true, "met": failures.is_empty(), "frames": frames, "frame": self.sim.frame, "unmet": failures });
        if c.get("observe").and_then(|v| v.as_bool()).unwrap_or(false) {
            r["obs"] = self.observe();
        }
        r
    }

    fn ensure_bot(&mut self) {
        if self.bot.is_none() {
            let label = if self.seed_label.is_empty() {
                self.sim
                    .game
                    .as_ref()
                    .map(|g| g.run_seed.to_string())
                    .unwrap_or_default()
            } else {
                self.seed_label.clone()
            };
            self.bot = Some(Bot::new(&label, self.bot_seed));
        }
    }

    fn bot_cmd(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        if c.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
            self.bot = None;
        }
        if let Some(s) = c.get("bot_seed").and_then(|v| v.as_u64()) {
            self.bot_seed = s as u32;
            self.bot = None;
        }
        self.ensure_bot();
        let n = c
            .get("frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(60)
            .clamp(1, MAX_UNTIL_FRAMES);
        let mut b = self.bot.take().unwrap();
        b.debug = c.get("debug").and_then(|v| v.as_bool()).unwrap_or(true);
        let mut done = 0;
        for _ in 0..n {
            if self
                .sim
                .game
                .as_ref()
                .map(|g| g.report.is_some())
                .unwrap_or(true)
            {
                break;
            }
            b.step(&mut self.sim);
            done += 1;
        }
        let stats = b.stats_json();
        let events: Vec<Value> = if c
            .get("bot_events")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            b.events.clone()
        } else {
            vec![]
        };
        self.bot = Some(b);
        let mut r = json!({ "ok": true, "frames": done, "frame": self.sim.frame, "bot": stats });
        if !events.is_empty() {
            r["bot_events"] = Value::Array(events);
        }
        if c.get("observe").and_then(|v| v.as_bool()).unwrap_or(true) {
            r["obs"] = self.observe();
        }
        r
    }

    fn admin(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let on = c.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
        let g = self.sim.game.as_mut().unwrap();
        match name {
            "god" => {
                g.admin.god = on;
                json!({ "ok": true, "god": on })
            }
            "no_target" => {
                g.admin.no_target = on;
                json!({ "ok": true, "no_target": on })
            }
            "teleport" => {
                // the bridge's {x, y, z}: x and z are the grid, y (height) is ignored
                let x = f(c, "x").unwrap_or(g.p.pos.0);
                let z = f(c, "z").unwrap_or(g.p.pos.1);
                self.teleport(&json!({ "x": x, "z": z }))
            }
            "give" => {
                let Some(item) = c.get("item").and_then(|v| v.as_str()).and_then(parse_item) else {
                    return err("give needs item: flashlight, taser, crowbar, axe, battery, stim");
                };
                let n = c.get("count").and_then(|v| v.as_i64()).unwrap_or(match item {
                    Item::Crowbar => CROWBAR_DURABILITY as i64,
                    Item::Axe => AXE_DURABILITY as i64,
                    _ => 1,
                }) as i32;
                let stack = item.stack_max() > 1;
                let mut given = 0;
                for _ in 0..(if stack { n.max(1) } else { 1 }) {
                    if g.give(item, if stack { 1 } else { n }) {
                        given += 1;
                    }
                }
                if given == 0 {
                    return err("inventory full");
                }
                json!({ "ok": true, "given": given })
            }
            "flicker" => {
                // force_flashlight_flicker: the beam strobes and [F] is locked out for `seconds` (default the splash's 1.4)
                let seconds = f(c, "seconds").unwrap_or(1.4).clamp(0.0, 30.0);
                let forced = g.force_flashlight_flicker(seconds);
                json!({ "ok": true, "forced": forced, "seized": g.p.flashlight_seized() })
            }
            "set" => {
                // body / meter values: lamp, stamina, temperature, ambient, exposure, noise, coins, yaw, pitch
                let mut set = Map::new();
                for (k, v) in c.as_object().unwrap() {
                    let Some(x) = v.as_f64().map(|x| x as f32) else { continue };
                    match k.as_str() {
                        "lamp" => g.p.lamp = x.clamp(0.0, MAX_LAMP_CHARGE),
                        "stamina" => g.p.stamina = x.clamp(0.0, MAX_STAMINA),
                        "temperature" => g.meters.temperature = x.clamp(0.0, 100.0),
                        "ambient" => g.meters.ambient = x.clamp(0.0, 100.0),
                        "exposure" => g.meters.exposure = x.clamp(0.0, 100.0),
                        "noise" => g.meters.noise_meter = x.clamp(0.0, 100.0),
                        "coins" => g.coins = x as i32,
                        "yaw" => g.p.yaw = x,
                        "pitch" => g.p.pitch = x.clamp(-1.45, 1.45),
                        _ => continue,
                    }
                    set.insert(k.clone(), json!(x));
                }
                json!({ "ok": true, "set": set })
            }
            "summon" => {
                // the admin's summon (ui/admin/actions.rs) for hunter `id`, `count` times, on the ring round the runner
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let count = c.get("count").and_then(|v| v.as_u64()).unwrap_or(1).min(16) as usize;
                let first = g.hunters.len();
                let mut made = 0;
                for slot in 0..count {
                    if crate::app::stress::summon(g, id, &self.sim.content, (first + slot) as u32, slot) {
                        made += 1;
                    }
                }
                if made == 0 {
                    return err(format!("no hunter '{id}' or no open floor"));
                }
                json!({ "ok": true, "summoned": made, "first_index": first })
            }
            "wake" => {
                let i = c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if i >= g.hunters.len() {
                    return err(format!("no hunter {i} (there are {})", g.hunters.len()));
                }
                let at = g.p.pos;
                g.hunters[i].activate_at(at);
                // state: "wander" / "idle" wakes it without a lead on the runner
                match c.get("state").and_then(|v| v.as_str()) {
                    Some("wander") => {
                        g.hunters[i].state = State::Wander;
                        g.hunters[i].last_known = None;
                    }
                    Some("idle") => {
                        g.hunters[i].state = State::Idle;
                        g.hunters[i].last_known = None;
                    }
                    _ => {}
                }
                json!({ "ok": true, "hunter": g.hunters[i].def.id, "state": format!("{:?}", g.hunters[i].state).to_lowercase() })
            }
            "hunter_to" => {
                let i = c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                // {x, z}, or {near: metres} in front of the runner
                let (x, z) = match (f(c, "x"), f(c, "z"), f(c, "near")) {
                    (Some(x), Some(z), _) => (x, z),
                    (_, _, Some(d)) => {
                        let (fx, fz) = g.p.forward2();
                        (g.p.pos.0 + fx * d, g.p.pos.1 + fz * d)
                    }
                    _ => return err("hunter_to needs index and {x, z} or {near}"),
                };
                if i >= g.hunters.len() {
                    return err(format!("no hunter {i}"));
                }
                g.hunters[i].pos = g.fac.nearest_open_or(x, z, HUNTER_RADIUS);
                g.hunters[i].clear_path();
                json!({ "ok": true, "pos": [r2(g.hunters[i].pos.0), r2(g.hunters[i].pos.1)] })
            }
            "noclip" => {
                g.admin.noclip = on;
                json!({ "ok": true, "noclip": on })
            }
            "speed" => {
                let k = f(c, "scale").unwrap_or(1.0).clamp(0.25, 3.0);
                g.admin.run_speed = k;
                json!({ "ok": true, "scale": k })
            }
            other => err(format!("unknown admin helper '{other}' (god, no_target, noclip, speed, teleport, give, set, summon, wake, hunter_to)")),
        }
    }

    /// Steps one frame with no input so the crosshair target (focus) catches up.
    fn settle(&mut self, frames: u64) {
        let held = std::mem::take(&mut self.sim.held);
        for _ in 0..frames {
            self.sim.step_with(Some(crate::game::Controls::default()));
        }
        self.sim.held = held;
    }

    fn look(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let g = self.sim.game.as_mut().unwrap();
        if let Some((x, z)) = pair(c, "at") {
            let (dx, dz) = (x - g.p.pos.0, z - g.p.pos.1);
            g.p.yaw = bot::yaw_to(dx, dz);
            let h = f(c, "height").unwrap_or(0.8);
            let horiz = (dx * dx + dz * dz).sqrt().max(0.05);
            g.p.pitch = (h - g.eye().y).atan2(horiz).clamp(-1.45, 1.45);
        }
        if let Some(y) = f(c, "yaw") {
            g.p.yaw = y;
        }
        if let Some(p) = f(c, "pitch") {
            g.p.pitch = p.clamp(-1.45, 1.45);
        }
        if let Some(d) = f(c, "dyaw") {
            g.p.yaw -= d;
        }
        if let Some(d) = f(c, "dpitch") {
            g.p.pitch = (g.p.pitch - d).clamp(-1.45, 1.45);
        }
        if let Some(d) = c.get("face").and_then(|v| v.as_str()) {
            g.p.yaw = match d {
                "north" => 0.0,
                "west" => std::f32::consts::FRAC_PI_2,
                "south" => std::f32::consts::PI,
                "east" => -std::f32::consts::FRAC_PI_2,
                _ => return err("face must be north, south, east or west"),
            };
        }
        let frames = c
            .get("settle")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(60);
        self.settle(frames);
        json!({ "ok": true, "player": self.sim.player_json(), "focus": self.sim.focus_value() })
    }

    /// Pulses [E] for one frame and reports what it hit. `kind` (e.g. "RELAY") refuses to press
    /// unless the crosshair is on that kind; `hold_frames` keeps [E] held afterwards.
    fn interact(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let want = c
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_uppercase());
        let before = self.sim.focus_value();
        if let Some(k) = &want {
            let g = self.sim.game.as_ref().unwrap();
            let have = g.focus.map(kind_name);
            if have != Some(k.as_str()) {
                return json!({ "ok": false, "error": format!("focus is {} not {k}", have.unwrap_or("nothing")), "focus": before });
            }
        }
        let seq = self.sim.event_seq;
        let ctl = crate::game::Controls {
            interact: true,
            interact_held: true,
            ..Default::default()
        };
        self.sim.step_with(Some(ctl));
        let hold = c
            .get("hold_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(MAX_STEP_FRAMES);
        for _ in 0..hold {
            self.sim.step_with(Some(crate::game::Controls {
                interact_held: true,
                ..Default::default()
            }));
        }
        let events = self.sim.take_events(Some(seq), true);
        json!({ "ok": true, "focus_before": before, "focus_after": self.sim.focus_value(), "events": events, "run": self.sim.run_json() })
    }

    /// Moves the runner: `{x, z}` (cells, 1 cell = 1 m; snapped to the nearest open spot),
    /// `{cell: [x, z]}`, or `{to: kind, index}` beside the object and facing it.
    fn teleport(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let g = self.sim.game.as_mut().unwrap();
        if let Some(to0) = c.get("to").and_then(|v| v.as_str()) {
            let mut i = c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let mut to = to0;
            if to0 == "ladder_down" {
                // the nearest sewer ladder going down (game/ladder.rs): mounted from its rungs
                let me = g.p.pos;
                match g
                    .fac
                    .stairs
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.dir < 0 && s.is_ladder())
                    .min_by(|a, b| {
                        pdist(a.1.cell.center(), me)
                            .partial_cmp(&pdist(b.1.cell.center(), me))
                            .unwrap()
                    }) {
                    Some((k, _)) => {
                        i = k;
                        to = "stairs";
                    }
                    None => return err("no ladder_down in this window"),
                }
            }
            if to0 == "stairs_down" || to0 == "stairs_up" {
                // the nearest flight going that way
                let dir = if to0 == "stairs_down" { -1 } else { 1 };
                let me = g.p.pos;
                match g
                    .fac
                    .stairs
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.dir == dir && s.kind == crate::world::facility::LinkKind::Stairs)
                    .min_by(|a, b| {
                        pdist(a.1.cell.center(), me)
                            .partial_cmp(&pdist(b.1.cell.center(), me))
                            .unwrap()
                    }) {
                    Some((k, _)) => {
                        i = k;
                        to = "stairs";
                    }
                    None => return err(format!("no {to0} in this window")),
                }
            }
            if to0 == "door" && c.get("index").is_none() {
                // no index: the room door nearest the runner
                let me = g.p.pos;
                let nearest = g.fac.doors().enumerate().min_by(|a, b| {
                    pdist(a.1 .1, me).partial_cmp(&pdist(b.1 .1, me)).unwrap()
                });
                match nearest {
                    Some((k, _)) => i = k,
                    None => return err("no door in this window"),
                }
            }
            use crate::world::Cell;
            let cell = match to {
                "relay" => g.fac.relays.get(i).copied(),
                "battery" => g.fac.batteries.get(i).copied(),
                "locker" | "hiding_spot" => g
                    .fac
                    .hiding
                    .get(i)
                    .map(|h| Cell::new(h.cell.x + h.facing.0, h.cell.y + h.facing.1)),
                "lift" | "exit" => Some(g.fac.exit),
                "stairs" => g.fac.stairs.get(i).map(|s| s.cell),
                "safe_zone" => g.fac.safe_zone,
                "heater" => g.fix.heater,
                "music_box" => g.fix.music_box,
                "vent" => g.fix.vents.get(i).copied(),
                "light_switch" => g.fac.room_lights.get(i).map(|l| l.switch_cell),
                "coin" => g.fix.coins.get(i).copied(),
                "door" => g.fac.doors().nth(i).map(|(prop, _)| g.fac.props[prop].cell),
                "pickup" => g
                    .pickups
                    .get(i)
                    .map(|p| Cell::new(p.pos.0 as i32, p.pos.1 as i32)),
                "hunter" => g
                    .hunters
                    .get(i)
                    .map(|h| Cell::new(h.pos.0 as i32, h.pos.1 as i32)),
                "start" => Some(g.fac.start),
                _ => return err(format!("unknown teleport target '{to}'")),
            };
            let Some(cell) = cell else {
                return err(format!("no {to} {i} on this floor"));
            };
            match to {
                "locker" | "hiding_spot" => {
                    let lc = g.fac.locker_center(&g.fac.hiding[i]);
                    let p = g
                        .fac
                        .nearest_open_or(cell.center().0, cell.center().1, PLAYER_RADIUS);
                    g.p.pos = p;
                    g.p.yaw = bot::yaw_to(lc.0 - p.0, lc.1 - p.1);
                    g.p.pitch = 0.0;
                }
                "stairs" => {
                    let s = g.fac.stairs[i];
                    if s.is_ladder() {
                        // a ladder: on its rungs, facing the wall (game/ladder.rs)
                        let side = crate::game::ladder::wall_side(s.cell, |x, y| g.fac.is_wall_cell(x, y));
                        g.p.pos = crate::game::ladder::stand(s.cell, side);
                        g.p.yaw = crate::game::ladder::facing(side);
                    } else {
                        // as the self-test: on the landing, facing down the flight
                        g.p.pos = s.cell.center();
                        g.p.yaw = (-(s.run.0 as f32)).atan2(-(s.run.1 as f32));
                    }
                    g.p.pitch = -0.45;
                }
                "start" | "hunter" | "safe_zone" => {
                    g.p.pos =
                        g.fac
                            .nearest_open_or(cell.center().0, cell.center().1, PLAYER_RADIUS);
                }
                "pickup" => {
                    let p = g.pickups[i].pos;
                    g.p.pos = g.fac.nearest_open_or(p.0, p.1 + 1.0, PLAYER_RADIUS);
                    g.p.yaw = bot::yaw_to(p.0 - g.p.pos.0, p.1 - g.p.pos.1);
                    g.p.pitch = -0.6;
                }
                _ => {
                    let before = g.p.pos;
                    g.teleport_beside(cell);
                    if g.p.pos == before {
                        let (cx, cy) = cell.center();
                        g.p.pos = g.fac.nearest_open_or(cx, cy, PLAYER_RADIUS);
                        g.p.yaw = bot::yaw_to(cx - g.p.pos.0, cy - g.p.pos.1);
                    }
                }
            }
        } else {
            let (x, z) = match (f(c, "x"), f(c, "z")) {
                (Some(x), Some(z)) => (x, z),
                _ => match pair(c, "cell") {
                    Some((x, z)) => (x.floor() + 0.5, z.floor() + 0.5),
                    None => return err("teleport needs {x, z}, {cell: [x, z]} or {to, index}"),
                },
            };
            let Some(p) = g.fac.nearest_open(x, z, PLAYER_RADIUS) else {
                return err("no open spot near there");
            };
            g.p.pos = p;
            if let Some(y) = f(c, "yaw") {
                g.p.yaw = y;
            }
        }
        g.p.hidden = None;
        self.settle(1);
        json!({ "ok": true, "player": self.sim.player_json(), "focus": self.sim.focus_value() })
    }

    /// Walks there with the bot's navigator (A* + steering): `{x, z}` or `{to, index}`; with
    /// `to` it stops beside the object and faces it.
    fn goto(&mut self, c: &Value) -> Value {
        if let Err(e) = self.need_game() {
            return e;
        }
        let g = self.sim.game.as_ref().unwrap();
        let to = c.get("to").and_then(|v| v.as_str());
        let i = c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let (target, height) = match to {
            Some(to) => {
                let t = match to {
                    "relay" => g.fac.relays.get(i).map(|c| (c.center(), 0.8)),
                    "battery" => g.fac.batteries.get(i).map(|c| (c.center(), 0.15)),
                    "lift" | "exit" => Some((g.fac.exit.center(), 1.2)),
                    "stairs" => g.fac.stairs.get(i).map(|s| (s.cell.center(), 0.3)),
                    "safe_zone" => g.fac.safe_zone.map(|c| (c.center(), 0.7)),
                    "locker" => g.fac.hiding.get(i).map(|h| (g.fac.locker_center(h), 1.0)),
                    "pickup" => g.pickups.get(i).map(|p| (p.pos, 0.15)),
                    "heater" => g.fix.heater.map(|c| (c.center(), 0.6)),
                    "music_box" => g.fix.music_box.map(|c| (c.center(), 0.5)),
                    "light_switch" => g
                        .fac
                        .room_lights
                        .get(i)
                        .map(|l| (l.switch_cell.center(), 1.2)),
                    _ => return err(format!("unknown goto target '{to}'")),
                };
                match t {
                    Some(t) => t,
                    None => return err(format!("no {to} {i} on this floor")),
                }
            }
            None => match (f(c, "x"), f(c, "z")) {
                (Some(x), Some(z)) => ((x, z), 0.8),
                _ => return err("goto needs {x, z} or {to, index}"),
            },
        };
        if bot::path_to(g, target).is_empty() {
            return json!({ "ok": true, "arrived": false, "reason": "no path", "frames": 0, "distance": r2(pdist(target, g.p.pos)) });
        }
        let stop_at = f(c, "radius").unwrap_or(if to.is_some() { 1.4 } else { 1.0 });
        let max = c
            .get("max_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(1800)
            .clamp(1, MAX_UNTIL_FRAMES);
        let mut b = Bot::new("goto", 1);
        b.debug = false;
        b.forced_reach = (stop_at - 0.15).max(0.4);
        let mut frames = 0;
        let mut arrived = false;
        while frames < max {
            let g = self.sim.game.as_ref().unwrap();
            if g.report.is_some() || g.escape.is_some() {
                break;
            }
            if pdist(target, g.p.pos) <= stop_at {
                arrived = true;
                break;
            }
            b.forced = Some(target);
            let ctl = b.decide(&self.sim);
            self.sim.step_with(Some(ctl));
            frames += 1;
        }
        if to.is_some() && arrived {
            let _ = self.look(&json!({ "at": [target.0, target.1], "height": height }));
        }
        let g = self.sim.game.as_ref().unwrap();
        let mut r = json!({ "ok": true, "arrived": arrived, "frames": frames, "distance": r2(pdist(target, g.p.pos)), "player": self.sim.player_json(), "focus": self.sim.focus_value() });
        if c.get("observe").and_then(|v| v.as_bool()).unwrap_or(false) {
            r["obs"] = self.observe();
        }
        r
    }

    pub fn director_json(&self) -> Value {
        let Some(g) = self.sim.game.as_ref() else {
            return Value::Null;
        };
        let d = &g.director;
        let s = &d.stats;
        json!({
            "mode": d.mode.id(),
            "relays_collected": d.relays_collected,
            "relay_rush_enabled": d.relay_rush_enabled,
            "relay_rush_active": d.relay_rush_active(),
            "stats": { "shares": s.shares, "flanks": s.flanks, "covers": s.covers, "scatters": s.scatters, "released": s.released, "rushes": s.rushes },
        })
    }

    /// Reachability from the start (validate_pickup_reach.gd): relays, batteries, lockers and
    /// the exit on the generator's walkable grid (Facility::distance_map).
    pub fn nav_json(&self) -> Value {
        use crate::world::Cell;
        let Some(g) = self.sim.game.as_ref() else {
            return Value::Null;
        };
        let from = g.fac.start;
        let dm = g.fac.distance_map(from);
        let reach = |c: Cell| -> bool {
            [
                c,
                Cell::new(c.x + 1, c.y),
                Cell::new(c.x - 1, c.y),
                Cell::new(c.x, c.y + 1),
                Cell::new(c.x, c.y - 1),
            ]
            .iter()
            .any(|n| dm.contains_key(n))
        };
        let relays: Vec<bool> = g.fac.relays.iter().map(|c| reach(*c)).collect();
        let batteries: Vec<bool> = g.fac.batteries.iter().map(|c| reach(*c)).collect();
        let lockers: Vec<bool> = g
            .fac
            .hiding
            .iter()
            .map(|h| reach(Cell::new(h.cell.x + h.facing.0, h.cell.y + h.facing.1)))
            .collect();
        let unreachable: Vec<Value> = g
            .fac
            .relays
            .iter()
            .zip(&relays)
            .filter(|(_, r)| !**r)
            .map(|(c, _)| json!({ "kind": "relay", "cell": [c.x, c.y] }))
            .chain(
                g.fac
                    .batteries
                    .iter()
                    .zip(&batteries)
                    .filter(|(_, r)| !**r)
                    .map(|(c, _)| json!({ "kind": "battery", "cell": [c.x, c.y] })),
            )
            .collect();
        json!({
            "from": [from.x, from.y],
            "walkable_cells": dm.len(),
            "relays_reachable": relays.iter().filter(|r| **r).count(),
            "relays_total": relays.len(),
            "batteries_reachable": batteries.iter().filter(|r| **r).count(),
            "batteries_total": batteries.len(),
            "lockers_reachable": lockers.iter().filter(|r| **r).count(),
            "lockers_total": lockers.len(),
            "exit_reachable": reach(g.fac.exit),
            "safe_zone_reachable": g.fac.safe_zone.map(reach),
            "unreachable": unreachable,
        })
    }

    /// The bridge's observation (LastRouteBot.observe) plus the port's meters / hotbar /
    /// escape / message / director blocks. Grid: x is the world x, y the world z (+0.5 is a
    /// cell centre); positions are in the live window (Infinite: add world.origin).
    pub fn observe(&self) -> Value {
        let mut obs = json!({
            "frame": self.sim.frame, "time": r2(self.sim.frame as f32 * self.sim.dt), "scene": self.scene(),
            "paused": false, "fps": (1.0 / self.sim.dt).round() as i64, "game": "last_route",
            "log": { "errors": self.errors, "warnings": self.warnings, "seq": self.log_seq },
        });
        let Some(g) = self.sim.game.as_ref() else {
            obs["run"] = json!({ "active": false, "over": false });
            return obs;
        };
        let me = g.p.pos;
        let d = |p: (f32, f32)| r2(pdist(p, me));
        let entry = |id: usize, c: crate::world::Cell, cap: i64| json!({ "id": id, "x": c.x, "y": c.y, "dist": d(c.center()), "capacity": cap });
        let sorted = |mut v: Vec<Value>, n: usize| {
            v.sort_by(|a, b| {
                a["dist"]
                    .as_f64()
                    .partial_cmp(&b["dist"].as_f64())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            v.truncate(n);
            Value::Array(v)
        };
        let level = g.inf.as_ref().map(|i| i.level).unwrap_or(1 - g.depth);
        let region = match g.fac.room_at(me.0 as f64, me.1 as f64) {
            Some(i) if !g.fac.rooms[i].lobby() => g.fac.rooms[i].region.clone(),
            _ => g.fac.region_at(me.0 as i32, me.1 as i32).to_string(),
        };
        let chunk = match &g.inf {
            Some(i) => {
                let ch = crate::world::infinite::CHUNK as f32;
                json!([
                    (me.0 / ch).floor() as i32 - 1 + i.center.0,
                    level,
                    (me.1 / ch).floor() as i32 - 1 + i.center.1
                ])
            }
            None => json!([0, level, 0]),
        };
        let death = g
            .report
            .as_ref()
            .map(|r| json!({ "hunter_id": r.hunter, "via": r.cause, "detail": r.hunter_name }));
        let rooms_discovered = g
            .fac
            .rooms
            .iter()
            .filter(|r| {
                g.discovered
                    .get(r.center.y as usize)
                    .and_then(|row| row.get(r.center.x as usize))
                    .copied()
                    .unwrap_or(false)
            })
            .count();
        let mode = if g.escape.is_some() {
            "captured"
        } else if g.p.hidden.is_some() {
            "hidden"
        } else if g.p.in_vent {
            "vent"
        } else if g.p.crouching {
            "crouch"
        } else if g.p.sprinting {
            "sprint"
        } else if g.p.moving {
            "walk"
        } else {
            "idle"
        };
        let mut interactables: Vec<Value> = vec![];
        let mut add = |kind: &str, p: (f32, f32), h: f32, done: bool| {
            let dist = pdist(p, me);
            if dist <= 45.0 {
                interactables.push(json!({ "kind": kind, "pos": [r2(p.0), r2(h), r2(p.1)], "dist": r2(dist), "completed": done }));
            }
        };
        for (i, c) in g.fac.relays.iter().enumerate() {
            add("RELAY", c.center(), 0.8, g.relays_taken[i]);
        }
        for (i, c) in g.fac.batteries.iter().enumerate() {
            add("BATTERY", c.center(), 0.15, g.batteries_taken[i]);
        }
        for p in g.pickups.iter() {
            add("TOOL_PICKUP", p.pos, 0.15, p.taken);
        }
        for (i, c) in g.fix.coins.iter().enumerate() {
            add("COIN", c.center(), 0.1, g.coins_taken[i]);
        }
        for (i, h) in g.fac.hiding.iter().enumerate() {
            add(
                "HIDING_SPOT",
                g.fac.locker_center(h),
                1.0,
                g.hiding_disabled[i],
            );
        }
        if let Some(s) = g.fac.safe_zone {
            add("SAFE_ZONE", s.center(), 0.7, false);
        }
        if g.fac.has_lift {
            add("LIFT", g.fac.exit.center(), 1.2, false);
        }
        for s in &g.fac.stairs {
            add("STAIRS", s.cell.center(), 0.3, false);
        }
        for (i, l) in g.fac.room_lights.iter().enumerate() {
            add("LIGHT_SWITCH", l.switch_cell.center(), 1.2, !g.lights_on[i]);
        }
        if let Some(c) = g.fix.heater {
            add(
                "HEATER_STATION",
                c.center(),
                0.6,
                g.heater == crate::game::Heater::Powered,
            );
        }
        if let Some(c) = g.fix.music_box {
            add("MUSIC_BOX", c.center(), 0.5, false);
        }
        for c in &g.fix.vents {
            add("VENT", c.center(), 0.4, false);
        }
        let interactables = sorted(interactables, 30);
        let mut hunters: Vec<Value> = g.hunters.iter().enumerate().map(|(i, h)| {
            let dist = pdist(h.pos, me);
            json!({
                "entity": i, "id": h.def.id, "state": state_upper(h.state), "pos": [r2(h.pos.0), r2(h.pos.1)],
                "dist": r2(dist), "los": dist < 40.0 && g.fac.has_line_of_sight(h.pos, me), "stuck": false,
                "stun": r2(h.stun), "active": h.active, "visible": !h.gimmick.hidden_from_view(),
            })
        }).collect();
        hunters.sort_by(|a, b| {
            a["dist"]
                .as_f64()
                .partial_cmp(&b["dist"].as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        obs["run"] = json!({
            "active": g.report.is_none(), "over": g.report.is_some(), "depth": g.depth, "score": g.score,
            "relays": g.relays_collected, "relays_total": g.fac.relays.len(), "coins": g.coins, "debug_run": true, "death": death,
            "seed": g.run_seed, "difficulty": g.difficulty.id(), "mode": self.sim.mode.id(), "level": level,
            "statistics": {
                "rooms_discovered": rooms_discovered, "relays_collected": g.relays_collected,
                "elapsed_seconds": r2(g.time), "distance_m": r2(g.p.walked),
                "visited_levels": self.sim.visited_levels.iter().copied().collect::<Vec<_>>(),
                "encountered_hunters": self.sim.encountered.iter().cloned().collect::<Vec<_>>(),
                "hunters_encountered": self.sim.encountered.len(),
                "items_collected": self.sim.items_collected, "chases_started": self.sim.chases_started,
                "chunks_visited": g.chunks_visited(),
            },
        });
        obs["player"] = json!({
            "pos": [r2(me.0), r2(g.p.feet_y), r2(me.1)], "cell": [me.0.floor() as i32, me.1.floor() as i32],
            "yaw": r2(g.p.yaw), "pitch": r2(g.p.pitch), "vel": [0.0, 0.0, 0.0],
            "hidden": g.p.hidden.is_some(), "stamina": r2(g.p.stamina), "lamp": r2(g.p.lamp),
            "flashlight": g.p.flashlight_on, "mode": mode, "noise": r2(g.p.noise), "frozen": g.p.stun > 0.0,
            "spectator": false, "item": g.selected_item().map(item_id), "in_safe_room": g.in_safe_zone(), "exhausted": g.p.exhausted,
        });
        obs["focus"] = match g.focus {
            Some(fc) => {
                let mut v = focus_json(g, fc);
                v["kind"] = Value::from(kind_name(fc));
                v["prompt"] = g.focus_prompt().map(Value::from).unwrap_or(Value::Null);
                v
            }
            None => json!({}),
        };
        let relays: Vec<Value> = g
            .fac
            .relays
            .iter()
            .enumerate()
            .filter(|(i, _)| !g.relays_taken[*i])
            .map(|(i, c)| entry(i, *c, 1))
            .collect();
        let batteries: Vec<Value> = g
            .fac
            .batteries
            .iter()
            .enumerate()
            .filter(|(i, _)| !g.batteries_taken[*i])
            .map(|(i, c)| entry(i, *c, 1))
            .collect();
        let safes: Vec<Value> = g.fac.safe_zone.iter().map(|c| entry(0, *c, 4)).collect();
        let spots: Vec<Value> = g
            .fac
            .hiding
            .iter()
            .enumerate()
            .filter(|(i, _)| !g.hiding_disabled[*i])
            .map(|(i, h)| entry(i, h.cell, 1))
            .collect();
        obs["world"] = json!({
            "chunk": chunk, "level": level, "loaded_chunks": if g.is_infinite() { 9 } else { 1 }, "region": region,
            "relays": sorted(relays, 8), "batteries": sorted(batteries, 4), "safe_zones": sorted(safes, 4),
            "hiding_spots": sorted(spots, 6), "origin": [g.fac.origin.0, g.fac.origin.1], "size": [g.fac.cols, g.fac.rows],
        });
        obs["hunters"] = Value::Array(hunters);
        obs["interactables"] = interactables;
        obs["online"] =
            json!({ "active": false, "host": false, "peers": 0, "remotes": 0, "code": "" });
        obs["meters"] = self.sim.meters_json();
        obs["hotbar"] = self.sim.hotbar_json();
        obs["escape"] = g.escape.as_ref().map(|e| json!({ "remaining": r2(e.remaining), "progress": e.progress, "needed": ESCAPE_PRESSES, "hunter": g.hunters[e.hunter].def.id })).unwrap_or(Value::Null);
        obs["message"] = if g.message_t > 0.0 {
            Value::from(g.message.clone())
        } else {
            Value::Null
        };
        obs["toast"] = if g.toast_t > 0.0 {
            Value::from(g.toast.clone())
        } else {
            Value::Null
        };
        obs["director"] = self.director_json();
        obs
    }

    /// The document `step_until` and the suites check; only the named sections are built
    /// (all cheap ones when `keys` is empty).
    pub fn probe(&self, keys: &HashSet<String>) -> Value {
        let all = keys.is_empty();
        let want = |k: &str| all || keys.contains(k);
        let mut o = Map::new();
        let mut put = |k: &str, v: Value| {
            o.insert(k.into(), v);
        };
        if want("run") {
            put("run", self.sim.run_json());
        }
        if want("player") {
            put("player", self.sim.player_json());
        }
        if want("meters") {
            put("meters", self.sim.meters_json());
        }
        if want("hotbar") {
            put("hotbar", self.sim.hotbar_json());
        }
        if want("hunters") {
            put("hunters", self.sim.hunters_json());
        }
        if want("relays") {
            put("relays", self.sim.relays_json());
        }
        if want("focus") {
            put("focus", self.sim.focus_value());
        }
        if want("report") {
            put("report", self.sim.report_json());
        }
        if want("director") {
            put("director", self.director_json());
        }
        if want("discovered") {
            put("discovered", self.sim.discovered_json(false));
        }
        if want("events") {
            put("events", Value::Array(self.sim.events.clone()));
        }
        if want("bot") {
            put(
                "bot",
                self.bot
                    .as_ref()
                    .map(|b| b.stats_json())
                    .unwrap_or(Value::Null),
            );
        }
        if want("last") {
            put("last", self.last.clone());
        }
        if !all {
            if want("objects") {
                put("objects", self.sim.objects_json());
            }
            if want("rooms") {
                put("rooms", self.sim.rooms_json());
            }
            if want("nav") {
                put("nav", self.nav_json());
            }
            if want("map") {
                put("map", self.sim.map_json(true, None));
            }
            if want("obs") {
                put("obs", self.observe());
            }
        }
        Value::Object(o)
    }

    /// PlaytestObserver.state_hash: frame, scene, run, player, world, hunters with floats
    /// snapped to 0.01, hashed (FNV-1a 64, hex).
    pub fn state_hash(&self) -> String {
        let obs = self.observe();
        let mut pick = Map::new();
        for k in ["frame", "scene", "run", "player", "world", "hunters"] {
            pick.insert(k.into(), snap(&obs[k]));
        }
        let text = Value::Object(pick).to_string();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }
}

fn snap(v: &Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap();
            json!((x * 100.0).round() / 100.0)
        }
        Value::Array(a) => Value::Array(a.iter().map(snap).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), snap(v))).collect()),
        other => other.clone(),
    }
}
