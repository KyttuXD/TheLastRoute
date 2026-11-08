#![allow(dead_code)]
//! NightWatchArt: material kinds, colour ramps, painted 16x16 tiles and the name -> kind table.
//! Port of `night_watch_art.gd`. The pure parts (`kind_for`, `tile_rgba`) need no GL context; only
//! `Textures::build` creates textures.

use macroquad::prelude::*;
use std::collections::HashMap;

pub const TEX: usize = 16;
/// Metres of cube face covered by one texture tile.
pub const TILE: f32 = 0.5;
/// Hazard tape density: 16 px per 0.25 m.
pub const HAZ_TILE: f32 = 0.25;
pub const HAZ_PERIOD_M: f32 = HAZ_TILE * 0.5;
/// Names (substring match, case-sensitive) that may be emissive.
pub const EMISSIVE_NAMES: [&str; 11] = [
    "Screen", "LampShade", "Led", "Label", "Button", "CeilingPanel", "ClockFace", "Glow", "Sign", "Feed", "Rec",
];

/// kind -> ramp (0xRRGGBB), lightest first.
pub const RAMPS: [(&str, [u32; 4]); 15] = [
    ("wood", [0xc08a55, 0xa06f40, 0x7d5230, 0x573820]),
    ("metal", [0x8e96a0, 0x6c7480, 0x4c525c, 0x2f333a]),
    ("plastic", [0x5a5e66, 0x42464d, 0x2e3137, 0x1c1e22]),
    ("fabric", [0x7a4e5e, 0x633d4c, 0x4d2f3b, 0x361f29]),
    ("concrete", [0x7d8088, 0x686b73, 0x52555c, 0x3d3f45]),
    ("glass", [0xb9d6d2, 0x86aaa8, 0x5b7c7d, 0x3a5558]),
    ("wall", [0x6d7587, 0x585f70, 0x444a59, 0x323744]),
    ("floor", [0x5f5a62, 0x4b4650, 0x39353f, 0x29262f]),
    ("screen", [0xbff0d8, 0x7fd1b0, 0x4a9c86, 0x2c6b60]),
    ("label", [0xf2c230, 0xd9a21f, 0x2b2416, 0x14110a]),
    ("paper", [0xe8e2d0, 0xc9c2ae, 0x6b6455, 0x3a352c]),
    ("hazard", [0xf2c230, 0xd9a21f, 0x2b2416, 0x14110a]),
    ("light", [0xfff3c8, 0xffe08f, 0xf0b954, 0xc98a35]),
    ("red", [0xe86a5a, 0xc44a3c, 0x8e2f28, 0x5a1d1a]),
    ("green", [0x8be89a, 0x55c46a, 0x379248, 0x1f5a2c]),
];

/// Ordered name -> kind table (first match of the lower-cased name containing the key).
const KIND_TABLE: &[(&str, &str)] = &[
    ("screen", "screen"), ("led", "green"), ("label", "label"), ("sign", "label"), ("hazard", "hazard"),
    ("lampshade", "light"), ("ceilingpanel", "light"), ("clockface", "light"), ("glow", "light"), ("feed", "screen"),
    ("desk", "wood"), ("frame", "wood"), ("shelf", "wood"), ("crate", "wood"), ("table", "wood"), ("platform", "wood"),
    ("box", "wood"), ("bench", "wood"), ("counter", "metal"), ("fridge", "metal"), ("locker", "metal"), ("slab", "metal"),
    ("panel", "metal"), ("breaker", "metal"), ("vent", "metal"), ("pipe", "metal"), ("conduit", "metal"), ("lamp", "metal"),
    ("bolt", "metal"), ("handle", "metal"), ("rail", "metal"), ("hatch", "metal"), ("door", "metal"),
    ("chair", "fabric"), ("poster", "fabric"), ("curtain", "fabric"), ("mat", "fabric"), ("cushion", "fabric"),
    ("floor", "floor"), ("runner", "floor"), ("stripe", "floor"), ("ceiling", "concrete"), ("pillar", "concrete"),
    ("monitor", "plastic"), ("keyboard", "plastic"), ("cup", "plastic"), ("bin", "plastic"), ("paper", "paper"),
    ("glass", "glass"), ("wall", "wall"), ("trim", "wall"), ("baseboard", "wall"), ("hall", "wall"),
];

/// Which material kind a node name asks for.
pub fn kind_for(name: &str) -> &'static str {
    let n = name.to_lowercase();
    for (key, kind) in KIND_TABLE.iter() {
        if n.contains(*key) {
            return *kind;
        }
    }
    "concrete"
}

pub fn is_emissive_name(name: &str) -> bool {
    EMISSIVE_NAMES.iter().any(|p| name.contains(*p))
}

fn ramp_of(kind: &str) -> [u32; 4] {
    RAMPS.iter().find(|(k, _)| *k == kind).map(|(_, r)| *r).unwrap_or(RAMPS[4].1)
}

/// A fixed per-kind salt (FNV-1a of the name, low 16 bits).
fn salt_of(kind: &str) -> i64 {
    let mut h: u32 = 0x811c9dc5;
    for b in kind.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    (h & 0xffff) as i64
}

/// `_hash(x, y, salt)` of the Godot file, in [0,1].
pub fn tile_hash(x: i64, y: i64, salt: i64) -> f64 {
    let mut h: i64 = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(salt.wrapping_mul(2246822519))
        & 0x7fffffff;
    h = ((h ^ (h >> 13)).wrapping_mul(1274126177)) & 0x7fffffff;
    (h & 0xffff) as f64 / 65535.0
}

fn tone_at(kind: &str, x: i64, y: i64, salt: i64) -> i64 {
    let t = TEX as i64;
    let n = tile_hash(x, y, salt);
    let mut tone: i64;
    match kind {
        "wood" => {
            tone = 1;
            if y % 5 == 0 {
                tone = 2;
            } else if n < 0.12 {
                tone = 0;
            } else if n > 0.9 {
                tone = 3;
            }
            if x % 8 == 0 {
                tone = 3;
            }
        }
        "metal" => {
            tone = 1 + (tile_hash(0, y, salt) * 2.0) as i64;
            if x == 0 || y == 0 {
                tone = 0;
            } else if x == t - 1 || y == t - 1 {
                tone = 3;
            }
            if (x == 3 && y == 3) || (x == 12 && y == 12) || (x == 3 && y == 12) || (x == 12 && y == 3) {
                tone = if (x + y) % 2 == 0 { 0 } else { 3 };
            }
        }
        "plastic" => {
            tone = 1;
            if n < 0.08 {
                tone = 0;
            } else if n > 0.92 {
                tone = 2;
            }
            if x == 0 || y == 0 {
                tone = 0;
            } else if x == t - 1 || y == t - 1 {
                tone = 3;
            }
        }
        "fabric" => {
            tone = 1 + ((x + y) % 2) + if n > 0.93 { 1 } else { 0 };
            tone = tone.min(3);
            if n < 0.05 {
                tone = 0;
            }
        }
        "concrete" => {
            tone = ((n * 4.0) as i64).clamp(0, 3);
            if n > 0.5 && n < 0.8 {
                tone = 1;
            }
        }
        "glass" => {
            tone = ((y as f64 / t as f64 * 3.0) as i64).clamp(0, 3);
            if (x - y).abs() <= 1 && y > 3 && y < 12 {
                tone = 0;
            }
        }
        "wall" => {
            tone = 1;
            if y == 8 {
                tone = 3;
            } else if y == 9 {
                tone = 0;
            } else if n < 0.1 {
                tone = 2;
            } else if n > 0.95 {
                tone = 0;
            }
            if (x == 1 || x == 14) && (y == 1 || y == 14) {
                tone = 0;
            }
        }
        "floor" => {
            tone = if ((x / 8) + (y / 8)) % 2 == 0 { 1 } else { 2 };
            if x % 8 == 0 || y % 8 == 0 {
                tone = 3;
            } else if n < 0.1 {
                tone = 3;
            } else if n > 0.93 {
                tone = 0;
            }
        }
        "screen" => {
            tone = if y % 2 == 0 { 1 } else { 2 };
            if y < 2 {
                tone = 0;
            } else if y > t - 3 {
                tone = 3;
            }
        }
        "label" | "paper" => {
            tone = 0;
            if (y == 5 || y == 6 || y == 9 || y == 10) && x >= 2 && x <= 13 {
                tone = 2;
            }
            if y == 0 || y == t - 1 || x == 0 || x == t - 1 {
                tone = 1;
            }
        }
        "hazard" => {
            let d = (x + y) % 8;
            tone = if d < 4 { 0 } else { 2 };
            if d == 0 {
                tone = 1;
            } else if d == 4 {
                tone = 3;
            }
        }
        "light" => {
            tone = (((x - 8).abs().max((y - 8).abs()) as f64 / 3.0) as i64).clamp(0, 3);
        }
        _ => {
            tone = ((n * 4.0) as i64).clamp(0, 3);
        }
    }
    tone.clamp(0, 3)
}

/// The painted 16x16 RGBA8 tile of a kind (unknown kinds paint with the concrete ramp).
pub fn tile_rgba(kind: &str) -> Vec<u8> {
    let ramp = ramp_of(kind);
    let salt = salt_of(kind);
    let mut out = Vec::with_capacity(TEX * TEX * 4);
    for y in 0..TEX as i64 {
        for x in 0..TEX as i64 {
            let c = ramp[tone_at(kind, x, y, salt) as usize];
            out.push(((c >> 16) & 0xff) as u8);
            out.push(((c >> 8) & 0xff) as u8);
            out.push((c & 0xff) as u8);
            out.push(255);
        }
    }
    out
}

fn make_texture(w: u16, h: u16, bytes: &[u8]) -> Texture2D {
    let t = Texture2D::from_rgba8(w, h, bytes);
    t.set_filter(FilterMode::Nearest);
    set_repeat(&t);
    t
}

/// Repeat wrapping (see last_route render/prop_model.rs `repeat`); sizes here are powers of two so WebGL1 accepts it.
fn set_repeat(t: &Texture2D) {
    use macroquad::miniquad::TextureWrap;
    let id = t.raw_miniquad_id();
    unsafe {
        get_internal_gl().quad_context.texture_set_wrap(id, TextureWrap::Repeat, TextureWrap::Repeat);
    }
}

/// The 15 kind textures (16x16, Nearest, Repeat) plus any extra named checker textures.
pub struct Textures {
    map: HashMap<String, Texture2D>,
    extra: HashMap<String, Texture2D>,
}

impl Textures {
    /// Needs a live GL context.
    pub fn build() -> Textures {
        let mut map = HashMap::new();
        for (kind, _) in RAMPS.iter() {
            map.insert(kind.to_string(), make_texture(TEX as u16, TEX as u16, &tile_rgba(kind)));
        }
        Textures { map, extra: HashMap::new() }
    }

    /// Unknown kind -> concrete.
    pub fn get(&self, kind: &str) -> &Texture2D {
        if let Some(t) = self.map.get(kind) {
            return t;
        }
        if let Some(t) = self.extra.get(kind) {
            return t;
        }
        &self.map["concrete"]
    }

    /// A 2x2 texture (FloorChecker, CheckerBand); cached by name and also reachable through `get(name)`.
    pub fn solid_2x2(&mut self, name: &str, px: [[u8; 4]; 4]) -> Texture2D {
        if let Some(t) = self.extra.get(name) {
            return t.clone();
        }
        let mut bytes = Vec::with_capacity(16);
        for p in px.iter() {
            bytes.extend_from_slice(p);
        }
        let t = make_texture(2, 2, &bytes);
        self.extra.insert(name.to_string(), t.clone());
        t
    }

    /// Cheap clones of every texture handle by name (the baked World keeps these).
    pub fn handles(&self) -> HashMap<String, Texture2D> {
        let mut m = self.map.clone();
        for (k, v) in self.extra.iter() {
            m.insert(k.clone(), v.clone());
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_table_matches_godot_order() {
        assert_eq!(kind_for("WallBack"), "wall");
        assert_eq!(kind_for("DeskLeg"), "wood");
        assert_eq!(kind_for("LedAlarm"), "green");
        assert_eq!(kind_for("Whatever"), "concrete");
        // Godot lists "mat" (fabric) before "floor": a node called FloorMat is fabric there too
        assert_eq!(kind_for("FloorMat"), "fabric");
        assert_eq!(kind_for("FloorTile"), "floor");
        assert_eq!(kind_for("CeilingPanel"), "light");
    }

    #[test]
    fn emissive_names_are_case_sensitive() {
        assert!(is_emissive_name("LedAlarm"));
        assert!(is_emissive_name("LampShade"));
        assert!(!is_emissive_name("ledalarm"));
        assert!(!is_emissive_name("DeskLeg"));
    }

    #[test]
    fn every_tile_is_16x16_with_at_most_four_colours() {
        for (kind, ramp) in RAMPS.iter() {
            let px = tile_rgba(kind);
            assert_eq!(px.len(), 16 * 16 * 4);
            let mut seen = std::collections::HashSet::new();
            for p in px.chunks(4) {
                seen.insert([p[0], p[1], p[2]]);
                assert_eq!(p[3], 255);
            }
            assert!(seen.len() <= 4 && !seen.is_empty(), "{kind}");
            for s in seen {
                let c = ((s[0] as u32) << 16) | ((s[1] as u32) << 8) | s[2] as u32;
                assert!(ramp.contains(&c), "{kind} uses a colour outside its ramp");
            }
        }
    }

    #[test]
    fn hazard_tile_repeats_every_eight_pixels() {
        let px = tile_rgba("hazard");
        let at = |x: usize, y: usize| px[(y * 16 + x) * 4..(y * 16 + x) * 4 + 3].to_vec();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(at(x, y), at(x + 8, y));
                assert_eq!(at(x, y), at(x, y + 8));
            }
        }
    }

    #[test]
    fn tiles_are_deterministic() {
        assert_eq!(tile_rgba("wood"), tile_rgba("wood"));
        assert_ne!(tile_rgba("wood"), tile_rgba("metal"));
        assert_eq!(tile_rgba("nonsense"), tile_rgba("nonsense"));
    }

    #[test]
    fn hash_is_in_unit_range() {
        for x in 0..16 {
            for y in 0..16 {
                let h = tile_hash(x, y, 1234);
                assert!((0.0..=1.0).contains(&h));
            }
        }
    }
}
