//! Who a revealed hunter is on the maps (KT, 2026-11-05: "hunter reveal should be more specific about which hunter it is").
//! A hunter that the maps already show (`Game::map_hunters`: Porcelle's radar ping, the admin HUNTER REVEAL) is drawn as a
//! dot that names it:
//!   * an A to Z hunter keeps the red danger dot and wears its capital letter (the display name is that letter),
//!   * a void hunter has no letter: its dot is filled with its colour (black, blue, brown, ...), ringed so white, black and
//!     gray stay visible on the dark map,
//!   * a mod-pack hunter, or any other hunter, gets a neutral dot with the first letter of its name.
//! Only the identity is added here. A mark is drawn only for a hunter the caller already placed on the map and fades with the
//! same alpha as that hunter's dot (Porcelle between pings, a sleeping hunter dimmed), so the letter or colour is never
//! visible where the position was not. The colours are data (data/ui/hunter_marker_colors.json), validated on load.

use crate::app::assets::Assets;
use crate::content::hunters::HunterDef;
use crate::ui::style::*;
use macroquad::prelude::*;
use std::collections::BTreeMap;

/// The embedded colour table (`lr.hunter_marker_colors/1`).
const FILE: &str = include_str!("../../data/ui/hunter_marker_colors.json");
const FORMAT: &str = "lr.hunter_marker_colors/1";

/// A dot's fill and the ring around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub fill: Color,
    pub outline: Color,
}

/// The validated table: the letter hunters' text, the neutral dot, and the colour of each void hunter by id.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerColors {
    letter_text: Color,
    letter_outline: Color,
    neutral: Ink,
    neutral_text: Color,
    void: BTreeMap<String, Ink>,
}

/// How one hunter is drawn: an optional capital letter, the dot's fill (`None` is the theme's danger red), the ring and the
/// letter's colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub label: Option<char>,
    pub fill: Option<Color>,
    pub outline: Color,
    pub text: Color,
}

/// One hunter ready to draw: its mark, the plan position of its dot (units), its dot radius and how visible it is (0..1; the
/// same factor the dot itself gets).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub mark: Mark,
    pub at: (f32, f32),
    pub radius: f32,
    pub alpha: f32,
}

fn hex6(v: &serde_json::Value, key: &str) -> Result<Color, String> {
    let s = v.as_str().ok_or_else(|| format!("{key}: expected a \"#rrggbb\" string, found {v}"))?;
    let digits = s.strip_prefix('#').filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_hexdigit()));
    let digits = digits.ok_or_else(|| format!("{key}: {s:?} is not a #rrggbb colour"))?;
    let n = u32::from_str_radix(digits, 16).map_err(|e| format!("{key}: {s:?}: {e}"))?;
    Ok(Color::new(
        ((n >> 16) & 0xff) as f32 / 255.0,
        ((n >> 8) & 0xff) as f32 / 255.0,
        (n & 0xff) as f32 / 255.0,
        1.0,
    ))
}

/// Brightness 0..1 (Rec. 601 luma on the stored values), enough to tell a fill from its ring.
fn luma(c: Color) -> f32 {
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b
}

fn field<'a>(v: &'a serde_json::Value, path: &str, key: &str) -> Result<&'a serde_json::Value, String> {
    v.get(key).ok_or_else(|| format!("{path}.{key}: missing"))
}

fn ink(v: &serde_json::Value, path: &str, min_contrast: f32) -> Result<Ink, String> {
    if !v.is_object() {
        return Err(format!("{path}: expected an object with fill and outline"));
    }
    let fill = hex6(field(v, path, "fill")?, &format!("{path}.fill"))?;
    let outline = hex6(field(v, path, "outline")?, &format!("{path}.outline"))?;
    let gap = (luma(fill) - luma(outline)).abs();
    if gap < min_contrast {
        return Err(format!(
            "{path}: the outline is too close to the fill in brightness ({gap:.2} < {min_contrast:.2}); it would not stand out"
        ));
    }
    Ok(Ink { fill, outline })
}

impl MarkerColors {
    /// Reads and validates a table. The error names the key at fault ("colors.void_red.fill: ...").
    pub fn parse(text: &str) -> Result<MarkerColors, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("hunter_marker_colors.json is not JSON: {e}"))?;
        match v.get("format").and_then(|f| f.as_str()) {
            Some(FORMAT) => {}
            other => return Err(format!("format: expected {FORMAT:?}, found {other:?}")),
        }
        let min_contrast = match v.get("min_contrast") {
            None => 0.3,
            Some(x) => x
                .as_f64()
                .filter(|c| (0.0..=1.0).contains(c))
                .ok_or_else(|| format!("min_contrast: expected a number from 0 to 1, found {x}"))? as f32,
        };
        let letter = field(&v, "", "letter").map_err(|e| e.trim_start_matches('.').to_string())?;
        let letter_text = hex6(field(letter, "letter", "text")?, "letter.text")?;
        let letter_outline = hex6(field(letter, "letter", "outline")?, "letter.outline")?;
        if (luma(letter_text) - luma(letter_outline)).abs() < min_contrast {
            return Err("letter: the text is too close to its outline in brightness".to_string());
        }
        let neutral_v = field(&v, "", "neutral").map_err(|e| e.trim_start_matches('.').to_string())?;
        let neutral = ink(neutral_v, "neutral", min_contrast)?;
        let neutral_text = hex6(field(neutral_v, "neutral", "text")?, "neutral.text")?;
        let colors = field(&v, "", "colors")
            .map_err(|e| e.trim_start_matches('.').to_string())?
            .as_object()
            .ok_or("colors: expected an object keyed by hunter id")?;
        if colors.is_empty() {
            return Err("colors: no hunter has a colour".to_string());
        }
        let mut void = BTreeMap::new();
        for (id, entry) in colors {
            let ok_id = id.starts_with("void_") && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !ok_id {
                return Err(format!("colors.{id}: the key must be a void hunter id (void_...)"));
            }
            void.insert(id.clone(), ink(entry, &format!("colors.{id}"), min_contrast)?);
        }
        Ok(MarkerColors { letter_text, letter_outline, neutral, neutral_text, void })
    }

    /// The embedded table. A broken file is reported once on stderr and every hunter then wears the neutral dot.
    pub fn load() -> &'static MarkerColors {
        static COLORS: std::sync::OnceLock<MarkerColors> = std::sync::OnceLock::new();
        COLORS.get_or_init(|| {
            Self::parse(FILE).unwrap_or_else(|e| {
                eprintln!("data/ui/hunter_marker_colors.json: {e}; revealed hunters use the neutral dot");
                MarkerColors {
                    letter_text: WHITE,
                    letter_outline: Color::new(0.02, 0.03, 0.04, 1.0),
                    neutral: Ink { fill: Color::new(0.73, 0.75, 0.77, 1.0), outline: Color::new(0.02, 0.03, 0.04, 1.0) },
                    neutral_text: WHITE,
                    void: BTreeMap::new(),
                }
            })
        })
    }

    /// The colour of a void hunter, if the table has it.
    pub fn void_ink(&self, id: &str) -> Option<Ink> {
        self.void.get(id).copied()
    }

    /// The ids that have a colour, in order.
    pub fn void_ids(&self) -> impl Iterator<Item = &str> {
        self.void.keys().map(|k| k.as_str())
    }

    /// How a hunter is drawn, from its id, display name, category and whether a mod pack supplies it.
    pub fn mark(&self, id: &str, name: &str, category: &str, is_pack: bool) -> Mark {
        let neutral = |c: &MarkerColors| Mark {
            label: Some(first_letter(name, id)),
            fill: Some(c.neutral.fill),
            outline: c.neutral.outline,
            text: c.neutral_text,
        };
        if is_pack {
            return neutral(self);
        }
        match category {
            "az" => Mark {
                label: Some(first_letter(name, id)),
                fill: None,
                outline: self.letter_outline,
                text: self.letter_text,
            },
            "void" => match self.void_ink(id) {
                Some(i) => Mark { label: None, fill: Some(i.fill), outline: i.outline, text: i.outline },
                None => neutral(self),
            },
            _ => neutral(self),
        }
    }
}

/// The capital letter that names a hunter: the first letter of its display name (the A to Z hunters' name is that letter),
/// else the first letter of its id, else "?".
pub fn first_letter(name: &str, id: &str) -> char {
    name.chars()
        .chain(id.chars())
        .find(|c| c.is_alphabetic())
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('?')
}

/// How a hunter definition is drawn on the maps.
pub fn mark_of(def: &HunterDef) -> Mark {
    MarkerColors::load().mark(&def.id, &def.name, &def.category, def.is_pack())
}

/// Offsets (plan units) that pull apart marks standing on top of each other: marks closer than `step` (width, height) share a
/// group, and a group of n is laid out in a centred row of up to [`MAX_ROW`] with further rows below. A mark standing alone
/// is not moved. The offsets follow the input order, so the layout does not shuffle from frame to frame.
pub fn spread(points: &[(f32, f32)], step: (f32, f32)) -> Vec<(f32, f32)> {
    let mut groups: Vec<((f32, f32), Vec<usize>)> = vec![];
    for (i, p) in points.iter().enumerate() {
        let near = groups
            .iter_mut()
            .find(|(anchor, _)| (anchor.0 - p.0).abs() < step.0 && (anchor.1 - p.1).abs() < step.1);
        match near {
            Some((_, members)) => members.push(i),
            None => groups.push((*p, vec![i])),
        }
    }
    let mut out = vec![(0.0, 0.0); points.len()];
    for (_, members) in &groups {
        let n = members.len();
        if n < 2 {
            continue;
        }
        let cols = n.min(MAX_ROW);
        for (k, &i) in members.iter().enumerate() {
            let (col, row) = (k % cols, k / cols);
            // the last row may be short: centre it on its own count
            let in_row = if row == (n - 1) / cols { n - row * cols } else { cols };
            out[i] = ((col as f32 - (in_row as f32 - 1.0) * 0.5) * step.0, row as f32 * step.1);
        }
    }
    out
}

/// The most marks side by side in one stack.
pub const MAX_ROW: usize = 4;

/// The size of a mark's letter in units: 6 on the minimap, 8 on the full map, and never under 12 pixels (as the HUD's text).
fn letter_size(full: bool) -> f32 {
    let floor = (12.0 / unit()).ceil().min(9.0);
    let base: f32 = if full { 8.0 } else { 6.0 };
    base.max(floor)
}

fn fade(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * a)
}

/// Draws the hunters in `marks` (already limited to the ones on this view): every dot first, then every letter, so no dot hides
/// a letter. Hunters on top of one another are fanned out ([`spread`]). A mark with no alpha is not drawn at all.
pub fn draw(a: &Assets, marks: &[Placed], full: bool) {
    let live: Vec<&Placed> = marks.iter().filter(|m| m.alpha > 0.001).collect();
    if live.is_empty() {
        return;
    }
    let size = letter_size(full);
    let widest = live.iter().map(|m| m.radius * 2.0).fold(0.0, f32::max);
    let any_label = live.iter().any(|m| m.mark.label.is_some());
    // a letter stands over its dot: the stack is as tall as the dot plus the letter
    let step = (
        widest.max(if any_label { size * 0.6 } else { 0.0 }) + 1.0,
        widest + if any_label { size } else { 0.0 } + 0.5,
    );
    let offsets = spread(&live.iter().map(|m| m.at).collect::<Vec<_>>(), step);
    let ring = if full { 0.9 } else { 0.6 };
    for (m, o) in live.iter().zip(&offsets) {
        let c = vec2(m.at.0 + o.0, m.at.1 + o.1);
        circle(c, m.radius + ring, fade(m.mark.outline, m.alpha));
        circle(c, m.radius, fade(m.mark.fill.unwrap_or(METER_LOW), m.alpha));
    }
    for (m, o) in live.iter().zip(&offsets) {
        let Some(letter) = m.mark.label else { continue };
        let text = letter.to_string();
        let cx = m.at.0 + o.0;
        // the baseline sits just over the ring
        let base = m.at.1 + o.1 - m.radius - ring - 0.4;
        let top = base - F::Body.ascent() * size;
        label_outline(
            a,
            F::Body,
            &text,
            cx - tw(a, F::Body, &text, size) * 0.5,
            top,
            size,
            fade(m.mark.text, m.alpha),
            fade(m.mark.outline, m.alpha),
            1.4,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;

    fn colors() -> MarkerColors {
        MarkerColors::parse(FILE).expect("the embedded colour table validates")
    }

    #[test]
    fn the_embedded_file_validates() {
        let c = colors();
        assert_eq!(c.void_ids().count(), 12);
        // the loader hands out the same table (no fallback was needed)
        assert_eq!(MarkerColors::load(), &c);
    }

    #[test]
    fn every_a_to_z_hunter_is_labelled_with_its_letter() {
        let content = Content::load();
        let c = colors();
        let mut seen = std::collections::BTreeSet::new();
        for h in content.hunters.iter().filter(|h| !h.is_pack() && h.category == "az") {
            let m = c.mark(&h.id, &h.name, &h.category, h.is_pack());
            let letter = h.id.chars().next().unwrap().to_ascii_uppercase();
            assert_eq!(m.label, Some(letter), "{}", h.id);
            assert_eq!(m.fill, None, "{}: a letter hunter keeps the red danger dot", h.id);
            assert_eq!(mark_of(h).label, Some(letter), "{}", h.id);
            seen.insert(letter);
        }
        assert_eq!(seen.len(), 26, "{seen:?}");
    }

    #[test]
    fn every_void_hunter_has_its_own_colour_and_no_letter() {
        let content = Content::load();
        let c = colors();
        let mut fills: Vec<(String, Color)> = vec![];
        for h in content.hunters.iter().filter(|h| !h.is_pack() && h.category == "void") {
            let m = c.mark(&h.id, &h.name, &h.category, false);
            assert_eq!(m.label, None, "{}", h.id);
            let fill = m.fill.unwrap_or_else(|| panic!("{} has no colour", h.id));
            assert_eq!(c.void_ink(&h.id).map(|i| i.fill), Some(fill));
            // the table is keyed by id and the colour is the display name ("void_red" is "Red")
            assert_eq!(h.id, format!("void_{}", h.name.to_lowercase()), "{}", h.id);
            fills.push((h.id.clone(), fill));
        }
        assert_eq!(fills.len(), 12);
        for id in c.void_ids() {
            assert!(fills.iter().any(|(f, _)| f == id), "{id} in the table is not a void hunter");
        }
        for (i, (a, ca)) in fills.iter().enumerate() {
            for (b, cb) in &fills[i + 1..] {
                let d = ((ca.r - cb.r).powi(2) + (ca.g - cb.g).powi(2) + (ca.b - cb.b).powi(2)).sqrt();
                assert!(d > 0.2, "{a} and {b} are too alike ({d:.2})");
            }
        }
    }

    #[test]
    fn white_black_and_gray_stay_visible_on_the_dark_map() {
        let c = colors();
        for id in ["void_white", "void_black", "void_gray"] {
            let i = c.void_ink(id).expect(id);
            assert!((luma(i.fill) - luma(i.outline)).abs() >= 0.3, "{id}: the ring must stand out from the dot");
        }
        // black has a light ring, white a dark one
        assert!(luma(c.void_ink("void_black").unwrap().outline) > 0.8);
        assert!(luma(c.void_ink("void_white").unwrap().outline) < 0.2);
    }

    #[test]
    fn an_unknown_id_falls_back_to_the_neutral_dot() {
        let c = colors();
        // a void hunter with no entry: neutral dot, first letter of its name
        let m = c.mark("void_teal", "Teal", "void", false);
        assert_eq!(m.label, Some('T'));
        assert_eq!(m.fill, Some(c.neutral.fill));
        // a category the maps know nothing of
        let m = c.mark("mystery", "Mystery", "elsewhere", false);
        assert_eq!(m.label, Some('M'));
        assert_eq!(m.fill, Some(c.neutral.fill));
        // a nameless one uses its id, and a nameless idless one a question mark
        assert_eq!(first_letter("", "zed"), 'Z');
        assert_eq!(first_letter("", ""), '?');
        assert_eq!(first_letter("7", "q"), 'Q');
    }

    #[test]
    fn a_pack_hunter_gets_the_first_letter_of_its_name_in_the_neutral_colour() {
        let c = colors();
        let m = c.mark("moss_walker", "moss walker", "az", true);
        assert_eq!(m.label, Some('M'));
        assert_eq!(m.fill, Some(c.neutral.fill));
        // even a pack that replaces a built-in id, and even one claiming to be a void colour
        let m = c.mark("void_red", "Crimson", "void", true);
        assert_eq!(m.label, Some('C'));
        assert_eq!(m.fill, Some(c.neutral.fill));
        // the secret creatures are no A to Z hunter either
        let m = c.mark("black_wolf_spider", "Black Wolf Spider", "secret", false);
        assert_eq!(m.fill, Some(c.neutral.fill));
    }

    #[test]
    fn a_bad_file_reports_its_key() {
        let bad = |from: &str, to: &str| FILE.replacen(from, to, 1);
        let err = |text: String| MarkerColors::parse(&text).expect_err("the broken file is rejected");
        // a colour that is not #rrggbb names the colour's key
        let e = err(bad("#2f6bff", "blue"));
        assert!(e.contains("colors.void_blue.fill"), "{e}");
        let e = err(bad("#19d3e6", "#19d3e"));
        assert!(e.contains("colors.void_cyan.fill") && e.contains("#19d3e"), "{e}");
        // a missing field
        let e = err(bad("\"outline\": \"#ffffff\" },\n    \"void_brown\"", "\"nothing\": 1 },\n    \"void_brown\""));
        assert!(e.contains("colors.void_blue.outline"), "{e}");
        // an outline that cannot be seen against its fill
        let e = err(bad("{ \"fill\": \"#ffffff\", \"outline\": \"#05080a\" },\n    \"void_yellow\"", "{ \"fill\": \"#ffffff\", \"outline\": \"#fafafa\" },\n    \"void_yellow\""));
        assert!(e.contains("colors.void_white"), "{e}");
        // a key that is not a void hunter
        let e = err(bad("\"void_pink\"", "\"pink\""));
        assert!(e.contains("colors.pink"), "{e}");
        // sections and the format
        assert!(err(bad("\"neutral\"", "\"neutrall\"")).contains("neutral"));
        assert!(err(bad("\"letter\": { \"text\": \"#ffffff\"", "\"letter\": { \"text\": 5")).contains("letter.text"));
        assert!(err(bad("lr.hunter_marker_colors/1", "lr.other/1")).contains("format"));
        assert!(err("{".to_string()).contains("not JSON"));
        assert!(err("{\"format\":\"lr.hunter_marker_colors/1\",\"letter\":{\"text\":\"#fff\"}}".to_string()).contains("letter.text"));
    }

    #[test]
    fn marks_on_top_of_each_other_are_fanned_out_and_a_lone_mark_stays_put() {
        let step = (6.0, 9.0);
        // one mark: not moved
        assert_eq!(spread(&[(10.0, 10.0)], step), vec![(0.0, 0.0)]);
        // far apart: not moved
        assert_eq!(spread(&[(10.0, 10.0), (60.0, 10.0)], step), vec![(0.0, 0.0), (0.0, 0.0)]);
        // two on one spot: side by side, centred on the spot
        let o = spread(&[(10.0, 10.0), (10.5, 10.0)], step);
        assert_eq!(o, vec![(-3.0, 0.0), (3.0, 0.0)]);
        // three: a centred row
        let o = spread(&[(5.0, 5.0), (5.0, 5.0), (5.0, 5.0)], step);
        assert_eq!(o, vec![(-6.0, 0.0), (0.0, 0.0), (6.0, 0.0)]);
        // six: a row of four, then a centred row of two below
        let o = spread(&[(5.0, 5.0); 6], step);
        assert_eq!(o[0], (-9.0, 0.0));
        assert_eq!(o[3], (9.0, 0.0));
        assert_eq!(o[4], (-3.0, 9.0));
        assert_eq!(o[5], (3.0, 9.0));
        // no two of them land on the same place
        for i in 0..o.len() {
            for j in i + 1..o.len() {
                assert_ne!(o[i], o[j]);
            }
        }
    }
}
