//! The solo route checkpoint (GameState.route_checkpoint): written at floor / level boundaries,
//! continued or cleared from the account menu. Stored in the per-user folder; the original keeps
//! it on the account server, which this offline build does not have.

use crate::config::Difficulty;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub run_seed: u32,
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
    if v["schema"].as_i64() != Some(1) || v["kind"].as_str() != Some("last_route") {
        return None;
    }
    let difficulty = match v["difficulty_id"].as_str()? {
        "relaxed" => Difficulty::Relaxed,
        "hard" => Difficulty::Hard,
        _ => Difficulty::Standard,
    };
    Some(Checkpoint {
        run_seed: v["run_seed"].as_u64()? as u32,
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
        "schema": 1, "kind": "last_route", "run_seed": c.run_seed, "depth": c.depth, "score": c.score,
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
