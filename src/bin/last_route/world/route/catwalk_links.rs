//! Stairs and ladders from the level 0 floor up to the catwalk decks and back down (`data/regions/catwalk_links.json`,
//! `lr.catwalk_links/1`, embedded by build.rs). They are direct connections like every other level link: no fade and
//! no halt. The landing offset and the prefetch distance are not repeated here, they come from
//! `data/stairs_links.json` (`game::transition::links`), so a link is arrived at and prefetched the same way as a stair.
//!
//! Where links appear and how many are data: a chunk with a deck draws once from `"<world>:CATWALK-LINKS:<cx>:<cy>"`
//! (a pure function of the world text and the chunk), takes `chance`, then places between `min_per_chunk` and
//! `per_chunk` links on the ground cells the caller offers as `candidates` (clear cells beside a deck), at least
//! `spacing` cells apart. Which cells are offered is the world's business; this module only chooses among them.
//!
//! These files are not `lr.region/1`, so the region catalogue does not read them (see maze_regions.rs).

use crate::content::embedded;
use crate::world::facility::Cell;
use crate::world::infinite::ChunkKey;
use crate::world::rng::Rng;
use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.catwalk_links/1";
pub const PATH: &str = "regions/catwalk_links.json";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatwalkRules {
    pub chance: f64,
    pub min_per_chunk: usize,
    pub per_chunk: usize,
    pub spacing: i32,
    pub ladder_share: f64,
    pub ladder_rise: f32,
    pub deck_height: f32,
    /// Half the side (m) of the footprint that must be free of solid props on a link's ground cell and the deck cell it
    /// steps onto (`Facility::apply_catwalks`): which catwalks survive, so it changes the fixed-seed output.
    pub link_clearance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatwalkKind {
    Stairs,
    Ladder,
}

/// One connection: the ground cell it starts from, and the deck it reaches. Taking it goes up; the deck end goes down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatwalkLink {
    pub cell: Cell,
    pub kind: CatwalkKind,
    pub deck_height: f32,
    /// The unit step from the ground `cell` to the adjacent deck cell it reaches (the deck landing). `links_for` leaves
    /// it `(0, 0)`: `Facility::apply_catwalks` fills it in once it knows which deck cells are kept.
    pub run: (i32, i32),
}

fn field(v: &Value, key: &str, min: f64, max: f64) -> Result<f64, String> {
    match v.get(key).and_then(Value::as_f64) {
        Some(n) if n.is_finite() && n >= min && n <= max => Ok(n),
        _ => Err(format!("catwalk_links: \"{key}\" must be a number from {min} to {max}")),
    }
}

pub fn parse(text: &str) -> Result<CatwalkRules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("catwalk_links: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("catwalk_links: \"schema\" must be \"{SCHEMA}\""));
    }
    let r = CatwalkRules {
        chance: field(&v, "chance", 0.0, 1.0)?,
        min_per_chunk: field(&v, "min_per_chunk", 0.0, 16.0)? as usize,
        per_chunk: field(&v, "per_chunk", 0.0, 16.0)? as usize,
        spacing: field(&v, "spacing", 0.0, 45.0)? as i32,
        ladder_share: field(&v, "ladder_share", 0.0, 1.0)?,
        ladder_rise: field(&v, "ladder_rise", 0.0, 100.0)? as f32,
        deck_height: field(&v, "deck_height", 0.1, 100.0)? as f32,
        link_clearance: field(&v, "link_clearance", 0.0, 0.5)?,
    };
    if r.min_per_chunk > r.per_chunk {
        return Err("catwalk_links: \"min_per_chunk\" must not exceed \"per_chunk\"".into());
    }
    Ok(r)
}

/// The rules from the embedded file, loaded once.
pub fn rules() -> CatwalkRules {
    static RULES: OnceLock<CatwalkRules> = OnceLock::new();
    *RULES.get_or_init(|| {
        let text = embedded::data_under("regions/")
            .find(|(p, _)| *p == PATH)
            .map(|(_, t)| t)
            .expect("data/regions/catwalk_links.json is embedded by build.rs");
        parse(text).expect("data/regions/catwalk_links.json is valid")
    })
}

/// The links of chunk `k` (level 0 only) over the offered ground `candidates`, for a deck `deck_height` metres up
/// (`None` takes the data's default). Empty when the chunk has no candidates, misses the chance, or is not level 0.
pub fn links_for(
    seed: &str,
    k: ChunkKey,
    candidates: &[Cell],
    deck_height: Option<f32>,
    rules: &CatwalkRules,
) -> Vec<CatwalkLink> {
    if k.level != 0 || candidates.is_empty() || rules.per_chunk == 0 {
        return vec![];
    }
    let mut rng = Rng::new(&format!("{seed}:CATWALK-LINKS:{}:{}", k.cx, k.cy));
    if rng.next_float() >= rules.chance {
        return vec![];
    }
    let height = deck_height.unwrap_or(rules.deck_height);
    let want = rules.min_per_chunk + rng.next_index(rules.per_chunk - rules.min_per_chunk + 1);
    let mut pool: Vec<Cell> = candidates.to_vec();
    pool.sort();
    pool.dedup();
    let mut out: Vec<CatwalkLink> = Vec::new();
    while out.len() < want && !pool.is_empty() {
        let c = pool.remove(rng.next_index(pool.len()));
        if out.iter().any(|l| (l.cell.x - c.x).abs().max((l.cell.y - c.y).abs()) < rules.spacing) {
            continue;
        }
        // always draw, so the stream does not depend on the deck's height
        let wants_ladder = rng.next_float() < rules.ladder_share;
        let kind = if wants_ladder && height <= rules.ladder_rise { CatwalkKind::Ladder } else { CatwalkKind::Stairs };
        out.push(CatwalkLink { cell: c, kind, deck_height: height, run: (0, 0) });
    }
    out
}

/// The level 0 regions whose chunks get catwalk decks (the file's `regions`), loaded once.
pub fn allowed_regions() -> &'static [String] {
    static REGIONS: OnceLock<Vec<String>> = OnceLock::new();
    REGIONS.get_or_init(|| {
        let text = embedded::data_under("regions/")
            .find(|(p, _)| *p == PATH)
            .map(|(_, t)| t)
            .expect("data/regions/catwalk_links.json is embedded by build.rs");
        parse_regions(text).expect("data/regions/catwalk_links.json has regions")
    })
}

/// The `regions` list of a catwalk_links file: an array of region ids.
pub fn parse_regions(text: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("catwalk_links: {e}"))?;
    let arr = v
        .get("regions")
        .and_then(Value::as_array)
        .ok_or("catwalk_links: \"regions\" must be an array of region ids")?;
    arr.iter()
        .map(|s| s.as_str().map(str::to_string).ok_or_else(|| "catwalk_links: \"regions\" must hold strings".to_string()))
        .collect()
}

/// Whether a chunk of `region` gets catwalk decks.
pub fn allowed_in(region: &str) -> bool {
    allowed_regions().iter().any(|r| r == region)
}

/// How close (cells) the runner gets to a catwalk link before the storey it leads to is built ahead: the stair data's.
pub fn prefetch_distance() -> f32 {
    crate::game::transition::links().prefetch_distance
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = include_str!("../../../../../data/regions/catwalk_links.json");

    fn std_rules() -> CatwalkRules {
        parse(FILE).unwrap()
    }

    fn grid() -> Vec<Cell> {
        (0..40).step_by(3).flat_map(|x| (0..40).step_by(3).map(move |y| Cell::new(x, y))).collect()
    }

    fn key(cx: i32) -> ChunkKey {
        ChunkKey { cx, cy: 0, level: 0 }
    }

    #[test]
    fn the_embedded_file_parses_and_matches_the_standard_values() {
        let r = rules();
        assert_eq!(r, std_rules());
        assert!(r.chance > 0.0 && r.per_chunk >= 1 && r.min_per_chunk <= r.per_chunk);
    }

    #[test]
    fn a_bad_file_is_refused() {
        assert!(parse("{}").is_err());
        assert!(parse(&FILE.replace("\"chance\": 0.6", "\"chance\": 2")).is_err());
        assert!(parse(&FILE.replace("\"min_per_chunk\": 1", "\"min_per_chunk\": 3")).is_err());
    }

    #[test]
    fn links_are_deterministic_within_count_and_spacing() {
        let r = std_rules();
        let mut any = false;
        for cx in -20..20 {
            let a = links_for("1426541270", key(cx), &grid(), None, &r);
            assert_eq!(a, links_for("1426541270", key(cx), &grid(), None, &r));
            assert!(a.len() <= r.per_chunk);
            any |= !a.is_empty();
            for (i, l) in a.iter().enumerate() {
                assert!(grid().contains(&l.cell));
                for m in &a[i + 1..] {
                    assert!((l.cell.x - m.cell.x).abs().max((l.cell.y - m.cell.y).abs()) >= r.spacing);
                }
            }
        }
        assert!(any);
    }

    #[test]
    fn chance_zero_gives_none_and_chance_one_always_gives_some() {
        let mut r = std_rules();
        r.chance = 0.0;
        assert!((0..30).all(|cx| links_for("w", key(cx), &grid(), None, &r).is_empty()));
        r.chance = 1.0;
        assert!((0..30).all(|cx| !links_for("w", key(cx), &grid(), None, &r).is_empty()));
    }

    #[test]
    fn per_chunk_comes_from_data() {
        let mut r = std_rules();
        r.chance = 1.0;
        r.min_per_chunk = 3;
        r.per_chunk = 3;
        r.spacing = 0;
        assert!((0..10).all(|cx| links_for("w", key(cx), &grid(), None, &r).len() == 3));
        r.per_chunk = 0;
        r.min_per_chunk = 0;
        assert!(links_for("w", key(0), &grid(), None, &r).is_empty());
    }

    #[test]
    fn ladders_follow_the_share_and_only_serve_low_decks() {
        let mut r = std_rules();
        r.chance = 1.0;
        r.min_per_chunk = 2;
        r.per_chunk = 2;
        r.spacing = 0;
        let kinds = |r: &CatwalkRules, h: f32| -> Vec<CatwalkKind> {
            (0..40).flat_map(|cx| links_for("w", key(cx), &grid(), Some(h), r)).map(|l| l.kind).collect()
        };
        r.ladder_share = 1.0;
        assert!(kinds(&r, 3.0).iter().all(|k| *k == CatwalkKind::Ladder));
        assert!(kinds(&r, r.ladder_rise + 1.0).iter().all(|k| *k == CatwalkKind::Stairs));
        r.ladder_share = 0.0;
        assert!(kinds(&r, 3.0).iter().all(|k| *k == CatwalkKind::Stairs));
    }

    #[test]
    fn only_level_zero_and_offered_cells_get_links() {
        let mut r = std_rules();
        r.chance = 1.0;
        assert!(links_for("w", ChunkKey { cx: 0, cy: 0, level: -1 }, &grid(), None, &r).is_empty());
        assert!(links_for("w", key(0), &[], None, &r).is_empty());
    }

    #[test]
    fn a_link_leaves_its_deck_step_to_the_facility() {
        // links_for knows only the ground cells: the step onto the deck is filled in by Facility::apply_catwalks
        let mut r = std_rules();
        r.chance = 1.0;
        let links = links_for("w", key(0), &grid(), None, &r);
        assert!(!links.is_empty());
        assert!(links.iter().all(|l| l.run == (0, 0)));
    }

    #[test]
    fn prefetch_uses_the_stair_data() {
        let s = crate::game::transition::links();
        assert_eq!(prefetch_distance(), s.prefetch_distance);
    }

    #[test]
    fn the_regions_list_comes_from_data() {
        assert!(allowed_in("maze"));
        assert!(!allowed_in("mansion"));
        assert!(parse_regions(&FILE.replace("[\"maze\"]", "[\"mall\", \"maze\"]")).unwrap().contains(&"mall".to_string()));
        assert!(parse_regions(&FILE.replace("\"regions\"", "\"x\"")).is_err());
    }
}
