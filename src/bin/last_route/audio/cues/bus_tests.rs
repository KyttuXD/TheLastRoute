//! Regression guard for the volume sliders: every sound plays on a bus (data/audio/buses.json), and every bus applies MASTER
//! and its own slider, so a new sound cannot play at a fixed volume. Adding a cue needs no change here: the lists below come
//! from the cue tables and cues.json. A bus nobody can silence, or a play path that skips `bus_gain`, fails one of these.

use super::extra;
use super::rules::{self, bus_for_cue, bus_gain, Bus, Play, SoundFront, Volumes};

/// What the engine plays a `Play` at: its own gain times its bus (`Engine::play_volume`).
fn played(p: &Play, v: &Volumes) -> f32 {
    p.gain * bus_gain(p.bus, v)
}

/// The default volumes with the slider that belongs to `bus` alone at zero.
fn zero_category(bus: Bus) -> Volumes {
    let mut v = Volumes::default();
    match bus {
        Bus::Sfx => v.gameplay = 0.0,
        Bus::Hunters => v.hunters = 0.0,
        Bus::Ambience => v.ambience = 0.0,
        Bus::Music => v.music = 0.0,
        Bus::Ui => v.ui = 0.0,
    }
    v
}

/// Every `Play` a rules constructor or a data event can produce.
fn every_play() -> Vec<Play> {
    let mut out: Vec<Play> = vec![];
    for name in extra::event_names() {
        for volume in [0.0, 0.045, 0.09] {
            out.push(extra::event(name, 0.3, volume).unwrap_or_else(|| panic!("event {name}")));
        }
    }
    let mut front = SoundFront::new();
    out.extend([
        rules::collect(),
        rules::recharge(25.0),
        rules::music_box(),
        rules::alert(),
        rules::chase_start(),
        rules::taser(),
        rules::door(true),
        rules::door(false),
        rules::door_breach(),
        rules::interaction_fail(),
        rules::tool_break(),
        rules::axe_swing(0.0, 1.0),
        rules::axe_chop(0.0, 1.0),
        rules::hide_search(0.0, 1.0),
        rules::light_switch(0.0, 1.0),
        rules::sleep_settle(),
        rules::hunter_snore(0.0, 1.0),
        rules::nyxie_timer_wake(),
        rules::nyxie_empty_wake(),
        rules::hunter_laugh(0.0, 0.09),
        rules::spider_bark(0.0, 1.0),
        rules::spider_hiss(0.0, 1.0),
        rules::spider_skitter(0.0, 1.0),
        rules::xivvy_screech(0.0, 1.0),
        rules::radar_static(),
        rules::system_offline(),
        rules::splash_big(0.0, 1.0),
        rules::water_drip(0.0, 1.0),
        rules::inferno_crackle(0.0, 1.0),
        rules::pit_rumble(0.0, 1.0),
        rules::music("piano", 0.0, 1.0),
        rules::ladder_rung(0.0, 1.0),
        rules::carnival("carousel_tune", 0.0, 1.0),
        rules::nw("breath", 0.0, 1.0),
        rules::nw("step", 0.0, 1.0),
        rules::mimic(false),
        rules::capture(),
        rules::escape(),
    ]);
    out.push(front.hunter_footstep(0.0, 0.09, ""));
    out.push(front.hunter_footstep(0.0, 0.09, "wood"));
    out.push(front.menu_blip(0).unwrap());
    out.push(front.footstep("walk", "", 3.0, 0).unwrap());
    out.push(front.heartbeat(0.8, 0).unwrap());
    out.push(front.struggle(0.5, 0).unwrap());
    out
}

#[test]
fn every_cue_name_maps_to_a_bus_and_the_play_constructors_use_it() {
    for name in super::cue_names() {
        // total: a name with no entry plays on the default bus, never on nothing
        assert!(Bus::ALL.contains(&bus_for_cue(name)), "{name}");
    }
    for name in extra::event_names() {
        let p = extra::event(name, 0.0, 0.09).unwrap();
        assert_eq!(p.bus, bus_for_cue(&p.cue), "event {name} plays cue {} on the bus buses.json gives it", p.cue);
    }
    for p in every_play() {
        assert_eq!(p.bus, bus_for_cue(&p.cue), "{} is routed by buses.json", p.cue);
    }
}

#[test]
fn every_sound_applies_master_and_its_category_gain() {
    let plays = every_play();
    assert!(plays.len() > 40);
    for p in &plays {
        let full = Volumes::default();
        assert!((played(p, &full) - p.gain).abs() < 1e-6, "{}: sliders at 100% leave the gain alone", p.cue);
        // MASTER at zero silences it, at half halves it
        let off = Volumes { master: 0.0, ..full };
        assert_eq!(played(p, &off), 0.0, "{}: MASTER 0 is silent", p.cue);
        let half = Volumes { master: 0.5, ..full };
        assert!((played(p, &half) - p.gain * 0.5).abs() < 1e-6, "{}: MASTER scales it", p.cue);
        // its own category slider silences it
        assert_eq!(played(p, &zero_category(p.bus)), 0.0, "{} ({:?}): its category at 0 is silent", p.cue, p.bus);
        // a sibling category does not touch it (GAMEPLAY is the parent of the others, tested below)
        for other in [Bus::Hunters, Bus::Ambience, Bus::Music, Bus::Ui] {
            if other != p.bus {
                assert!((played(p, &zero_category(other)) - p.gain).abs() < 1e-6, "{}: {:?} must not scale a {:?} sound", p.cue, other, p.bus);
            }
        }
        // GAMEPLAY is the parent of HUNTERS, AMBIENCE and MUSIC; MENU / UI is not under it
        let no_gameplay = Volumes { gameplay: 0.0, ..full };
        let expect = if p.bus == Bus::Ui { p.gain } else { 0.0 };
        assert!((played(p, &no_gameplay) - expect).abs() < 1e-6, "{}: GAMEPLAY 0", p.cue);
    }
}

#[test]
fn every_bus_carries_at_least_one_sound_so_no_slider_is_dead() {
    let names = super::cue_names();
    for bus in Bus::ALL {
        assert!(names.iter().any(|n| bus_for_cue(n) == bus), "no cue plays on {bus:?}");
    }
    assert_eq!(bus_for_cue("glyra_hum"), Bus::Hunters);
    assert_eq!(bus_for_cue("music_box_tune"), Bus::Music);
    assert_eq!(bus_for_cue("menu"), Bus::Ui);
    assert_eq!(bus_for_cue("music_box_wind"), Bus::Sfx, "the music box's wind and jam are mechanics, not music");
    assert_eq!(bus_for_cue("step:hunter:wood:0"), Bus::Hunters);
    assert_eq!(bus_for_cue("footstep_walk"), Bus::Sfx);
    assert_eq!(bus_for_cue("a_brand_new_cue"), Bus::Sfx, "an unknown or new cue falls back to GAMEPLAY, never to no slider");
}

#[test]
fn buses_json_names_only_real_buses_and_real_cues() {
    let text = include_str!("../../data/audio/buses.json");
    let v: serde_json::Value = serde_json::from_str(text).expect("buses.json parses");
    assert!(Bus::from_key(v["default"].as_str().unwrap()).is_some());
    for bus in Bus::ALL {
        let label = v["labels"][bus.key()].as_str().unwrap_or_else(|| panic!("a label for {bus:?}"));
        assert!(!label.is_empty());
    }
    let names = super::cue_names();
    for (key, section) in [("exact", v["exact"].as_object().unwrap()), ("prefix", v["prefix"].as_object().unwrap())] {
        for (pattern, bus) in section {
            assert!(Bus::from_key(bus.as_str().unwrap()).is_some(), "{key} {pattern} names an unknown bus {bus}");
            let hit = if key == "exact" { names.contains(&pattern.as_str()) } else { names.iter().any(|n| n.starts_with(pattern.as_str())) };
            assert!(hit, "buses.json {key} entry {pattern} matches no cue (stale name)");
        }
    }
    let (exact, prefix) = rules::bus_map_entries();
    assert_eq!(exact.len(), v["exact"].as_object().unwrap().len());
    assert_eq!(prefix.len(), v["prefix"].as_object().unwrap().len());
}

/// Every `.rs` under the game's source, except the vendored audio crate.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            if path.is_dir() {
                if path.file_name().map(|n| n == "quad-snd").unwrap_or(false) {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().map(|x| x == "rs").unwrap_or(false) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((path.to_string_lossy().replace('\\', "/"), text));
                }
            }
        }
    }
    let mut out = vec![];
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/last_route"), &mut out);
    out
}

#[test]
fn only_the_engine_starts_sounds_and_every_loop_names_a_bus() {
    let files = sources();
    assert!(files.len() > 50, "the source scan found the game ({} files)", files.len());
    for (path, text) in &files {
        if path.ends_with("audio/cues/bus_tests.rs") {
            continue;
        }
        let engine_side = path.ends_with("audio/engine.rs") || path.ends_with("audio/mod.rs");
        if !engine_side {
            for banned in ["play_sound(", "set_sound_volume("] {
                assert!(!text.contains(banned), "{path} calls {banned} itself: play through audio::engine so the volume sliders apply");
            }
        }
        if path.ends_with("audio/engine.rs") {
            continue;
        }
        for call in [".set_loop(", ".play_frames("] {
            for (at, _) in text.match_indices(call) {
                let args: String = text[at..].chars().take(140).collect();
                assert!(args.contains("Bus::"), "{path}: {call} must name its Bus (a slider), got {args:?}");
            }
        }
    }
}
