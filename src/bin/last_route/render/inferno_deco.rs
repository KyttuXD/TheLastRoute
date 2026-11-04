//! Inferno caves decoration (level -3): lava glow, embers, stalactites and stalagmites, charred props and
//! floor cracks. Pure placement: given a carved chunk it returns where each piece goes and how it looks. The
//! rules and colours come from data/render/inferno.json (compiled in by build.rs); the code only places.

use std::sync::OnceLock;

use crate::world::facility::Cell;
use crate::world::infinite::{ChunkKey, CHUNK};
use crate::world::inferno::{Inferno, Tier};

const PATH: &str = "render/inferno.json";

/// The decoration kinds, in the order they are placed (so the output order is stable).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecoKind {
    LavaGlow,
    Ember,
    Stalactite,
    Stalagmite,
    CharredProp,
    FloorCrack,
}

const KINDS: [(DecoKind, &str); 6] = [
    (DecoKind::LavaGlow, "lava_glow"),
    (DecoKind::Ember, "ember"),
    (DecoKind::Stalactite, "stalactite"),
    (DecoKind::Stalagmite, "stalagmite"),
    (DecoKind::CharredProp, "charred_prop"),
    (DecoKind::FloorCrack, "floor_crack"),
];

/// One kind's placement rules and look.
#[derive(Clone, Debug)]
pub struct KindRule {
    pub tiers: Vec<Tier>,
    /// Chance per eligible open cell, 0..=1.
    pub chance: f32,
    /// Cells (Chebyshev) kept clear between two pieces of this kind.
    pub min_spacing: i32,
    pub max_per_chunk: usize,
    pub size_m: [f32; 2],
    pub color: [f32; 3],
    pub glow_radius_m: f32,
    pub glow_strength: f32,
    pub flicker_hz: f32,
    pub rise_speed_mps: [f32; 2],
    pub life_s: [f32; 2],
}

/// All kinds' rules.
#[derive(Clone, Debug)]
pub struct InfernoDeco {
    pub rules: Vec<(DecoKind, KindRule)>,
}

/// One placed piece. `cell` is chunk-local.
#[derive(Clone, Copy, Debug)]
pub struct Deco {
    pub kind: DecoKind,
    pub cell: Cell,
    /// Offset inside the cell, in cells from its centre, each -0.5..0.5.
    pub offset: [f32; 2],
    pub size_m: f32,
    pub yaw: f32,
    pub color: [f32; 3],
    /// Floor height from the carve (0 on a ledge, negative in the basin).
    pub floor_y: f32,
    /// Per-piece phase for flicker and ember timing, 0..1.
    pub phase: f32,
}

impl InfernoDeco {
    /// Parses `{ "kinds": { "<name>": { tiers, chance, min_spacing, max_per_chunk, size_m, color, ... } } }`.
    /// Every kind must be present; the optional glow and ember fields default to zero.
    pub fn parse(text: &str) -> Result<InfernoDeco, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let kinds = root.get("kinds").ok_or("kinds: missing")?;
        let mut rules = Vec::new();
        for (kind, name) in KINDS {
            let v = kinds.get(name).ok_or_else(|| format!("{name}: missing"))?;
            rules.push((kind, parse_rule(name, v)?));
        }
        Ok(InfernoDeco { rules })
    }

    pub fn rule(&self, kind: DecoKind) -> Option<&KindRule> {
        self.rules.iter().find(|(k, _)| *k == kind).map(|(_, r)| r)
    }

    /// Places every piece for a carved chunk. Deterministic from `seed` and `key`; nothing lands on a closed cell.
    pub fn place(&self, seed: &str, key: ChunkKey, inf: &Inferno) -> Vec<Deco> {
        let mut out: Vec<Deco> = Vec::new();
        let open = inf.open_cells();
        for (kind, rule) in &self.rules {
            let mut kept: Vec<Cell> = Vec::new();
            for &c in &open {
                if kept.len() >= rule.max_per_chunk {
                    break;
                }
                if !rule.tiers.contains(&inf.tier_at(c)) {
                    continue;
                }
                let h = hash(seed, key, c, *kind as i32);
                if unit(h, 0) >= rule.chance {
                    continue;
                }
                if kept.iter().any(|k| (k.x - c.x).abs().max((k.y - c.y).abs()) < rule.min_spacing) {
                    continue;
                }
                kept.push(c);
                out.push(Deco {
                    kind: *kind,
                    cell: c,
                    offset: [unit(h, 1) - 0.5, unit(h, 2) - 0.5],
                    size_m: lerp(rule.size_m, unit(h, 3)),
                    yaw: unit(h, 4) * std::f32::consts::TAU,
                    color: rule.color,
                    floor_y: inf.height_at(c),
                    phase: unit(h, 5),
                });
            }
        }
        out
    }
}

fn parse_rule(name: &str, v: &serde_json::Value) -> Result<KindRule, String> {
    let num = |key: &str, default: Option<f32>| -> Result<f32, String> {
        match v.get(key) {
            Some(x) => x
                .as_f64()
                .filter(|n| *n >= 0.0)
                .map(|n| n as f32)
                .ok_or_else(|| format!("{name}.{key}: expected a number, zero or more")),
            None => default.ok_or_else(|| format!("{name}.{key}: missing")),
        }
    };
    let pair = |key: &str, required: bool| -> Result<[f32; 2], String> {
        match v.get(key).and_then(|a| a.as_array()) {
            Some(a) if a.len() == 2 => {
                let lo = a[0].as_f64().filter(|n| *n >= 0.0);
                let hi = a[1].as_f64().filter(|n| *n >= 0.0);
                match (lo, hi) {
                    (Some(lo), Some(hi)) if lo <= hi => Ok([lo as f32, hi as f32]),
                    _ => Err(format!("{name}.{key}: expected [low, high], zero or more")),
                }
            }
            Some(_) => Err(format!("{name}.{key}: expected [low, high]")),
            None if required => Err(format!("{name}.{key}: missing")),
            None => Ok([0.0, 0.0]),
        }
    };
    let tiers = v
        .get("tiers")
        .and_then(|t| t.as_array())
        .ok_or_else(|| format!("{name}.tiers: missing"))?
        .iter()
        .map(|t| match t.as_str() {
            Some("ledge") => Ok(Tier::Ledge),
            Some("slope") => Ok(Tier::Slope),
            Some("basin") => Ok(Tier::Basin),
            _ => Err(format!("{name}.tiers: expected ledge, slope or basin")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let color = v
        .get("color")
        .and_then(|c| c.as_array())
        .filter(|c| c.len() == 3)
        .and_then(|c| {
            let ch: Vec<f32> = c
                .iter()
                .filter_map(|x| x.as_f64().filter(|n| (0.0..=1.0).contains(n)).map(|n| n as f32))
                .collect();
            (ch.len() == 3).then(|| [ch[0], ch[1], ch[2]])
        })
        .ok_or_else(|| format!("{name}.color: expected [r, g, b], each 0 to 1"))?;
    let chance = num("chance", None)?;
    if chance > 1.0 {
        return Err(format!("{name}.chance: expected 0 to 1"));
    }
    Ok(KindRule {
        tiers,
        chance,
        min_spacing: num("min_spacing", None)? as i32,
        max_per_chunk: num("max_per_chunk", None)? as usize,
        size_m: pair("size_m", true)?,
        color,
        glow_radius_m: num("glow_radius_m", Some(0.0))?,
        glow_strength: num("glow_strength", Some(0.0))?,
        flicker_hz: num("flicker_hz", Some(0.0))?,
        rise_speed_mps: pair("rise_speed_mps", false)?,
        life_s: pair("life_s", false)?,
    })
}

/// The decoration rules compiled into the executable. A table that fails to parse is reported on stderr and
/// nothing is decorated.
pub fn inferno_deco() -> &'static InfernoDeco {
    static DECO: OnceLock<InfernoDeco> = OnceLock::new();
    DECO.get_or_init(|| {
        let text = crate::content::embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        InfernoDeco::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            InfernoDeco { rules: Vec::new() }
        })
    })
}

/// Flicker multiplier for a glow at time `t_s`: 1 plus or minus a fifth.
pub fn flicker(t_s: f32, phase: f32, hz: f32) -> f32 {
    (1.0 + 0.2 * ((t_s * hz + phase) * std::f32::consts::TAU).sin()).max(0.0)
}

fn lerp(r: [f32; 2], t: f32) -> f32 {
    r[0] + (r[1] - r[0]) * t
}

/// FNV-1a over the seed text, the chunk, the cell and the kind.
fn hash(seed: &str, key: ChunkKey, c: Cell, kind: i32) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    };
    seed.bytes().for_each(&mut eat);
    for v in [key.cx, key.cy, key.level, c.x, c.y, kind] {
        v.to_le_bytes().into_iter().for_each(&mut eat);
    }
    h
}

/// The n-th independent 0..1 value drawn from a hash.
fn unit(h: u64, n: u64) -> f32 {
    let mut z = h.wrapping_add(n.wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::INFERNO_LEVEL;

    const SEED: &str = "deco-seed";

    fn key(cx: i32, cy: i32) -> ChunkKey {
        ChunkKey { cx, cy, level: INFERNO_LEVEL }
    }

    fn placed(deco: &InfernoDeco, cx: i32, cy: i32) -> (Inferno, Vec<Deco>) {
        let inf = Inferno::carve(SEED, key(cx, cy));
        let d = deco.place(SEED, key(cx, cy), &inf);
        (inf, d)
    }

    #[test]
    fn standard_data_parses_and_places_every_kind_somewhere() {
        let deco = inferno_deco();
        assert_eq!(deco.rules.len(), KINDS.len());
        let mut seen = std::collections::HashSet::new();
        for cx in 0..4 {
            for cy in 0..4 {
                for d in placed(deco, cx, cy).1 {
                    seen.insert(d.kind as i32);
                }
            }
        }
        assert_eq!(seen.len(), KINDS.len(), "every kind appears across a few chunks");
    }

    #[test]
    fn placement_is_deterministic_and_stays_on_open_cells() {
        let deco = inferno_deco();
        let (inf, a) = placed(deco, 1, 2);
        let (_, b) = placed(deco, 1, 2);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!((x.kind, x.cell, x.size_m), (y.kind, y.cell, y.size_m));
        }
        for d in &a {
            assert!(inf.is_open(d.cell));
            assert!(d.cell.x < CHUNK && d.cell.y < CHUNK);
        }
    }

    #[test]
    fn rules_hold_tier_spacing_count_and_size() {
        let deco = inferno_deco();
        let (inf, all) = placed(deco, 0, 0);
        for (kind, rule) in &deco.rules {
            let mine: Vec<&Deco> = all.iter().filter(|d| d.kind == *kind).collect();
            assert!(mine.len() <= rule.max_per_chunk);
            for d in &mine {
                assert!(rule.tiers.contains(&inf.tier_at(d.cell)));
                assert!(d.size_m >= rule.size_m[0] && d.size_m <= rule.size_m[1]);
                assert_eq!(d.color, rule.color);
            }
            for (i, a) in mine.iter().enumerate() {
                for b in &mine[i + 1..] {
                    assert!((a.cell.x - b.cell.x).abs().max((a.cell.y - b.cell.y).abs()) >= rule.min_spacing);
                }
            }
        }
    }

    #[test]
    fn lava_glow_sits_in_the_basin_and_stalagmites_stay_out_of_it() {
        let deco = inferno_deco();
        let (inf, all) = placed(deco, 0, 1);
        for d in all {
            match d.kind {
                DecoKind::LavaGlow => assert_eq!(inf.tier_at(d.cell), Tier::Basin),
                DecoKind::Stalagmite => assert_ne!(inf.tier_at(d.cell), Tier::Basin),
                _ => {}
            }
        }
    }

    #[test]
    fn changing_the_data_changes_the_placement() {
        // CRLF checkouts (Windows CI) must match the same text, so compare on LF.
        let text = include_str!("../data/render/inferno.json").replace("\r\n", "\n");
        let off_text = text.replacen("\"chance\": 0.05,\n      \"min_spacing\": 2", "\"chance\": 0.0,\n      \"min_spacing\": 2", 1);
        assert_ne!(off_text, text);
        let off = InfernoDeco::parse(&off_text).unwrap();
        let on = InfernoDeco::parse(&text).unwrap();
        let inf = Inferno::carve(SEED, key(0, 0));
        let count = |d: &InfernoDeco| d.place(SEED, key(0, 0), &inf).iter().filter(|x| x.kind == DecoKind::Stalactite).count();
        assert_eq!(count(&off), 0);
        assert!(count(&on) > 0);
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(InfernoDeco::parse("{}").is_err());
        assert!(InfernoDeco::parse("[1]").is_err());
        let text = include_str!("../data/render/inferno.json");
        assert!(InfernoDeco::parse(&text.replacen("[\"basin\"]", "[\"moon\"]", 1)).is_err());
        assert!(InfernoDeco::parse(&text.replacen("[1.0, 0.42, 0.08]", "[2.0, 0.4, 0.1]", 1)).is_err());
        assert!(InfernoDeco::parse(&text.replacen("[1.2, 2.4]", "[3.0, 1.0]", 1)).is_err());
    }

    #[test]
    fn flicker_stays_near_one() {
        for i in 0..50 {
            let f = flicker(i as f32 * 0.1, 0.3, 1.3);
            assert!((0.8..=1.2).contains(&f));
        }
    }
}
