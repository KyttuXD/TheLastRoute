//! The cue table: a line-for-line port of `Sound._cue_components` (game/autoload/sound.gd 901-1081). Numbers, argument
//! order and expression order are the source's, so the f64 results (and therefore the rendered bytes) are the same.
//! Integer literals of the source are written as floats; `pan` is passed only where the source passes it (several cues,
//! e.g. collect and heartbeat, ignore the caller's pan).

use super::nw;
use super::render::{noise as n, thump as th, tone as t, Component};
use super::surfaces;

/// Every plain cue name the table answers (the match arms of `_cue_components`, in source order).
pub const NAMES: &[&str] = &[
    "carnival_start",
    "carnival_throw",
    "carnival_ring_on",
    "carnival_ring_miss",
    "carnival_windup",
    "carnival_swing",
    "carnival_thud",
    "carnival_bell",
    "carnival_win",
    "carnival_lose",
    "carnival_shot",
    "carnival_drop",
    "carnival_hit",
    "carnival_miss_swing",
    "carnival_miss",
    "carnival_hook",
    "carnival_carousel_tune",
    "carnival_lever",
    "carnival_tick",
    "carnival_pop",
    "carnival_clang",
    "carnival_splash",
    "carnival_motor",
    "carnival_grip",
    "collect",
    "recharge",
    "menu",
    "music_box",
    "music_box_tune",
    "glyra_hum",
    "alert",
    "chase_start",
    "taser",
    "door_open",
    "door_close",
    "door_breach",
    "interaction_fail",
    "axe_swing",
    "axe_chop",
    "light_switch",
    "sleep_settle",
    "sleep_yawn",
    "sleep_stir",
    "sleep_jolt",
    "hunter_snore",
    "hide_search",
    "tool_break",
    "nyxie_timer",
    "nyxie_empty",
    "hunter_laugh",
    "xivvy_screech",
    "radar_static",
    "system_offline",
    "signal_node",
    "xivvy_wing",
    "spider_hiss",
    "spider_bark",
    "splash_big",
    "music_mic",
    "music_podium",
    "music_drums",
    "music_electric",
    "music_acoustic",
    "music_bass",
    "music_keys",
    "music_piano",
    "water_drip",
    "spider_skitter",
    "inferno_crackle",
    "pit_rumble",
    "ladder_rung",
    "hunter_footstep",
    "teammate_footstep_sprint",
    "teammate_footstep_crouch",
    "teammate_footstep_walk",
    "mimic",
    "mimic_rare",
    "heartbeat",
    "footstep_sprint",
    "footstep_crouch",
    "footstep_walk",
    "capture",
    "struggle",
    "escape",
    "office_camera",
    "office_warning",
    "office_crimson_door",
    "office_presence",
    "office_scrape",
    "office_static",
    "office_drain",
    "office_footsteps",
    "office_breath",
    "office_shift",
    "office_control",
    "office_jumpscare",
];

/// The beep any unknown name gets in the original (sound.gd 1080). `components` returns `None` instead, so a typo is
/// visible; play this if you want the original's behaviour.
pub fn fallback() -> Vec<Component> {
    vec![t(265.0, 0.08, 0.045, 0.0, "square", 0.0)]
}

/// `_cue_components(cue, pan, intensity)` with the already clean (snapped) pan and intensity. `None` for a name the
/// original does not know (it would play `fallback()`; an unknown `nw_` name renders 50 ms of silence there).
pub fn components(cue: &str, pan: f64, intensity: f64) -> Option<Vec<Component>> {
    if cue.starts_with("step:") {
        return surfaces::surface_step(cue, pan, intensity);
    }
    if let Some(rest) = cue.strip_prefix("nw_") {
        return nw::components(rest, pan, intensity);
    }
    let i = intensity;
    let c: Vec<Component> = match cue {
        "carnival_start" => vec![
            t(1318.0, 0.22, 0.07, 0.0, "triangle", pan),
            t(1760.0, 0.3, 0.05, 0.08, "triangle", pan),
        ],
        "carnival_throw" => vec![
            n(0.16, 0.05, 1500.0, 0.0, pan, 0.8),
            t(420.0, 0.12, 0.04, 0.0, "triangle", pan),
        ],
        "carnival_ring_on" => vec![
            t(1568.0, 0.12, 0.09, 0.0, "triangle", pan),
            t(2093.0, 0.2, 0.07, 0.06, "triangle", pan),
            th(300.0, 0.05, 0.05, 0.0, pan),
        ],
        "carnival_ring_miss" => vec![
            th(120.0, 0.12, 0.09, 0.0, pan),
            n(0.09, 0.04, 500.0, 0.0, pan, 0.7),
        ],
        "carnival_windup" => vec![
            t(180.0, 0.35, 0.05, 0.0, "saw", pan),
            t(270.0, 0.3, 0.04, 0.2, "saw", pan),
        ],
        "carnival_swing" => vec![
            n(0.18, 0.08, 900.0, 0.0, pan, 0.7),
            th(70.0, 0.22, 0.16, 0.1, pan),
        ],
        "carnival_thud" => vec![
            th(90.0, 0.25, 0.14, 0.0, pan),
            n(0.1, 0.06, 400.0, 0.0, pan, 0.6),
        ],
        "carnival_bell" => vec![
            t(880.0, 1.6, 0.2 * i.min(1.5), 0.0, "sine", pan),
            t(1320.0, 1.3, 0.13, 0.0, "sine", pan),
            t(1760.0, 1.0, 0.1, 0.0, "triangle", pan),
            th(60.0, 0.3, 0.15, 0.0, pan),
        ],
        "carnival_win" => vec![
            t(523.0, 0.12, 0.08, 0.0, "square", pan),
            t(659.0, 0.12, 0.08, 0.1, "square", pan),
            t(784.0, 0.12, 0.08, 0.2, "square", pan),
            t(1046.0, 0.3, 0.09, 0.3, "triangle", pan),
        ],
        "carnival_lose" => vec![
            t(330.0, 0.18, 0.07, 0.0, "triangle", pan),
            t(262.0, 0.2, 0.07, 0.15, "triangle", pan),
            t(196.0, 0.4, 0.07, 0.3, "triangle", pan),
        ],
        "carnival_shot" => vec![
            n(0.12, 0.22, 2200.0, 0.0, pan, 1.1),
            th(70.0, 0.18, 0.2, 0.0, pan),
            t(180.0, 0.08, 0.06, 0.0, "saw", pan),
        ],
        "carnival_drop" => vec![
            t(110.0, 0.5, 0.05, 0.0, "saw", pan),
            t(160.0, 0.4, 0.03, 0.1, "square", pan),
        ],
        "carnival_hit" => vec![
            th(110.0, 0.12, 0.16, 0.0, pan),
            t(520.0, 0.06, 0.07, 0.0, "square", pan),
        ],
        "carnival_miss_swing" => vec![n(0.1, 0.05, 800.0, 0.0, pan, 0.8)],
        "carnival_miss" => vec![
            t(220.0, 0.2, 0.05, 0.0, "triangle", pan),
            t(165.0, 0.3, 0.05, 0.15, "triangle", pan),
        ],
        "carnival_hook" => vec![
            n(0.08, 0.04, 1600.0, 0.0, pan, 1.0),
            t(900.0, 0.05, 0.04, 0.0, "triangle", pan),
        ],
        "carnival_carousel_tune" => {
            // a waltz on a music box that has been left out in the damp: every note a little off, one flat, the bass a
            // beat late
            let melody = [
                523.3, 659.3, 784.0, 659.3, 880.0, 784.0, 659.3, 587.3, 523.3, 659.3, 784.0,
                1046.5, 880.0, 784.0, 698.5, 659.3,
            ];
            let mut c = Vec::new();
            for (idx, note) in melody.iter().enumerate() {
                let fi = idx as f64;
                let off = 1.0 + 0.018 * (fi * 2.3).sin() - if idx == 11 { 0.04 } else { 0.0 };
                c.push(t(note * off, 0.42, 0.05, fi * 0.5, "triangle", pan));
                if idx % 4 == 0 {
                    c.push(t(
                        if idx % 8 == 0 { 130.8 } else { 196.0 },
                        0.9,
                        0.035,
                        fi * 0.5 + 0.03,
                        "square",
                        pan,
                    ));
                }
            }
            c
        }
        "carnival_lever" => vec![
            th(80.0, 0.2, 0.2, 0.0, pan),
            t(220.0, 0.15, 0.08, 0.0, "square", pan),
            n(0.12, 0.08, 700.0, 0.05, pan, 0.8),
        ],
        "carnival_tick" => vec![t(1400.0, 0.02, 0.03, 0.0, "square", pan)],
        "carnival_pop" => vec![
            th(160.0, 0.08, 0.1, 0.0, pan),
            t(700.0, 0.05, 0.05, 0.0, "square", pan),
        ],
        "carnival_clang" => vec![
            t(1200.0, 0.3, 0.1, 0.0, "square", pan),
            t(1900.0, 0.25, 0.06, 0.0, "triangle", pan),
            n(0.06, 0.08, 3000.0, 0.0, pan, 1.2),
        ],
        "carnival_splash" => vec![
            n(0.3, 0.06, 900.0, 0.0, pan, 0.9),
            t(500.0, 0.08, 0.04, 0.02, "sine", pan),
        ],
        "carnival_motor" => vec![
            t(90.0, 0.3, 0.05, 0.0, "saw", pan),
            t(120.0, 0.3, 0.03, 0.0, "square", pan),
        ],
        "carnival_grip" => vec![
            t(240.0, 0.1, 0.06, 0.0, "square", pan),
            th(110.0, 0.12, 0.08, 0.05, pan),
        ],
        "collect" => vec![
            t(220.0, 0.12, 0.13, 0.0, "square", 0.0),
            t(330.0, 0.16, 0.10, 0.09, "square", 0.0),
            t(660.0, 0.22, 0.07, 0.19, "sine", 0.0),
        ],
        "recharge" => {
            let lift = 120.0f64.min(30.0 * i);
            vec![
                t(180.0 + lift, 0.09, 0.10, 0.0, "square", 0.0),
                t(360.0 + lift, 0.14, 0.08, 0.08, "sine", 0.0),
            ]
        }
        "menu" => vec![
            t(620.0, 0.035, 0.026, 0.0, "square", 0.0),
            t(930.0, 0.055, 0.018, 0.022, "sine", 0.0),
        ],
        "music_box" => {
            let phrase = [1046.5, 1318.5, 1568.0, 1318.5, 1760.0, 1568.0];
            phrase
                .iter()
                .enumerate()
                .map(|(k, f)| t(*f, 0.22, 0.07, k as f64 * 0.2, "triangle", 0.0))
                .collect()
        }
        "music_box_tune" => {
            // the music box's loop (MusicBoxTune): Night Watch's Masked Hours tune, 13 sine notes 380 ms apart, each ringing
            // 0.5 s (music_box.rs holds the notes and the spacing). Each note starts on a whole frame of the grid (the half
            // frame keeps `delay * rate` from truncating one frame early), so the spacing is exactly constant. No breath:
            // `music_box::tune` folds the last note's ring onto the start so the loop has no seam.
            use super::music_box::{NOTE_RING_SECONDS, NOTE_STEP_FRAMES, TUNE_NOTES};
            TUNE_NOTES
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    let at = ((k * NOTE_STEP_FRAMES) as f64 + 0.5) / super::SAMPLE_RATE as f64;
                    t(*f, NOTE_RING_SECONDS, 0.14, at, "sine", 0.0)
                })
                .collect()
        }
        "glyra_hum" => {
            // Glyra humming along, low and soft (HunterView3D, only while her music plays)
            let hum = [261.6, 329.6, 392.0, 329.6, 440.0, 392.0, 329.6, 293.7];
            let mut c: Vec<Component> = hum
                .iter()
                .enumerate()
                .map(|(k, f)| t(*f, 0.5, 0.12, k as f64 * 0.45, "sine", 0.0))
                .collect();
            c.push(t(
                0.0,
                0.01,
                0.0,
                hum.len() as f64 * 0.45 + 0.6,
                "sine",
                0.0,
            ));
            c
        }
        "alert" => vec![
            t(150.0, 0.12, 0.12, 0.0, "saw", 0.0),
            t(96.0, 0.2, 0.10, 0.06, "square", 0.0),
        ],
        "chase_start" => vec![
            t(92.0, 0.16, 0.16, 0.0, "saw", 0.0),
            t(138.0, 0.19, 0.13, 0.07, "square", 0.0),
            t(207.0, 0.24, 0.10, 0.15, "saw", 0.0),
            n(0.24, 0.045, 860.0, 0.04, 0.0, 1.12),
            th(48.0, 0.30, 0.14, 0.19, 0.0),
        ],
        "taser" => vec![
            n(0.12, 0.16, 4200.0, 0.0, pan, 1.35),
            t(860.0, 0.07, 0.13, 0.0, "square", pan),
            t(430.0, 0.11, 0.10, 0.055, "saw", pan),
            th(76.0, 0.10, 0.06, 0.045, pan),
        ],
        "door_open" => vec![
            n(0.24, 0.038, 310.0, 0.0, pan, 0.46),
            t(118.0, 0.18, 0.032, 0.03, "triangle", pan),
            n(0.055, 0.055, 980.0, 0.19, pan, 0.82),
        ],
        "door_close" => vec![
            n(0.15, 0.042, 360.0, 0.0, pan, 0.56),
            th(82.0, 0.12, 0.085, 0.11, pan),
            t(145.0, 0.08, 0.038, 0.12, "square", pan),
        ],
        "door_breach" => vec![
            n(0.34, 0.18, 720.0, 0.0, pan, 0.88),
            th(54.0, 0.42, 0.23, 0.015, pan),
            t(96.0, 0.18, 0.13, 0.06, "saw", pan),
            n(0.20, 0.10, 1900.0, 0.16, pan, 1.18),
        ],
        "interaction_fail" => {
            vec![
                t(196.0, 0.075, 0.052, 0.0, "square", pan),
                t(132.0, 0.12, 0.045, 0.065, "square", pan),
            ]
        }
        "axe_swing" => vec![
            n(0.20, 0.05, 1250.0, 0.0, pan, 0.72),
            n(0.11, 0.035, 2300.0, 0.07, pan, 0.95),
        ],
        "axe_chop" => vec![
            th(64.0, 0.24, 0.20, 0.0, pan),
            n(0.14, 0.11, 760.0, 0.0, pan, 0.9),
            t(168.0, 0.09, 0.07, 0.02, "square", pan),
            n(0.09, 0.06, 2100.0, 0.08, pan, 1.1),
        ],
        "light_switch" => vec![
            t(2600.0, 0.012, 0.1, 0.0, "square", pan),
            th(140.0, 0.04, 0.08, 0.01, pan),
        ],
        "sleep_settle" => vec![
            n(0.5, 0.05, 400.0, 0.0, pan, 0.5),
            t(110.0, 0.4, 0.04, 0.1, "sine", pan),
        ],
        "sleep_yawn" => vec![
            t(220.0, 0.55, 0.07, 0.0, "triangle", pan),
            t(165.0, 0.5, 0.05, 0.35, "triangle", pan),
            n(0.4, 0.04, 900.0, 0.2, pan, 0.6),
        ],
        "sleep_stir" => vec![
            th(55.0, 0.3, 0.12, 0.0, pan),
            n(0.35, 0.05, 300.0, 0.05, pan, 0.5),
        ],
        "sleep_jolt" => vec![
            th(80.0, 0.18, 0.22, 0.0, pan),
            t(330.0, 0.12, 0.09, 0.0, "square", pan),
            n(0.12, 0.1, 1600.0, 0.02, pan, 1.0),
        ],
        // "sawtooth" is not a wave name the renderer knows: a sine, in the original too
        "hunter_snore" => vec![
            n(0.7, 0.07, 260.0, 0.0, pan, 0.7),
            t(70.0, 0.6, 0.06, 0.05, "sawtooth", pan),
        ],
        "hide_search" => vec![
            t(92.0, 0.42, 0.09, 0.0, "sawtooth", pan),
            n(0.3, 0.07, 520.0, 0.05, pan, 0.8),
            th(70.0, 0.16, 0.12, 0.18, pan),
            th(64.0, 0.16, 0.10, 0.46, pan),
        ],
        "tool_break" => vec![
            n(0.16, 0.095, 1850.0, 0.0, pan, 1.22),
            t(310.0, 0.08, 0.065, 0.025, "square", pan),
            t(118.0, 0.18, 0.052, 0.09, "triangle", pan),
        ],
        "nyxie_timer" => vec![
            t(72.0, 0.5, 0.09, 0.0, "sine", 0.0),
            t(108.0, 0.4, 0.07, 0.18, "triangle", 0.0),
            t(144.0, 0.3, 0.05, 0.34, "triangle", 0.0),
        ],
        "nyxie_empty" => vec![
            t(48.0, 0.7, 0.13, 0.0, "saw", 0.0),
            t(32.0, 0.9, 0.08, 0.1, "sine", 0.0),
        ],
        "hunter_laugh" => vec![
            t(392.0, 0.07, 0.05 * i, 0.0, "triangle", pan),
            t(349.0, 0.07, 0.045 * i, 0.10, "triangle", pan),
            t(311.0, 0.09, 0.04 * i, 0.20, "triangle", pan),
        ],
        "xivvy_screech" => vec![
            t(1250.0, 0.12, 0.07 * i, 0.0, "saw", pan),
            t(1760.0, 0.16, 0.08 * i, 0.07, "saw", pan),
            t(2350.0, 0.22, 0.075 * i, 0.16, "square", pan),
            n(0.34, 0.05 * i, 3400.0, 0.06, pan, 1.3),
        ],
        "radar_static" => vec![
            n(0.55, 0.09 * i, 2600.0, 0.0, pan, 1.4),
            n(0.35, 0.06 * i, 900.0, 0.12, pan, 1.1),
            t(980.0, 0.08, 0.03 * i, 0.4, "square", pan),
        ],
        "system_offline" => vec![
            t(260.0, 0.12, 0.06 * i, 0.0, "square", pan),
            t(180.0, 0.16, 0.06 * i, 0.1, "square", pan),
            t(110.0, 0.3, 0.07 * i, 0.22, "saw", pan),
        ],
        "signal_node" => {
            vec![
                t(420.0 * i, 0.07, 0.07, 0.0, "square", pan),
                t(630.0 * i, 0.05, 0.04, 0.05, "triangle", pan),
            ]
        }
        "xivvy_wing" => vec![
            n(0.13, 0.05 * i, 520.0, 0.0, pan, 0.9),
            th(120.0, 0.09, 0.03 * i, 0.02, pan),
        ],
        "spider_hiss" => vec![
            n(0.9, 0.16 * i, 520.0, 0.0, pan, 0.7),
            t(61.0, 0.8, 0.15 * i, 0.02, "saw", pan),
            n(0.55, 0.09 * i, 900.0, 0.25, pan, 1.1),
        ],
        "spider_bark" => vec![
            t(74.0, 0.30, 0.20 * i, 0.0, "saw", pan),
            t(55.0, 0.36, 0.18 * i, 0.04, "square", pan),
            n(0.22, 0.09 * i, 240.0, 0.02, pan, 0.8),
            t(96.0, 0.22, 0.15 * i, 0.46, "saw", pan),
            n(0.18, 0.07 * i, 300.0, 0.46, pan, 0.9),
        ],
        "splash_big" => vec![
            n(0.9, 0.22 * i, 700.0, 0.0, pan, 0.8),
            n(0.5, 0.16 * i, 2200.0, 0.02, pan, 1.2),
            th(70.0, 0.35, 0.16 * i, 0.0, pan),
            n(0.7, 0.08 * i, 1400.0, 0.25, pan, 1.0),
        ],
        "music_mic" => vec![
            t(2100.0, 0.5, 0.05 * i, 0.0, "sine", pan),
            t(2800.0, 0.45, 0.035 * i, 0.08, "sine", pan),
            th(90.0, 0.12, 0.1 * i, 0.0, pan),
        ],
        "music_podium" => vec![
            th(110.0, 0.1, 0.14 * i, 0.0, pan),
            th(110.0, 0.1, 0.12 * i, 0.28, pan),
            n(0.06, 0.03 * i, 1800.0, 0.0, pan, 1.0),
        ],
        "music_drums" => vec![
            th(60.0, 0.3, 0.2 * i, 0.0, pan),
            n(0.14, 0.12 * i, 1900.0, 0.22, pan, 1.0),
            th(60.0, 0.3, 0.18 * i, 0.44, pan),
            n(0.12, 0.11 * i, 1900.0, 0.66, pan, 1.0),
            n(0.9, 0.07 * i, 6200.0, 0.66, pan, 1.4),
        ],
        "music_electric" => vec![
            t(82.4, 0.9, 0.07 * i, 0.0, "saw", pan),
            t(123.5, 0.9, 0.06 * i, 0.01, "saw", pan),
            t(164.8, 0.9, 0.05 * i, 0.02, "square", pan),
        ],
        "music_acoustic" => vec![
            t(196.0, 0.8, 0.05 * i, 0.0, "triangle", pan),
            t(247.0, 0.75, 0.05 * i, 0.04, "triangle", pan),
            t(294.0, 0.7, 0.05 * i, 0.08, "triangle", pan),
            t(392.0, 0.65, 0.045 * i, 0.12, "triangle", pan),
        ],
        "music_bass" => vec![
            t(55.0, 0.7, 0.14 * i, 0.0, "saw", pan),
            t(110.0, 0.5, 0.04 * i, 0.0, "triangle", pan),
        ],
        "music_keys" => vec![
            t(261.6, 0.7, 0.05 * i, 0.0, "square", pan),
            t(329.6, 0.7, 0.045 * i, 0.0, "square", pan),
            t(392.0, 0.7, 0.045 * i, 0.0, "square", pan),
        ],
        "music_piano" => vec![
            t(220.0, 1.1, 0.07 * i, 0.0, "triangle", pan),
            t(277.2, 1.0, 0.06 * i, 0.02, "triangle", pan),
            t(329.6, 0.95, 0.06 * i, 0.04, "triangle", pan),
            t(440.0, 0.8, 0.03 * i, 0.04, "sine", pan),
        ],
        "water_drip" => vec![
            t(980.0, 0.05, 0.05 * i, 0.0, "triangle", pan),
            t(640.0, 0.09, 0.04 * i, 0.04, "triangle", pan),
            n(0.05, 0.03 * i, 3000.0, 0.0, pan, 1.2),
        ],
        "spider_skitter" => vec![
            n(0.04, 0.07 * i, 3100.0, 0.0, pan, 1.3),
            n(0.04, 0.06 * i, 2700.0, 0.07, pan, 1.3),
            n(0.04, 0.065 * i, 3300.0, 0.13, pan, 1.3),
            n(0.04, 0.055 * i, 2500.0, 0.21, pan, 1.3),
            n(0.04, 0.06 * i, 3000.0, 0.26, pan, 1.3),
            th(150.0, 0.05, 0.03 * i, 0.0, pan),
        ],
        "inferno_crackle" => vec![
            n(0.5, 0.05 * i, 1500.0, 0.0, pan, 1.4),
            n(0.06, 0.08 * i, 2600.0, 0.12, pan, 1.5),
            n(0.05, 0.07 * i, 2200.0, 0.31, pan, 1.5),
            th(70.0, 0.22, 0.06 * i, 0.4, pan),
            t(46.0, 0.6, 0.05 * i, 0.38, "triangle", pan),
        ],
        "pit_rumble" => vec![
            t(38.0, 1.1, 0.07 * i, 0.0, "triangle", pan),
            n(0.9, 0.035 * i, 380.0, 0.1, pan, 1.2),
            th(52.0, 0.35, 0.06 * i, 0.5, pan),
        ],
        "ladder_rung" => vec![
            t(540.0, 0.07, 0.05 * i, 0.0, "square", pan),
            t(820.0, 0.1, 0.03 * i, 0.01, "triangle", pan),
            th(120.0, 0.05, 0.03 * i, 0.0, pan),
        ],
        "hunter_footstep" => vec![
            th(58.0, 0.10, 0.05 * i, 0.0, pan),
            th(41.0, 0.13, 0.035 * i, 0.05, pan),
        ],
        "teammate_footstep_sprint" => {
            vec![
                n(0.07, 0.066 * i, 610.0, 0.0, pan, 1.15),
                th(92.0, 0.065, 0.045 * i, 0.0, pan),
            ]
        }
        "teammate_footstep_crouch" => {
            vec![
                n(0.045, 0.012 * i, 430.0, 0.0, pan, 0.92),
                th(68.0, 0.045, 0.009 * i, 0.0, pan),
            ]
        }
        "teammate_footstep_walk" => {
            vec![
                n(0.06, 0.035 * i, 525.0, 0.0, pan, 1.03),
                th(82.0, 0.06, 0.028 * i, 0.0, pan),
            ]
        }
        "mimic" => vec![
            t(132.0, 0.2, 0.24, 0.0, "saw", 0.0),
            t(82.0, 0.32, 0.21, 0.06, "square", 0.0),
        ],
        "mimic_rare" => vec![
            t(1046.5, 0.28, 0.2, 0.0, "triangle", 0.0),
            t(1568.0, 0.34, 0.18, 0.1, "triangle", 0.0),
        ],
        "heartbeat" => {
            // the double pulse, centred above sub-bass with a short midrange body (audible on laptop speakers)
            let pulse_volume = 0.11 + i * 0.14;
            vec![
                th(96.0 + i * 20.0, 0.085, pulse_volume, 0.0, 0.0),
                t(
                    182.0 + i * 28.0,
                    0.07,
                    pulse_volume * 0.38,
                    0.012,
                    "triangle",
                    0.0,
                ),
                th(82.0 + i * 14.0, 0.10, pulse_volume * 0.76, 0.12, 0.0),
                t(
                    154.0 + i * 22.0,
                    0.075,
                    pulse_volume * 0.28,
                    0.135,
                    "triangle",
                    0.0,
                ),
            ]
        }
        "footstep_sprint" => vec![
            n(0.085, 0.105, 470.0, 0.0, pan, 1.08),
            th(69.0, 0.085, 0.085, 0.0, pan),
        ],
        "footstep_crouch" => vec![
            n(0.055, 0.017, 285.0, 0.0, pan, 0.86),
            th(43.0, 0.055, 0.012, 0.0, pan),
        ],
        "footstep_walk" => vec![
            n(0.07, 0.052, 385.0, 0.0, pan, 0.96),
            th(55.0, 0.07, 0.042, 0.0, pan),
        ],
        "capture" => vec![
            t(92.0, 0.26, 0.10, 0.0, "triangle", 0.0),
            t(61.0, 0.44, 0.085, 0.16, "sine", 0.0),
            t(41.0, 0.62, 0.06, 0.36, "sine", 0.0),
        ],
        "struggle" => vec![
            n(
                0.045,
                0.025 + i * 0.045,
                330.0 + i * 210.0,
                0.0,
                pan,
                0.92 + i * 0.16,
            ),
            th(72.0 + i * 22.0, 0.055, 0.025 + i * 0.045, 0.0, 0.0),
        ],
        "escape" => vec![
            t(174.0, 0.08, 0.08, 0.0, "triangle", 0.0),
            t(261.0, 0.10, 0.075, 0.07, "triangle", 0.0),
            t(392.0, 0.14, 0.055, 0.15, "sine", 0.0),
        ],
        "office_camera" => vec![
            n(0.065, 0.04, 1250.0, 0.0, pan, 1.18),
            t(690.0, 0.045, 0.045, 0.025, "square", pan),
            t(430.0, 0.11, 0.035, 0.075, "triangle", pan),
        ],
        "office_warning" => vec![
            th(68.0, 0.1, 0.09, 0.0, pan),
            t(118.0, 0.27, 0.075, 0.055, "saw", pan),
            n(0.09, 0.04, 520.0, 0.13, pan, 1.0),
        ],
        "office_crimson_door" => {
            let mut c = Vec::new();
            for index in 0..3i64 {
                let fi = index as f64;
                c.push(th(
                    (56 + index * 5) as f64,
                    0.065,
                    0.055 + fi * 0.008,
                    fi * 0.16,
                    pan,
                ));
                c.push(t(
                    [132.0, 158.0, 190.0][index as usize],
                    0.12,
                    0.045 + fi * 0.007,
                    fi * 0.16 + 0.018,
                    "square",
                    pan,
                ));
            }
            c.push(n(0.07, 0.026, 620.0, 0.37, pan, 0.9));
            c
        }
        "office_presence" => vec![
            n(0.11, 0.016, 330.0, 0.0, pan, 0.72),
            t(154.0, 0.16, 0.022, 0.035, "triangle", pan),
            th(58.0, 0.08, 0.024, 0.1, pan),
        ],
        "office_scrape" => vec![
            n(0.18, 0.018, 240.0, 0.0, pan, 0.48),
            n(0.055, 0.012, 1160.0, 0.15, pan, 0.66),
        ],
        "office_static" => vec![
            n(0.16, 0.026, 1650.0, 0.0, pan, 1.45),
            t(410.0, 0.11, 0.018, 0.035, "square", pan),
        ],
        "office_drain" => vec![
            t(190.0, 0.22, 0.026, 0.0, "saw", pan),
            t(104.0, 0.18, 0.024, 0.1, "triangle", pan),
            n(0.06, 0.013, 760.0, 0.19, pan, 0.7),
        ],
        "office_footsteps" => vec![
            th(92.0, 0.045, 0.022, 0.0, pan),
            th(78.0, 0.055, 0.02, 0.15, pan),
            th(84.0, 0.04, 0.017, 0.29, pan),
        ],
        "office_breath" => vec![
            n(0.38, 0.014 * i, 210.0, 0.0, pan, 0.34),
            t(52.0, 0.4, 0.02 * i, 0.05, "sine", pan),
            n(0.55, 0.017 * i, 150.0, 0.48, pan, 0.26),
            t(40.0, 0.58, 0.024 * i, 0.52, "sine", pan),
        ],
        "office_shift" => vec![
            n(0.11, 0.035, 900.0, 0.0, 0.0, 0.84),
            t(210.0, 0.2, 0.045, 0.07, "triangle", 0.0),
            t(315.0, 0.22, 0.035, 0.2, "triangle", 0.0),
        ],
        "office_control" => vec![
            t(265.0, 0.08, 0.045, 0.0, "square", 0.0),
            n(0.045, 0.025, 720.0, 0.025, pan, 1.0),
        ],
        "office_jumpscare" => {
            let tint = (i - 1.0) * 900.0;
            vec![
                n(0.58, 0.24, 1320.0 + tint, 0.0, 0.0, 0.72),
                th(72.0, 0.34, 0.22, 0.015, 0.0),
                t(740.0 + tint, 0.13, 0.15, 0.025, "saw", 0.0),
                t(360.0 + tint * 0.4, 0.46, 0.13, 0.09, "square", 0.0),
                t(56.0, 0.72, 0.1, 0.12, "sine", 0.0),
            ]
        }
        _ => return None,
    };
    Some(c)
}
