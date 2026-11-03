//! Floor surfaces and the surface footstep: a port of SurfaceSounds (game/audio/surface_sounds.gd) and
//! `Sound._surface_step` (game/autoload/sound.gd 817-832).

use super::render::{noise, thump, tone, Component};

/// SurfaceSounds.SURFACES (surface_sounds.gd 8), in the original's order.
pub const SURFACES: [&str; 8] = [
    "carpet", "wood", "tile", "concrete", "metal", "stone", "grass", "water",
];

/// The step gaits `_surface_step` knows ("hunter" is a hunter's step on a floor, played scaled).
pub const GAITS: [&str; 4] = ["walk", "sprint", "crouch", "hunter"];

/// A surface's step shape (SurfaceSounds.PROFILES, surface_sounds.gd 31-40). Missing optional keys are 0.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub noise_hz: f64,
    pub noise: f64,
    pub thump_hz: f64,
    pub thump: f64,
    pub click: f64,
    pub ring: f64,
    pub splash: f64,
}

const fn p(
    noise_hz: f64,
    noise: f64,
    thump_hz: f64,
    thump: f64,
    click: f64,
    ring: f64,
    splash: f64,
) -> Profile {
    Profile {
        noise_hz,
        noise,
        thump_hz,
        thump,
        click,
        ring,
        splash,
    }
}

/// The profile of a surface; an unknown name steps like concrete (sound.gd 820).
pub fn profile(surface: &str) -> Profile {
    match surface {
        "carpet" => p(240.0, 0.55, 48.0, 0.7, 0.0, 0.0, 0.0),
        "wood" => p(330.0, 0.55, 92.0, 1.25, 0.0, 0.0, 0.0),
        "tile" => p(900.0, 0.7, 60.0, 0.6, 1.0, 0.0, 0.0),
        "metal" => p(760.0, 0.8, 110.0, 0.8, 0.7, 1.0, 0.0),
        "stone" => p(480.0, 0.95, 58.0, 0.85, 0.4, 0.0, 0.0),
        "grass" => p(1400.0, 0.6, 40.0, 0.45, 0.0, 0.0, 0.0),
        "water" => p(1100.0, 1.2, 70.0, 0.5, 0.0, 0.0, 1.0),
        _ => p(560.0, 1.15, 55.0, 0.9, 0.0, 0.0, 0.0), // concrete
    }
}

/// SurfaceSounds.FLOOR_KINDS (surface_sounds.gd 10-28): RefTextures floor kind -> surface; unknown kinds are concrete.
pub fn for_floor(kind: &str) -> &'static str {
    match kind {
        "metal0" | "metal1" | "metal2" | "metal3" | "grate" | "hazard" | "safe" | "safe_plain"
        | "safe_mark" => "metal",
        "floral0" | "floral1" | "star0" | "star1" | "carpet_red" | "carpet_teal"
        | "carpet_green" | "carpet_plum" | "carpet_tile0" | "carpet_tile1" | "runner"
        | "runnerz" | "runner_l" | "runner_c" | "runner_r" | "runnerz_l" | "runnerz_c"
        | "runnerz_r" => "carpet",
        "parquet0" | "parquet1" | "driftwood0" | "driftwood1" => "wood",
        "sand0" | "sand1" | "wetsand0" => "grass",
        "check0" | "check1" | "vinyl0" | "vinyl1" | "epoxy0" | "epoxy1" | "terrazzo0"
        | "terrazzo1" => "tile",
        "stone0" | "stone1" | "stone2" | "rock0" | "rock1" | "rock2" | "terracotta0"
        | "terracotta1" => "stone",
        _ => "concrete",
    }
}

/// `"step:<gait>:<surface>:<variant>"` -> components. `None` if the cue does not have the four parts (the original would
/// index out of range there).
pub fn surface_step(cue: &str, pan: f64, intensity: f64) -> Option<Vec<Component>> {
    let parts: Vec<&str> = cue.split(':').collect();
    if parts.len() < 4 {
        return None;
    }
    let gait = parts[1];
    let pr = profile(parts[2]);
    // int("x") of a non-number is 0 in GDScript
    let variant = parts[3].trim().parse::<i64>().unwrap_or(0) as f64;
    let pitch = 1.0 + (variant - 1.5) * 0.035;
    let base = match gait {
        "sprint" => 0.095,
        "crouch" => 0.017,
        "hunter" => 0.045,
        _ => 0.05,
    };
    let level = base * intensity;
    let length = match gait {
        "sprint" => 0.085,
        "crouch" => 0.055,
        "hunter" => 0.08,
        _ => 0.07,
    };
    let mut c = vec![
        noise(
            length,
            level * pr.noise,
            pr.noise_hz * pitch,
            0.0,
            pan,
            pitch,
        ),
        thump(
            pr.thump_hz * pitch,
            length,
            level * 0.8 * pr.thump,
            0.0,
            pan,
        ),
    ];
    if pr.click > 0.0 {
        c.push(tone(
            2400.0 * pitch,
            0.012,
            level * 0.35 * pr.click,
            0.0,
            "square",
            pan,
        ));
    }
    if pr.ring > 0.0 {
        c.push(tone(
            620.0 * pitch,
            0.14,
            level * 0.22,
            0.005,
            "triangle",
            pan,
        ));
    }
    if pr.splash > 0.0 {
        c.push(noise(0.16, level * 0.6, 1800.0 * pitch, 0.02, pan, 1.3));
    }
    Some(c)
}
