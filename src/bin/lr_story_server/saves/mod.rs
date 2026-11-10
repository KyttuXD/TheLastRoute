//! The shared party save: one per room code, kept in `<data dir>/parties/<CODE>.json`.
//!
//! It holds what the party shares: the scene id, the flags, the inventory and the choices the votes settled. The room
//! server writes it (only the host changes scene, flags or inventory, and a vote result is recorded here when it
//! settles). A recreated room with the same code loads it. Writes go through `util::write_atomic`, so a crash mid-write
//! leaves the previous save in place.

use crate::util::{self, write_atomic};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SAVE_SCHEMA: &str = "lr_story.party.v1";

/// What the party shares. `choices` holds one record per settled vote.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartyState {
    pub scene: Option<String>,
    pub flags: Vec<String>,
    pub inventory: Vec<String>,
    pub choices: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartySave {
    pub code: String,
    /// The pack id and fingerprint the party was played with (a save only loads with the same pack).
    pub pack: String,
    pub fingerprint: String,
    pub state: PartyState,
    pub updated_at: i64,
}

impl PartySave {
    pub fn to_value(&self) -> Value {
        json!({
            "schema": SAVE_SCHEMA,
            "code": self.code,
            "pack": self.pack,
            "packFingerprint": self.fingerprint,
            "updatedAt": self.updated_at,
            "scene": self.state.scene,
            "flags": self.state.flags,
            "inventory": self.state.inventory,
            "choices": self.state.choices,
        })
    }

    pub fn from_value(value: &Value) -> Result<PartySave, String> {
        if value.get("schema").and_then(Value::as_str) != Some(SAVE_SCHEMA) {
            return Err("not a party save (wrong schema)".into());
        }
        let text = |key: &str| -> Result<String, String> {
            value.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("party save has no \"{key}\""))
        };
        let strings = |key: &str| -> Result<Vec<String>, String> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| item.as_str().map(str::to_string).ok_or_else(|| format!("party save \"{key}\" must hold strings")))
                    .collect(),
                Some(_) => Err(format!("party save \"{key}\" must be a list")),
            }
        };
        let choices = match value.get("choices") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err("party save \"choices\" must be a list".into()),
        };
        Ok(PartySave {
            code: text("code")?,
            pack: text("pack")?,
            fingerprint: text("packFingerprint")?,
            state: PartyState {
                scene: value.get("scene").and_then(Value::as_str).map(str::to_string),
                flags: strings("flags")?,
                inventory: strings("inventory")?,
                choices,
            },
            updated_at: value.get("updatedAt").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

pub struct PartySaves {
    dir: PathBuf,
}

impl PartySaves {
    pub fn new(dir: PathBuf) -> PartySaves {
        PartySaves { dir }
    }

    /// The file of a room code. Only a checked code names a file, so a code can never escape the folder.
    fn path_of(&self, code: &str) -> Result<PathBuf, String> {
        if !crate::rooms::valid_room_code(code) {
            return Err(format!("'{code}' is not a room code"));
        }
        Ok(self.dir.join(format!("{code}.json")))
    }

    pub fn exists(&self, code: &str) -> bool {
        self.path_of(code).map(|path| path.is_file()).unwrap_or(false)
    }

    /// The saved party of `code`: Ok(None) when none was ever written.
    pub fn load(&self, code: &str) -> Result<Option<PartySave>, String> {
        let path = self.path_of(code)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let save = PartySave::from_value(&value).map_err(|e| format!("{}: {e}", path.display()))?;
        if save.code != code {
            return Err(format!("{} holds the party of another code", path.display()));
        }
        Ok(Some(save))
    }

    /// Writes the save of its room code, atomically.
    pub fn save(&self, save: &PartySave) -> Result<(), String> {
        let path = self.path_of(&save.code)?;
        fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let text = util::stringify(&save.to_value());
        write_atomic(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lr_story_saves_{name}_{}", util::hex(&util::random_bytes(6))))
    }

    #[test]
    fn a_party_save_round_trips_through_its_file() {
        let saves = PartySaves::new(temp_dir("round"));
        assert_eq!(saves.load("ABC23"), Ok(None), "a code never saved has no party");
        let save = PartySave {
            code: "ABC23".into(),
            pack: "party_pack".into(),
            fingerprint: "ab".repeat(32),
            state: PartyState {
                scene: Some("cellar_door".into()),
                flags: vec!["heard_the_bell".into()],
                inventory: vec!["lantern".into(), "key_b".into()],
                choices: vec![json!({"scene": "hall", "index": 1, "label": "Open it", "tally": [0, 2], "at": 5})],
            },
            updated_at: 42,
        };
        saves.save(&save).unwrap();
        assert!(saves.exists("ABC23"));
        assert_eq!(saves.load("ABC23").unwrap(), Some(save));
        assert!(saves.load("../ABC23").is_err(), "a code that is not a room code never names a file");
    }

    #[test]
    fn a_save_for_another_code_or_schema_is_refused() {
        let dir = temp_dir("wrong");
        let saves = PartySaves::new(dir.clone());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ABC23.json"), r#"{"schema":"other","code":"ABC23"}"#).unwrap();
        assert!(saves.load("ABC23").is_err());
        fs::write(dir.join("ABC23.json"), r#"{"schema":"lr_story.party.v1","code":"XYZ99","pack":"p","packFingerprint":"f"}"#)
            .unwrap();
        assert!(saves.load("ABC23").is_err());
    }
}
