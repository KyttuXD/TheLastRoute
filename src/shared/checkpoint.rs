//! The solo route checkpoint (GameState.route_checkpoint): written at floor / level boundaries,
//! continued or cleared from the account menu. Stored in the per-user folder; the original keeps
//! it on the account server, which this offline build does not have.

use crate::config::Difficulty;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub run_seed: u32,
    /// The run's room seed text (`ROOM-<code>-<hex>`). Saves from before it was stored have no
    /// text: `load` gives them `route::room_text(run_seed)`, the text that run was started with.
    pub room_text: String,
    pub depth: i32,
    pub score: i32,
    pub coins: i32,
    pub difficulty: Difficulty,
    pub infinite: bool,
}

fn path() -> Option<PathBuf> {
    super::paths::user_file("checkpoint.json")
}

pub fn load() -> Option<Checkpoint> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path()?).ok()?).ok()?;
    from_json(&v)
}

/// The checkpoint a saved file holds (schema 1, kind last_route). A save from before the room
/// text was stored gets the text its run was started with, `route::room_text(run_seed)`.
fn from_json(v: &serde_json::Value) -> Option<Checkpoint> {
    if v["schema"].as_i64() != Some(1) || v["kind"].as_str() != Some("last_route") {
        return None;
    }
    let difficulty = match v["difficulty_id"].as_str()? {
        "relaxed" => Difficulty::Relaxed,
        "hard" => Difficulty::Hard,
        _ => Difficulty::Standard,
    };
    let run_seed = v["run_seed"].as_u64()? as u32;
    let room_text = v["room_text"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::world::route::room_text(run_seed));
    Some(Checkpoint {
        run_seed,
        room_text,
        depth: v["depth"].as_i64()? as i32,
        score: v["score"].as_i64()? as i32,
        coins: v["coins"].as_i64().unwrap_or(0) as i32,
        difficulty,
        infinite: v["infinite"].as_bool().unwrap_or(false),
    })
}

/// Off while the self-test drives a run, so it never touches the player's real save.
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn save(c: &Checkpoint) {
    if !enabled() {
        return;
    }
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let v = serde_json::json!({
        "schema": 1, "kind": "last_route", "run_seed": c.run_seed, "room_text": c.room_text,
        "depth": c.depth, "score": c.score,
        "coins": c.coins, "difficulty_id": c.difficulty.id(), "infinite": c.infinite,
    });
    let _ = std::fs::write(p, v.to_string());
}

pub fn clear() {
    if !enabled() {
        return;
    }
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn save_without_room_text_keeps_its_world() {
        let v = json!({"schema": 1, "kind": "last_route", "run_seed": 20261101u32,
            "depth": 2, "score": 5, "coins": 1, "difficulty_id": "hard", "infinite": true});
        let c = from_json(&v).expect("a legacy save loads");
        assert_eq!(c.room_text, crate::world::route::room_text(20261101));
        assert_eq!(c.run_seed, 20261101);
        assert!(c.infinite);
    }

    #[test]
    fn stored_room_text_is_kept() {
        let text = "ROOM-ABCDE-0123456789ABCDEF0123";
        let v = json!({"schema": 1, "kind": "last_route", "run_seed": 7,
            "room_text": text, "depth": 1, "score": 0, "difficulty_id": "standard"});
        let c = from_json(&v).expect("a save with text loads");
        assert_eq!(c.room_text, text);
        assert_eq!(crate::world::route::world_seed(&c.room_text), crate::world::route::run_seed_of_text(text).to_string());
    }

    #[test]
    fn wrong_schema_is_refused() {
        let v = json!({"schema": 2, "kind": "last_route", "run_seed": 7, "depth": 1, "score": 0,
            "difficulty_id": "standard"});
        assert!(from_json(&v).is_none());
    }
}
