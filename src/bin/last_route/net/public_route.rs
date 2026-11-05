//! The PUBLIC ROUTE title button (Godot `public_route.gd`, `RoomState.PUBLIC_CODE`): one always-open room every runner
//! joins. The first runner to find the room free opens it and holds the route; every other runner joins that host.
//! The code and whether a hosting runner starts the route at once are data (`data/net/public_route.json`).
//!
//! The join itself is `world_sync::join_public_room`; this module holds the data and the one decision it makes: what a
//! directory lookup of the public code means.

use super::api::RoomEndpoint;
use super::NetError;
use std::sync::OnceLock;

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
}

impl Default for Rules {
    fn default() -> Rules {
        Rules { code: FALLBACK_CODE.to_string(), start_when_hosting: true, heartbeat_seconds: 5.0 }
    }
}

/// Parses the file. The code must be five letters or digits; unknown keys are errors, so a typo never goes unnoticed.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.as_object().ok_or("public_route is not an object")?;
    if let Some(k) = o.keys().find(|k| !["schema", "_about", "code", "start_when_hosting", "heartbeat_seconds"].contains(&k.as_str())) {
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
    fn only_a_miss_opens_the_public_room() {
        assert!(matches!(step(Err(miss(404))), Ok(Step::Host)));
        assert!(matches!(step(Err(miss(500))), Err(NetError::Http { status: 500, .. })));
        assert!(matches!(step(Err(NetError::Offline)), Err(NetError::Offline)));
        assert!(matches!(step(Ok(RoomEndpoint::default())), Ok(Step::Join(_))));
    }
}
