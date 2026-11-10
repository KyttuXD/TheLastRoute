//! Weather for open-sky maps: a small state machine (current kind, a target it blends to, scheduled changes picked by
//! weight from a seeded generator, wind, storm lightning) and pure functions for the look (sky tint, fog, particle
//! counts and particle positions). No drawing and no clock of its own: `Game` ticks it, `main.rs` reads it.

use crate::model::{WeatherDef, WeatherKind};
use crate::rpg::Rng;

impl WeatherKind {
    /// The word the HUD shows under the map name.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            WeatherKind::Clear => "Clear",
            WeatherKind::Rain => "Rain",
            WeatherKind::Snow => "Snow",
            WeatherKind::Fog => "Fog",
            WeatherKind::Storm => "Storm",
            WeatherKind::Leaves => "Windy",
        }
    }
}

/// Intensity used when a pack gives none.
pub const DEFAULT_INTENSITY: f32 = 0.6;
/// Seconds between possible changes when a pack gives none.
pub const DEFAULT_EVERY_SECS: f32 = 90.0;
/// Seconds one change takes to blend in.
pub const BLEND_SECS: f32 = 8.0;
/// Particles are placed in a box this many metres around the camera (half width) and this tall.
#[allow(dead_code)]
pub const AREA_R: f32 = 14.0;
#[allow(dead_code)]
pub const AREA_H: f32 = 9.0;
/// Above this much rain, snow or storm, people look for shelter.
const WET_LEVEL: f32 = 0.3;
/// Storm lightning needs at least this much storm.
const STORM_LEVEL: f32 = 0.3;

#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    /// What the weather is now (the "from" side of a blend).
    pub kind: WeatherKind,
    pub intensity: f32,
    /// Where it is heading. Equal to (kind, intensity) when nothing is changing.
    pub target: (WeatherKind, f32),
    /// 0..1 progress from the current look to the target.
    pub blend: f32,
    /// Seconds since the last scheduled change.
    pub timer: f32,
    /// Lightning brightness 0..1, decaying fast.
    pub flash: f32,
    /// Wind in metres per second on (x, z).
    pub wind: (f32, f32),
    /// Seconds this weather has run (drives the particles and the wind).
    pub time: f32,
    /// The intensity scheduled changes use.
    pub base_intensity: f32,
    /// Kinds a scheduled change may pick, with weights.
    pub changes: Vec<(WeatherKind, f32)>,
    pub every_secs: f32,
    next_flash: f32,
    phase: f32,
    rng: Rng,
}

fn unit(rng: &mut Rng) -> f32 {
    rng.next_u32() as f32 / u32::MAX as f32
}

impl Weather {
    /// A weather for the map `map_id`, seeded from the id so the same map always changes the same way.
    pub fn new(map_id: &str, kind: WeatherKind, intensity: f32, changes: &[(WeatherKind, f32)], every_secs: f32) -> Weather {
        let mut rng = Rng::new(Rng::seed_from(map_id));
        let intensity = intensity.clamp(0.0, 1.0);
        let next_flash = 2.0 + unit(&mut rng) * 6.0;
        let phase = unit(&mut rng) * 6.28;
        Weather {
            kind,
            intensity,
            target: (kind, intensity),
            blend: 0.0,
            timer: 0.0,
            flash: 0.0,
            wind: (0.0, 0.0),
            time: 0.0,
            base_intensity: if kind == WeatherKind::Clear { DEFAULT_INTENSITY } else { intensity },
            changes: changes.to_vec(),
            every_secs: every_secs.max(1.0),
            next_flash,
            phase,
            rng,
        }
    }

    /// The weather a map's `"weather"` block describes.
    pub fn from_def(map_id: &str, def: &WeatherDef) -> Weather {
        Weather::new(map_id, def.kind, def.intensity, &def.changes, def.every_secs)
    }

    /// How much of `kind` shows right now (0..1), counting the blend.
    pub fn level(&self, kind: WeatherKind) -> f32 {
        if kind == WeatherKind::Clear {
            return 0.0;
        }
        let mut l = 0.0;
        if self.kind == kind {
            l += self.intensity * (1.0 - self.blend);
        }
        if self.target.0 == kind {
            l += self.target.1 * self.blend;
        }
        l
    }

    /// Whether it is wet or snowy enough that people take shelter.
    pub fn is_wet(&self) -> bool {
        self.level(WeatherKind::Rain) + self.level(WeatherKind::Snow) + self.level(WeatherKind::Storm) > WET_LEVEL
    }

    /// The word for the HUD: the kind that is mostly showing.
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        if self.blend > 0.5 { self.target.0.label() } else { self.kind.label() }
    }

    /// Heads for a new weather, blending in over [`BLEND_SECS`]. A change under way is first settled to whichever side
    /// is nearer, so the blend never jumps back.
    pub fn set_target(&mut self, kind: WeatherKind, intensity: f32) {
        if self.blend >= 0.5 {
            self.kind = self.target.0;
            self.intensity = self.target.1;
        }
        self.blend = 0.0;
        self.target = (kind, if kind == WeatherKind::Clear { 0.0 } else { intensity.clamp(0.0, 1.0) });
        if self.target == (self.kind, self.intensity) {
            self.blend = 0.0;
        }
    }

    /// The `weather` op: sets the weather (blending in) and restarts the change schedule.
    pub fn set(&mut self, kind: WeatherKind, intensity: f32) {
        self.set_target(kind, intensity);
        if kind != WeatherKind::Clear {
            self.base_intensity = intensity.clamp(0.0, 1.0);
        }
        self.timer = 0.0;
    }

    /// Picks the next kind by weight from `changes` and heads for it (nothing happens with no changes listed).
    fn pick_change(&mut self) {
        let total: f32 = self.changes.iter().map(|c| c.1.max(0.0)).sum();
        if total <= 0.0 {
            return;
        }
        let mut roll = unit(&mut self.rng) * total;
        let mut chosen = self.changes[self.changes.len() - 1].0;
        for (k, w) in &self.changes {
            let w = w.max(0.0);
            if roll < w {
                chosen = *k;
                break;
            }
            roll -= w;
        }
        if chosen != self.target.0 {
            self.set_target(chosen, self.base_intensity);
        }
    }

    /// Advances the clock: scheduled changes, the blend, the wind and the lightning.
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.5);
        self.time += dt;
        self.timer += dt;
        if self.timer >= self.every_secs {
            self.timer = 0.0;
            self.pick_change();
        }
        if self.target != (self.kind, self.intensity) {
            self.blend += dt / BLEND_SECS;
            if self.blend >= 1.0 {
                self.kind = self.target.0;
                self.intensity = self.target.1;
                self.blend = 0.0;
            }
        } else {
            self.blend = 0.0;
        }
        // Wind drifts slowly; storms and rain push harder.
        let push = 0.6 + 2.4 * self.level(WeatherKind::Storm) + 0.9 * self.level(WeatherKind::Rain) + 1.0 * self.level(WeatherKind::Leaves);
        self.wind = ((self.time * 0.07 + self.phase).sin() * push, (self.time * 0.05 + self.phase * 1.7).cos() * push * 0.6);
        // Lightning: only while there is storm.
        self.flash = (self.flash - dt * 2.5).max(0.0);
        if self.level(WeatherKind::Storm) > STORM_LEVEL {
            self.next_flash -= dt;
            if self.next_flash <= 0.0 {
                self.flash = 1.0;
                self.next_flash = 3.0 + unit(&mut self.rng) * 9.0;
            }
        }
    }
}

/// How each kind tints the sky at full strength: a colour multiplier and a fog density.
fn look(kind: WeatherKind) -> ([f32; 3], f32) {
    match kind {
        WeatherKind::Clear => ([1.0, 1.0, 1.0], 0.0),
        WeatherKind::Rain => ([0.72, 0.76, 0.84], 0.25),
        WeatherKind::Snow => ([0.88, 0.91, 1.0], 0.3),
        WeatherKind::Fog => ([0.78, 0.8, 0.82], 0.8),
        WeatherKind::Storm => ([0.42, 0.45, 0.55], 0.4),
        WeatherKind::Leaves => ([1.0, 0.94, 0.84], 0.05),
    }
}

const SHOWN: [WeatherKind; 5] = [WeatherKind::Rain, WeatherKind::Snow, WeatherKind::Fog, WeatherKind::Storm, WeatherKind::Leaves];

/// The colour to multiply the sky (and the light) by, and the fog density 0..1. A lightning flash brightens the colour.
pub fn sky_tint(w: &Weather) -> ([f32; 3], f32) {
    let mut mul = [1.0f32; 3];
    let mut fog = 0.0;
    for k in SHOWN {
        let l = w.level(k);
        if l <= 0.0 {
            continue;
        }
        let (c, f) = look(k);
        for i in 0..3 {
            mul[i] += l * (c[i] - 1.0);
        }
        fog += l * f;
    }
    for m in mul.iter_mut() {
        *m = (*m + w.flash * 0.6).clamp(0.0, 1.6);
    }
    (mul, fog.clamp(0.0, 1.0))
}

/// The most particles each kind draws at full intensity.
fn max_particles(kind: WeatherKind) -> f32 {
    match kind {
        WeatherKind::Rain => 700.0,
        WeatherKind::Storm => 1000.0,
        WeatherKind::Snow => 450.0,
        WeatherKind::Leaves => 60.0,
        _ => 0.0,
    }
}

/// How many particles of each kind to draw now (rain, storm, snow, leaves), counting the blend.
pub fn layers(w: &Weather) -> [(WeatherKind, usize); 4] {
    let n = |k: WeatherKind| (w.level(k) * max_particles(k)).round().max(0.0) as usize;
    [
        (WeatherKind::Rain, n(WeatherKind::Rain)),
        (WeatherKind::Storm, n(WeatherKind::Storm)),
        (WeatherKind::Snow, n(WeatherKind::Snow)),
        (WeatherKind::Leaves, n(WeatherKind::Leaves)),
    ]
}

/// The total particles to draw now.
#[cfg(test)]
pub fn particle_budget(w: &Weather) -> usize {
    layers(w).iter().map(|l| l.1).sum()
}

/// A stable pseudo-random number 0..1 from two integers.
#[allow(dead_code)]
pub fn hash01(a: u64, b: u64) -> f32 {
    let mut z = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0xD1B5_4A32_D192_ED03).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// One particle: its place relative to the camera's ground point (x, z in -AREA_R..AREA_R, y metres above the ground),
/// its velocity (for the length and slant of a rain streak) and a spin angle (for tumbling leaves).
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    pub spin: f32,
}

/// Where particle `index` of `kind` is at `time`, from a stable hash: nothing is stored, so there is no per-frame
/// allocation. Each particle falls through the box, wraps to a fresh random column when it lands and drifts with `wind`.
#[allow(dead_code)]
pub fn particle(kind: WeatherKind, index: usize, time: f32, wind: (f32, f32)) -> Particle {
    let id = index as u64 | ((kind as u64 + 1) << 40);
    let (fall, sway) = match kind {
        WeatherKind::Rain => (11.0, 0.0),
        WeatherKind::Storm => (15.0, 0.0),
        WeatherKind::Snow => (1.1, 0.5),
        WeatherKind::Leaves => (1.3, 1.2),
        _ => (0.0, 0.0),
    };
    let phase = hash01(id, 1) * AREA_H;
    let t = time.max(0.0) * fall + phase;
    let cycle = (t / AREA_H).floor();
    let frac = t - cycle * AREA_H;
    let c = cycle as u64;
    let mut x = (hash01(id, 2 + c * 7) * 2.0 - 1.0) * AREA_R;
    let mut z = (hash01(id, 3 + c * 7) * 2.0 - 1.0) * AREA_R;
    let secs = if fall > 0.0 { frac / fall } else { 0.0 };
    x += wind.0 * secs;
    z += wind.1 * secs;
    let spin = time * 3.0 + phase * 5.0;
    if sway > 0.0 {
        x += (time * 1.3 + phase * 5.0).sin() * sway;
        z += (time * 1.1 + phase * 3.0).cos() * sway;
    }
    Particle { x, y: AREA_H - frac, z, vx: wind.0, vy: -fall, vz: wind.1, spin }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm(kind: WeatherKind, i: f32) -> Weather {
        Weather::new("m", kind, i, &[], 1000.0)
    }

    #[test]
    fn names_round_trip_and_unknown_names_list_the_choices() {
        for n in WeatherKind::NAMES {
            assert_eq!(WeatherKind::parse(n).unwrap().name(), n);
        }
        let e = WeatherKind::parse("hail").unwrap_err();
        assert!(e.contains("unknown weather kind 'hail'") && e.contains("clear, rain, snow, fog, storm, leaves"), "{e}");
    }

    #[test]
    fn changes_are_deterministic_per_map_and_do_happen() {
        let changes = [(WeatherKind::Rain, 1.0), (WeatherKind::Snow, 1.0), (WeatherKind::Fog, 2.0), (WeatherKind::Clear, 1.0)];
        let run = |id: &str| {
            let mut w = Weather::new(id, WeatherKind::Clear, 0.6, &changes, 10.0);
            let mut seen = Vec::new();
            for _ in 0..1200 {
                w.tick(0.5);
                seen.push((w.kind, w.target.0));
            }
            seen
        };
        let a = run("city");
        assert_eq!(a, run("city"), "same map, same weather");
        let kinds: std::collections::BTreeSet<_> = a.iter().map(|s| s.1).collect();
        assert!(kinds.len() > 1, "the weather changed at least once: {kinds:?}");
    }

    #[test]
    fn a_change_blends_in_smoothly() {
        let mut w = calm(WeatherKind::Clear, 0.0);
        w.set(WeatherKind::Rain, 1.0);
        assert_eq!(w.kind, WeatherKind::Clear);
        let mut last = 0.0;
        for _ in 0..40 {
            w.tick(0.1);
            let l = w.level(WeatherKind::Rain);
            assert!(l >= last, "the rain only grows while blending in");
            last = l;
        }
        assert!((w.blend - 0.5).abs() < 0.02, "half way after half the blend time: {}", w.blend);
        assert!((w.level(WeatherKind::Rain) - 0.5).abs() < 0.02);
        let (mul, _) = sky_tint(&w);
        assert!(mul[0] < 1.0 && mul[0] > look(WeatherKind::Rain).0[0], "the sky is part way dark");
        for _ in 0..60 {
            w.tick(0.1);
        }
        assert_eq!((w.kind, w.intensity, w.blend), (WeatherKind::Rain, 1.0, 0.0));
        assert!(w.is_wet());
    }

    #[test]
    fn lightning_only_flashes_in_storms() {
        let mut clear = calm(WeatherKind::Clear, 0.0);
        let mut rain = calm(WeatherKind::Rain, 0.9);
        let mut fog = calm(WeatherKind::Fog, 0.9);
        for _ in 0..3000 {
            for w in [&mut clear, &mut rain, &mut fog] {
                w.tick(0.1);
                assert_eq!(w.flash, 0.0, "no lightning in {:?}", w.kind);
            }
        }
        let mut storm = calm(WeatherKind::Storm, 0.8);
        let mut peak: f32 = 0.0;
        for _ in 0..600 {
            storm.tick(0.1);
            peak = peak.max(storm.flash);
        }
        assert!(peak > 0.99, "a storm flashes");
        // The flash lights the sky.
        storm.flash = 1.0;
        assert!(sky_tint(&storm).0[0] > sky_tint(&calm(WeatherKind::Storm, 0.8)).0[0]);
        // Once the storm has blended away the flashing stops.
        storm.set(WeatherKind::Clear, 0.0);
        for _ in 0..200 {
            storm.tick(0.1);
        }
        assert_eq!((storm.kind, storm.flash), (WeatherKind::Clear, 0.0));
    }

    #[test]
    fn tint_and_budget_follow_the_kind() {
        let clear = calm(WeatherKind::Clear, 0.0);
        assert_eq!(sky_tint(&clear), ([1.0, 1.0, 1.0], 0.0));
        assert_eq!(particle_budget(&clear), 0);
        let fog = calm(WeatherKind::Fog, 1.0);
        let rain = calm(WeatherKind::Rain, 1.0);
        assert!(sky_tint(&fog).1 > sky_tint(&rain).1, "fog is thicker than rain");
        assert_eq!(particle_budget(&fog), 0, "fog is haze, not particles");
        assert_eq!(particle_budget(&rain), 700);
        assert!(particle_budget(&calm(WeatherKind::Storm, 1.0)) > particle_budget(&rain));
        assert!(particle_budget(&calm(WeatherKind::Rain, 0.3)) < particle_budget(&rain));
    }

    #[test]
    fn particles_are_stable_bounded_and_fall() {
        let wind = (1.5, -0.5);
        for kind in [WeatherKind::Rain, WeatherKind::Snow, WeatherKind::Leaves, WeatherKind::Storm] {
            for i in 0..50 {
                let p = particle(kind, i, 12.34, wind);
                assert_eq!(p, particle(kind, i, 12.34, wind), "same inputs, same place");
                assert!(p.y > 0.0 && p.y <= AREA_H, "{kind:?} {i}: y {}", p.y);
                assert!(p.x.abs() < AREA_R + 20.0 && p.z.abs() < AREA_R + 20.0);
            }
        }
        // A raindrop is lower a moment later (inside one fall).
        let a = particle(WeatherKind::Rain, 3, 1.0, (0.0, 0.0));
        let b = particle(WeatherKind::Rain, 3, 1.02, (0.0, 0.0));
        assert!(b.y < a.y || b.y - a.y > AREA_H - 1.0, "falls (or wrapped)");
        assert_ne!(particle(WeatherKind::Rain, 1, 5.0, wind).x, particle(WeatherKind::Rain, 2, 5.0, wind).x);
    }
}
