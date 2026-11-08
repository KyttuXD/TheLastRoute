//! Board and Custom Night helpers over `core::data::ModesData`. Everything here is pure (no drawing, no input), so the
//! screens stay thin and the rules are unit tested. Text matches `night_watch_menu.gd`.
#![allow(dead_code)]

use crate::core::{rating_of, Mulberry32};
use macroquad::prelude::Rect;
use std::collections::BTreeMap;

pub use crate::core::{ModeEntry, ModesData, Preset, MAX_AI, POINTS_PER_LEVEL};

/// `cur + step` wrapped into `0..n`.
pub fn wrap_index(cur: usize, step: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (cur as i64 + step as i64).rem_euclid(n as i64) as usize
}

/// The next open tile from `cur` in direction `step`, skipping locked ones; `cur` again when none is open.
pub fn next_open_tile(cur: usize, step: i32, open: &[bool]) -> usize {
    let n = open.len();
    if n == 0 {
        return 0;
    }
    let mut i = cur.min(n - 1);
    for _ in 0..n {
        i = wrap_index(i, step, n);
        if open[i] {
            return i;
        }
    }
    cur.min(n - 1)
}

/// Cleared shifts that count towards the board (`level <= scored`).
pub fn cleared_count(cleared: &[u32], scored: usize) -> usize {
    cleared.iter().filter(|&&lv| (lv as usize) <= scored).count()
}

/// The small stat line under a mode's text.
pub fn stat_line(playable: bool, cleared: usize, scored: usize, preset_count: usize, stars: usize, best: i32) -> String {
    if !playable {
        return "NOT BUILT YET".to_string();
    }
    let mut s = format!("{} / {} SHIFTS CLEARED", cleared, scored);
    if preset_count > 0 {
        s.push_str(&format!(" · ★ {}/{} STARS", stars, preset_count));
        if best > 0 {
            s.push_str(&format!(" · BEST {}", best));
        }
    }
    s
}

/// Lines of a shift tile: `01  NAME` (+ `✔ CLEARED`), or `01  LOCKED` over `?????`.
pub fn tile_lines(index: usize, name: &str, open: bool, cleared: bool, scored: usize) -> Vec<String> {
    if !open {
        return vec![format!("{:02}  LOCKED", index + 1), "?????".to_string()];
    }
    let mut v = vec![format!("{:02}  {}", index + 1, name)];
    if cleared && index < scored {
        v.push("✔ CLEARED".to_string());
    }
    v
}

/// Columns of the tile grid: 4 above six shifts, else 3.
pub fn tile_columns(levels: usize) -> usize {
    if levels > 6 {
        4
    } else {
        3
    }
}

/// Modes that have a Custom Night (presets and sliders), in board order.
pub fn custom_modes(modes: &ModesData) -> Vec<&ModeEntry> {
    modes.modes.iter().filter(|m| !m.presets.is_empty() && m.custom.is_some()).collect()
}

/// Slider ids and labels of a mode's Custom Night.
pub fn custom_slots(m: &ModeEntry) -> &[(String, String)] {
    m.custom.as_deref().unwrap_or(&[])
}

/// Slider values: the saved value, else the first preset's, else 0; clamped to 0..=20.
pub fn custom_default(m: &ModeEntry, saved: &BTreeMap<String, i32>) -> BTreeMap<String, i32> {
    let mut out = BTreeMap::new();
    for (id, _) in custom_slots(m) {
        let first = m
            .presets
            .first()
            .and_then(|p| p.ai.iter().find(|(k, _)| k == id).map(|(_, v)| *v))
            .unwrap_or(0);
        let v = saved.get(id).copied().unwrap_or(first);
        out.insert(id.clone(), v.clamp(0, MAX_AI));
    }
    out
}

/// A preset as slider values (only the mode's own sliders).
pub fn preset_values(m: &ModeEntry, p: &Preset) -> BTreeMap<String, i32> {
    let mut out = BTreeMap::new();
    for (id, _) in custom_slots(m) {
        let v = p.ai.iter().find(|(k, _)| k == id).map(|(_, v)| *v).unwrap_or(0);
        out.insert(id.clone(), v.clamp(0, MAX_AI));
    }
    out
}

/// "All 0", "All 10", "All 20".
pub fn all_values(m: &ModeEntry, v: i32) -> BTreeMap<String, i32> {
    custom_slots(m).iter().map(|(id, _)| (id.clone(), v.clamp(0, MAX_AI))).collect()
}

/// "Random": each slider 0..=20, drawn in slider order.
pub fn random_values(m: &ModeEntry, rng: &mut Mulberry32) -> BTreeMap<String, i32> {
    let mut out = BTreeMap::new();
    for (id, _) in custom_slots(m) {
        out.insert(id.clone(), rng.pick(MAX_AI as usize + 1) as i32);
    }
    out
}

pub fn custom_score(vals: &BTreeMap<String, i32>) -> i32 {
    vals.values().map(|v| (*v).clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL
}

pub fn custom_max(slots: usize) -> i32 {
    slots as i32 * MAX_AI * POINTS_PER_LEVEL
}

/// `SCORE 120 OF 200 · AMAZING! · BEST 90`
pub fn score_line(score: i32, max: i32, best: i32) -> String {
    format!("SCORE {} OF {} · {} · BEST {}", score, max, rating_of(score, max).to_uppercase(), best)
}

/// Lay out boxes of the given widths left to right, wrapping at `max_x`. Returns the rects and the bottom edge.
pub fn flow_rects(widths: &[f32], x0: f32, y0: f32, max_x: f32, h: f32, gap_x: f32, gap_y: f32) -> (Vec<Rect>, f32) {
    let mut out = Vec::with_capacity(widths.len());
    let (mut x, mut y) = (x0, y0);
    for &w in widths {
        if x > x0 && x + w > max_x {
            x = x0;
            y += h + gap_y;
        }
        out.push(Rect::new(x, y, w, h));
        x += w + gap_x;
    }
    let bottom = if widths.is_empty() { y0 } else { y + h };
    (out, bottom)
}

/// `HH:MM` (UTC) from seconds since the epoch.
pub fn utc_hhmm(epoch_secs: f64) -> String {
    let s = (epoch_secs.max(0.0) as i64) % 86_400;
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// The title clock drifts from 12 AM to 6 AM, one hour per 12 s.
pub fn title_clock(t: f32) -> String {
    let h = ((t / 12.0).floor() as i64).rem_euclid(7);
    if h == 0 {
        "12 AM".to_string()
    } else {
        format!("{} AM", h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_navigation_wraps_both_ways() {
        assert_eq!(wrap_index(0, -1, 9), 8);
        assert_eq!(wrap_index(8, 1, 9), 0);
        assert_eq!(wrap_index(3, 1, 9), 4);
        assert_eq!(wrap_index(0, 1, 0), 0);
    }

    #[test]
    fn next_open_tile_skips_locked_ones() {
        let open = [true, true, false, false, true];
        assert_eq!(next_open_tile(1, 1, &open), 4);
        assert_eq!(next_open_tile(4, 1, &open), 0);
        assert_eq!(next_open_tile(0, -1, &open), 4);
        assert_eq!(next_open_tile(4, -1, &open), 1);
        // nothing else open: stay
        assert_eq!(next_open_tile(0, 1, &[true, false, false]), 0);
        assert_eq!(next_open_tile(0, 1, &[]), 0);
    }

    #[test]
    fn cleared_count_ignores_levels_past_the_scored_ones() {
        assert_eq!(cleared_count(&[1, 2, 5, 8], 7), 3);
        assert_eq!(cleared_count(&[], 7), 0);
    }

    #[test]
    fn stat_line_texts() {
        assert_eq!(stat_line(false, 0, 7, 3, 0, 0), "NOT BUILT YET");
        assert_eq!(stat_line(true, 3, 7, 0, 0, 0), "3 / 7 SHIFTS CLEARED");
        assert_eq!(stat_line(true, 5, 7, 4, 2, 0), "5 / 7 SHIFTS CLEARED · ★ 2/4 STARS");
        assert_eq!(stat_line(true, 5, 7, 4, 2, 140), "5 / 7 SHIFTS CLEARED · ★ 2/4 STARS · BEST 140");
    }

    #[test]
    fn tile_lines_open_cleared_and_locked() {
        assert_eq!(tile_lines(0, "Night 1", true, false, 7), vec!["01  Night 1".to_string()]);
        assert_eq!(tile_lines(0, "Night 1", true, true, 7), vec!["01  Night 1".to_string(), "✔ CLEARED".to_string()]);
        assert_eq!(tile_lines(7, "Bonus", true, true, 7), vec!["08  Bonus".to_string()]);
        assert_eq!(tile_lines(2, "X", false, false, 7), vec!["03  LOCKED".to_string(), "?????".to_string()]);
        assert_eq!(tile_columns(7), 4);
        assert_eq!(tile_columns(6), 3);
    }

    #[test]
    fn custom_score_and_rating_text() {
        let mut v = BTreeMap::new();
        v.insert("a".to_string(), 10);
        v.insert("b".to_string(), 25); // clamped to 20
        v.insert("c".to_string(), -3); // clamped to 0
        assert_eq!(custom_score(&v), 300);
        assert_eq!(custom_max(4), 800);
        assert_eq!(score_line(800, 800, 0), "SCORE 800 OF 800 · UNBEATABLE! · BEST 0");
        assert_eq!(score_line(0, 800, 120), "SCORE 0 OF 800 · GREAT JOB! · BEST 120");
        assert_eq!(score_line(200, 800, 5), "SCORE 200 OF 800 · FANTASTIC! · BEST 5");
        assert_eq!(score_line(330, 800, 5), "SCORE 330 OF 800 · AMAZING! · BEST 5");
    }

    #[test]
    fn random_values_stay_in_range() {
        let mut rng = Mulberry32::new(12345);
        for _ in 0..200 {
            let v = rng.pick(MAX_AI as usize + 1) as i32;
            assert!((0..=MAX_AI).contains(&v));
        }
    }

    #[test]
    fn flow_wraps_and_never_overlaps() {
        let widths = [60.0, 60.0, 60.0, 60.0, 60.0];
        let (rects, bottom) = flow_rects(&widths, 8.0, 10.0, 200.0, 12.0, 3.0, 2.0);
        assert_eq!(rects.len(), 5);
        assert_eq!(rects[0].y, 10.0);
        assert_eq!(rects[2].y, 10.0);
        assert!(rects[3].y > 10.0, "fourth box wraps");
        assert_eq!(rects[3].x, 8.0);
        for (i, a) in rects.iter().enumerate() {
            assert!(a.x + a.w <= 200.0 + 0.001);
            for b in rects.iter().skip(i + 1) {
                assert!(!a.overlaps(b), "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(bottom, rects[3].y + 12.0);
        assert_eq!(flow_rects(&[], 8.0, 10.0, 200.0, 12.0, 3.0, 2.0).1, 10.0);
    }

    #[test]
    fn clocks() {
        assert_eq!(utc_hhmm(0.0), "00:00");
        assert_eq!(utc_hhmm(3600.0 * 13.0 + 60.0 * 7.0 + 5.0), "13:07");
        assert_eq!(title_clock(0.0), "12 AM");
        assert_eq!(title_clock(12.0), "1 AM");
        assert_eq!(title_clock(12.0 * 6.0 + 1.0), "6 AM");
        assert_eq!(title_clock(12.0 * 7.0), "12 AM");
    }
}
