//! NightWatchSounds: which cue plays for which game event. Every function returns the cue list so logic tests can
//! assert them without audio. Cue names are the Night Watch 2D cues (`nw_<name>` in the renderer).
#![allow(dead_code)]

use crate::core::Cue;

/// Every Night Watch cue name (the Godot `CUES` list).
pub const CUES: [&str; 36] = [
    "move", "pick", "deny", "burst", "step", "clank", "bound", "pad", "ring", "lure", "crackle", "hum", "hum_muffled",
    "breath", "talk", "door", "light_on", "camera", "scare", "scare_short", "power_out", "chime", "tape", "radio",
    "rustle", "zip", "clunk", "glass_tap", "hinge", "latch", "squeak", "bed_breath", "bed_step", "bed_leave",
    "bed_stir", "bed_restless",
];

/// The room tones an office spec's `tone_cue` may use.
pub const OFFICE_TONES: [&str; 11] = [
    "camera", "static", "footsteps", "presence", "scrape", "warning", "drain", "shift", "control", "crimson_door",
    "breath",
];

/// One cue by name. A name of the table is kept as is; an unknown name becomes "move" so `Cue.name` stays static.
pub fn cue(name: &str, pan: f32, intensity: f32) -> Cue {
    let name: &'static str = CUES.iter().copied().find(|c| *c == name).unwrap_or("move");
    Cue { name, pan, intensity }
}

fn one(name: &'static str, pan: f32, intensity: f32) -> Vec<Cue> {
    vec![Cue { name, pan, intensity }]
}

/// The old shared-cue entry point: a Last Route room tone mapped to the 2D cue that does the same job.
/// `vol` is the intensity multiplier.
pub fn office(kind: &str, pan: f32, vol: f32) -> Vec<Cue> {
    match kind {
        "camera" => one("camera", pan, vol),
        "static" => one("burst", pan, vol),
        "footsteps" => one("step", pan, vol),
        "presence" => one("clank", pan, vol),
        "scrape" => one("step", pan, 0.5 * vol),
        "warning" => one("crackle", pan, vol),
        "drain" => one("power_out", pan, vol),
        "shift" => one("chime", pan, vol),
        "crimson_door" => one("clank", pan, vol),
        "breath" => one("breath", pan, 0.7 * vol),
        _ => one("move", pan, vol),
    }
}

/// A door slab moved (the same cue for both directions).
pub fn door(_shut: bool) -> Vec<Cue> {
    one("door", 0.0, 1.0)
}

pub fn panel_button() -> Vec<Cue> {
    one("move", 0.0, 1.0)
}

pub fn monitor_toggle() -> Vec<Cue> {
    one("camera", 0.0, 1.0)
}

pub fn camera_switch() -> Vec<Cue> {
    vec![Cue { name: "camera", pan: 0.0, intensity: 1.0 }, Cue { name: "burst", pan: 0.0, intensity: 0.8 }]
}

pub fn light_on() -> Vec<Cue> {
    one("light_on", 0.0, 1.0)
}

/// A hunter event heard by room pan. `kind`: moved, arrived, flinch, repelled, pushed. The caller only asks for
/// "moved" when the hunter is in a hall (west or east), as in Godot; any other kind is silent here.
pub fn hunter_event(kind: &str, pan: f32) -> Vec<Cue> {
    match kind {
        "moved" => one("step", pan, 1.0),
        "arrived" => vec![
            Cue { name: "step", pan, intensity: 0.55 / 0.35 },
            Cue { name: "clank", pan, intensity: 1.0 },
        ],
        "flinch" => one("burst", pan, 1.0),
        "repelled" | "pushed" => vec![
            Cue { name: "door", pan: 0.0, intensity: 1.0 },
            Cue { name: "step", pan, intensity: 0.5 / 0.35 },
        ],
        _ => Vec::new(),
    }
}

pub fn power_out() -> Vec<Cue> {
    one("power_out", 0.0, 1.0)
}

pub fn hour_change() -> Vec<Cue> {
    one("chime", 0.0, 1.0)
}

/// Zellia's ring, when "Incoming call..." comes up.
pub fn call_ring() -> Vec<Cue> {
    one("ring", 0.0, 1.0)
}

/// A talking blip on every second typed word.
pub fn call_line() -> Vec<Cue> {
    one("talk", 0.0, 1.0)
}

pub fn attack_warning(pan: f32) -> Vec<Cue> {
    one("crackle", pan, 1.0)
}

pub fn win() -> Vec<Cue> {
    one("chime", 0.0, 1.0)
}

pub fn caught(short: bool) -> Vec<Cue> {
    one(if short { "scare_short" } else { "scare" }, 0.0, 1.0)
}

/// Menu sounds: "move" (hover or focus), "pick" (press), "deny" (a locked choice). Anything else is "move".
pub fn menu(kind: &str) -> Vec<Cue> {
    match kind {
        "pick" => one("pick", 0.0, 1.0),
        "deny" => one("deny", 0.0, 1.0),
        _ => one("move", 0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[Cue]) -> Vec<&'static str> {
        v.iter().map(|c| c.name).collect()
    }

    #[test]
    fn door_is_one_door_cue_both_ways() {
        for shut in [true, false] {
            let d = door(shut);
            assert_eq!(names(&d), vec!["door"]);
            assert_eq!(d[0].pan, 0.0);
        }
    }

    #[test]
    fn hunter_events_follow_the_classic_table() {
        let m = hunter_event("moved", -0.8);
        assert_eq!(names(&m), vec!["step"]);
        assert_eq!(m[0].pan, -0.8);
        assert_eq!(m[0].intensity, 1.0);
        let a = hunter_event("arrived", 0.5);
        assert_eq!(names(&a), vec!["step", "clank"]);
        assert!((a[0].intensity - 0.55 / 0.35).abs() < 1e-6);
        assert_eq!(names(&hunter_event("flinch", 0.2)), vec!["burst"]);
        for k in ["repelled", "pushed"] {
            let r = hunter_event(k, 0.9);
            assert_eq!(names(&r), vec!["door", "step"]);
            assert!((r[1].intensity - 0.5 / 0.35).abs() < 1e-6);
            assert_eq!(r[1].pan, 0.9);
        }
        assert!(hunter_event("nothing", 0.0).is_empty());
    }

    #[test]
    fn attack_warning_is_crackle() {
        let w = attack_warning(-0.4);
        assert_eq!(names(&w), vec!["crackle"]);
        assert_eq!(w[0].pan, -0.4);
    }

    #[test]
    fn camera_switch_adds_a_quieter_burst() {
        let c = camera_switch();
        assert_eq!(names(&c), vec!["camera", "burst"]);
        assert_eq!(c[1].intensity, 0.8);
    }

    #[test]
    fn office_tones_map_like_godot() {
        assert_eq!(names(&office("static", 0.1, 1.0)), vec!["burst"]);
        assert_eq!(names(&office("footsteps", 0.0, 1.0)), vec!["step"]);
        let s = office("scrape", 0.0, 0.8);
        assert_eq!(names(&s), vec!["step"]);
        assert!((s[0].intensity - 0.4).abs() < 1e-6);
        assert_eq!(names(&office("crimson_door", 0.0, 1.0)), vec!["clank"]);
        assert_eq!(names(&office("presence", 0.0, 1.0)), vec!["clank"]);
        assert_eq!(names(&office("drain", 0.0, 1.0)), vec!["power_out"]);
        assert_eq!(names(&office("control", 0.0, 1.0)), vec!["move"]);
        assert_eq!(names(&office("shift", 0.0, 1.0)), vec!["chime"]);
        assert_eq!(names(&office("warning", 0.0, 1.0)), vec!["crackle"]);
        let b = office("breath", 0.0, 1.0);
        assert_eq!(names(&b), vec!["breath"]);
        assert!((b[0].intensity - 0.7).abs() < 1e-6);
        assert_eq!(names(&office("camera", 0.0, 1.0)), vec!["camera"]);
        assert_eq!(names(&office("unknown", 0.0, 1.0)), vec!["move"]);
    }

    #[test]
    fn every_office_tone_gives_a_known_cue() {
        for t in OFFICE_TONES {
            for c in office(t, 0.0, 1.0) {
                assert!(CUES.contains(&c.name), "{t} -> {}", c.name);
            }
        }
    }

    #[test]
    fn simple_events() {
        assert_eq!(names(&panel_button()), vec!["move"]);
        assert_eq!(names(&monitor_toggle()), vec!["camera"]);
        assert_eq!(names(&light_on()), vec!["light_on"]);
        assert_eq!(names(&power_out()), vec!["power_out"]);
        assert_eq!(names(&hour_change()), vec!["chime"]);
        assert_eq!(names(&win()), vec!["chime"]);
        assert_eq!(names(&call_ring()), vec!["ring"]);
        assert_eq!(names(&call_line()), vec!["talk"]);
        assert_eq!(names(&caught(true)), vec!["scare_short"]);
        assert_eq!(names(&caught(false)), vec!["scare"]);
        assert_eq!(names(&menu("pick")), vec!["pick"]);
        assert_eq!(names(&menu("deny")), vec!["deny"]);
        assert_eq!(names(&menu("move")), vec!["move"]);
    }

    #[test]
    fn cue_helper_keeps_known_names() {
        assert_eq!(cue("bed_step", 0.1, 0.5).name, "bed_step");
        assert_eq!(cue("zzz", 0.0, 1.0).name, "move");
    }
}
