//! The PUBLIC ROUTE title button (Godot `public_route.gd`, `RoomState.PUBLIC_CODE`): one always-open room every runner
//! joins. The public route has NO player host in the design: a server process (`last_route --dedicated-host PUBLC`,
//! headless/dedicated.rs) holds the room and every runner joins it as a guest, with no owner, no kick and no route
//! command. Until that process is deployed, the first runner to find the room free holds it as a stand-in
//! (`client_host_fallback`), and when that runner leaves, the others hand the room over: the lowest session id opens
//! it again and the rest rejoin, so a host leaving never ends the room for the others ([`Reform`]).
//! The code, the fixed difficulty and the handover timings are data (`data/net/public_route.json`).
//!
//! The join itself is `world_sync::join_public_room`; this module holds the data and the decisions it makes: what a
//! directory lookup of the public code means, and who opens the room again when its holder is gone.

use super::api::RoomEndpoint;
use super::NetError;
use crate::config::difficulty::LOBBY;
use crate::config::Difficulty;
use crate::shared::clock::Instant;
use std::sync::OnceLock;
use std::time::Duration;

pub const SCHEMA: &str = "lr.public_route/1";
/// The code when the data file is missing or broken (the Godot constant).
const FALLBACK_CODE: &str = "PUBLC";
/// The directory drops a public entry this long after its last heartbeat (server `PUBLIC_ENET_TTL_MS`).
pub const DIRECTORY_PUBLIC_TTL_SECONDS: f64 = 15.0;
/// The longest allowed public heartbeat interval: a stalled beat (see `HEARTBEAT_TIMEOUT_SECONDS`) still lands in time.
pub const MAX_HEARTBEAT_SECONDS: f64 = 10.0;
/// Request timeout of a directory heartbeat; with the interval it stays under `DIRECTORY_PUBLIC_TTL_SECONDS`.
pub const HEARTBEAT_TIMEOUT_SECONDS: u64 = 4;

/// The parsed `lr.public_route/1` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub code: String,
    /// A runner that opens the public room starts the route at once (nobody else is there to press START).
    pub start_when_hosting: bool,
    /// Seconds between directory heartbeats of the public room (the directory forgets a public entry after 15 s).
    pub heartbeat_seconds: f64,
    /// The one difficulty of the public route. Nobody on it chooses another: a runner who holds the room as a stand-in
    /// for the server runs this, and a guest takes the room's.
    pub difficulty: Difficulty,
    /// Whether a runner may open the public room when nobody holds it (no server process is up). Off once the server
    /// process holds the route: a client then never becomes its host.
    pub client_host_fallback: bool,
    /// Seconds between the handover turns of two runners who both want to open the room: the runner with the lowest
    /// session id opens it at once, the next one this many seconds later, and so on.
    pub handover_step_seconds: f64,
    /// Seconds a runner keeps trying to reach the room again before it gives up and goes to the title.
    pub handover_grace_seconds: f64,
    /// Seconds the runners wait for the server process to come back when IT was the host, before a stand-in may open
    /// the room (a restart takes a few seconds).
    pub handover_dedicated_seconds: f64,
    /// Seconds between two tries to reach the room again.
    pub handover_retry_seconds: f64,
    /// Seconds a rejoined room has to link before the try counts as failed.
    pub handover_settle_seconds: f64,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            code: FALLBACK_CODE.to_string(),
            start_when_hosting: true,
            heartbeat_seconds: 5.0,
            difficulty: Difficulty::Standard,
            client_host_fallback: true,
            handover_step_seconds: 1.5,
            handover_grace_seconds: 30.0,
            handover_dedicated_seconds: 45.0,
            handover_retry_seconds: 1.0,
            handover_settle_seconds: 6.0,
        }
    }
}

const KEYS: [&str; 12] = [
    "schema",
    "_about",
    "code",
    "start_when_hosting",
    "heartbeat_seconds",
    "difficulty",
    "client_host_fallback",
    "handover_step_seconds",
    "handover_grace_seconds",
    "handover_dedicated_seconds",
    "handover_retry_seconds",
    "handover_settle_seconds",
];

/// Parses the file. The code must be five letters or digits; unknown keys are errors, so a typo never goes unnoticed.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.as_object().ok_or("public_route is not an object")?;
    if let Some(k) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("public_route: unknown key {k}"));
    }
    if o.get("schema").and_then(|s| s.as_str()) != Some(SCHEMA) {
        return Err(format!("public_route.schema must be {SCHEMA}"));
    }
    let mut rules = Rules::default();
    if let Some(code) = o.get("code") {
        let code = code.as_str().ok_or("public_route.code must be text")?;
        if code.len() != 5 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("public_route.code must be five letters or digits".into());
        }
        rules.code = code.to_ascii_uppercase();
    }
    if let Some(b) = o.get("start_when_hosting") {
        rules.start_when_hosting = b.as_bool().ok_or("public_route.start_when_hosting must be true or false")?;
    }
    if let Some(n) = o.get("heartbeat_seconds") {
        let n = n.as_f64().ok_or("public_route.heartbeat_seconds must be a number")?;
        if !(1.0..=MAX_HEARTBEAT_SECONDS).contains(&n) {
            return Err(format!("public_route.heartbeat_seconds must be from 1 to {MAX_HEARTBEAT_SECONDS}"));
        }
        rules.heartbeat_seconds = n;
    }
    if let Some(d) = o.get("difficulty") {
        let name = d.as_str().ok_or("public_route.difficulty must be text")?.trim().to_ascii_uppercase();
        rules.difficulty = LOBBY
            .iter()
            .copied()
            .find(|d| d.name() == name)
            .ok_or_else(|| format!("public_route.difficulty {name} is not a difficulty"))?;
    }
    if let Some(b) = o.get("client_host_fallback") {
        rules.client_host_fallback = b.as_bool().ok_or("public_route.client_host_fallback must be true or false")?;
    }
    for (key, slot, low, high) in [
        ("handover_step_seconds", &mut rules.handover_step_seconds, 0.2, 10.0),
        ("handover_grace_seconds", &mut rules.handover_grace_seconds, 5.0, 300.0),
        ("handover_dedicated_seconds", &mut rules.handover_dedicated_seconds, 0.0, 300.0),
        ("handover_retry_seconds", &mut rules.handover_retry_seconds, 0.2, 10.0),
        ("handover_settle_seconds", &mut rules.handover_settle_seconds, 1.0, 60.0),
    ] {
        if let Some(n) = o.get(key) {
            let n = n.as_f64().ok_or_else(|| format!("public_route.{key} must be a number"))?;
            if !(low..=high).contains(&n) {
                return Err(format!("public_route.{key} must be from {low} to {high}"));
            }
            *slot = n;
        }
    }
    Ok(rules)
}

/// Whether `code` is the public route's. Its host needs no authorization: the server takes a relay HOST_REGISTER on it
/// without a token and answers `HOST_JOIN_AS_GUEST` when a live host already holds it (docs/SERVERS.md).
pub fn is_public(code: &str) -> bool {
    code == rules().code
}

/// Seconds between directory heartbeats for a hosted room: the short public interval for the public code, the
/// ordinary `HEARTBEAT_SECONDS` for every other room. The code match is exact, like the server's.
pub fn heartbeat_seconds(code: &str) -> f64 {
    let r = rules();
    if code == r.code {
        r.heartbeat_seconds
    } else {
        super::config::HEARTBEAT_SECONDS
    }
}

/// The embedded rules (data/net/public_route.json), read once; the defaults when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("net/public_route.json")
            .next()
            .and_then(|(_, text)| parse(text).ok())
            .unwrap_or_default()
    })
}

/// Text the owner controls answer with on the public route: it has no owner.
pub const NO_OWNER_TEXT: &str = "THE PUBLIC ROUTE HAS NO OWNER // NO ROUTE COMMANDS";

/// Whether a reason a room link ended is a removal by the room (a kick or a moderation removal) rather than the host
/// going away: a removed runner goes to the title and does not rejoin.
pub fn is_removal(reason: &str) -> bool {
    reason.to_ascii_uppercase().contains("REMOVED")
}

/// What a guest of the public room knows when the room's host is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// This runner's place in the line to open the room again (0 opens it first); None when it must only join: it lost
    /// a race for the room, so another runner holds it.
    pub rank: Option<usize>,
    /// The host that left was a server process, not a runner.
    pub was_dedicated: bool,
}

impl Plan {
    /// Joins only, never opens the room.
    pub fn join_only() -> Plan {
        Plan { rank: None, was_dedicated: false }
    }
}

/// The plan of guest `me` (its relay session id) in a room whose runners the host last listed as `peers` (their session
/// ids, the host's peer 1 not among them). The runner with the lowest session id is first in line: on the WebSocket relay
/// ids count up, so that is the oldest joiner; on the UDP relay they are random, which is as fair and as deterministic.
/// A runner the host had not listed yet counts as nobody: a clash with another first-in-line is settled by the directory
/// (only one registration of the code succeeds).
pub fn plan(me: u32, peers: &[u32], was_dedicated: bool) -> Plan {
    let ahead = peers.iter().filter(|p| **p > 1 && **p < me).count();
    Plan { rank: Some(ahead), was_dedicated }
}

/// One runner's way back into the public route after its host is gone: when to try, and whether this try may open the room.
#[derive(Clone, Debug)]
pub struct Reform {
    plan: Plan,
    started: Instant,
    next_try: Instant,
    tries: u32,
    rules: Rules,
}

/// A beat before the first try, so the relay and the directory have let go of the old host.
const FIRST_TRY_AFTER: Duration = Duration::from_millis(300);

impl Reform {
    pub fn new(plan: Plan, now: Instant) -> Reform {
        Reform::with_rules(plan, now, rules().clone())
    }

    pub fn with_rules(plan: Plan, now: Instant, rules: Rules) -> Reform {
        Reform { plan, started: now, next_try: now + FIRST_TRY_AFTER, tries: 0, rules }
    }

    pub fn plan(&self) -> Plan {
        self.plan
    }

    /// How many tries were made.
    pub fn tries(&self) -> u32 {
        self.tries
    }

    /// When a runner of this rank may first open the room: after the server process had its time to come back (when it was
    /// the host), then after the runners ahead in line had theirs.
    fn host_from(&self) -> Option<Duration> {
        let rank = self.plan.rank?;
        let wait = if self.plan.was_dedicated { self.rules.handover_dedicated_seconds } else { 0.0 };
        Some(Duration::from_secs_f64(wait + self.rules.handover_step_seconds * rank.min(1000) as f64))
    }

    /// Whether a try is due now.
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next_try
    }

    /// A try was made (and did not end the wait): the next one comes after the retry interval.
    pub fn tried(&mut self, now: Instant) {
        self.tries += 1;
        self.next_try = now + Duration::from_secs_f64(self.rules.handover_retry_seconds);
    }

    /// Whether a try made now may open the room when it finds none. Never when the data turned the stand-in off
    /// (a server process holds the route) or this runner only joins; not before its turn in line.
    pub fn may_host(&self, now: Instant) -> bool {
        if !self.rules.client_host_fallback {
            return false;
        }
        self.host_from().is_some_and(|from| now.duration_since(self.started) >= from)
    }

    /// Whether the time is up: the runner has had its turn and the grace after it.
    pub fn expired(&self, now: Instant) -> bool {
        let from = self.host_from().unwrap_or_default();
        now.duration_since(self.started) >= from + Duration::from_secs_f64(self.rules.handover_grace_seconds)
    }

    /// How long a rejoined room has to link before the try counts as failed.
    pub fn settle(&self) -> Duration {
        Duration::from_secs_f64(self.rules.handover_settle_seconds)
    }
}

/// What the directory said about the public code.
#[derive(Debug)]
pub enum Step {
    /// A runner holds the route: join it.
    Join(RoomEndpoint),
    /// Nobody holds it (404): open it and hold the route.
    Host,
}

/// The step for a lookup of the public code. Only a miss (404) opens the room; any other failure is returned as it is.
pub fn step(lookup: Result<RoomEndpoint, NetError>) -> Result<Step, NetError> {
    match lookup {
        Ok(room) => Ok(Step::Join(room)),
        Err(NetError::Http { status: 404, .. }) => Ok(Step::Host),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn miss(status: u16) -> NetError {
        NetError::Http { status, code: String::new(), error: "ROOM NOT FOUND".into(), body: Value::Null }
    }

    #[test]
    fn the_embedded_file_names_the_godot_public_code() {
        let r = parse(include_str!("../data/net/public_route.json")).expect("parses");
        assert_eq!(r.code, "PUBLC", "Godot RoomState.PUBLIC_CODE");
        assert_eq!(rules(), &r);
    }

    #[test]
    fn changing_the_data_changes_the_code_and_a_bad_file_is_refused() {
        let text = include_str!("../data/net/public_route.json");
        let other = text.replace("PUBLC", "TOWNS");
        assert_ne!(other, text);
        assert_eq!(parse(&other).unwrap().code, "TOWNS");
        assert!(parse(&text.replace("PUBLC", "TOO-LONG")).is_err());
        assert!(parse(&text.replace("\"code\"", "\"cod\"")).unwrap_err().contains("cod"));
    }

    #[test]
    fn the_public_heartbeat_beats_the_directory_ttl_and_private_rooms_keep_20_s() {
        assert!(heartbeat_seconds("PUBLC") < DIRECTORY_PUBLIC_TTL_SECONDS);
        assert_eq!(heartbeat_seconds("publc"), super::super::config::HEARTBEAT_SECONDS, "the match is case-sensitive");
        assert!(MAX_HEARTBEAT_SECONDS + HEARTBEAT_TIMEOUT_SECONDS as f64 + 1.0 <= DIRECTORY_PUBLIC_TTL_SECONDS);
        assert_eq!(heartbeat_seconds("ABCDE"), super::super::config::HEARTBEAT_SECONDS);
        let text = include_str!("../data/net/public_route.json");
        assert!(parse(&text.replace("\"heartbeat_seconds\": 5", "\"heartbeat_seconds\": 20")).is_err());
        assert!(parse(&text.replace("\"heartbeat_seconds\": 5", "\"heartbeat_seconds\": 10.5")).is_err());
        assert!(parse(&text.replace("\"heartbeat_seconds\": 5", "\"heartbeat_seconds\": 10")).is_ok());
    }

    #[test]
    fn the_embedded_file_fixes_the_difficulty_and_keeps_the_stand_in_on_until_the_server_holds_the_route() {
        let r = rules();
        assert_eq!(r.difficulty, Difficulty::Standard, "nobody on the public route chooses a difficulty");
        assert!(r.client_host_fallback, "before the server process is deployed a runner must be able to open the room");
        assert!(r.handover_dedicated_seconds > r.handover_step_seconds, "the server process gets longer to come back than a runner's turn");
        let text = include_str!("../data/net/public_route.json");
        assert_eq!(parse(&text.replace("\"STANDARD\"", "\"hard\"")).unwrap().difficulty, Difficulty::Hard);
        assert!(parse(&text.replace("\"STANDARD\"", "\"IMPOSSIBLE\"")).unwrap_err().contains("difficulty"));
        assert!(!parse(&text.replace("\"client_host_fallback\": true", "\"client_host_fallback\": false")).unwrap().client_host_fallback);
        assert!(parse(&text.replace("\"handover_step_seconds\": 1.5", "\"handover_step_seconds\": 0")).is_err());
        assert!(parse(&text.replace("\"handover_grace_seconds\": 30", "\"handover_grace_seconds\": \"soon\"")).is_err());
    }

    #[test]
    fn the_runner_with_the_lowest_session_id_is_first_in_line() {
        // the host listed 7, 9 and 12; the host itself (peer 1) is never in line
        let peers = [7, 9, 12];
        assert_eq!(plan(7, &peers, false), Plan { rank: Some(0), was_dedicated: false });
        assert_eq!(plan(9, &peers, false).rank, Some(1));
        assert_eq!(plan(12, &peers, true), Plan { rank: Some(2), was_dedicated: true });
        // the order does not depend on the order of the list
        assert_eq!(plan(9, &[12, 9, 7], false).rank, Some(1));
        assert_eq!(plan(5, &[1, 7], false).rank, Some(0), "peer 1 is the host, not a runner");
        // a host that sent no ids (an older host) leaves everyone first in line; the directory settles the clash
        assert_eq!(plan(30, &[], false).rank, Some(0));
        assert_eq!(Plan::join_only().rank, None);
    }

    #[test]
    fn a_removal_is_not_a_host_leaving() {
        assert!(is_removal("YOU WERE REMOVED FROM THE ROOM"));
        assert!(!is_removal("THE HOST LEFT THE ROOM"));
        assert!(!is_removal("HOST LEFT // ROOM CLOSED"));
        assert!(!is_removal("LOST THE CONNECTION TO THE ROOM"));
        assert!(!is_removal("THE PUBLIC ROUTE HAS A HOST // JOIN IT"));
    }

    fn reform(plan: Plan, rules: Rules) -> (Reform, Instant) {
        let t0 = Instant::now();
        (Reform::with_rules(plan, t0, rules), t0)
    }

    #[test]
    fn only_the_first_in_line_may_open_the_room_at_once_and_the_others_wait_their_turn() {
        let rules = Rules::default();
        let step = Duration::from_secs_f64(rules.handover_step_seconds);
        let (first, t0) = reform(Plan { rank: Some(0), was_dedicated: false }, rules.clone());
        let at_t0 = |rank| Reform::with_rules(Plan { rank: Some(rank), was_dedicated: false }, t0, rules.clone());
        let (second, third) = (at_t0(1), at_t0(2));
        assert!(first.may_host(t0) && !second.may_host(t0) && !third.may_host(t0), "one runner opens the room first");
        assert!(second.may_host(t0 + step) && !third.may_host(t0 + step));
        assert!(third.may_host(t0 + step * 2), "the next in line takes over if the ones ahead never came");
        // nobody tries before the beat that lets the relay forget the old host, and then every retry interval
        assert!(!first.due(t0) && first.due(t0 + FIRST_TRY_AFTER));
    }

    #[test]
    fn a_runner_that_only_joins_never_opens_the_room_and_the_data_can_turn_the_stand_in_off() {
        let (only, t0) = reform(Plan::join_only(), Rules::default());
        assert!(!only.may_host(t0 + Duration::from_secs(3600)), "lost the race: another runner holds the route");
        assert!(only.expired(t0 + Duration::from_secs_f64(Rules::default().handover_grace_seconds)));
        let off = Rules { client_host_fallback: false, ..Rules::default() };
        let (first, t0) = reform(Plan { rank: Some(0), was_dedicated: true }, off);
        assert!(!first.may_host(t0 + Duration::from_secs(3600)), "with the server process holding the route no client opens it");
    }

    #[test]
    fn a_runner_waits_for_the_server_process_to_come_back_before_a_stand_in_opens_the_room() {
        let rules = Rules::default();
        let wait = Duration::from_secs_f64(rules.handover_dedicated_seconds);
        let (first, t0) = reform(Plan { rank: Some(0), was_dedicated: true }, rules);
        assert!(!first.may_host(t0 + Duration::from_secs(10)), "a systemd restart takes a few seconds");
        assert!(first.may_host(t0 + wait));
    }

    #[test]
    fn a_runner_gives_up_after_its_turn_and_the_grace_and_retries_on_the_interval() {
        let rules = Rules::default();
        let grace = Duration::from_secs_f64(rules.handover_grace_seconds);
        let (mut r, t0) = reform(Plan { rank: Some(2), was_dedicated: false }, rules.clone());
        let turn = Duration::from_secs_f64(rules.handover_step_seconds * 2.0);
        assert!(!r.expired(t0 + grace), "the turn in line comes before the grace");
        assert!(r.expired(t0 + turn + grace));
        let now = t0 + Duration::from_secs(1);
        r.tried(now);
        assert_eq!(r.tries(), 1);
        assert!(!r.due(now) && r.due(now + Duration::from_secs_f64(rules.handover_retry_seconds)));
    }

    #[test]
    fn only_a_miss_opens_the_public_room() {
        assert!(matches!(step(Err(miss(404))), Ok(Step::Host)));
        assert!(matches!(step(Err(miss(500))), Err(NetError::Http { status: 500, .. })));
        assert!(matches!(step(Err(NetError::Offline)), Err(NetError::Offline)));
        assert!(matches!(step(Ok(RoomEndpoint::default())), Ok(Step::Join(_))));
    }
}
