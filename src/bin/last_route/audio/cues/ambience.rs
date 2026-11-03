//! Ambient beds: what the original really plays on the finite route is nothing.
//!
//! The original does have an ambience generator (game/audio/ambience_synth.gd, beds from data/ambience/*.json via
//! ambience_table.gd): 10 s loops of room tone, resonant hums, drips/ticks, chirps, wind, distant thuds, metal creaks,
//! pipe groans, running water and ember crackle, with a small Schroeder reverb, all driven by Godot's
//! RandomNumberGenerator seeded with hash(key). On top of that LevelAmbience (game/audio/level_ambience.gd) adds a
//! per-level bed, a "Level" bus reverb and positional loops (EmitterLoops / FallsLoop: waterfalls, floods, outflows,
//! lava).
//!
//! None of it is reached from the finite route (game/main_3d.gd, the solo floor-by-floor run this port is):
//! - `Sound.set_ambience` is called only by game/infinite_route/infinite_route_test.gd (1754, 7937);
//! - `LevelAmbience` is created only there (infinite_route_test.gd 193), so no level bed, no Level bus, no emitters,
//!   and `Sound.footstep_bus` stays "SFX";
//! - `Sound.prerender_ambience` (platform_profile.gd 108) only pre-renders beds at boot, it never plays one.
//!
//! So on the finite route the only continuous sounds are the music box tune (see music_box.rs) and Glyra's hum loop
//! (`glyra_hum`, looped), both positional. This function therefore returns silence of the requested length, which is
//! exactly what the original plays under the finite route. If the port ever grows the infinite route, port
//! AmbienceSynth.render then (it needs Godot's PCG32 RandomNumberGenerator, randf/randf_range/randi_range in f32).

use super::render::SAMPLE_RATE;

/// A loopable bed for `region` lasting `seconds`. Always silent: no bed plays on the finite route (see module docs).
pub fn ambience(_region: &str, seconds: f32) -> Vec<[f32; 2]> {
    let frames = ((seconds.max(0.0) as f64) * SAMPLE_RATE as f64)
        .round()
        .max(1.0) as usize;
    vec![[0.0, 0.0]; frames]
}
