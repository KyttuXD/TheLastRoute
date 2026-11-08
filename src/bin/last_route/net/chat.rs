//! Co-op chat lines, from `game/network/remote_players.gd` (`send_chat_payload`, `_submit_chat_payload`,
//! `_chat_payload`, `sanitize_payload`, `accept_line`, `rate_ok`, `is_duplicate`, `collapse_repeats`).
//!
//! The original already speaks chat aloud (text to speech; `MAX_CHAT_LENGTH = 80` "spoken aloud: short lines") and
//! has two channels: `team` (everyone) and `nearby` (proximity: the payload carries the sender's map position `at`,
//! world position `world` and floor `depth`, so a listener can decide whether it is close enough to hear it).
//! A guest sends `_submit_chat_payload(payload)` to the host; the host sanitises it, applies the moderation rules
//! (room mutes, 3 lines per 10 s then a 5 s cooldown, one map ping per 2 s, near-duplicates within 20 s dropped)
//! and broadcasts `_chat_payload(peer_id, display_name, payload)` to every peer (and shows it itself).
//! Presence travels separately: `_wire_profile(profile)` (name/skin), `_place(packet)` (the 20 Hz position
//! sample on the STATE channel) and `_receive_pings(pings)`; see [`super::protocol::Message`].
//!
//! HOOK for the TTS chat (`src/chat/`): feed every received `Message::ChatPayload` / `Message::Chat` through
//! [`ChatLine::from_message`] and speak `line.text` when `line.audible_from(listener_world, listener_depth, radius)`.

use super::protocol::Message;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const MAX_CHAT_LENGTH: usize = 80;
pub const CHAT_CHANNELS: [&str; 2] = ["team", "nearby"];
pub const CHAT_RATE_WINDOW_MS: i64 = 10_000;
pub const CHAT_RATE_MAX: usize = 3;
pub const CHAT_COOLDOWN_MS: i64 = 5_000;
pub const PING_MIN_INTERVAL_MS: i64 = 2_000;
pub const DUPLICATE_WINDOW_MS: i64 = 20_000;

/// One chat line with its context (the original's payload Dictionary).
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub peer_id: i64,
    pub display_name: String,
    pub text: String,
    /// "team" or "nearby".
    pub channel: String,
    pub preset: String,
    pub ping: bool,
    /// Map position (Vector2) of the sender.
    pub at: [f64; 2],
    /// World position (Vector3) of the sender.
    pub world: [f64; 3],
    pub depth: i64,
    pub system: bool,
    pub target: Option<[f64; 2]>,
}

/// "aaaaaaa" -> "aaa": any character repeated more than 3 times in a row is cut to 3.
pub fn collapse_repeats(text: &str) -> String {
    let mut out = String::new();
    let (mut run, mut last) = (0, None);
    for c in text.chars() {
        run = if Some(c) == last { run + 1 } else { 1 };
        last = Some(c);
        if run <= 3 {
            out.push(c);
        }
    }
    out
}

/// `_sanitize_chat`: one line, trimmed, repeats collapsed, at most 80 characters.
pub fn sanitize_text(text: &str) -> String {
    collapse_repeats(text.replace('\r', " ").replace('\n', " ").trim()).chars().take(MAX_CHAT_LENGTH).collect()
}

/// `line_key`: case, spacing, punctuation and repeats ignored.
pub fn line_key(text: &str) -> String {
    collapse_repeats(&text.to_lowercase()).chars().filter(|c| c.is_alphanumeric() || *c == '_').collect()
}

fn vec_of<const N: usize>(value: Option<&Value>) -> Option<[f64; N]> {
    let items = value?.as_array()?;
    if items.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, v) in out.iter_mut().zip(items) {
        *slot = v.as_f64()?;
    }
    Some(out)
}

impl ChatLine {
    /// `sanitize_payload(payload, allow_system)`; vectors travel as JSON arrays `[x, y]` / `[x, y, z]`.
    pub fn sanitize(peer_id: i64, display_name: &str, payload: &Map<String, Value>, allow_system: bool) -> ChatLine {
        let channel = payload.get("channel").and_then(Value::as_str).unwrap_or("team");
        ChatLine {
            peer_id,
            display_name: display_name.trim().chars().take(32).collect(),
            text: sanitize_text(payload.get("text").and_then(Value::as_str).unwrap_or("")),
            channel: if CHAT_CHANNELS.contains(&channel) { channel.to_string() } else { "team".into() },
            preset: payload.get("preset").and_then(Value::as_str).unwrap_or("").chars().take(24).collect(),
            ping: payload.get("ping").and_then(Value::as_bool).unwrap_or(false),
            at: vec_of::<2>(payload.get("at")).unwrap_or([0.0; 2]),
            world: vec_of::<3>(payload.get("world")).unwrap_or([0.0; 3]),
            depth: payload.get("depth").and_then(Value::as_i64).unwrap_or(0),
            system: allow_system && payload.get("system").and_then(Value::as_bool).unwrap_or(false),
            target: vec_of::<2>(payload.get("target")),
        }
    }

    pub fn payload(&self) -> Map<String, Value> {
        let v = json!({
            "text": self.text, "channel": self.channel, "preset": self.preset, "ping": self.ping,
            "at": self.at, "world": self.world, "depth": self.depth, "system": self.system,
            "target": self.target.map(|t| json!(t)).unwrap_or(Value::Null),
        });
        v.as_object().cloned().unwrap_or_default()
    }

    /// A received chat message (`_chat_payload` or the legacy plain `_chat`) as a line; None for other messages
    /// or empty text.
    pub fn from_message(message: &Message) -> Option<ChatLine> {
        let line = match message {
            Message::ChatPayload { peer_id, display_name, payload } => {
                let system = payload.get("system").and_then(Value::as_bool).unwrap_or(false);
                ChatLine::sanitize(*peer_id, display_name, payload, system)
            }
            Message::Chat { peer_id, display_name, message } => {
                let mut p = Map::new();
                p.insert("text".into(), json!(message));
                ChatLine::sanitize(*peer_id, display_name, &p, false)
            }
            _ => return None,
        };
        (!line.text.is_empty()).then_some(line)
    }

    /// Whether a listener hears it: team lines always; nearby lines on the same floor within `radius` world units.
    pub fn audible_from(&self, listener_world: [f64; 3], listener_depth: i64, radius: f64) -> bool {
        if self.channel != "nearby" {
            return true;
        }
        let d: f64 = (0..3).map(|i| (self.world[i] - listener_world[i]).powi(2)).sum::<f64>().sqrt();
        self.depth == listener_depth && d <= radius
    }
}

/// How many lines one guest may send in a window, and the pause after going over (data/chat.json `rate_lines`,
/// `rate_window_seconds`, `cooldown_seconds`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatLimits {
    pub lines: usize,
    pub window_ms: i64,
    pub cooldown_ms: i64,
}

impl Default for ChatLimits {
    fn default() -> Self {
        ChatLimits { lines: CHAT_RATE_MAX, window_ms: CHAT_RATE_WINDOW_MS, cooldown_ms: CHAT_COOLDOWN_MS }
    }
}

impl ChatLimits {
    pub fn from_rules(rules: &ChatRules) -> ChatLimits {
        ChatLimits {
            lines: rules.rate_lines,
            window_ms: (f64::from(rules.rate_window_seconds) * 1000.0) as i64,
            cooldown_ms: (f64::from(rules.cooldown_seconds) * 1000.0) as i64,
        }
    }
}

/// The host's per-peer chat moderation (`accept_line`).
pub struct ChatModeration {
    pub room_muted: HashSet<i64>,
    limits: ChatLimits,
    chat_times: HashMap<i64, Vec<i64>>,
    ping_times: HashMap<i64, i64>,
    cooldown_until: HashMap<i64, i64>,
    recent_lines: HashMap<i64, Vec<(String, i64)>>,
}

impl Default for ChatModeration {
    fn default() -> Self {
        ChatModeration::with_limits(ChatLimits::from_rules(ChatRules::load()))
    }
}

impl ChatModeration {
    pub fn with_limits(limits: ChatLimits) -> ChatModeration {
        ChatModeration {
            room_muted: HashSet::new(),
            limits,
            chat_times: HashMap::new(),
            ping_times: HashMap::new(),
            cooldown_until: HashMap::new(),
            recent_lines: HashMap::new(),
        }
    }

    pub fn rate_ok(&mut self, peer: i64, ping: bool, now: i64) -> bool {
        if now < *self.cooldown_until.get(&peer).unwrap_or(&0) {
            return false;
        }
        let mut times: Vec<i64> = self.chat_times.get(&peer).cloned().unwrap_or_default().into_iter().filter(|t| now - t < self.limits.window_ms).collect();
        if times.len() >= self.limits.lines {
            self.chat_times.insert(peer, times);
            self.cooldown_until.insert(peer, now + self.limits.cooldown_ms);
            return false;
        }
        if ping && now - *self.ping_times.get(&peer).unwrap_or(&-PING_MIN_INTERVAL_MS) < PING_MIN_INTERVAL_MS {
            return false;
        }
        times.push(now);
        self.chat_times.insert(peer, times);
        if ping {
            self.ping_times.insert(peer, now);
        }
        true
    }

    pub fn is_duplicate(&mut self, peer: i64, text: &str, now: i64) -> bool {
        let key = line_key(text);
        let recent: Vec<(String, i64)> =
            self.recent_lines.get(&peer).cloned().unwrap_or_default().into_iter().filter(|(_, t)| now - t < DUPLICATE_WINDOW_MS).collect();
        let (kl, dup) = (key.chars().count(), recent.iter().any(|(other, _)| {
            let ol = other.chars().count();
            *other == key || (key.chars().count() >= 6 && (other.starts_with(&key) || key.starts_with(other.as_str())) && ol.abs_diff(key.chars().count()) <= 3)
        }));
        let _ = kl;
        let mut recent = recent;
        if !dup {
            recent.push((key, now));
        }
        self.recent_lines.insert(peer, recent);
        dup
    }

    /// `accept_line(peer, payload)`.
    pub fn accept_line(&mut self, peer: i64, line: &ChatLine, now: i64) -> bool {
        if self.room_muted.contains(&peer) {
            return false;
        }
        if line.system {
            return true;
        }
        if self.is_duplicate(peer, &line.text, now) {
            return false;
        }
        self.rate_ok(peer, line.ping, now)
    }
}

/// Chat display tuning, from data/chat.json (the Standard defaults if it is missing or broken).
#[derive(Clone, Debug, PartialEq)]
pub struct ChatRules {
    pub max_lines: usize,
    pub line_seconds: f32,
    pub nearby_radius: f64,
    pub inbox_max: usize,
    /// The key that opens the input box (a letter; "T").
    pub open_key: String,
    /// Characters the input box takes (never more than [`MAX_CHAT_LENGTH`], which the host cuts to anyway).
    pub input_max: usize,
    /// The last seconds of a line's life over which it fades out.
    pub fade_seconds: f32,
    /// Lines the feed remembers for the open box, and how many the open box shows.
    pub history_max: usize,
    pub open_lines: usize,
    /// Per guest, on the host: this many lines per window, then a cooldown.
    pub rate_lines: usize,
    pub rate_window_seconds: f32,
    pub cooldown_seconds: f32,
    /// Sender name colours (r, g, b in 0..1), picked by peer id.
    pub name_colors: Vec<[f32; 3]>,
}

impl Default for ChatRules {
    fn default() -> Self {
        ChatRules {
            max_lines: 6,
            line_seconds: 12.0,
            nearby_radius: 30.0,
            inbox_max: 64,
            open_key: "T".into(),
            input_max: MAX_CHAT_LENGTH,
            fade_seconds: 2.0,
            history_max: 40,
            open_lines: 12,
            rate_lines: CHAT_RATE_MAX,
            rate_window_seconds: 10.0,
            cooldown_seconds: 5.0,
            name_colors: vec![[0.55, 0.85, 1.0], [1.0, 0.75, 0.4], [0.6, 1.0, 0.6], [1.0, 0.6, 0.8], [0.85, 0.7, 1.0], [1.0, 1.0, 0.55]],
        }
    }
}

impl ChatRules {
    pub fn parse(v: &Value) -> ChatRules {
        let d = ChatRules::default();
        ChatRules {
            max_lines: v.get("max_lines").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 20)).unwrap_or(d.max_lines),
            line_seconds: v.get("line_seconds").and_then(Value::as_f64).filter(|s| *s > 0.0).map(|s| s as f32).unwrap_or(d.line_seconds),
            nearby_radius: v.get("nearby_radius").and_then(Value::as_f64).filter(|r| *r > 0.0).unwrap_or(d.nearby_radius),
            inbox_max: v.get("inbox_max").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 1000)).unwrap_or(d.inbox_max),
            open_key: v
                .get("open_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|k| k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()))
                .map(str::to_uppercase)
                .unwrap_or(d.open_key),
            input_max: v.get("input_max").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, MAX_CHAT_LENGTH)).unwrap_or(d.input_max),
            fade_seconds: v.get("fade_seconds").and_then(Value::as_f64).filter(|s| *s >= 0.0).map(|s| s as f32).unwrap_or(d.fade_seconds),
            history_max: v.get("history_max").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 200)).unwrap_or(d.history_max),
            open_lines: v.get("open_lines").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 40)).unwrap_or(d.open_lines),
            rate_lines: v.get("rate_lines").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 100)).unwrap_or(d.rate_lines),
            rate_window_seconds: v.get("rate_window_seconds").and_then(Value::as_f64).filter(|s| *s > 0.0).map(|s| s as f32).unwrap_or(d.rate_window_seconds),
            cooldown_seconds: v.get("cooldown_seconds").and_then(Value::as_f64).filter(|s| *s >= 0.0).map(|s| s as f32).unwrap_or(d.cooldown_seconds),
            name_colors: v
                .get("name_colors")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|c| vec_of::<3>(Some(c)).map(|x| [x[0] as f32, x[1] as f32, x[2] as f32])).collect::<Vec<_>>())
                .filter(|c| !c.is_empty())
                .unwrap_or(d.name_colors),
        }
    }

    pub fn load() -> &'static ChatRules {
        static RULES: std::sync::OnceLock<ChatRules> = std::sync::OnceLock::new();
        RULES.get_or_init(|| {
            crate::content::embedded::data_under("chat.json")
                .next()
                .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
                .map(|v| ChatRules::parse(&v))
                .unwrap_or_default()
        })
    }
}

/// The lines on screen: the app pushes what [`super::world_sync::NetSession::take_chat_lines`] returns, ticks it each
/// frame and the HUD draws [`ChatFeed::visible`].
#[derive(Default)]
pub struct ChatFeed {
    lines: Vec<(ChatLine, f32)>,
}

impl ChatFeed {
    /// Adds lines that this listener hears (nearby lines only within the data's radius on the same floor).
    pub fn push(&mut self, new: Vec<ChatLine>, listener_world: [f64; 3], listener_depth: i64) {
        let rules = ChatRules::load();
        for line in new {
            if line.audible_from(listener_world, listener_depth, rules.nearby_radius) {
                self.lines.push((line, 0.0));
            }
        }
        let extra = self.lines.len().saturating_sub(rules.history_max.max(rules.max_lines));
        self.lines.drain(..extra);
    }

    /// Ages the lines. A line leaves the screen after `line_seconds` but stays in the history (for the open box) until
    /// the history is full.
    pub fn tick(&mut self, dt: f32) {
        for (_, age) in &mut self.lines {
            *age += dt.max(0.0);
        }
    }

    /// The lines on screen while the box is closed, oldest first: the newest `max_lines` that have not expired.
    pub fn visible(&self) -> impl Iterator<Item = &ChatLine> {
        self.faded().into_iter().map(|(l, _)| l)
    }

    /// Like [`ChatFeed::visible`] with each line's opacity (1.0, then fading to 0 over the last `fade_seconds`).
    pub fn faded(&self) -> Vec<(&ChatLine, f32)> {
        let rules = ChatRules::load();
        let live: Vec<(&ChatLine, f32)> = self
            .lines
            .iter()
            .filter(|(_, age)| *age < rules.line_seconds)
            .map(|(l, age)| (l, fade_alpha(*age, rules.line_seconds, rules.fade_seconds)))
            .collect();
        let skip = live.len().saturating_sub(rules.max_lines);
        live.into_iter().skip(skip).collect()
    }

    /// The newest `open_lines` remembered lines, oldest first: what the open box shows.
    pub fn recent(&self) -> Vec<&ChatLine> {
        let skip = self.lines.len().saturating_sub(ChatRules::load().open_lines);
        self.lines.iter().skip(skip).map(|(l, _)| l).collect()
    }
}

/// Opacity of a line of `age` seconds: 1 until the last `fade` seconds of `life`, then down to 0.
pub fn fade_alpha(age: f32, life: f32, fade: f32) -> f32 {
    if age >= life {
        0.0
    } else if fade <= 0.0 || age <= life - fade {
        1.0
    } else {
        ((life - age) / fade).clamp(0.0, 1.0)
    }
}

/// Guest side: the message to send for a typed line (`send_chat(text, channel)` + position context).
pub fn submit(text: &str, channel: &str, at: [f64; 2], world: [f64; 3], depth: i64) -> Option<Message> {
    let mut payload = Map::new();
    payload.insert("text".into(), json!(text));
    payload.insert("channel".into(), json!(channel));
    payload.insert("at".into(), json!(at));
    payload.insert("world".into(), json!(world));
    payload.insert("depth".into(), json!(depth));
    let line = ChatLine::sanitize(0, "", &payload, false);
    (!line.text.is_empty()).then(|| Message::SubmitChatPayload { payload: line.payload() })
}

/// Host side: a guest's `_submit_chat_payload` -> the `_chat_payload` to broadcast (None when moderation drops it).
pub fn relay_submission(moderation: &mut ChatModeration, sender: i64, display_name: &str, payload: &Map<String, Value>, now: i64) -> Option<Message> {
    let line = ChatLine::sanitize(sender, display_name, payload, false);
    if sender <= 1 || line.text.is_empty() || !moderation.accept_line(sender, &line, now) {
        return None;
    }
    Some(Message::ChatPayload { peer_id: sender, display_name: line.display_name.clone(), payload: line.payload() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitising_moderation_and_proximity() {
        assert_eq!(collapse_repeats("aaaaaaa!!!!!!"), "aaa!!!");
        assert_eq!(sanitize_text("  hi\nthere  "), "hi there");
        assert_eq!(sanitize_text(&"x1".repeat(60)).chars().count(), 80);
        let submitted = submit("HUNTER BEHIND YOU", "nearby", [1.0, 2.0], [3.0, 0.0, 4.0], 2).unwrap();
        let Message::SubmitChatPayload { payload } = &submitted else { panic!() };
        let mut moderation = ChatModeration::default();
        let out = relay_submission(&mut moderation, 42, "R-4823", payload, 0).unwrap();
        let line = ChatLine::from_message(&out).unwrap();
        assert_eq!((line.peer_id, line.channel.as_str(), line.text.as_str()), (42, "nearby", "HUNTER BEHIND YOU"));
        assert!(line.audible_from([3.0, 0.0, 6.0], 2, 5.0));
        assert!(!line.audible_from([3.0, 0.0, 40.0], 2, 5.0));
        assert!(!line.audible_from([3.0, 0.0, 4.0], 3, 5.0));
        assert!(relay_submission(&mut moderation, 42, "R-4823", payload, 1000).is_none(), "a duplicate is dropped");
        let line_n = |n: i64| submit(&format!("line number {n} here"), "team", [0.0; 2], [0.0; 3], 0).unwrap();
        let mut accepted = 0;
        for n in 0..5 {
            if let Message::SubmitChatPayload { payload } = line_n(n) {
                if relay_submission(&mut moderation, 7, "A", &payload, 100 + n).is_some() {
                    accepted += 1;
                }
            }
        }
        assert_eq!(accepted, 3, "3 lines per 10 s");
        assert!(relay_submission(&mut moderation, 1, "HOST", payload, 0).is_none(), "the host does not submit to itself");
        moderation.room_muted.insert(9);
        assert!(relay_submission(&mut moderation, 9, "M", payload, 0).is_none());
    }
    #[test]
    fn chat_rules_read_data_and_fall_back() {
        let embedded: Value = serde_json::from_str(include_str!("../data/chat.json")).unwrap();
        assert_eq!(ChatRules::parse(&embedded), ChatRules::default());
        let r = ChatRules::parse(&json!({ "max_lines": 2, "line_seconds": 1.5, "nearby_radius": 4.0 }));
        assert_eq!((r.max_lines, r.line_seconds, r.nearby_radius), (2, 1.5, 4.0));
        assert_eq!(ChatRules::parse(&json!({ "max_lines": "x", "line_seconds": -1 })), ChatRules::default());
    }

    #[test]
    fn the_feed_keeps_the_newest_lines_hears_only_near_ones_and_expires() {
        let rules = ChatRules::load();
        let mut feed = ChatFeed::default();
        let mk = |text: &str, channel: &str, z: f64| {
            let Some(Message::SubmitChatPayload { payload }) = submit(text, channel, [0.0; 2], [0.0, 0.0, z], 0) else { panic!() };
            ChatLine::sanitize(2, "A", &payload, false)
        };
        let many: Vec<ChatLine> = (0..rules.max_lines + 3).map(|n| mk(&format!("line {n}"), "team", 0.0)).collect();
        feed.push(many, [0.0; 3], 0);
        assert_eq!(feed.visible().count(), rules.max_lines);
        assert_eq!(feed.visible().last().unwrap().text, format!("line {}", rules.max_lines + 2));
        feed.push(vec![mk("far away", "nearby", rules.nearby_radius * 3.0)], [0.0; 3], 0);
        assert!(feed.visible().all(|l| l.text != "far away"), "a distant nearby line is not heard");
        feed.tick(rules.line_seconds + 1.0);
        assert_eq!(feed.visible().count(), 0, "lines expire");
        assert!(!feed.recent().is_empty(), "but the open box still shows them");
    }

    #[test]
    fn a_guest_over_the_data_rate_is_dropped_until_the_cooldown_ends() {
        let limits = ChatLimits { lines: 2, window_ms: 10_000, cooldown_ms: 5_000 };
        let mut m = ChatModeration::with_limits(limits);
        let say = |m: &mut ChatModeration, peer: i64, n: i64, now: i64| {
            let Some(Message::SubmitChatPayload { payload }) = submit(&format!("different words {n} here"), "team", [0.0; 2], [0.0; 3], 0) else { panic!() };
            relay_submission(m, peer, "G", &payload, now).is_some()
        };
        let sent: Vec<bool> = (0..6).map(|n| say(&mut m, 5, n, 100 + n)).collect();
        assert_eq!(sent.iter().filter(|s| **s).count(), 2, "only the data's lines per window pass: {sent:?}");
        assert!(say(&mut m, 6, 0, 120), "another guest has their own allowance");
        assert!(!say(&mut m, 5, 10, 4_000), "still cooling down");
        assert!(say(&mut m, 5, 11, 20_000), "allowed again after the window and cooldown");
        let rules = ChatRules::parse(&json!({ "rate_lines": 7, "rate_window_seconds": 4.0, "cooldown_seconds": 1.0 }));
        assert_eq!(ChatLimits::from_rules(&rules), ChatLimits { lines: 7, window_ms: 4_000, cooldown_ms: 1_000 });
        assert_eq!(ChatLimits::from_rules(ChatRules::load()), ChatLimits::default(), "Standard values");
    }

    #[test]
    fn lines_fade_over_the_last_seconds() {
        assert_eq!(fade_alpha(0.0, 12.0, 2.0), 1.0);
        assert_eq!(fade_alpha(10.0, 12.0, 2.0), 1.0);
        assert!((fade_alpha(11.0, 12.0, 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(fade_alpha(12.0, 12.0, 2.0), 0.0);
        assert_eq!(fade_alpha(11.9, 12.0, 0.0), 1.0, "no fade: full until it expires");
    }

}
