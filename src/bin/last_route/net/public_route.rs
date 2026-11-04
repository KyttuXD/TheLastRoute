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

/// The parsed `lr.public_route/1` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub code: String,
    /// A runner that opens the public room starts the route at once (nobody else is there to press START).
    pub start_when_hosting: bool,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules { code: FALLBACK_CODE.to_string(), start_when_hosting: true }
    }
}

/// Parses the file. The code must be five letters or digits; unknown keys are errors, so a typo never goes unnoticed.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.as_object().ok_or("public_route is not an object")?;
    if let Some(k) = o.keys().find(|k| !["schema", "_about", "code", "start_when_hosting"].contains(&k.as_str())) {
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
    Ok(rules)
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
    fn only_a_miss_opens_the_public_room() {
        assert!(matches!(step(Err(miss(404))), Ok(Step::Host)));
        assert!(matches!(step(Err(miss(500))), Err(NetError::Http { status: 500, .. })));
        assert!(matches!(step(Err(NetError::Offline)), Err(NetError::Offline)));
        assert!(matches!(step(Ok(RoomEndpoint::default())), Ok(Step::Join(_))));
    }
}
