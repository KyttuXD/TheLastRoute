//! Keeps the built windows of every level near the runner, so going up and down is instant. Pure keep and evict logic;
//! the numbers are data (data/stairs_links.json, `preload_*`). The hooks that call it are in the integration notes.

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

/// The levels worth keeping built around `current`, nearest first (current, then alternating up and down), cut to
/// `max_windows`. `allowed` filters levels the run may not enter.
pub fn keep_levels(current: i32, cfg: &Preload, allowed: impl Fn(i32) -> bool) -> Vec<i32> {
    let mut out = vec![current];
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

/// The levels to build this frame: kept levels not held yet, nearest first, at most `builds_per_frame`.
pub fn to_build(keep: &[i32], held: &[i32], cfg: &Preload) -> Vec<i32> {
    keep.iter().copied().filter(|l| !held.contains(l)).take(cfg.builds_per_frame).collect()
}

/// The held levels to drop: all not in `keep`, then, while the held bytes pass the memory cap, the farthest from
/// `current` among the rest (never `current`). `held` is (level, bytes).
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
        assert_eq!(k, vec![0, 1, -1, 2, -2]);
        let small = Preload { max_windows: 3, ..cfg() };
        assert_eq!(keep_levels(0, &small, |_| true), vec![0, 1, -1]);
    }

    #[test]
    fn keep_skips_levels_not_allowed() {
        let k = keep_levels(0, &cfg(), |l| l <= 0);
        assert_eq!(k, vec![0, -1, -2]);
    }

    #[test]
    fn a_changed_value_changes_the_levels_kept() {
        let down_only = Preload { levels_up: 0, ..cfg() };
        assert_eq!(keep_levels(3, &down_only, |_| true), vec![3, 2, 1]);
    }

    #[test]
    fn builds_are_budgeted_per_frame() {
        let keep = [0, 1, -1];
        assert_eq!(to_build(&keep, &[0], &cfg()), vec![1]);
        let two = Preload { builds_per_frame: 2, ..cfg() };
        assert_eq!(to_build(&keep, &[0], &two), vec![1, -1]);
        assert!(to_build(&keep, &[0, 1, -1], &cfg()).is_empty());
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
    fn parse_refuses_bad_values() {
        assert!(parse("{}").is_err());
        let text = embedded::data_under("").find(|(p, _)| *p == PATH).map(|(_, t)| t).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["preload_builds_per_frame"] = 0.into();
        assert!(parse(&v.to_string()).is_err());
    }
}
