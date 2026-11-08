//! Umbriel: shapeshifter. Every 30 s she wears another hunter's body (never Kathy's); her
//! stats and behaviour stay her own (umbriel_gimmick.gd). On the difficulties listed in
//! data/gimmicks/umbriel.json `copy_abilities_on` (Nightmare) she also takes on the worn hunter's
//! ability: `copied_ability()` names the hunter whose gimmick the dispatcher runs for her.
//! The pool, the interval and the exclusions all come from that data file.

use crate::config::Difficulty;
use crate::hunters::brain::Brain;

/// The tunables of data/gimmicks/umbriel.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub interval: f32,
    pub excluded: Vec<String>,
    pub copy_on: Vec<String>,
    pub pool: Vec<String>,
}

impl Tuning {
    /// Parse the data text; a bad document gives an empty pool (she then keeps her own body).
    pub fn parse(text: &str) -> Tuning {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        let interval = v
            .get("interval_seconds")
            .and_then(|x| x.as_f64())
            .filter(|n| *n > 0.0 && n.is_finite())
            .map_or(30.0, |n| n as f32);
        Tuning { interval, excluded: list("excluded"), copy_on: list("copy_abilities_on"), pool: list("pool") }
    }
}

fn tuning() -> &'static Tuning {
    static T: std::sync::OnceLock<Tuning> = std::sync::OnceLock::new();
    T.get_or_init(|| Tuning::parse(include_str!("../../data/gimmicks/umbriel.json")))
}

/// The bodies she may wear (the secret creatures are not among them), from data.
pub fn pool() -> &'static [&'static str] {
    static P: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    P.get_or_init(|| tuning().pool.iter().map(|s| &*Box::leak(s.clone().into_boxed_str())).collect())
}

/// The ids she may pick next, given the body she wears now: the pool less the excluded ids and the current body.
pub fn candidates<'a>(t: &'a Tuning, current: &str) -> Vec<&'a str> {
    t.pool
        .iter()
        .map(String::as_str)
        .filter(|id| *id != current && !t.excluded.iter().any(|e| e == id))
        .collect()
}

pub struct Umbriel {
    timer: f32,
    copies: bool,
}

impl Umbriel {
    /// Umbriel on Standard: the disguise is only a look.
    pub fn new() -> Umbriel {
        Umbriel::with_difficulty(Difficulty::Standard)
    }

    /// Umbriel on `diff`: she copies the worn hunter's ability where the data says so.
    pub fn with_difficulty(diff: Difficulty) -> Umbriel {
        let t = tuning();
        Umbriel { timer: t.interval, copies: t.copy_on.iter().any(|d| d == diff.id()) }
    }

    /// True when she runs the worn hunter's ability too (Nightmare in the shipped data).
    pub fn copies_abilities(&self) -> bool {
        self.copies
    }

    /// The hunter whose ability she runs now: the body she wears, when she copies abilities.
    pub fn copied_ability<'a>(&self, b: &'a Brain) -> Option<&'a str> {
        if self.copies {
            b.disguise.as_deref()
        } else {
            None
        }
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        let t = tuning();
        self.timer = t.interval;
        let current = b.disguise.clone().unwrap_or_default();
        let pool = candidates(t, &current);
        if pool.is_empty() {
            return;
        }
        let pick = pool[b.next_index(pool.len())];
        b.disguise = Some(pick.to_string());
        // the change: she goes rigid for a moment as the new body settles on her
        b.raise_cue("shapeshift");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped pool is the 35 roster bodies, never Kathy, and each of the draws comes out of it.
    #[test]
    fn the_shipped_pool_comes_from_data_and_never_offers_kathy() {
        assert_eq!(pool().len(), 35);
        assert!(!pool().iter().any(|id| matches!(*id, "umbriel" | "nyxie" | "solyth")));
        let t = tuning();
        for current in ["", "aki", "void_red"] {
            let c = candidates(t, current);
            assert!(!c.is_empty());
            assert!(!c.contains(&"kathy"), "Kathy is excluded");
            assert!(!c.contains(&current) || current.is_empty(), "never the body she wears now");
        }
    }

    /// A data edit changes the pool, the exclusions and the interval.
    #[test]
    fn a_data_edit_changes_the_pool_and_the_interval() {
        let t = Tuning::parse(r#"{"interval_seconds": 7, "excluded": ["b"], "pool": ["a", "b", "c"], "copy_abilities_on": ["hard"]}"#);
        assert_eq!(t.interval, 7.0);
        assert_eq!(candidates(&t, "a"), vec!["c"]);
        assert_eq!(Tuning::parse("nope").interval, 30.0);
        assert!(Tuning::parse("nope").pool.is_empty());
    }

    /// Only Nightmare copies the worn hunter's ability.
    #[test]
    fn only_nightmare_copies_the_worn_ability() {
        assert!(Umbriel::with_difficulty(Difficulty::Nightmare).copies_abilities());
        for d in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard] {
            assert!(!Umbriel::with_difficulty(d).copies_abilities());
        }
        assert!(!Umbriel::new().copies_abilities());
    }
}
