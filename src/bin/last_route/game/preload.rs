//! Keeps the resident neighbours of the active level built beside it (game/level_stack.rs `LevelStack`), so a flight is
//! one continuous space. Pure keep and evict logic; the numbers are data (data/stairs_links.json, `preload_*`). Called
//! every frame by `Game::keep_stack` (game/endless.rs): `keep_levels` names the neighbour levels held (never the active
//! one), `to_evict` drops the rest and the farthest over the memory cap (counting `Resident::bytes`), `to_build` meters
//! the builds per frame.

use super::level_stack::LevelStack;
use crate::content::embedded;
use std::sync::OnceLock;

const PATH: &str = "stairs_links.json";

/// The preload numbers from data/stairs_links.json.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preload {
    pub levels_up: i32,
    pub levels_down: i32,
    pub max_windows: usize,
    pub memory_cap_mb: f32,
    pub builds_per_frame: usize,
}

/// Reads the preload numbers from the JSON text.
pub fn parse(text: &str) -> Result<Preload, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let n = |k: &str| v.get(k).and_then(|x| x.as_f64()).ok_or_else(|| format!("{k}: a number is needed"));
    let p = Preload {
        levels_up: n("preload_levels_up")? as i32,
        levels_down: n("preload_levels_down")? as i32,
        max_windows: n("preload_max_windows")? as usize,
        memory_cap_mb: n("preload_memory_cap_mb")? as f32,
        builds_per_frame: n("preload_builds_per_frame")? as usize,
    };
    if p.levels_up < 0 || p.levels_down < 0 {
        return Err("preload levels must not be negative".into());
    }
    if p.max_windows == 0 || p.builds_per_frame == 0 || !(p.memory_cap_mb > 0.0) {
        return Err("preload_max_windows, preload_builds_per_frame and preload_memory_cap_mb must be above zero".into());
    }
    Ok(p)
}

static CFG: OnceLock<Preload> = OnceLock::new();

/// The preload data, loaded once from the embedded file.
pub fn preload() -> Preload {
    *CFG.get_or_init(|| {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == PATH)
            .map(|(_, t)| t)
            .expect("data/stairs_links.json is embedded by build.rs");
        parse(text).expect("data/stairs_links.json preload fields are valid")
    })
}

/// The neighbour levels worth keeping resident around `current`, nearest first (alternating up and down), cut to
/// `max_windows`. `current` itself is never listed. `allowed` filters levels the run may not enter.
pub fn keep_levels(current: i32, cfg: &Preload, allowed: impl Fn(i32) -> bool) -> Vec<i32> {
    let mut out = Vec::new();
    let reach = cfg.levels_up.max(cfg.levels_down);
    for d in 1..=reach {
        if d <= cfg.levels_up && allowed(current + d) {
            out.push(current + d);
        }
        if d <= cfg.levels_down && allowed(current - d) {
            out.push(current - d);
        }
    }
    out.truncate(cfg.max_windows.max(1));
    out
}

/// The neighbour levels to build this frame: kept levels not resident yet, nearest first, at most `builds_per_frame`.
/// The active level is never in `keep`, so it is never built.
pub fn to_build(keep: &[i32], held: &[i32], cfg: &Preload) -> Vec<i32> {
    keep.iter().copied().filter(|l| !held.contains(l)).take(cfg.builds_per_frame).collect()
}

/// The (level, bytes) of every resident in the stack, the `held` input of `to_evict`.
pub fn held_bytes(stack: &LevelStack) -> Vec<(i32, usize)> {
    stack.residents.iter().map(|r| (r.level, r.bytes())).collect()
}

/// The resident levels to drop: all not in `keep`, then, while the resident bytes pass the memory cap, the farthest from
/// `current` among the rest (never `current`). `held` is (level, bytes), see `held_bytes`.
pub fn to_evict(held: &[(i32, usize)], keep: &[i32], current: i32, cfg: &Preload) -> Vec<i32> {
    let mut out: Vec<i32> = held.iter().map(|h| h.0).filter(|l| !keep.contains(l)).collect();
    let cap = (cfg.memory_cap_mb * 1024.0 * 1024.0) as usize;
    let mut rest: Vec<(i32, usize)> = held.iter().copied().filter(|h| keep.contains(&h.0)).collect();
    rest.sort_by_key(|h| std::cmp::Reverse((h.0 - current).abs()));
    let mut total: usize = rest.iter().map(|h| h.1).sum();
    for (l, b) in rest {
        if total <= cap {
            break;
        }
        if l != current {
            out.push(l);
            total -= b;
        }
    }
    out
}

const PREWARM_PATH: &str = "chunk_prewarm.json";

/// The chunk-window prewarm numbers from data/chunk_prewarm.json.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkPrewarm {
    pub margin_m: f32,
    pub gap_frames: u32,
    pub chunks_per_step: usize,
}

/// Reads the prewarm numbers from the JSON text.
pub fn parse_prewarm(text: &str) -> Result<ChunkPrewarm, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let n = |k: &str| v.get(k).and_then(|x| x.as_f64()).ok_or_else(|| format!("{k}: a number is needed"));
    let p = ChunkPrewarm {
        margin_m: n("margin_m")? as f32,
        gap_frames: n("gap_frames")?.max(0.0) as u32,
        chunks_per_step: n("chunks_per_step")? as usize,
    };
    if !(p.margin_m > 0.0) || p.chunks_per_step == 0 {
        return Err("margin_m and chunks_per_step must be above zero".into());
    }
    Ok(p)
}

static PREWARM: OnceLock<ChunkPrewarm> = OnceLock::new();

/// The prewarm data, loaded once from the embedded file.
pub fn chunk_prewarm() -> ChunkPrewarm {
    *PREWARM.get_or_init(|| {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == PREWARM_PATH)
            .map(|(_, t)| t)
            .expect("data/chunk_prewarm.json is embedded by build.rs");
        parse_prewarm(text).expect("data/chunk_prewarm.json is valid")
    })
}

/// The chunks to generate this frame: none while `wait` frames remain, else the first `chunks_per_step` of `wanted`
/// (see `infinite::approach_chunks`) that `has` does not hold yet. Single-threaded on every target (wasm has no
/// threads), so the per-step budget is what keeps a frame short.
pub fn prewarm_step(
    cfg: &ChunkPrewarm,
    wait: u32,
    wanted: &[crate::world::infinite::ChunkKey],
    has: impl Fn(&crate::world::infinite::ChunkKey) -> bool,
) -> Vec<crate::world::infinite::ChunkKey> {
    if wait > 0 {
        return Vec::new();
    }
    wanted.iter().copied().filter(|k| !has(k)).take(cfg.chunks_per_step).collect()
}

/// Frames a prewarm of `missing` chunks takes, counting the wait after each step.
pub fn prewarm_frames(cfg: &ChunkPrewarm, missing: usize) -> u32 {
    let steps = missing.div_ceil(cfg.chunks_per_step) as u32;
    if steps == 0 {
        0
    } else {
        (steps - 1) * (cfg.gap_frames + 1) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Preload {
        Preload { levels_up: 2, levels_down: 2, max_windows: 6, memory_cap_mb: 1.0, builds_per_frame: 1 }
    }

    #[test]
    fn the_embedded_data_is_the_standard_values() {
        let p = preload();
        assert!(p.levels_up >= 1 && p.levels_down >= 1, "more than the adjacent storey is kept");
        assert!(p.max_windows >= 3 && p.builds_per_frame >= 1 && p.memory_cap_mb > 0.0);
    }

    #[test]
    fn keep_is_nearest_first_and_capped() {
        let k = keep_levels(0, &cfg(), |_| true);
        assert_eq!(k, vec![1, -1, 2, -2], "neighbours only, the active level is never listed");
        let small = Preload { max_windows: 3, ..cfg() };
        assert_eq!(keep_levels(0, &small, |_| true), vec![1, -1, 2]);
    }

    #[test]
    fn keep_skips_levels_not_allowed() {
        let k = keep_levels(0, &cfg(), |l| l <= 0);
        assert_eq!(k, vec![-1, -2]);
    }

    #[test]
    fn a_changed_value_changes_the_levels_kept() {
        let down_only = Preload { levels_up: 0, ..cfg() };
        assert_eq!(keep_levels(3, &down_only, |_| true), vec![2, 1]);
    }

    #[test]
    fn builds_are_budgeted_per_frame() {
        let keep = [1, -1];
        assert_eq!(to_build(&keep, &[], &cfg()), vec![1]);
        let two = Preload { builds_per_frame: 2, ..cfg() };
        assert_eq!(to_build(&keep, &[], &two), vec![1, -1]);
        assert_eq!(to_build(&keep, &[1], &cfg()), vec![-1]);
        assert!(to_build(&keep, &[1, -1], &cfg()).is_empty());
    }

    #[test]
    fn evicts_levels_out_of_the_keep_set() {
        let held = [(0, 10), (1, 10), (5, 10)];
        assert_eq!(to_evict(&held, &[0, 1], 0, &cfg()), vec![5]);
    }

    #[test]
    fn evicts_farthest_when_over_the_memory_cap() {
        let mb = 1024 * 1024;
        let held = [(0, mb), (1, mb), (-2, mb)];
        let two_mb = Preload { memory_cap_mb: 2.0, ..cfg() };
        let out = to_evict(&held, &[0, 1, -2], 0, &two_mb);
        assert_eq!(out, vec![-2], "the farthest goes first, stopping once under the cap");
        let tight = Preload { memory_cap_mb: 0.1, ..cfg() };
        let out = to_evict(&held, &[0, 1, -2], 0, &tight);
        assert!(!out.contains(&0), "the current level is never evicted");
    }

    #[test]
    fn the_memory_cap_counts_resident_bytes() {
        use crate::game::level_stack::Resident;
        use crate::world::catalog::Catalog;
        use crate::world::facility::Options;
        use crate::world::fixtures::Fixtures;
        use crate::world::Facility;
        let cat = Catalog::load();
        let mut st = LevelStack::default();
        for level in [1, -1, -2] {
            let mut fac = Facility::generate("PRELOAD-MEM", &Options::default(), &cat);
            fac.level = level;
            st.put(Resident { level, center: (0, 0), fac, fix: Fixtures::default() });
        }
        let held = held_bytes(&st);
        assert_eq!(held.len(), 3);
        assert!(held.iter().all(|h| h.1 > 0), "a resident holds memory");
        let keep = [1, -1, -2];
        let roomy = Preload { memory_cap_mb: 1000.0, ..cfg() };
        assert!(to_evict(&held, &keep, 0, &roomy).is_empty());
        // a cap below the three residents' bytes drops the farthest first and never the active level
        let total: usize = held.iter().map(|h| h.1).sum();
        let tight = Preload { memory_cap_mb: (total as f32 * 0.7) / (1024.0 * 1024.0), ..cfg() };
        let out = to_evict(&held, &keep, 0, &tight);
        assert_eq!(out.first(), Some(&-2), "the farthest resident goes first");
        assert!(!out.contains(&0));
    }

    fn pcfg() -> ChunkPrewarm {
        ChunkPrewarm { margin_m: 8.0, gap_frames: 2, chunks_per_step: 2 }
    }

    #[test]
    fn the_prewarm_data_is_the_standard_values() {
        let p = chunk_prewarm();
        assert!(p.margin_m > 0.0 && p.chunks_per_step >= 1);
        assert!(parse_prewarm("{}").is_err());
    }

    #[test]
    fn the_budgeted_builder_finishes_in_the_expected_frames() {
        use crate::world::infinite::ChunkKey;
        let cfg = pcfg();
        let wanted: Vec<ChunkKey> = (0..5).map(|i| ChunkKey { cx: i, cy: 0, level: 0 }).collect();
        let mut held: Vec<ChunkKey> = vec![];
        let (mut wait, mut frames) = (0u32, 0u32);
        while held.len() < wanted.len() && frames < 100 {
            frames += 1;
            wait = wait.saturating_sub(1);
            let step = prewarm_step(&cfg, wait, &wanted, |k| held.contains(k));
            assert!(step.len() <= cfg.chunks_per_step, "a step stays inside the budget");
            if !step.is_empty() {
                held.extend(step);
                wait = cfg.gap_frames + 1;
            }
        }
        assert_eq!(held.len(), 5);
        assert_eq!(frames, prewarm_frames(&cfg, 5));
        let one = ChunkPrewarm { chunks_per_step: 1, ..cfg };
        assert!(prewarm_frames(&one, 5) > prewarm_frames(&cfg, 5), "a changed budget changes the time");
        assert_eq!(prewarm_frames(&cfg, 0), 0);
    }

    #[test]
    fn a_prebuilt_chunk_equals_a_fresh_one() {
        use crate::world::catalog::Catalog;
        use crate::world::infinite::{chunk, ChunkKey};
        let cat = Catalog::load();
        let loot = crate::content::loot::LootTables::default();
        let k = ChunkKey { cx: 2, cy: -1, level: 0 };
        let (a, fa) = chunk("PREWARM-SEED", k, &cat, &loot);
        let (b, fb) = chunk("PREWARM-SEED", k, &cat, &loot);
        assert_eq!((a.cols, a.rows), (b.cols, b.rows));
        assert_eq!(a.wall, b.wall, "the same seed builds the same chunk");
        assert_eq!(a.start, b.start);
        assert_eq!(a.relays, b.relays);
        assert_eq!(fa.coins, fb.coins);
        assert_eq!(fa.vents, fb.vents);
    }

    #[test]
    fn parse_refuses_bad_values() {
        assert!(parse("{}").is_err());
        let text = embedded::data_under("").find(|(p, _)| *p == PATH).map(|(_, t)| t).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["preload_builds_per_frame"] = 0.into();
        assert!(parse(&v.to_string()).is_err());
    }
}
