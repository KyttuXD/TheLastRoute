//! Tests for the data-driven sound: the per-region ambience beds (data/audio/ambience.json) and the extra cues and
//! events (data/audio/cues.json).

use super::ambience::{bed_for, has_bed, settings, Mixer, SlotOut};
use super::*;

const COLOUR_KEY_REGIONS: [&str; 13] = [
    "apartments", "prison", "maze", "ice", "stalls", "inferno_zero", "tunnels", "lab", "offices", "utility",
    "greenhouse", "sewer", "inferno",
];

#[test]
fn a_bed_is_a_sound_loop_of_the_asked_length() {
    let bed = ambience("lab", 2.0);
    assert_eq!(bed.len(), 88200);
    assert!(bed.iter().any(|f| f[0].abs() > 1e-4 && f[1].abs() > 1e-4), "a bed is not silent");
    assert!(bed.iter().all(|f| f[0].abs() <= 0.98 && f[1].abs() <= 0.98));
    // a seamless loop: the last frame flows into the first (no jump bigger than the bed's own steps)
    let biggest_step = bed.windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0.0f32, f32::max);
    let seam = (bed[0][0] - bed[bed.len() - 1][0]).abs();
    assert!(seam <= biggest_step * 1.5 + 1e-6, "seam {seam} vs {biggest_step}");
    assert_eq!(ambience("lab", 2.0), bed, "a bed renders the same every time");
}

#[test]
fn every_region_has_an_ambience_entry_with_layers() {
    let s = settings();
    for region in COLOUR_KEY_REGIONS {
        assert!(s.regions.iter().any(|r| r == region), "{region} is listed in data/audio/ambience.json");
        assert!(has_bed(region), "{region} has no bed");
        assert!(!s.beds[bed_for(region)].layers.is_empty(), "{region} bed has no layers");
    }
    for r in &s.regions {
        assert!(s.beds.contains_key(r), "listed region {r} has no bed");
    }
    for (alias, target) in &s.aliases {
        assert!(s.beds.contains_key(target), "alias {alias} -> {target}");
    }
    for (id, bed) in &s.beds {
        assert!(s.spaces.contains_key(&bed.space), "{id} has an unknown space {}", bed.space);
    }
    // the region files that ship resolve to a bed of their own or an alias, not just the fallback
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/last_route/data/regions");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            assert!(has_bed(&stem), "region file {stem} has no ambience bed or alias");
        }
    }
}

#[test]
fn every_bed_sits_at_the_data_loudness() {
    let target = 10f64.powf(settings().target_rms_db / 20.0);
    for region in ["ice", "stalls", "tunnels", "inferno"] {
        let bed = ambience(region, 3.0);
        let rms = (bed.iter().map(|f| ((f[0] + f[1]) as f64 * 0.5).powi(2)).sum::<f64>() / bed.len() as f64).sqrt();
        assert!(rms > target * 0.5 && rms < target * 2.0, "{region} rms {rms} vs target {target}");
    }
}

#[test]
fn leaving_a_region_crossfades_the_bed_out_and_the_new_one_in() {
    let mut m = Mixer::default();
    assert!(m.step(0.1, None).iter().all(|o| o.bed.is_none()));
    let first = m.step(0.1, Some("tunnels"));
    assert_eq!(first.iter().filter(|o| o.bed == Some("tunnels")).count(), 1);
    let mut gain_in = 0.0f32;
    for _ in 0..40 {
        let out = m.step(0.1, Some("tunnels"));
        let g = out.iter().find(|o| o.bed == Some("tunnels")).unwrap().gain;
        assert!(g >= gain_in - 1e-6, "fades in monotonically");
        gain_in = g;
    }
    assert!(gain_in > 0.0);
    let mid = m.step(0.2, Some("ice"));
    assert!(mid.iter().any(|o| o.bed == Some("tunnels") && o.gain > 0.0), "old bed still fading out");
    assert!(mid.iter().any(|o| o.bed == Some("ice")), "new bed fading in");
    let mut last = mid;
    for _ in 0..60 {
        last = m.step(0.1, Some("ice"));
    }
    assert!(last.iter().all(|o| o.bed != Some("tunnels")), "old bed stopped");
    assert!(last.iter().any(|o| o.bed == Some("ice") && o.gain > 0.0));
    for _ in 0..60 {
        last = m.step(0.1, None);
    }
    assert!(last.iter().all(|o| o.bed.is_none()), "leaving every region fades all out");
}

#[test]
fn ducking_turns_the_bed_down() {
    let mut quiet = Mixer::default();
    let mut loud = Mixer::default();
    let mut q = [SlotOut { bed: None, gain: 0.0 }; 2];
    let mut l = q;
    for _ in 0..40 {
        q = quiet.step_ducked(0.1, Some("lab"), 1.0);
        l = loud.step_ducked(0.1, Some("lab"), 0.0);
    }
    let top = |o: [SlotOut; 2]| o.iter().map(|s| s.gain).fold(0.0f32, f32::max);
    assert!(top(q) < top(l) * 0.5 && top(q) > 0.0, "ducked {} vs {}", top(q), top(l));
}

#[test]
fn every_data_cue_and_event_resolves_to_a_sound() {
    use super::extra;
    assert!(!extra::names().is_empty());
    for name in extra::names() {
        let frames = synth(name, 0).unwrap_or_else(|| panic!("data cue {name} did not render"));
        assert!(frames.len() > 100 && frames.iter().any(|f| f[0].abs() > 1e-4), "{name} is silent");
        assert!(knows_cue(name));
        assert!(cue_names().contains(&name), "{name} is listed");
        if let Some(file) = extra::file_of(name) {
            assert!(crate::content::embedded::asset(&format!("audio/{file}")).is_some(), "{name} names a missing file {file}");
        }
    }
    for needed in [
        "locker_open", "locker_close", "relay_pickup", "item_pickup", "music_box_wind", "music_box_jam", "vent_open",
        "vent_close", "hunter_sniff", "hunter_knock",
    ] {
        assert!(extra::names().contains(&needed), "{needed} is defined");
        assert!(extra::event_names().contains(&needed), "{needed} has an event");
    }
    for event in extra::event_names() {
        let cue = extra::event_cue(event).unwrap();
        assert!(knows_cue(cue), "event {event} plays unknown cue {cue}");
        let p = extra::event(event, 0.3, 0.05).unwrap();
        assert!(p.frames().is_some(), "event {event} does not render");
    }
    for surface in SURFACES {
        let name = format!("hunter_step_{surface}");
        assert!(extra::event_names().contains(&name.as_str()), "no hunter step for {surface}");
    }
    assert!(extra::event("no_such_event", 0.0, 1.0).is_none());
}

#[test]
fn a_scaled_event_carries_its_loudness_and_a_plain_one_ignores_it() {
    use super::extra;
    let loud = extra::event("hunter_knock", 0.0, 0.1).unwrap();
    let soft = extra::event("hunter_knock", 0.0, 0.025).unwrap();
    assert!(loud.gain > soft.gain);
    assert_eq!(loud.intensity, 1.0);
    let a = extra::event("locker_open", 0.0, 0.1).unwrap();
    let b = extra::event("locker_open", 0.0, 0.5).unwrap();
    assert_eq!(a, b);
}

#[test]
fn a_data_cue_does_not_shadow_a_built_in_one() {
    use super::extra;
    for name in extra::names() {
        assert!(!table::NAMES.contains(&name), "{name} is also a built-in cue");
    }
}
