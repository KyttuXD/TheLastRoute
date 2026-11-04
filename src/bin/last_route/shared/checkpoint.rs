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
    /// Absolute feet y (the body's world y in the one vertical frame, level * STOREY + local),
    /// JSON `y_abs`. None for a legacy save or a finite route; `depth` stays the level label.
    pub y: Option<f32>,
}

impl Checkpoint {
    /// The level label to resume on: an infinite save with a finite y resolves through the runtime
    /// hysteresis (previous label = stored depth); otherwise the stored depth.
    pub fn level(&self) -> i32 {
        match self.y {
            Some(y) if self.infinite && y.is_finite() => {
                crate::game::level_stack::level_at(y, self.depth)
            }
            _ => self.depth,
        }
    }

    /// The absolute y to resume at: the stored y, or (a legacy save carrying only the label) the
    /// floor of its level, local pose 0.
    pub fn migrated_y(&self) -> f32 {
        match self.y {
            Some(y) if y.is_finite() => y,
            _ => crate::game::level_stack::floor_y_of(self.level()),
        }
    }
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
        "nightmare" => Difficulty::Nightmare,
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
        y: v["y_abs"].as_f64().filter(|y| y.is_finite()).map(|y| y as f32),
    })
}

/// Off while the self-test drives a run, so it never touches the player's real save.
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn enabled() -> bool {
    // the lib's unit tests drive relays, checkpoints and stairs: they never touch the player's real save
    !cfg!(test) && ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn save(c: &Checkpoint) {
    if !enabled() {
        return;
    }
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = std::fs::write(p, to_json(c).to_string());
}

/// The JSON a save writes. `y_abs` only for an infinite route with a finite y.
fn to_json(c: &Checkpoint) -> serde_json::Value {
    let mut v = serde_json::json!({
        "schema": 1, "kind": "last_route", "run_seed": c.run_seed, "room_text": c.room_text,
        "depth": c.depth, "score": c.score,
        "coins": c.coins, "difficulty_id": c.difficulty.id(), "infinite": c.infinite,
    });
    if let Some(y) = c.y.filter(|y| c.infinite && y.is_finite()) {
        v["y_abs"] = serde_json::json!(y);
    }
    v
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

    fn cp(depth: i32, infinite: bool, y: Option<f32>) -> Checkpoint {
        Checkpoint {
            run_seed: 7,
            room_text: "ROOM-ABCDE-0123456789ABCDEF0123".into(),
            depth,
            score: 1,
            coins: 2,
            difficulty: Difficulty::Standard,
            infinite,
            y,
        }
    }

    #[test]
    fn legacy_save_without_y_loads_at_the_floor_of_its_label() {
        use crate::game::level_stack::floor_y_of;
        for d in [-1, -3, 0] {
            let v = json!({"schema": 1, "kind": "last_route", "run_seed": 7, "depth": d,
                "score": 0, "difficulty_id": "standard", "infinite": true});
            let c = from_json(&v).expect("legacy save loads");
            assert_eq!(c.y, None);
            assert_eq!(c.level(), d);
            assert_eq!(c.migrated_y(), floor_y_of(d));
        }
    }

    #[test]
    fn y_abs_round_trips() {
        let y = crate::game::level_stack::floor_y_of(-2) + 0.75;
        let v = to_json(&cp(-2, true, Some(y)));
        assert_eq!(v["depth"], -2);
        let c = from_json(&v).expect("round trip");
        assert_eq!(c.y, Some(y));
        assert_eq!(c.migrated_y(), y);
    }

    #[test]
    fn a_mid_flight_y_resolves_with_the_runtime_hysteresis() {
        use crate::game::level_stack::{floor_y_of, level_at};
        let depth = -1;
        let near = floor_y_of(depth) + 0.1;
        assert_eq!(cp(depth, true, Some(near)).level(), level_at(near, depth));
        assert_eq!(cp(depth, true, Some(near)).level(), depth);
        let far = floor_y_of(depth - 1);
        assert_eq!(cp(depth, true, Some(far)).level(), level_at(far, depth));
        assert_eq!(cp(depth, true, Some(far)).level(), depth - 1);
    }

    #[test]
    fn a_non_finite_or_missing_y_never_rejects_the_save() {
        let v = json!({"schema": 1, "kind": "last_route", "run_seed": 7, "depth": -2,
            "score": 0, "difficulty_id": "standard", "infinite": true, "y_abs": "bad"});
        let c = from_json(&v).expect("a bad y_abs is dropped, not fatal");
        assert_eq!(c.y, None);
        assert_eq!(c.level(), -2);
        let nan = cp(-2, true, Some(f32::NAN));
        assert_eq!(nan.level(), -2);
        assert_eq!(nan.migrated_y(), crate::game::level_stack::floor_y_of(-2));
        assert!(to_json(&nan).get("y_abs").is_none());
    }

    #[test]
    fn a_finite_route_save_writes_no_y_abs() {
        let v = to_json(&cp(3, false, Some(4.0)));
        assert!(v.get("y_abs").is_none());
        assert_eq!(v["depth"], 3);
    }
}
