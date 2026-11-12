//! Empties the glyphs a TrueType font does not need, before the font is parsed.
//!
//! fontdue builds every glyph's outline when it loads a font: about 4 KB a glyph, so a 2400-glyph face (Liberation,
//! Courier New) costs about 10 MB of memory however little of it is drawn. [`subset`] rewrites the font's `glyf` and
//! `loca` tables so that only the glyphs behind [`wanted`] characters (and the parts composite glyphs are built from)
//! keep their outlines. Glyph numbers, the character map, metrics and kerning are untouched, so text measures and
//! draws exactly as before; a character outside the set draws as blank space of its normal width.
//!
//! Anything unexpected in the file (not TrueType outlines, a table out of bounds) returns `None` and the caller
//! loads the font as it is.

/// The characters kept: Latin (Basic, Latin-1, Extended-A), general punctuation, currency, arrows, maths, box drawing
/// and shapes. That covers every string the game ships and Western European text; usernames are ASCII already.
/// Other scripts (Greek, Cyrillic, Vietnamese) draw blank; `LR_FULL_FONTS=1` loads whole fonts for a pack that
/// needs them (see `shared/assets.rs`).
pub fn wanted(c: char) -> bool {
    matches!(c as u32,
        0x20..=0x17F | 0x2000..=0x206F | 0x20A0..=0x20BF | 0x2190..=0x21FF | 0x2200..=0x22FF | 0x2500..=0x25FF | 0xFFFD)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

struct Table {
    tag: [u8; 4],
    offset: usize,
    length: usize,
}

fn tables(font: &[u8]) -> Option<Vec<Table>> {
    let version = u32_at(font, 0)?;
    if version != 0x0001_0000 && version != 0x7472_7565 {
        return None;
    }
    let count = u16_at(font, 4)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rec = 12 + 16 * i;
        let offset = u32_at(font, rec + 8)? as usize;
        let length = u32_at(font, rec + 12)? as usize;
        font.get(offset..offset.checked_add(length)?)?;
        out.push(Table { tag: font.get(rec..rec + 4)?.try_into().ok()?, offset, length });
    }
    Some(out)
}

fn table<'a>(font: &'a [u8], tables: &[Table], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let t = tables.iter().find(|t| &t.tag == tag)?;
    font.get(t.offset..t.offset + t.length)
}

/// Every (character, glyph) pair of the font's Unicode character map (formats 4 and 12).
fn char_glyphs(cmap: &[u8]) -> Option<Vec<(u32, u16)>> {
    let count = u16_at(cmap, 2)? as usize;
    // prefer the full-Unicode subtable, then the 16-bit one
    let mut best: Option<(u8, usize)> = None;
    for i in 0..count {
        let rec = 4 + 8 * i;
        let (platform, encoding) = (u16_at(cmap, rec)?, u16_at(cmap, rec + 2)?);
        let offset = u32_at(cmap, rec + 4)? as usize;
        let format = u16_at(cmap, offset)?;
        let rank = match (platform, encoding, format) {
            (3, 10, 12) | (0, _, 12) => 3,
            (3, 1, 4) => 2,
            (0, _, 4) => 1,
            _ => 0,
        };
        if rank > 0 && best.map_or(true, |(r, _)| rank > r) {
            best = Some((rank, offset));
        }
    }
    let (_, at) = best?;
    let mut out = vec![];
    if u16_at(cmap, at)? == 12 {
        let groups = u32_at(cmap, at + 12)? as usize;
        for g in 0..groups.min(100_000) {
            let rec = at + 16 + 12 * g;
            let (start, end, glyph) = (u32_at(cmap, rec)?, u32_at(cmap, rec + 4)?, u32_at(cmap, rec + 8)?);
            if end < start || end - start > 0x11_0000 {
                return None;
            }
            for c in start..=end {
                out.push((c, (glyph + (c - start)) as u16));
            }
        }
    } else {
        let segs = u16_at(cmap, at + 6)? as usize / 2;
        let ends = at + 14;
        let starts = ends + 2 * segs + 2;
        let deltas = starts + 2 * segs;
        let ranges = deltas + 2 * segs;
        for s in 0..segs {
            let (start, end) = (u16_at(cmap, starts + 2 * s)?, u16_at(cmap, ends + 2 * s)?);
            let delta = u16_at(cmap, deltas + 2 * s)?;
            let range = u16_at(cmap, ranges + 2 * s)? as usize;
            if end < start {
                return None;
            }
            for c in start..=end {
                let glyph = if range == 0 {
                    c.wrapping_add(delta)
                } else {
                    let g = u16_at(cmap, ranges + 2 * s + range + 2 * (c - start) as usize)?;
                    if g == 0 { 0 } else { g.wrapping_add(delta) }
                };
                if c != 0xFFFF {
                    out.push((c as u32, glyph));
                }
            }
        }
    }
    Some(out)
}

/// The font with only the glyphs for `keep` characters left filled in, or `None` when it cannot be done safely.
pub fn subset(font: &[u8], keep: impl Fn(char) -> bool) -> Option<Vec<u8>> {
    let tabs = tables(font)?;
    let head = table(font, &tabs, b"head")?;
    let long = match u16_at(head, 50)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let glyphs = u16_at(table(font, &tabs, b"maxp")?, 4)? as usize;
    let loca = table(font, &tabs, b"loca")?;
    let glyf = table(font, &tabs, b"glyf")?;
    let span = |g: usize| -> Option<(usize, usize)> {
        let (a, b) = if long {
            (u32_at(loca, 4 * g)? as usize, u32_at(loca, 4 * g + 4)? as usize)
        } else {
            (u16_at(loca, 2 * g)? as usize * 2, u16_at(loca, 2 * g + 2)? as usize * 2)
        };
        (a <= b && b <= glyf.len()).then_some((a, b))
    };
    let mut kept = vec![false; glyphs];
    let mut stack = vec![0usize];
    for (c, g) in char_glyphs(table(font, &tabs, b"cmap")?)? {
        if char::from_u32(c).map_or(false, &keep) {
            stack.push(g as usize);
        }
    }
    // a composite glyph is built from other glyphs: keep those too
    while let Some(g) = stack.pop() {
        if g >= glyphs || kept[g] {
            continue;
        }
        kept[g] = true;
        let (a, b) = span(g)?;
        let data = &glyf[a..b];
        if data.len() < 10 || (u16_at(data, 0)? as i16) >= 0 {
            continue;
        }
        let mut at = 10;
        loop {
            let flags = u16_at(data, at)?;
            stack.push(u16_at(data, at + 2)? as usize);
            at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
            at += if flags & 0x0008 != 0 {
                2
            } else if flags & 0x0040 != 0 {
                4
            } else if flags & 0x0080 != 0 {
                8
            } else {
                0
            };
            if flags & 0x0020 == 0 {
                break;
            }
        }
    }
    let mut new_glyf = Vec::with_capacity(glyf.len() / 2);
    let mut new_loca = Vec::with_capacity(loca.len());
    let mut put = |offset: usize, out: &mut Vec<u8>| {
        if long {
            out.extend_from_slice(&(offset as u32).to_be_bytes());
        } else {
            out.extend_from_slice(&((offset / 2) as u16).to_be_bytes());
        }
    };
    for (g, keep) in kept.iter().enumerate() {
        put(new_glyf.len(), &mut new_loca);
        if *keep {
            let (a, b) = span(g)?;
            new_glyf.extend_from_slice(&glyf[a..b]);
            while new_glyf.len() % 4 != 0 {
                new_glyf.push(0);
            }
        }
    }
    put(new_glyf.len(), &mut new_loca);
    // the same table directory, with the two tables swapped and every offset recomputed
    let header = 12 + 16 * tabs.len();
    let mut out = font.get(..header)?.to_vec();
    for (i, t) in tabs.iter().enumerate() {
        let data: &[u8] = match &t.tag {
            b"glyf" => &new_glyf,
            b"loca" => &new_loca,
            _ => &font[t.offset..t.offset + t.length],
        };
        let rec = 12 + 16 * i;
        let offset = out.len() as u32;
        out[rec + 8..rec + 12].copy_from_slice(&offset.to_be_bytes());
        out[rec + 12..rec + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;

    fn glyph_bytes(font: &[u8], glyph: usize) -> Vec<u8> {
        let tabs = tables(font).unwrap();
        let long = u16_at(table(font, &tabs, b"head").unwrap(), 50).unwrap() == 1;
        let loca = table(font, &tabs, b"loca").unwrap();
        let glyf = table(font, &tabs, b"glyf").unwrap();
        let at = |g: usize| if long { u32_at(loca, 4 * g).unwrap() as usize } else { u16_at(loca, 2 * g).unwrap() as usize * 2 };
        glyf[at(glyph)..at(glyph + 1)].to_vec()
    }

    fn glyph_of(font: &[u8], c: char) -> Option<usize> {
        let tabs = tables(font).unwrap();
        char_glyphs(table(font, &tabs, b"cmap").unwrap()).unwrap().into_iter().find(|(ch, _)| *ch == c as u32).map(|(_, g)| g as usize)
    }

    /// Glyph data without the 4-byte padding either file may add.
    fn trimmed(mut v: Vec<u8>) -> Vec<u8> {
        while v.last() == Some(&0) {
            v.pop();
        }
        v
    }

    #[test]
    fn kept_glyphs_are_unchanged_and_the_rest_are_empty() {
        for file in ["LiberationMono-Bold.ttf", "LiberationMono-Italic.ttf", "Anton-Regular.ttf"] {
            let font = embedded::asset(&format!("fonts/{file}")).expect("the embedded font");
            let small = subset(font, wanted).unwrap_or_else(|| panic!("{file} did not subset"));
            assert!(small.len() < font.len(), "{file}");
            // the tables other than glyf and loca are byte-identical
            let (a, b) = (tables(font).unwrap(), tables(&small).unwrap());
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.tag, y.tag);
                if &x.tag != b"glyf" && &x.tag != b"loca" {
                    assert_eq!(font[x.offset..x.offset + x.length], small[y.offset..y.offset + y.length], "{file}");
                }
            }
            for c in ['A', 'z', '0', '%', 'é', 'Ü', '—', '→', '€'] {
                if let Some(g) = glyph_of(font, c) {
                    assert_eq!(trimmed(glyph_bytes(font, g)), trimmed(glyph_bytes(&small, g)), "{file} {c}");
                    assert!(g == 0 || !glyph_bytes(&small, g).is_empty(), "{file} {c}");
                }
            }
            // a character outside the set keeps its glyph number but loses its outline
            let pairs = char_glyphs(table(font, &tables(font).unwrap(), b"cmap").unwrap()).unwrap();
            let kept: std::collections::HashSet<u16> =
                pairs.iter().filter(|(c, _)| char::from_u32(*c).map_or(false, wanted)).map(|p| p.1).collect();
            let dropped = pairs.iter().find(|(_, g)| *g != 0 && !kept.contains(g) && !glyph_bytes(font, *g as usize).is_empty());
            if let Some((_, g)) = dropped {
                assert!(glyph_bytes(&small, *g as usize).is_empty() || kept_by_composite(font, &small, *g as usize), "{file}");
            }
        }
    }

    /// A glyph outside the set can still be kept as a part of a composite glyph inside it.
    fn kept_by_composite(font: &[u8], small: &[u8], g: usize) -> bool {
        trimmed(glyph_bytes(font, g)) == trimmed(glyph_bytes(small, g))
    }

    #[test]
    fn a_damaged_or_foreign_file_is_refused() {
        assert!(subset(b"", wanted).is_none());
        assert!(subset(b"OTTO\0\0\0\0\0\0\0\0", wanted).is_none());
        let font = embedded::asset("fonts/Anton-Regular.ttf").unwrap();
        for cut in [10, 100, 1000, font.len() / 2] {
            let _ = subset(&font[..cut], wanted);
        }
    }
}
