//! Every embedded file: the copied Night Watch data (src/bin/night_watch/data/) and the hunter rigs and models from
//! the Last Route assets. Hub file (integrator).
#![allow(dead_code)]

pub fn mode_json(id: &str) -> &'static str {
    match id {
        "classic" => include_str!("data/modes/classic.json"),
        "mask" => include_str!("data/modes/mask.json"),
        "lure" => include_str!("data/modes/lure.json"),
        "bedroom" => include_str!("data/modes/bedroom.json"),
        "story" => include_str!("data/modes/story.json"),
        "descent" => include_str!("data/modes/descent.json"),
        "dayshift" => include_str!("data/modes/dayshift.json"),
        "extra" => include_str!("data/modes/extra.json"),
        "mixed" => include_str!("data/modes/mixed.json"),
        _ => "",
    }
}

/// The Zellia calls file of a mode ("" for modes without one, such as mixed).
pub fn calls_json(id: &str) -> &'static str {
    match id {
        "classic" => include_str!("data/calls/classic.json"),
        "mask" => include_str!("data/calls/mask.json"),
        "lure" => include_str!("data/calls/lure.json"),
        "bedroom" => include_str!("data/calls/bedroom.json"),
        "story" => include_str!("data/calls/story.json"),
        "descent" => include_str!("data/calls/descent.json"),
        "dayshift" => include_str!("data/calls/dayshift.json"),
        "extra" => include_str!("data/calls/extra.json"),
        _ => "",
    }
}

pub const RULES_JSON: &str = include_str!("data/rules/classic.json");
pub const BUILDING_JSON: &str = include_str!("data/buildings/classic.json");
pub const CAST_JSON: &str = include_str!("data/casts/classic.json");
pub const POSES_JSON: &str = include_str!("data/poses/night_watch_poses.json");
pub const DEATH_LINES_JSON: &str = include_str!("../last_route/assets/ui/death_lines.json");

/// Every hunter id that has an embedded rig and model (kathy included).
pub const HUNTER_IDS: [&str; 25] = [
    "dovelle",
    "ravelle",
    "aki",
    "ilyra",
    "nyxie",
    "kathy",
    "thessine",
    "umbriel",
    "verrine",
    "haelith",
    "glyra",
    "ferryn",
    "briselle",
    "mavrine",
    "solyth",
    "wynelle",
    "xivvy",
    "loxelle",
    "javelle",
    "elise",
    "ceryth",
    "ysolde",
    "opalie",
    "porcelle",
    "quenelle",
];

/// Rig JSON (anim/<id>.json) of a hunter.
pub fn rig_text(id: &str) -> Option<&'static str> {
    Some(match id {
        "dovelle" => include_str!("../last_route/assets/hunters/anim/dovelle.json"),
        "ravelle" => include_str!("../last_route/assets/hunters/anim/ravelle.json"),
        "aki" => include_str!("../last_route/assets/hunters/anim/aki.json"),
        "ilyra" => include_str!("../last_route/assets/hunters/anim/ilyra.json"),
        "nyxie" => include_str!("../last_route/assets/hunters/anim/nyxie.json"),
        "kathy" => include_str!("../last_route/assets/hunters/anim/kathy.json"),
        "thessine" => include_str!("../last_route/assets/hunters/anim/thessine.json"),
        "umbriel" => include_str!("../last_route/assets/hunters/anim/umbriel.json"),
        "verrine" => include_str!("../last_route/assets/hunters/anim/verrine.json"),
        "haelith" => include_str!("../last_route/assets/hunters/anim/haelith.json"),
        "glyra" => include_str!("../last_route/assets/hunters/anim/glyra.json"),
        "ferryn" => include_str!("../last_route/assets/hunters/anim/ferryn.json"),
        "briselle" => include_str!("../last_route/assets/hunters/anim/briselle.json"),
        "mavrine" => include_str!("../last_route/assets/hunters/anim/mavrine.json"),
        "solyth" => include_str!("../last_route/assets/hunters/anim/solyth.json"),
        "wynelle" => include_str!("../last_route/assets/hunters/anim/wynelle.json"),
        "xivvy" => include_str!("../last_route/assets/hunters/anim/xivvy.json"),
        "loxelle" => include_str!("../last_route/assets/hunters/anim/loxelle.json"),
        "javelle" => include_str!("../last_route/assets/hunters/anim/javelle.json"),
        "elise" => include_str!("../last_route/assets/hunters/anim/elise.json"),
        "ceryth" => include_str!("../last_route/assets/hunters/anim/ceryth.json"),
        "ysolde" => include_str!("../last_route/assets/hunters/anim/ysolde.json"),
        "opalie" => include_str!("../last_route/assets/hunters/anim/opalie.json"),
        "porcelle" => include_str!("../last_route/assets/hunters/anim/porcelle.json"),
        "quenelle" => include_str!("../last_route/assets/hunters/anim/quenelle.json"),
        _ => return None,
    })
}

/// Model JSON (<id>.bbmodel, texture embedded as a data URI) of a hunter.
pub fn bbmodel_text(id: &str) -> Option<&'static str> {
    Some(match id {
        "dovelle" => include_str!("../last_route/assets/hunters/dovelle.bbmodel"),
        "ravelle" => include_str!("../last_route/assets/hunters/ravelle.bbmodel"),
        "aki" => include_str!("../last_route/assets/hunters/aki.bbmodel"),
        "ilyra" => include_str!("../last_route/assets/hunters/ilyra.bbmodel"),
        "nyxie" => include_str!("../last_route/assets/hunters/nyxie.bbmodel"),
        "kathy" => include_str!("../last_route/assets/hunters/kathy.bbmodel"),
        "thessine" => include_str!("../last_route/assets/hunters/thessine.bbmodel"),
        "umbriel" => include_str!("../last_route/assets/hunters/umbriel.bbmodel"),
        "verrine" => include_str!("../last_route/assets/hunters/verrine.bbmodel"),
        "haelith" => include_str!("../last_route/assets/hunters/haelith.bbmodel"),
        "glyra" => include_str!("../last_route/assets/hunters/glyra.bbmodel"),
        "ferryn" => include_str!("../last_route/assets/hunters/ferryn.bbmodel"),
        "briselle" => include_str!("../last_route/assets/hunters/briselle.bbmodel"),
        "mavrine" => include_str!("../last_route/assets/hunters/mavrine.bbmodel"),
        "solyth" => include_str!("../last_route/assets/hunters/solyth.bbmodel"),
        "wynelle" => include_str!("../last_route/assets/hunters/wynelle.bbmodel"),
        "xivvy" => include_str!("../last_route/assets/hunters/xivvy.bbmodel"),
        "loxelle" => include_str!("../last_route/assets/hunters/loxelle.bbmodel"),
        "javelle" => include_str!("../last_route/assets/hunters/javelle.bbmodel"),
        "elise" => include_str!("../last_route/assets/hunters/elise.bbmodel"),
        "ceryth" => include_str!("../last_route/assets/hunters/ceryth.bbmodel"),
        "ysolde" => include_str!("../last_route/assets/hunters/ysolde.bbmodel"),
        "opalie" => include_str!("../last_route/assets/hunters/opalie.bbmodel"),
        "porcelle" => include_str!("../last_route/assets/hunters/porcelle.bbmodel"),
        "quenelle" => include_str!("../last_route/assets/hunters/quenelle.bbmodel"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hunter_has_rig_and_model() {
        for id in HUNTER_IDS {
            assert!(rig_text(id).is_some(), "rig {id}");
            assert!(bbmodel_text(id).is_some(), "model {id}");
        }
        assert!(rig_text("nobody").is_none());
    }

    #[test]
    fn data_blobs_are_not_empty() {
        for id in ["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra", "mixed"] {
            assert!(mode_json(id).len() > 10, "mode {id}");
        }
        assert!(calls_json("mixed").is_empty());
        assert!(calls_json("classic").len() > 10);
        assert!(RULES_JSON.len() > 10 && BUILDING_JSON.len() > 10 && CAST_JSON.len() > 10);
        assert!(POSES_JSON.len() > 10 && DEATH_LINES_JSON.len() > 10);
    }
}
