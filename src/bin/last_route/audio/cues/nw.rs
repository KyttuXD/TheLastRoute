//! Night Watch's cues (`Sound._nw_components`, game/autoload/sound.gd 854-898), played as "nw_<name>" through
//! `Sound.nw(name, pan, intensity)`. They belong to the Night Watch minigame, not the solo route, and are ported only so
//! the table is complete. `k` is the intensity.

use super::render::{noise as n, tone as t, Component};

pub const NAMES: &[&str] = &[
    "move",
    "pick",
    "deny",
    "burst",
    "step",
    "clank",
    "bound",
    "pad",
    "ring",
    "lure",
    "crackle",
    "hum",
    "hum_muffled",
    "breath",
    "talk",
    "door",
    "light_on",
    "camera",
    "scare",
    "scare_short",
    "power_out",
    "chime",
    "tape",
    "radio",
    "rustle",
    "zip",
    "clunk",
    "glass_tap",
    "hinge",
    "latch",
    "squeak",
    "bed_breath",
    "bed_step",
    "bed_leave",
    "bed_stir",
    "bed_restless",
];

pub fn components(name: &str, pan: f64, k: f64) -> Option<Vec<Component>> {
    let c: Vec<Component> = match name {
        "move" => vec![t(520.0, 0.04, 0.08, 0.0, "square", pan)],
        "pick" => vec![
            t(330.0, 0.08, 0.08, 0.0, "square", pan),
            t(660.0, 0.12, 0.08, 0.0, "square", pan),
        ],
        "deny" => vec![t(110.0, 0.18, 0.08, 0.0, "triangle", pan)],
        "burst" => vec![t(110.0, 0.2, 0.05, 0.0, "sine", pan)],
        "step" => vec![
            t(60.0, 0.09, 0.35 * k, 0.0, "sine", pan),
            n(0.05, 0.35 * k * 0.45, 600.0, 0.0, pan, 1.0),
        ],
        "clank" => vec![
            t(310.0, 0.1, 0.08, 0.0, "triangle", pan),
            t(233.0, 0.16, 0.06, 0.0, "triangle", pan),
            n(0.06, 0.08, 2500.0, 0.0, pan, 1.0),
        ],
        "bound" => (0..3)
            .map(|i| {
                t(
                    [700.0, 830.0, 990.0][i],
                    0.045,
                    0.075 * k,
                    i as f64 * 0.075,
                    "triangle",
                    pan,
                )
            })
            .collect(),
        "pad" => vec![
            t(92.0, 0.2, 0.12 * k, 0.0, "sine", pan),
            t(184.0, 0.12, 0.06 * k, 0.0, "sine", pan),
            n(0.14, 0.12 * k * 0.6, 520.0, 0.0, pan, 1.0),
        ],
        "ring" => {
            let mut c = Vec::new();
            for i in 0..4 {
                c.push(t(880.0, 0.12, 0.2, i as f64 * 0.18, "sine", pan));
                c.push(t(1175.0, 0.12, 0.16, i as f64 * 0.18, "sine", pan));
            }
            c
        }
        "lure" => vec![
            t(880.0, 0.12, 0.07, 0.0, "triangle", pan),
            t(660.0, 0.12, 0.07, 0.13, "triangle", pan),
            t(494.0, 0.25, 0.07, 0.26, "triangle", pan),
        ],
        "crackle" => vec![
            n(0.18, 0.32, 5200.0, 0.0, pan, 1.0),
            n(0.12, 0.22, 3800.0, 0.14, pan, 1.0),
            t(47.0, 0.35, 0.05, 0.0, "saw", pan),
        ],
        "hum" => vec![
            t(82.0, 0.6, 0.12, 0.0, "sine", pan),
            t(123.0, 0.45, 0.04, 0.0, "sine", pan),
        ],
        "hum_muffled" => vec![
            t(82.0, 0.7, 0.045, 0.0, "sine", pan),
            t(123.0, 0.5, 0.008, 0.0, "sine", pan),
        ],
        "breath" => vec![
            n(0.4, 0.2 * k, 700.0, 0.0, pan, 1.0),
            n(0.35, 0.2 * k * 0.8, 500.0, 0.43, pan, 1.0),
        ],
        "talk" => vec![
            t(470.0, 0.09, 0.26, 0.0, "triangle", pan),
            t(705.0, 0.06, 0.1, 0.0, "sine", pan),
        ],
        "door" => vec![
            n(0.25, 0.5, 600.0, 0.0, pan, 1.0),
            t(70.0, 0.2, 0.15, 0.0, "square", pan),
        ],
        "light_on" => vec![t(180.0, 0.3, 0.03, 0.0, "triangle", pan)],
        "camera" => vec![t(900.0, 0.05, 0.04, 0.0, "square", pan)],
        "scare" => vec![
            n(1.2, 0.9, 6000.0, 0.0, pan, 1.0),
            t(140.0, 1.1, 0.3, 0.0, "saw", pan),
            t(97.0, 1.1, 0.2, 0.0, "square", pan),
        ],
        "scare_short" => vec![
            n(0.65, 0.9, 6000.0, 0.0, pan, 1.0),
            t(140.0, 0.55, 0.3, 0.0, "saw", pan),
            t(97.0, 0.55, 0.2, 0.0, "square", pan),
        ],
        "power_out" => vec![
            t(200.0, 1.2, 0.15, 0.0, "saw", pan),
            n(0.6, 0.3, 400.0, 0.0, pan, 1.0),
        ],
        "chime" => vec![
            t(523.0, 0.4, 0.15, 0.0, "sine", pan),
            t(784.0, 0.8, 0.15, 0.0, "sine", pan),
        ],
        "tape" => vec![
            t(210.0, 0.6, 0.135 * k, 0.0, "sine", pan),
            t(211.0, 0.6, 0.075 * k, 0.0, "triangle", pan),
            n(0.5, 0.054 * k, 3200.0, 0.0, pan, 1.0),
        ],
        "radio" => vec![
            t(1050.0, 0.12, 0.042 * k, 0.0, "square", pan),
            n(0.2, 0.105 * k, 2400.0, 0.0, pan, 1.0),
            t(640.0, 0.04, 0.036 * k, 0.17, "square", pan),
        ],
        "rustle" => vec![
            n(0.18, 0.25 * k, 1400.0, 0.0, pan, 1.0),
            n(0.22, 0.2 * k, 1100.0, 0.24, pan, 1.0),
        ],
        "zip" => vec![
            n(0.3, 0.12, 4200.0, 0.0, pan, 1.0),
            t(1500.0, 0.05, 0.02, 0.0, "square", pan),
        ],
        "clunk" => vec![
            t(84.0, 0.16, 0.1, 0.0, "sine", pan),
            n(0.08, 0.12, 320.0, 0.0, pan, 1.0),
        ],
        "glass_tap" => vec![t(2100.0, 0.04, 0.05, 0.0, "sine", pan)],
        "hinge" => vec![
            t(150.0, 0.5, 0.06, 0.0, "sine", pan),
            n(0.5, 0.08, 480.0, 0.0, pan, 1.0),
        ],
        "latch" => vec![
            t(1300.0, 0.02, 0.045, 0.0, "square", pan),
            n(0.03, 0.06, 2600.0, 0.0, pan, 1.0),
        ],
        "squeak" => vec![t(2900.0, 0.12, 0.035, 0.0, "sine", pan)],
        "bed_breath" => vec![
            n(0.4, 0.2 * k * 7.0, 950.0, 0.0, pan, 1.0),
            n(0.35, 0.2 * k * 5.6, 800.0, 0.43, pan, 1.0),
        ],
        "bed_step" => vec![
            t(60.0, 0.09, 0.16 * k, 0.0, "sine", pan),
            t(220.0, 0.09, 0.16 * k * 4.0, 0.0, "triangle", pan),
            n(0.05, 0.16 * k * 18.0, 950.0, 0.0, pan, 1.0),
        ],
        "bed_leave" => vec![
            n(0.25, 3.5 * 0.12, 950.0, 0.0, pan, 1.0),
            t(240.0, 0.2, 0.7 * 0.3, 0.0, "triangle", pan),
        ],
        "bed_stir" => vec![
            t(310.0, 0.1, 0.56, 0.0, "triangle", pan),
            t(233.0, 0.16, 0.42, 0.0, "triangle", pan),
            n(0.06, 0.56, 1800.0, 0.0, pan, 1.0),
        ],
        "bed_restless" => vec![
            n(0.4, 0.22 * 7.0, 950.0, 0.0, pan, 1.0),
            n(0.35, 0.22 * 5.6, 800.0, 0.43, pan, 1.0),
            t(294.0, 0.4, 0.08, 0.0, "triangle", pan),
            t(311.0, 0.4, 0.08, 0.0, "triangle", pan),
        ],
        _ => return None,
    };
    Some(c)
}
