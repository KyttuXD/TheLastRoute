//! Text for the 480x270 canvas: draw helpers, word wrap, and ASCII fallbacks for glyphs the default font lacks.
//! `wrap_with`, `fit_with` and `glyph` are pure (tests use them); `wrap`, `fit` and `measure` read the font.
#![allow(dead_code)]

use super::theme::frame;
use macroquad::prelude::*;

/// Replace characters the default macroquad font cannot draw by ASCII. Anything left that is not ASCII becomes `?`.
pub fn glyph(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    // the arrow pair first, so "↑ ↓" reads UP/DOWN
    let mut t = s.replace("↑ ↓", "UP/DOWN").replace("↑↓", "UP/DOWN");
    const MAP: &[(&str, &str)] = &[
        ("▶", ">"),
        ("▲", "^"),
        ("●", "o"),
        ("★", "*"),
        ("☆", "-"),
        ("✔", "OK"),
        ("▮", "|"),
        ("↑", "^"),
        ("↓", "v"),
        ("←", "<-"),
        ("→", "->"),
        ("Ⅱ", "II"),
        ("🔒", "[LOCKED]"),
        ("…", "..."),
        ("·", "|"),
        ("—", "-"),
        ("–", "-"),
        ("’", "'"),
        ("‘", "'"),
        ("“", "\""),
        ("”", "\""),
    ];
    for (from, to) in MAP {
        if t.contains(from) {
            t = t.replace(from, to);
        }
    }
    t.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect()
}

/// Height of one text line of `size` virtual px.
pub fn line_height(size: f32) -> f32 {
    (size * 1.25).ceil()
}

fn font_px(size: f32, k: f32) -> u16 {
    (size * k).round().max(5.0) as u16
}

/// Width in virtual px of `s` at `size`.
pub fn measure(s: &str, size: f32) -> f32 {
    let (k, _, _) = frame();
    let g = glyph(s);
    measure_text(&g, None, font_px(size, k), 1.0).width / k
}

/// Draw `s` with its top-left at (x, y) in virtual coordinates.
pub fn text(s: &str, x: f32, y: f32, size: f32, c: Color) {
    let (k, ox, oy) = frame();
    let g = glyph(s);
    draw_text_ex(
        &g,
        (ox + x * k).round(),
        (oy + (y + size * 0.82) * k).round(),
        TextParams { font_size: font_px(size, k), color: c, ..Default::default() },
    );
}

/// Centred on `cx`.
pub fn text_center(s: &str, cx: f32, y: f32, size: f32, c: Color) {
    let w = measure(s, size);
    text(s, cx - w * 0.5, y, size, c);
}

/// Right edge at `rx`.
pub fn text_right(s: &str, rx: f32, y: f32, size: f32, c: Color) {
    let w = measure(s, size);
    text(s, rx - w, y, size, c);
}

/// Greedy word wrap to `width` using `measure`. `\n` starts a new paragraph; a word wider than the width is broken by
/// characters. Pure (the caller supplies the measuring function).
pub fn wrap_with(s: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let mut word = word.to_string();
            loop {
                let cand = if line.is_empty() { word.clone() } else { format!("{} {}", line, word) };
                if measure(&cand) <= width || (line.is_empty() && word.chars().count() <= 1) {
                    line = cand;
                    break;
                }
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                // a single word wider than the line: cut it at the longest prefix that fits
                let chars: Vec<char> = word.chars().collect();
                let mut n = chars.len() - 1;
                while n > 1 && measure(&chars[..n].iter().collect::<String>()) > width {
                    n -= 1;
                }
                out.push(chars[..n].iter().collect());
                word = chars[n..].iter().collect();
            }
        }
        out.push(line);
    }
    out
}

/// Word wrap at the current font. Lines are glyph-converted first so the measure matches what is drawn.
pub fn wrap(s: &str, width: f32, size: f32) -> Vec<String> {
    let g = glyph(s);
    wrap_with(&g, width, &|t: &str| measure(t, size))
}

/// Shorten `s` with ".." so it fits `width` (Godot `clip_text`). Pure.
pub fn fit_with(s: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(s) <= width {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cand: String = chars.iter().collect::<String>() + "..";
        if measure(&cand) <= width {
            return cand;
        }
    }
    String::new()
}

pub fn fit(s: &str, width: f32, size: f32) -> String {
    let g = glyph(s);
    fit_with(&g, width, &|t: &str| measure(t, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five(s: &str) -> f32 {
        s.chars().count() as f32 * 5.0
    }

    #[test]
    fn glyph_replaces_the_missing_symbols() {
        assert_eq!(glyph("▶ START"), "> START");
        assert_eq!(glyph("← BACK"), "<- BACK");
        assert_eq!(glyph("a → b"), "a -> b");
        assert_eq!(glyph("★ 2/3"), "* 2/3");
        assert_eq!(glyph("☆ Easy"), "- Easy");
        assert_eq!(glyph("✔ CLEARED"), "OK CLEARED");
        assert_eq!(glyph("● REC"), "o REC");
        assert_eq!(glyph("Ⅱ"), "II");
        assert_eq!(glyph("TAB 🔒"), "TAB [LOCKED]");
        assert_eq!(glyph("wait…"), "wait...");
        assert_eq!(glyph("↑ ↓ select · Enter"), "UP/DOWN select | Enter");
        assert_eq!(glyph("▲"), "^");
        assert_eq!(glyph("▮"), "|");
    }

    #[test]
    fn glyph_leaves_ascii_and_masks_the_rest() {
        assert_eq!(glyph("PLAY"), "PLAY");
        assert_eq!(glyph("caf\u{e9}"), "caf?");
    }

    #[test]
    fn wrap_never_exceeds_the_width() {
        let s = "the quick brown fox jumps over the lazy dog and keeps running far away";
        let lines = wrap_with(s, 50.0, &five);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(five(l) <= 50.0, "line too wide: {l:?}");
        }
        assert_eq!(lines.join(" "), s);
    }

    #[test]
    fn wrap_breaks_a_word_wider_than_the_line() {
        let lines = wrap_with("abcdefghijklmnopqrstuvwxyz", 50.0, &five);
        assert!(lines.len() >= 3);
        for l in &lines {
            assert!(five(l) <= 50.0);
        }
        assert_eq!(lines.concat(), "abcdefghijklmnopqrstuvwxyz");
    }

    #[test]
    fn wrap_keeps_paragraphs() {
        let lines = wrap_with("one\ntwo", 100.0, &five);
        assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn fit_truncates_with_dots() {
        assert_eq!(fit_with("short", 100.0, &five), "short");
        let f = fit_with("a very long tile name", 50.0, &five);
        assert!(five(&f) <= 50.0);
        assert!(f.ends_with(".."));
    }
}
