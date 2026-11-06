//! Umbriel: shapeshifter. Every 30 s she wears another hunter's body (never Kathy's); her
//! stats and behaviour stay her own (umbriel_gimmick.gd).

use crate::hunters::brain::Brain;

/// HunterRoster.disguise_ids: every profile id but umbriel, nyxie and solyth, sorted.
const POOL: [&str; 35] = [
    "aki",
    "briselle",
    "ceryth",
    "dovelle",
    "elise",
    "ferryn",
    "glyra",
    "haelith",
    "ilyra",
    "javelle",
    "kathy",
    "loxelle",
    "mavrine",
    "opalie",
    "porcelle",
    "quenelle",
    "ravelle",
    "thessine",
    "verrine",
    "void_black",
    "void_blue",
    "void_brown",
    "void_cyan",
    "void_gray",
    "void_green",
    "void_orange",
    "void_pink",
    "void_purple",
    "void_red",
    "void_white",
    "void_yellow",
    "wynelle",
    "xivvy",
    "ysolde",
    "zellia",
];

/// The bodies she may wear (the secret creatures are not among them).
pub fn pool() -> &'static [&'static str] {
    &POOL
}

pub struct Umbriel {
    timer: f32,
}

impl Umbriel {
    pub fn new() -> Umbriel {
        Umbriel { timer: 30.0 }
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = 30.0;
        let current = b.disguise.clone().unwrap_or_default();
        let pool: Vec<&str> = POOL
            .iter()
            .copied()
            .filter(|id| *id != "kathy" && *id != current)
            .collect();
        let pick = pool[b.next_index(pool.len())];
        b.disguise = Some(pick.to_string());
        // the change: she goes rigid for a moment as the new body settles on her
        b.raise_cue("shapeshift");
    }
}
