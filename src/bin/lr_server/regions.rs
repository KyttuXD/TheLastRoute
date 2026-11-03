//! Port of `backend/regions.mjs`. Each VPS runs a region agent (`region_agent.mjs`) that spawns and stops dedicated
//! headless hosts. The directory (this backend, VPS 3) stays the one list of rooms; agents report to it every few
//! seconds and pick up spawn/stop requests in the answer, so an agent needs no inbound port.
//!
//! Agent requests are signed and never carry the token: `x-lr-region`, `x-lr-time` (ms) and `x-lr-signature` =
//! HMAC-SHA256(token, `${region}\n${time}\n${body}`) hex. Tokens come from `LASTROUTE_AGENT_TOKENS` on the directory
//! (`"virginia:<token>,texas:<token>"`).

use crate::util::{self, Clock, Reply};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const SIGNATURE_WINDOW_MS: i64 = 60_000;
pub const REGION_STALE_MS: i64 = 20_000;

/// Matches `^[a-z][a-z0-9-]{1,23}$`.
pub fn is_region(region: &str) -> bool {
    let bytes = region.as_bytes();
    (2..=24).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `ROOM_CODE = /^[A-Z0-9]{5}$/`
fn is_room_code(code: &str) -> bool {
    code.len() == 5 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `signAgentRequest(token, region, time, body)`; `time` is formatted like a JS number.
pub fn sign_agent_request(token: &str, region: &str, time: f64, body: &str) -> String {
    let message = format!("{region}\n{}\n{body}", util::js_number_string(time));
    util::hex(&util::hmac_sha256(token.as_bytes(), message.as_bytes()))
}

/// `parseAgentTokens(value)`: `"region:token,..."`; regions must pass [`is_region`], tokens be 24+ characters.
pub fn parse_agent_tokens(value: &str) -> HashMap<String, String> {
    let mut tokens = HashMap::new();
    for pair in value.split(',') {
        let mut parts = pair.split(':');
        let region = util::js_trim(parts.next().unwrap_or("")).to_string();
        let token = util::js_trim(parts.next().unwrap_or("")).to_string();
        if is_region(&region) && !token.is_empty() && util::js_len(&token) >= 24 {
            tokens.insert(region, token);
        }
    }
    tokens
}

fn safe_hex_equal(left: &str, right: &str) -> bool {
    let a = util::from_hex(left);
    let b = util::from_hex(right);
    a.len() == b.len() && !a.is_empty() && util::timing_safe_equal(&a, &b)
}

/// `createRegions({...})` options.
pub struct RegionsOptions {
    /// region -> token (see [`parse_agent_tokens`])
    pub tokens: HashMap<String, String>,
    pub now: Clock,
}

impl Default for RegionsOptions {
    fn default() -> Self {
        RegionsOptions { tokens: HashMap::new(), now: util::system_clock() }
    }
}

#[derive(Clone, Debug)]
struct RegionStatus {
    region: String,
    label: String,
    relay: Value,
    capacity: f64,
    free: f64,
    load: f64,
    cores: f64,
    mem_free_mb: f64,
    rooms: Vec<Value>,
    version: String,
    last_seen: i64,
}

impl RegionStatus {
    fn to_map(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("region".into(), Value::String(self.region.clone()));
        map.insert("label".into(), Value::String(self.label.clone()));
        map.insert("relay".into(), self.relay.clone());
        map.insert("capacity".into(), util::num(self.capacity));
        map.insert("free".into(), util::num(self.free));
        map.insert("load".into(), util::num(self.load));
        map.insert("cores".into(), util::num(self.cores));
        map.insert("memFreeMb".into(), util::num(self.mem_free_mb));
        map.insert("rooms".into(), Value::Array(self.rooms.clone()));
        map.insert("version".into(), Value::String(self.version.clone()));
        map.insert("lastSeen".into(), Value::from(self.last_seen));
        map
    }
}

/// The region directory (`createRegions`).
pub struct Regions {
    tokens: HashMap<String, String>,
    now: Clock,
    /// region -> last report, in first-report order (a JS Map)
    regions: Vec<RegionStatus>,
    /// region -> queued actions `{id, action, code, options?}`
    pending: HashMap<String, Vec<Value>>,
    /// region -> last accepted x-lr-time (replay guard)
    seen: HashMap<String, f64>,
}

/// `Number(x) || 0`.
fn number_or_zero(value: Option<&Value>) -> f64 {
    util::number_or(value, 0.0)
}

fn action_id() -> String {
    util::hex(&util::random_bytes(6))
}

impl Regions {
    pub fn new(options: RegionsOptions) -> Regions {
        Regions { tokens: options.tokens, now: options.now, regions: Vec::new(), pending: HashMap::new(), seen: HashMap::new() }
    }

    fn status(&self, region: &str) -> Option<&RegionStatus> {
        self.regions.iter().find(|status| status.region == region)
    }

    /// The region a signed request comes from, or "" when the signature, clock or region is wrong. `headers` has
    /// lower-case names; `raw_body` is the exact request body that was signed.
    pub fn verify(&mut self, headers: &HashMap<String, String>, raw_body: &str) -> String {
        let region = headers.get("x-lr-region").cloned().unwrap_or_default();
        let time = match headers.get("x-lr-time") {
            Some(text) => util::js_number_from_str(text),
            None => f64::NAN,
        };
        let Some(token) = self.tokens.get(&region) else { return String::new() };
        if !time.is_finite() || ((self.now)() as f64 - time).abs() > SIGNATURE_WINDOW_MS as f64 {
            return String::new();
        }
        let last = self.seen.get(&region).copied().unwrap_or(0.0);
        if time <= last {
            return String::new();
        }
        let signature = match headers.get("x-lr-signature") {
            Some(text) => text.clone(),
            None => "undefined".into(),
        };
        if !safe_hex_equal(&signature, &sign_agent_request(token, &region, time, raw_body)) {
            return String::new();
        }
        self.seen.insert(region.clone(), time);
        region
    }

    /// An agent's status report; returns the actions it should carry out now (each handed out once).
    pub fn report(&mut self, region: &str, body: &Value) -> Vec<Value> {
        let rooms: Vec<Value> = match util::field(body, "rooms") {
            Some(Value::Array(items)) => items
                .iter()
                .take(64)
                .filter(|room| is_room_code(&util::str_or_empty(util::field(room, "code"))))
                .map(|room| {
                    json!({
                        "code": util::str_nullish(util::field(room, "code")),
                        "pid": util::num(number_or_zero(util::field(room, "pid"))),
                        "uptime": util::num(number_or_zero(util::field(room, "uptime"))),
                        "rssMb": util::num(number_or_zero(util::field(room, "rssMb"))),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        let capacity = number_or_zero(util::field(body, "capacity")).min(64.0).max(0.0);
        let label_source = util::field(body, "label");
        let label = if util::truthy(label_source) { util::js_string_of(label_source.unwrap()) } else { region.to_string() };
        let relay = match util::field(body, "relay") {
            Some(relay @ (Value::Object(_) | Value::Array(_))) => json!({
                "address": util::js_slice(&util::str_or_empty(util::field(relay, "address")), 100),
                "port": util::num(number_or_zero(util::field(relay, "port"))),
            }),
            _ => Value::Null,
        };
        let status = RegionStatus {
            region: region.to_string(),
            label: util::js_slice(&label, 40),
            relay,
            capacity,
            free: (capacity - rooms.len() as f64).max(0.0),
            load: number_or_zero(util::field(body, "load")),
            cores: number_or_zero(util::field(body, "cores")),
            mem_free_mb: number_or_zero(util::field(body, "memFreeMb")),
            rooms,
            version: util::js_slice(&util::str_or_empty(util::field(body, "version")), 40),
            last_seen: (self.now)(),
        };
        match self.regions.iter_mut().find(|existing| existing.region == region) {
            Some(existing) => *existing = status,
            None => self.regions.push(status),
        }
        self.pending.remove(region).unwrap_or_default()
    }

    /// `live(region)`: reported within [`REGION_STALE_MS`].
    pub fn live(&self, region: &str) -> bool {
        self.status(region).map(|status| (self.now)() - status.last_seen <= REGION_STALE_MS).unwrap_or(false)
    }

    /// Public: where a client may host or ping (no load details beyond free slots).
    pub fn public_list(&self) -> Vec<Value> {
        self.regions
            .iter()
            .filter(|status| self.live(&status.region))
            .map(|status| {
                json!({ "region": status.region, "label": status.label, "relay": status.relay, "free": util::num(status.free) })
            })
            .collect()
    }

    /// Owner/backend: queue a dedicated host in a region. `options` is `{seed, difficulty, idleMinutes, spawnTicket}`
    /// (missing / null fields behave like undefined). 202 `{ok, queued}` (the spawn ticket is never echoed).
    pub fn request_spawn(&mut self, region: &str, code: &str, options: &Value) -> Reply {
        if !self.live(region) {
            return (409, json!({ "ok": false, "code": "REGION_OFFLINE", "error": format!("Region {region} is not reporting") }));
        }
        let free = self.status(region).map(|status| status.free).unwrap_or(0.0);
        let queued = self
            .pending
            .get(region)
            .map(|entries| entries.iter().filter(|entry| entry["action"] == "spawn").count())
            .unwrap_or(0);
        if free - queued as f64 <= 0.0 {
            return (409, json!({ "ok": false, "code": "REGION_FULL", "error": format!("Region {region} has no free slot") }));
        }
        if !is_room_code(code) {
            return (400, json!({ "ok": false, "code": "INVALID_ROOM_CODE", "error": "Room codes are 5 letters/digits" }));
        }
        let ticket = util::field(options, "spawnTicket");
        let seed = util::js_slice(&util::str_or_empty(util::field(options, "seed")), 40);
        let difficulty = util::num(number_or_zero(util::field(options, "difficulty")));
        let idle_minutes = util::num(util::number_or(util::field(options, "idleMinutes"), 10.0).max(1.0).min(240.0));
        let action = json!({
            "id": action_id(),
            "action": "spawn",
            "code": code,
            "options": {
                "seed": seed,
                "difficulty": difficulty,
                "idleMinutes": idle_minutes,
                "spawnTicket": if util::truthy(ticket) { ticket.cloned().unwrap() } else { Value::String(String::new()) },
            },
        });
        let mut echoed = action.clone();
        if let Some(Value::Object(options)) = echoed.get_mut("options") {
            options.remove("spawnTicket");
        }
        self.pending.entry(region.to_string()).or_default().push(action);
        (202, json!({ "ok": true, "queued": echoed }))
    }

    /// Owner: ask a region's agent to stop the dedicated host for `code`.
    pub fn request_stop(&mut self, region: &str, code: &str) -> Reply {
        if self.status(region).is_none() {
            return (404, json!({ "ok": false, "code": "UNKNOWN_REGION", "error": "Unknown region" }));
        }
        let action = json!({ "id": action_id(), "action": "stop", "code": code });
        self.pending.entry(region.to_string()).or_default().push(action.clone());
        (202, json!({ "ok": true, "queued": action }))
    }

    /// Owner view: everything the agents reported, plus `live` and the queued `{action, code}` list.
    pub fn owner_list(&self) -> Vec<Value> {
        self.regions
            .iter()
            .map(|status| {
                let mut map = status.to_map();
                map.insert("live".into(), Value::Bool(self.live(&status.region)));
                let pending: Vec<Value> = self
                    .pending
                    .get(&status.region)
                    .map(|entries| entries.iter().map(|entry| json!({ "action": entry["action"], "code": entry["code"] })).collect())
                    .unwrap_or_default();
                map.insert("pending".into(), Value::Array(pending));
                Value::Object(map)
            })
            .collect()
    }

    /// The region currently running a dedicated host for `code`, or "".
    pub fn region_of(&self, code: &str) -> String {
        for status in &self.regions {
            if self.live(&status.region) && status.rooms.iter().any(|room| room["code"] == code) {
                return status.region.clone();
            }
        }
        String::new()
    }

    #[cfg(test)]
    pub fn has_tokens(&self) -> bool {
        !self.tokens.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "virginia-test-token-0123456789abcdef";

    fn signed(body: &str, time: i64, token: &str) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-lr-region".to_string(), "virginia".to_string());
        headers.insert("x-lr-time".to_string(), time.to_string());
        headers.insert("x-lr-signature".to_string(), sign_agent_request(token, "virginia", time as f64, body));
        headers
    }

    #[test]
    fn agent_requests_are_signed_and_a_bad_replayed_or_stale_signature_is_refused() {
        let (clock, handle) = util::manual_clock(1_000_000);
        let mut regions = Regions::new(RegionsOptions { tokens: parse_agent_tokens(&format!("virginia:{TOKEN},bad:short")), now: clock });
        let body = r#"{"capacity":3}"#;
        assert_eq!(regions.verify(&signed(body, handle.get(), TOKEN), body), "virginia");
        assert_eq!(regions.verify(&signed(body, handle.get(), TOKEN), body), "", "a replayed request (same time) is refused");
        handle.advance(1);
        assert_eq!(regions.verify(&signed(body, handle.get(), "wrong-token-0123456789abcdef"), body), "", "wrong token");
        assert_eq!(regions.verify(&signed(body, handle.get(), TOKEN), &format!("{body} ")), "", "a changed body breaks the signature");
        assert_eq!(regions.verify(&signed(body, handle.get() - 120_000, TOKEN), body), "", "a stale clock is refused");
        assert_eq!(parse_agent_tokens("bad:short").len(), 0, "short tokens are ignored");
        assert!(regions.has_tokens());
    }

    #[test]
    fn signatures_match_node() {
        // node: createHmac("sha256", TOKEN).update("virginia\n1000000\n{\"capacity\":3}").digest("hex")
        assert_eq!(
            sign_agent_request(TOKEN, "virginia", 1_000_000.0, r#"{"capacity":3}"#),
            "a8b22b82804491422db226b360f2dd4c892f9552c9cc64fe980142a142831cd9"
        );
        let (clock, _) = util::manual_clock(1_000_000);
        let mut regions = Regions::new(RegionsOptions { tokens: parse_agent_tokens(&format!("virginia:{TOKEN}")), now: clock });
        let mut headers = signed("{}", 1_000_000, TOKEN);
        headers.remove("x-lr-signature");
        assert_eq!(regions.verify(&headers, "{}"), "", "no signature header");
        headers.insert("x-lr-time".into(), "soon".into());
        assert_eq!(regions.verify(&headers, "{}"), "");
        let tokens = parse_agent_tokens(&format!(" virginia : {TOKEN} :extra, Texas:{TOKEN},texas:{TOKEN}"));
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens["virginia"], TOKEN);
    }

    // The module half of "agent <-> directory": report, owner spawn, stop. The HTTP routing, spawn tickets and the
    // agent itself (region_agent.mjs) belong to the router / are not ported.
    #[test]
    fn report_owner_spawn_and_stop() {
        let (clock, handle) = util::manual_clock(5_000_000);
        let mut regions = Regions::new(RegionsOptions { tokens: parse_agent_tokens(&format!("virginia:{TOKEN}")), now: clock });
        assert!(regions.public_list().is_empty());
        let report = json!({ "label": "US East", "relay": { "address": "vps1.example", "port": 1105 }, "capacity": 2,
            "load": 0.1, "cores": 12, "memFreeMb": 30000, "rooms": [], "version": "region-agent/1" });
        assert!(regions.report("virginia", &report).is_empty());
        let listed = regions.public_list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], json!({ "region": "virginia", "label": "US East", "relay": { "address": "vps1.example", "port": 1105 }, "free": 2 }));

        let queued = regions.request_spawn("virginia", "KATHY",
            &json!({ "seed": "KT SEED", "difficulty": null, "idleMinutes": 15, "spawnTicket": "secret-ticket" }));
        assert_eq!(queued.0, 202, "{}", queued.1);
        assert_eq!(queued.1["queued"]["options"], json!({ "seed": "KT SEED", "difficulty": 0, "idleMinutes": 15 }),
            "the owner never sees the spawn ticket");
        assert_eq!(regions.request_spawn("texas", "AB2CD", &json!({})).1["code"], json!("REGION_OFFLINE"));
        assert_eq!(regions.request_spawn("virginia", "bad", &json!({})).1["code"], json!("INVALID_ROOM_CODE"));
        assert_eq!(regions.request_spawn("virginia", "AB2CD", &json!({})).0, 202);
        assert_eq!(regions.request_spawn("virginia", "AB3CD", &json!({})).1["code"], json!("REGION_FULL"));

        let owner = regions.owner_list();
        assert_eq!(owner[0]["pending"], json!([{ "action": "spawn", "code": "KATHY" }, { "action": "spawn", "code": "AB2CD" }]));
        assert_eq!(owner[0]["live"], json!(true));

        let actions = regions.report("virginia", &json!({ "capacity": 2, "rooms": [{ "code": "KATHY", "pid": 4242, "uptime": 3 }, { "code": "nope" }] }));
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["options"]["spawnTicket"], json!("secret-ticket"), "the agent gets the ticket");
        assert_eq!(actions[0]["options"]["idleMinutes"], json!(15));
        assert_eq!(actions[1]["options"]["idleMinutes"], json!(10));
        assert!(regions.report("virginia", &json!({ "capacity": 2 })).is_empty(), "each action is handed out once");
        regions.report("virginia", &json!({ "capacity": 2, "rooms": [{ "code": "KATHY", "pid": 4242 }] }));

        let owner = regions.owner_list();
        assert_eq!(owner[0]["rooms"], json!([{ "code": "KATHY", "pid": 4242, "uptime": 0, "rssMb": 0 }]));
        assert_eq!(owner[0]["capacity"], json!(2));
        assert_eq!(owner[0]["label"], json!("virginia"), "the label falls back to the region");
        assert_eq!(owner[0]["relay"], Value::Null);
        assert_eq!(regions.region_of("KATHY"), "virginia");
        assert_eq!(regions.request_stop("texas", "KATHY").0, 404);
        let stop = regions.request_stop("virginia", "KATHY");
        assert_eq!(stop.0, 202);
        assert_eq!(stop.1["queued"]["action"], json!("stop"));
        assert_eq!(stop.1["queued"]["id"].as_str().unwrap().len(), 12);
        let actions = regions.report("virginia", &json!({ "capacity": 2, "rooms": [] }));
        assert_eq!(actions, vec![stop.1["queued"].clone()]);

        handle.advance(REGION_STALE_MS + 1);
        assert!(!regions.live("virginia"));
        assert!(regions.public_list().is_empty());
        assert_eq!(regions.owner_list()[0]["live"], json!(false));
        assert_eq!(regions.region_of("KATHY"), "");
        assert_eq!(regions.request_spawn("virginia", "KATHY", &json!({})).1["code"], json!("REGION_OFFLINE"));
    }
}
