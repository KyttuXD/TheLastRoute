//! The Inferno's secret creatures, the white and the black wolf spider (docs/SECRET_WOLF_SPIDER.md): their ids, the numbers of
//! their behaviour and presentation, and the plan that says who comes for a route (config/secret_creatures.gd).
//!
//! They are hunters (data/hunters/white_wolf_spider.json, black_wolf_spider.json, category "secret") but stay out of every list
//! that shows or draws the roster: `is_secret` is the one test. Everything tunable is in data/hunters/secret_creatures.json
//! (`lr.secret_creatures/1`), parsed once by [`Rules::get`].

use crate::content::embedded;
use crate::world::rng::Rng;
use serde_json::Value;
use std::sync::OnceLock;

pub const WHITE: &str = "white_wolf_spider";
pub const BLACK: &str = "black_wolf_spider";
pub const IDS: [&str; 2] = [WHITE, BLACK];
/// The admin picker's entry that summons both, linked; never a real hunter id.
pub const PAIR_ID: &str = "wolf_spider_pair";
/// The data file (content/mod.rs skips it when it reads hunter definitions).
pub const SECRET_PATH: &str = "hunters/secret_creatures.json";
pub const SCHEMA: &str = "lr.secret_creatures/1";

/// A secret creature id (not the pair entry).
pub fn is_secret(id: &str) -> bool {
    IDS.contains(&id)
}

/// A secret creature must never leave its home level: the stack's ramp, ladder and drop edges are not planned for it.
pub fn stays_home(id: &str) -> bool {
    is_secret(id)
}

/// The admin picker's extra entries: the two creatures and the pair.
pub fn admin_ids() -> [&'static str; 3] {
    [WHITE, BLACK, PAIR_ID]
}

/// The picker label of the pair entry.
pub const PAIR_LABEL: &str = "WOLF SPIDER PAIR (WHITE + BLACK)";

/// One variant's behaviour numbers (`behaviour.white` / `behaviour.black`).
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    /// Cells: a runner this close that it can see is marked.
    pub alert_radius: f32,
    pub bark_seconds: f32,
    pub hunt_seconds: f32,
    /// After a retreat it ignores runners for this long.
    pub cooldown_seconds: f32,
    pub hunt_speed_scale: f32,
    /// Seconds of flashlight beam on it that drive it off; None = it ignores the light.
    pub flash_hold: Option<f32>,
    /// The tell is a hiss (longer, lower) instead of a bark.
    pub hiss: bool,
}

/// The per-difficulty multipliers on a creature's own numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs {
    /// Sight range.
    pub notice: f32,
    /// Chase speed.
    pub chase: f32,
    /// The rest after a hunt.
    pub rest: f32,
    /// The alert radius.
    pub alert: f32,
}

impl Default for Knobs {
    fn default() -> Knobs {
        Knobs { notice: 1.0, chase: 1.0, rest: 1.0, alert: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Always the white one alone.
    WhiteSolo,
    /// `pair` chance of the pair (spawned together); otherwise one, the black with chance `black_solo`.
    SoloOrPair,
    /// `pair` chance of the pair where the black one joins after the white one has noticed the runner.
    PairLikely,
    /// Always the pair, spawned together and flanking.
    PairAlways,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DifficultyRow {
    pub mode: Mode,
    pub pair: f32,
    pub black_solo: f32,
    pub join_delay: f32,
    pub stagger: f32,
    pub knobs: Knobs,
}

/// Presentation numbers (creature_view.gd).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewRules {
    pub attack_range: f32,
    pub attack_cooldown: f32,
    pub moving_speed: f32,
    pub bark_cue_radius: f32,
    pub skitter_radius: f32,
    pub lair_radius: f32,
    pub skitter_every_walk: f32,
    pub skitter_every_run: f32,
    pub crackle_every: (f32, f32),
}

#[derive(Clone, Debug)]
pub struct Rules {
    pub home_level: i32,
    /// "inferno" or "off".
    pub trigger: String,
    /// "difficulty", "pair", "black_rare" or "black_follows_white".
    pub relationship: String,
    pub black_chance: f32,
    pub dwell_seconds: f32,
    pub spawn_min_distance: f32,
    pub spawn_max_distance: f32,
    pub discover_range: f32,
    pub discover_every: f32,
    pub clearance_radius: i32,
    pub wide_clearance: f32,
    pub roomy_clearance: f32,
    pub lair_samples: usize,
    pub pair_min_gap: f32,
    pub pair_flank_degrees: f32,
    pub pair_stagger: f32,
    pub body_radius: f32,
    pub body_height: f32,
    pub hear_radius_scale: f32,
    pub retreat_speed_scale: f32,
    pub lair_radius: f32,
    pub sense_every: f32,
    pub white: Variant,
    pub black: Variant,
    pub view: ViewRules,
    pub difficulty: Vec<(String, DifficultyRow)>,
    pub dossier: Vec<(String, String)>,
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    v[key].as_f64().map(|x| x as f32).filter(|x| x.is_finite()).ok_or_else(|| format!("secret creatures: `{key}` must be a number"))
}

fn variant(v: &Value, who: &str) -> Result<Variant, String> {
    let row = &v["behaviour"][who];
    let flash = &row["flash_hold"];
    Ok(Variant {
        alert_radius: num(row, "alert_radius")?,
        bark_seconds: num(row, "bark_seconds")?,
        hunt_seconds: num(row, "hunt_seconds")?,
        cooldown_seconds: num(row, "cooldown_seconds")?,
        hunt_speed_scale: num(row, "hunt_speed_scale")?,
        flash_hold: if flash.is_null() { None } else { Some(num(row, "flash_hold")?) },
        hiss: match row["tell"].as_str() {
            Some("bark") => false,
            Some("hiss") => true,
            _ => return Err(format!("secret creatures: {who}.tell must be \"bark\" or \"hiss\"")),
        },
    })
}

fn knobs(v: &Value) -> Result<Knobs, String> {
    Ok(Knobs { notice: num(v, "notice")?, chase: num(v, "chase")?, rest: num(v, "rest")?, alert: num(v, "alert")? })
}

impl Rules {
    pub fn parse(v: &Value) -> Result<Rules, String> {
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("secret creatures: schema must be {SCHEMA}"));
        }
        let text = |k: &str, allowed: &[&str]| -> Result<String, String> {
            v[k].as_str().filter(|s| allowed.contains(s)).map(str::to_string).ok_or_else(|| format!("secret creatures: `{k}` must be one of {allowed:?}"))
        };
        let view = &v["view"];
        let crackle = view["crackle_every"].as_array().filter(|a| a.len() == 2).ok_or("secret creatures: view.crackle_every must be [min, max]")?;
        let bound = |i: usize| crackle[i].as_f64().map(|x| x as f32).ok_or("secret creatures: view.crackle_every must be numbers");
        let mut difficulty = vec![];
        for (id, row) in v["difficulty"].as_object().ok_or("secret creatures: `difficulty` must be an object")? {
            let mode = match row["mode"].as_str() {
                Some("white_solo") => Mode::WhiteSolo,
                Some("solo_or_pair") => Mode::SoloOrPair,
                Some("pair_likely") => Mode::PairLikely,
                Some("pair_always") => Mode::PairAlways,
                _ => return Err(format!("secret creatures: difficulty.{id}.mode is not a mode")),
            };
            difficulty.push((
                id.clone(),
                DifficultyRow {
                    mode,
                    pair: num(row, "pair")?,
                    black_solo: num(row, "black_solo")?,
                    join_delay: num(row, "join_delay")?,
                    stagger: num(row, "stagger")?,
                    knobs: knobs(&row["knobs"])?,
                },
            ));
        }
        if !difficulty.iter().any(|(id, _)| id == "standard") {
            return Err("secret creatures: difficulty needs a `standard` row (the fallback)".into());
        }
        let dossier = v["dossier"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, t)| Some((k.clone(), t.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        Ok(Rules {
            home_level: v["home_level"].as_i64().and_then(|l| i32::try_from(l).ok()).ok_or("secret creatures: `home_level` must be an integer")?,
            trigger: text("trigger", &["inferno", "off"])?,
            relationship: text("relationship", &["difficulty", "pair", "black_rare", "black_follows_white"])?,
            black_chance: num(v, "black_chance")?,
            dwell_seconds: num(v, "dwell_seconds")?,
            spawn_min_distance: num(v, "spawn_min_distance")?,
            spawn_max_distance: num(v, "spawn_max_distance")?,
            discover_range: num(v, "discover_range")?,
            discover_every: num(v, "discover_every")?,
            clearance_radius: v["clearance_radius"].as_i64().and_then(|r| i32::try_from(r).ok()).ok_or("secret creatures: `clearance_radius` must be an integer")?,
            wide_clearance: num(v, "wide_clearance")?,
            roomy_clearance: num(v, "roomy_clearance")?,
            lair_samples: v["lair_samples"].as_u64().ok_or("secret creatures: `lair_samples` must be an integer")? as usize,
            pair_min_gap: num(v, "pair_min_gap")?,
            pair_flank_degrees: num(v, "pair_flank_degrees")?,
            pair_stagger: num(v, "pair_stagger")?,
            body_radius: num(&v["body"], "radius")?,
            body_height: num(&v["body"], "height")?,
            hear_radius_scale: num(&v["behaviour"], "hear_radius_scale")?,
            retreat_speed_scale: num(&v["behaviour"], "retreat_speed_scale")?,
            lair_radius: num(&v["behaviour"], "lair_radius")?,
            sense_every: num(&v["behaviour"], "sense_every")?,
            white: variant(v, "white")?,
            black: variant(v, "black")?,
            view: ViewRules {
                attack_range: num(view, "attack_range")?,
                attack_cooldown: num(view, "attack_cooldown")?,
                moving_speed: num(view, "moving_speed")?,
                bark_cue_radius: num(view, "bark_cue_radius")?,
                skitter_radius: num(view, "skitter_radius")?,
                lair_radius: num(view, "lair_radius")?,
                skitter_every_walk: num(view, "skitter_every_walk")?,
                skitter_every_run: num(view, "skitter_every_run")?,
                crackle_every: (bound(0)?, bound(1)?),
            },
            difficulty,
            dossier,
        })
    }

    /// The embedded data/hunters/secret_creatures.json, read once.
    pub fn get() -> &'static Rules {
        static RULES: OnceLock<Rules> = OnceLock::new();
        RULES.get_or_init(|| {
            let (_, text) = embedded::data_under(SECRET_PATH).next().expect("data/hunters/secret_creatures.json is embedded");
            let v: Value = serde_json::from_str(text).expect("data/hunters/secret_creatures.json is JSON");
            Rules::parse(&v).unwrap_or_else(|e| panic!("{e}"))
        })
    }

    pub fn variant_of(&self, id: &str) -> &Variant {
        if id == BLACK {
            &self.black
        } else {
            &self.white
        }
    }

    /// The row of a difficulty id; an unknown id reads as `standard`.
    pub fn row(&self, difficulty_id: &str) -> &DifficultyRow {
        let find = |want: &str| self.difficulty.iter().find(|(id, _)| id == want).map(|(_, r)| r);
        find(difficulty_id).or_else(|| find("standard")).expect("the standard row exists")
    }

    /// The dossier line of a creature, shown only once it has been met.
    pub fn dossier_text(&self, id: &str) -> Option<&str> {
        self.dossier.iter().find(|(k, _)| k == id).map(|(_, t)| t.as_str())
    }
}

/// Who comes for a route, how they join and the knobs (config/secret_creatures.gd `plan_for`).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub ids: Vec<&'static str>,
    pub knobs: Knobs,
    pub stagger: f32,
    /// The second creature joins after the first has barked (`join_delay` s later) instead of appearing with it.
    pub join_after_bark: bool,
    pub join_delay: f32,
    /// The pair comes from opposite sides.
    pub flank: bool,
}

impl Plan {
    pub fn none() -> Plan {
        Plan { ids: vec![], knobs: Knobs::default(), stagger: 0.0, join_after_bark: false, join_delay: 0.0, flank: false }
    }
}

/// Deterministic from the route seed and the difficulty, so every peer agrees and a restart repeats it: the draws are
/// `"<seed>:SECRET-PLAN:<difficulty>"`, pair roll first, black roll second (the same stream as the original).
pub fn plan_for(rules: &Rules, difficulty_id: &str, seed: &str) -> Plan {
    let row = rules.row(difficulty_id);
    let mut rng = Rng::new(&format!("{seed}:SECRET-PLAN:{difficulty_id}"));
    let roll_pair = rng.next_float() as f32;
    let roll_black = rng.next_float() as f32;
    let pair = row.mode == Mode::PairAlways || (matches!(row.mode, Mode::SoloOrPair | Mode::PairLikely) && roll_pair < row.pair);
    if pair {
        return Plan {
            ids: vec![WHITE, BLACK],
            knobs: row.knobs,
            stagger: row.stagger,
            join_after_bark: row.mode == Mode::PairLikely,
            join_delay: row.join_delay,
            flank: matches!(row.mode, Mode::PairAlways | Mode::PairLikely),
        };
    }
    let solo_black = matches!(row.mode, Mode::SoloOrPair | Mode::PairLikely) && roll_black < row.black_solo;
    Plan {
        ids: vec![if solo_black { BLACK } else { WHITE }],
        knobs: row.knobs,
        stagger: 0.0,
        join_after_bark: false,
        join_delay: 0.0,
        flank: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_file_loads_and_matches_the_original_numbers() {
        let r = Rules::get();
        assert_eq!(r.home_level, -3);
        assert_eq!(r.trigger, "inferno");
        assert_eq!(r.relationship, "difficulty");
        assert_eq!((r.white.alert_radius, r.white.bark_seconds, r.white.hunt_seconds, r.white.cooldown_seconds), (15.0, 1.25, 7.0, 30.0));
        assert_eq!((r.black.alert_radius, r.black.bark_seconds, r.black.hunt_seconds, r.black.cooldown_seconds), (19.0, 1.1, 9.0, 60.0));
        assert_eq!(r.white.flash_hold, Some(0.0));
        assert_eq!(r.black.flash_hold, None, "the black one ignores the flashlight");
        assert!(!r.white.hiss && r.black.hiss);
        assert_eq!(r.dwell_seconds, 30.0);
        assert_eq!((r.spawn_min_distance, r.spawn_max_distance), (16.0, 23.0));
    }

    #[test]
    fn the_table_has_exactly_the_games_difficulty_ids() {
        let r = Rules::get();
        let mut ids: Vec<&str> = r.difficulty.iter().map(|(id, _)| id.as_str()).collect();
        // nightmare is not selectable in the menu yet, its row is ready (config/difficulty.rs `ALL` lists the selectable ones)
        let mut want: Vec<&str> = crate::config::difficulty::ALL.iter().map(|d| d.id()).chain(["nightmare"]).collect();
        want.dedup();
        ids.sort_unstable();
        want.sort_unstable();
        assert_eq!(ids, want, "a difficulty was added or removed without a row");
    }

    #[test]
    fn plans_follow_the_difficulty_and_repeat() {
        let r = Rules::get();
        for seed in 0..40 {
            let seed = format!("ROUTE-{seed}");
            let relaxed = plan_for(r, "relaxed", &seed);
            assert_eq!(relaxed.ids, vec![WHITE], "relaxed brings the white one alone");
            let nightmare = plan_for(r, "nightmare", &seed);
            assert_eq!(nightmare.ids, vec![WHITE, BLACK], "nightmare always brings the pair");
            assert!(nightmare.flank && !nightmare.join_after_bark);
            assert_eq!(plan_for(r, "hard", &seed), plan_for(r, "hard", &seed), "the same seed repeats");
            let hard = plan_for(r, "hard", &seed);
            if hard.ids.len() == 2 {
                assert!(hard.join_after_bark && hard.join_delay == 3.0);
            }
        }
        let pairs = |id: &str| (0..400).filter(|n| plan_for(r, id, &format!("S{n}")).ids.len() == 2).count();
        assert!(pairs("standard") < pairs("hard"), "a pair is likelier on hard");
        assert!(pairs("hard") < 400 && pairs("standard") > 0);
    }

    #[test]
    fn an_unknown_difficulty_reads_as_standard() {
        let r = Rules::get();
        assert_eq!(r.row("nope"), r.row("standard"));
    }

    #[test]
    fn a_bad_file_is_refused() {
        let good: Value = serde_json::from_str(embedded::data_under(SECRET_PATH).next().unwrap().1).unwrap();
        let mut v = good.clone();
        v["trigger"] = Value::from("sometimes");
        assert!(Rules::parse(&v).is_err());
        let mut v = good.clone();
        v["behaviour"]["white"]["tell"] = Value::from("roar");
        assert!(Rules::parse(&v).is_err());
        let mut v = good;
        v["schema"] = Value::from("lr.other/1");
        assert!(Rules::parse(&v).is_err());
    }

    #[test]
    fn only_the_two_creatures_are_secret() {
        assert!(is_secret(WHITE) && is_secret(BLACK));
        assert!(!is_secret(PAIR_ID) && !is_secret("aki"));
        assert!(stays_home(BLACK) && !stays_home("nyxie"));
    }
}

#[cfg(test)]
mod secrecy_tests {
    use super::*;
    use crate::content::Content;

    /// Files that may name the creatures: their own data, models, rigs and grab pools, the per-hunter rule tables (peek) and the death
    /// lines (which are keyed by id).
    fn may_name_them(path: &str) -> bool {
        path.contains("wolf_spider") || path == SECRET_PATH || path == "peek.json" || path == "ui/death_lines.json"
    }

    fn names_them(text: &str) -> bool {
        let t = text.to_lowercase();
        ["wolf spider", "wolf_spider", "wolfspider", "spider dog"].iter().any(|k| t.contains(k))
    }

    /// Nothing player-facing says they exist: no tip, story line, menu text or other data file mentions them.
    #[test]
    fn no_other_file_names_the_creatures() {
        for (path, text) in embedded::data_under("") {
            assert!(may_name_them(path) || !names_them(text), "data/{path} names a secret creature");
        }
        for (path, bytes) in embedded::assets_under("") {
            let textual = path.ends_with(".json") || path.ends_with(".txt") || path.ends_with(".md");
            if !textual || may_name_them(path) {
                continue;
            }
            if let Ok(text) = std::str::from_utf8(bytes) {
                assert!(!names_them(text), "assets/{path} names a secret creature");
            }
        }
    }

    #[test]
    fn they_are_hunters_but_never_in_the_draw_pool_or_the_roster_lists() {
        let content = Content::load();
        for id in IDS {
            let def = content.hunter(id).expect("the hunter definition loads");
            assert_eq!(def.category, "secret");
            assert!(!def.is_pack());
            assert!(!crate::content::hunters::IMPLEMENTED_IDS.contains(&id) && !crate::content::hunters::SPECIAL_IDS.contains(&id));
            assert!(!crate::hunters::residency::A_Z_IDS.contains(&id) && !crate::hunters::residency::VOID_IDS.contains(&id));
            let index = content.hunter_index(id).unwrap();
            assert!(!content.draw_pool().contains(&index), "{id} is in the per-floor draw");
        }
        // no floor on any difficulty or depth ever draws one
        for diff in ["relaxed", "standard", "hard", "nightmare"] {
            for depth in 1..=60 {
                let i = crate::hunters::pick_for_floor_id(&content, "SECRET-SEED", depth, diff);
                assert!(!is_secret(&content.hunters[i].id));
            }
        }
        // the shapeshifter never wears one (her pool is a fixed list)
        assert!(IDS.iter().all(|id| !crate::hunters::gimmicks::umbriel::pool().contains(id)));
    }
}
