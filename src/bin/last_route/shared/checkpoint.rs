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
    /// JSON `y_abs`. None for a legacy save or a finite route; `depth` stays the level label. It decides the label
    /// (`level()`) and, when the body was standing near its floor, the resume height (`migrated_y()`).
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

    /// The absolute y to resume at (saves store absolute height, owner decision of stairs phase 9). A saved y within
    /// `standing_tolerance` (data/app/resume.json) of the resumed level's floor is kept: it is a body that stood or
    /// crouched there. A y further off is a save made mid-flight (the relabel autosaves a couple of metres off the
    /// old floor); the save stores no x/z, so the resume stands at the level's start cell, which is on the floor, and
    /// keeping that y would hang the body in the air over it and drop it with a hard landing. Those, a legacy save
    /// and a non-finite y resume on the floor of the level.
    pub fn migrated_y(&self) -> f32 {
        let level = self.level();
        let floor = crate::game::level_stack::floor_y_of(level);
        match self.y {
            Some(y) if self.infinite && y.is_finite() && (y - floor).abs() <= resume_rules().standing_tolerance => y,
            _ => floor,
        }
    }
}

/// How the resume treats a saved height (data/app/resume.json, `lr.app.resume/1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResumeRules {
    /// A saved y this close to the floor of the resumed level (metres) is a standing body and is kept.
    pub standing_tolerance: f32,
}

impl Default for ResumeRules {
    fn default() -> Self {
        ResumeRules { standing_tolerance: 0.5 }
    }
}

impl ResumeRules {
    /// A missing, negative or non-finite tolerance keeps the default.
    pub fn parse(v: &serde_json::Value) -> ResumeRules {
        let d = ResumeRules::default();
        let t = v["standing_tolerance"].as_f64().map(|t| t as f32).filter(|t| t.is_finite() && *t >= 0.0);
        ResumeRules { standing_tolerance: t.unwrap_or(d.standing_tolerance) }
    }
}

/// The embedded resume rules; the defaults when the file is missing or broken.
pub fn resume_rules() -> &'static ResumeRules {
    static RULES: std::sync::OnceLock<ResumeRules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("app/resume.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
            .map(|v| ResumeRules::parse(&v))
            .unwrap_or_default()
    })
}

fn path() -> Option<PathBuf> {
    super::paths::user_file("checkpoint.json")
}

pub fn load() -> Option<Checkpoint> {
    // the browser build keeps no run: nothing is saved there, so nothing is ever continued (paths::WRITES_TO_DISK)
    if !super::paths::WRITES_TO_DISK {
        return None;
    }
    let p = path()?;
    read_checkpoint(&p).or_else(|| read_checkpoint(&backup_path(&p)))
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
    !cfg!(test) && super::paths::WRITES_TO_DISK && ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn save(c: &Checkpoint) {
    if !enabled() {
        return;
    }
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = write_atomic(&p, &to_json(c).to_string());
}

/// Saves the checkpoint with the body's position (data/progression.json `autosave.store_position`): the same file plus
/// `pos_x`, `pos_y`, `pos_z`, read back by [`load_position`]. The hook for pause-quit and pickup autosaves.
pub fn save_with_position(c: &Checkpoint, pos: [f32; 3]) {
    if !enabled() {
        return;
    }
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = write_atomic(&p, &to_json_with_position(c, pos).to_string());
}

fn to_json_with_position(c: &Checkpoint, pos: [f32; 3]) -> serde_json::Value {
    let mut v = to_json(c);
    if crate::game::progression::Autosave::load().store_position && pos.iter().all(|n| n.is_finite()) {
        v["pos_x"] = serde_json::json!(pos[0]);
        v["pos_y"] = serde_json::json!(pos[1]);
        v["pos_z"] = serde_json::json!(pos[2]);
    }
    v
}

fn position_of(v: &serde_json::Value) -> Option<[f32; 3]> {
    let g = |k: &str| v[k].as_f64().filter(|n| n.is_finite()).map(|n| n as f32);
    Some([g("pos_x")?, g("pos_y")?, g("pos_z")?])
}

/// The stored body position of the saved run, when the save has one.
pub fn load_position() -> Option<[f32; 3]> {
    if !super::paths::WRITES_TO_DISK {
        return None;
    }
    let p = path()?;
    [p.clone(), backup_path(&p)].iter().find_map(|f| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        position_of(&v)
    })
}

/// The `.bak` beside a checkpoint file: the previous good save.
fn backup_path(p: &std::path::Path) -> PathBuf {
    let mut b = p.as_os_str().to_owned();
    b.push(".bak");
    PathBuf::from(b)
}

fn read_checkpoint(p: &std::path::Path) -> Option<Checkpoint> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    from_json(&v)
}

/// Writes `text` to a sibling temp file, then renames it over `p`, so an interrupted write never leaves a truncated checkpoint.
fn write_atomic(p: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = p.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    let _ = f.sync_all();
    drop(f);
    // keep the previous good save as `.bak`, so a bad write or a corrupted file still has a run to continue
    if crate::game::progression::Autosave::load().keep_backup && p.exists() {
        let _ = std::fs::copy(p, backup_path(p));
    }
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e
    })
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
        // the .bak goes too, or a cleared run would come back from it
        let _ = std::fs::remove_file(backup_path(&p));
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
        assert_eq!(c.migrated_y(), crate::game::level_stack::floor_y_of(-2), "a mid-flight y resumes on the floor");
        let standing = crate::game::level_stack::floor_y_of(-2) + 0.25;
        let c = from_json(&to_json(&cp(-2, true, Some(standing)))).unwrap();
        assert_eq!(c.migrated_y(), standing, "a standing save keeps its absolute height");
    }

    #[test]
    fn resume_json_is_embedded_and_range_checked() {
        let (_, text) = crate::content::embedded::data_under("app/resume.json").next().expect("resume.json is embedded");
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["schema"], "lr.app.resume/1");
        assert_eq!(ResumeRules::parse(&v), *resume_rules());
        assert!(resume_rules().standing_tolerance > 0.0 && resume_rules().standing_tolerance < crate::world::vertical::STOREY / 2.0);
        let bad = ResumeRules::parse(&json!({"standing_tolerance": -1.0}));
        assert_eq!(bad, ResumeRules::default());
        assert_eq!(ResumeRules::parse(&json!({"standing_tolerance": 0.2})).standing_tolerance, 0.2);
    }

    /// (depth, saved y offset from that depth's floor) to (resolved label, resume offset from the resolved floor),
    /// across the hysteresis edges and the standing tolerance.
    #[test]
    fn saved_heights_resolve_to_a_label_and_a_resume_height() {
        use crate::game::level_stack::{floor_y_of, level_at};
        let tol = resume_rules().standing_tolerance;
        let cases = [
            (-1, 0.0, -1, 0.0),
            (-1, tol - 0.01, -1, tol - 0.01),
            (-1, tol + 0.1, -1, 0.0),
            (-1, 2.6, -1, 0.0),
            (-1, -2.6, -1, 0.0),
            (-1, -crate::world::vertical::STOREY, -2, 0.0),
            (0, 0.2, 0, 0.2),
        ];
        for (depth, off, label, resume_off) in cases {
            let y = floor_y_of(depth) + off;
            let c = cp(depth, true, Some(y));
            assert_eq!(c.level(), level_at(y, depth));
            assert_eq!(c.level(), label, "depth {depth} off {off}");
            assert!((c.migrated_y() - (floor_y_of(label) + resume_off)).abs() < 1e-4, "depth {depth} off {off}: {}", c.migrated_y());
        }
        assert_eq!(cp(-1, false, Some(floor_y_of(-1) + 0.2)).migrated_y(), floor_y_of(-1), "a finite route stores no height");
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

    #[test]
    fn the_atomic_write_replaces_the_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("lr_cp_atomic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("checkpoint.json");
        std::fs::write(&p, "old").unwrap();
        write_atomic(&p, "{\"a\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"a\":1}");
        assert!(!dir.join("checkpoint.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_atomic_write_over_a_save_leaves_the_previous_one_as_bak() {
        let dir = std::env::temp_dir().join(format!("lr_cp_bak_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("checkpoint.json");
        write_atomic(&p, "first").unwrap();
        assert!(!backup_path(&p).exists(), "nothing to back up on the first write");
        write_atomic(&p, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(backup_path(&p)).unwrap(), "first");
        assert!(!dir.join("checkpoint.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_save_falls_back_to_the_bak() {
        let dir = std::env::temp_dir().join(format!("lr_cp_fallback_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("checkpoint.json");
        write_atomic(&p, &to_json(&cp(-1, true, None)).to_string()).unwrap();
        write_atomic(&p, "{ broken").unwrap();
        assert!(read_checkpoint(&p).is_none());
        let c = read_checkpoint(&p).or_else(|| read_checkpoint(&backup_path(&p))).expect("the bak loads");
        assert_eq!(c.depth, -1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stored_position_round_trips_and_a_bad_one_is_dropped() {
        let v = to_json_with_position(&cp(-2, true, None), [1.5, 2.0, -3.25]);
        assert_eq!(position_of(&v), Some([1.5, 2.0, -3.25]));
        assert!(from_json(&v).is_some(), "the position does not disturb the checkpoint");
        let bad = to_json_with_position(&cp(-2, true, None), [f32::NAN, 0.0, 0.0]);
        assert_eq!(position_of(&bad), None);
        assert_eq!(position_of(&to_json(&cp(1, false, None))), None);
    }
}
