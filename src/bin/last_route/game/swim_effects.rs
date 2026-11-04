//! Swimming and wading with consequences (Godot player_controller_3d.gd 336-372, 1154-1256): the soaked status
//! (louder steps and drips), stamina drain in deep water, noise hunters hear, flashlight flicker or drain while
//! swimming, and an optional chance to drop a held item when deep. Every number is in data/swim/effects.json.
//! Pure state: the caller feeds `SwimInput` each frame and applies `SwimOutput`.

use crate::content::embedded;

#[derive(Clone, Debug)]
pub struct SwimConfig {
    pub soaked_seconds: f32,
    pub wade_floor_fraction: f32,
    pub noise_scale: f32,
    pub drip_every: (f32, f32),
    pub drip_gain_base: f32,
    pub drip_gain_span: f32,
    pub max_depth: f32,
    pub deep_depth: f32,
    pub wade_noise: f32,
    pub wade_deep_noise: f32,
    pub splash_every: f32,
    pub splash_gain: f32,
    pub tiring_depth: f32,
    pub wade_stamina_drain: f32,
    pub swim_noise: f32,
    pub swim_stamina_drain: f32,
    pub flicker_chance: f32,
    pub flicker_seconds: f32,
    pub flashlight_drain: f32,
    pub drop_chance: f32,
    pub drop_min_depth: f32,
}

impl Default for SwimConfig {
    /// Standard values (Godot constants).
    fn default() -> Self {
        SwimConfig {
            soaked_seconds: 30.0,
            wade_floor_fraction: 0.5,
            noise_scale: 1.45,
            drip_every: (1.1, 2.4),
            drip_gain_base: 0.35,
            drip_gain_span: 0.65,
            max_depth: 0.35,
            deep_depth: 0.9,
            wade_noise: 12.0,
            wade_deep_noise: 17.0,
            splash_every: 0.42,
            splash_gain: 0.6,
            tiring_depth: 0.6,
            wade_stamina_drain: 6.0,
            swim_noise: 0.8,
            swim_stamina_drain: 9.0,
            flicker_chance: 0.35,
            flicker_seconds: 0.4,
            flashlight_drain: 0.0,
            drop_chance: 0.0,
            drop_min_depth: 1.2,
        }
    }
}

impl SwimConfig {
    /// Parse effects.json; anything missing keeps the Standard value.
    pub fn parse(v: &serde_json::Value) -> SwimConfig {
        let mut c = SwimConfig::default();
        let f = |sec: &str, key: &str, dst: &mut f32| {
            if let Some(x) = v.get(sec).and_then(|s| s.get(key)).and_then(|x| x.as_f64()) {
                *dst = x as f32;
            }
        };
        f("soaked", "seconds", &mut c.soaked_seconds);
        f("soaked", "wade_floor_fraction", &mut c.wade_floor_fraction);
        f("soaked", "noise_scale", &mut c.noise_scale);
        f("soaked", "drip_gain_base", &mut c.drip_gain_base);
        f("soaked", "drip_gain_span", &mut c.drip_gain_span);
        if let Some(a) = v.pointer("/soaked/drip_every").and_then(|a| a.as_array()) {
            if let (Some(lo), Some(hi)) = (a.first().and_then(|x| x.as_f64()), a.get(1).and_then(|x| x.as_f64())) {
                c.drip_every = (lo as f32, (hi as f32).max(lo as f32));
            }
        }
        f("wade", "max_depth", &mut c.max_depth);
        f("wade", "deep_depth", &mut c.deep_depth);
        f("wade", "noise", &mut c.wade_noise);
        f("wade", "deep_noise", &mut c.wade_deep_noise);
        f("wade", "splash_every", &mut c.splash_every);
        f("wade", "splash_gain", &mut c.splash_gain);
        f("wade", "tiring_depth", &mut c.tiring_depth);
        f("wade", "stamina_drain", &mut c.wade_stamina_drain);
        f("swim", "noise", &mut c.swim_noise);
        f("swim", "stamina_drain", &mut c.swim_stamina_drain);
        f("swim", "flashlight_flicker_chance_per_second", &mut c.flicker_chance);
        f("swim", "flashlight_flicker_seconds", &mut c.flicker_seconds);
        f("swim", "flashlight_drain_per_second", &mut c.flashlight_drain);
        f("swim", "drop_item_chance_per_second", &mut c.drop_chance);
        f("swim", "drop_item_min_depth", &mut c.drop_min_depth);
        c
    }

    /// The embedded data file, or Standard if it is missing or broken.
    pub fn load() -> SwimConfig {
        embedded::data_under("swim/effects.json")
            .next()
            .and_then(|(_, t)| serde_json::from_str::<serde_json::Value>(t).ok())
            .map(|v| SwimConfig::parse(&v))
            .unwrap_or_default()
    }
}

/// What the runner is doing this frame. Only the real player's runner may be fed in: bots and fake players
/// never use this, so they never raise the danger readout, heartbeat or chase cue.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwimInput {
    /// Water over the floor at the runner's feet (0 = dry).
    pub depth: f32,
    pub moving: bool,
    pub god_mode: bool,
    pub holding_item: bool,
    pub flashlight_on: bool,
    /// The scripted waterfall plunge handles its own noise; skip wading while it runs.
    pub plunging: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwimOutput {
    /// Noise to merge with max() into the runner's noise (already soaked-scaled).
    pub noise: f32,
    /// Stamina to subtract this frame.
    pub stamina_drain: f32,
    /// Play a splash with this gain.
    pub splash: Option<f32>,
    /// Play a drip with this gain.
    pub drip: Option<f32>,
    /// Force a flashlight flicker for this many seconds.
    pub flicker_seconds: f32,
    /// Flashlight charge to drain this frame.
    pub flashlight_drain: f32,
    /// Drop the held item now.
    pub drop_item: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SwimState {
    pub soaked_left: f32,
    drip_left: f32,
    splash_left: f32,
}

impl SwimState {
    pub fn is_soaked(&self) -> bool {
        self.soaked_left > 0.0
    }

    /// Noise multiplier while soaked (1.0 when dry).
    pub fn noise_scale(&self, cfg: &SwimConfig) -> f32 {
        if self.is_soaked() {
            cfg.noise_scale
        } else {
            1.0
        }
    }

    /// Soak fully (after the waterfall plunge).
    pub fn soak(&mut self, cfg: &SwimConfig) {
        self.soaked_left = cfg.soaked_seconds;
        self.drip_left = 1.0;
    }

    /// 0..1 between the splash depth and the deep depth.
    fn deep_fraction(cfg: &SwimConfig, depth: f32) -> f32 {
        let span = (cfg.deep_depth - cfg.max_depth).max(0.0001);
        ((depth - cfg.max_depth) / span).clamp(0.0, 1.0)
    }

    /// Swimming means water at or over the deep depth.
    pub fn is_swimming(cfg: &SwimConfig, depth: f32) -> bool {
        depth >= cfg.deep_depth
    }

    /// Advance one frame. `rnd` yields uniform 0..1 values.
    pub fn update(&mut self, cfg: &SwimConfig, dt: f32, i: SwimInput, rnd: &mut dyn FnMut() -> f32) -> SwimOutput {
        let mut out = SwimOutput::default();
        if !i.plunging {
            if i.depth > 0.0 {
                self.soaked_left = self.soaked_left.max(cfg.soaked_seconds * cfg.wade_floor_fraction);
                let swimming = Self::is_swimming(cfg, i.depth);
                if i.moving {
                    let deep = Self::deep_fraction(cfg, i.depth);
                    let base = cfg.wade_noise * (i.depth / cfg.max_depth.max(0.0001)).clamp(0.5, 1.0);
                    let n = (base + (cfg.wade_deep_noise - base) * deep).max(cfg.swim_noise);
                    out.noise = n * self.noise_scale(cfg);
                    if i.depth >= cfg.tiring_depth && !i.god_mode {
                        let rate = if swimming { cfg.swim_stamina_drain } else { cfg.wade_stamina_drain };
                        out.stamina_drain = rate * dt;
                    }
                    self.splash_left -= dt;
                    if self.splash_left <= 0.0 {
                        self.splash_left = cfg.splash_every;
                        out.splash = Some(cfg.splash_gain);
                    }
                }
                if swimming {
                    if i.flashlight_on {
                        if rnd() < cfg.flicker_chance * dt {
                            out.flicker_seconds = cfg.flicker_seconds;
                        }
                        out.flashlight_drain = cfg.flashlight_drain * dt;
                    }
                    if i.holding_item && i.depth >= cfg.drop_min_depth && rnd() < cfg.drop_chance * dt {
                        out.drop_item = true;
                    }
                }
            } else {
                self.splash_left = 0.0;
            }
        }
        if self.soaked_left > 0.0 {
            self.soaked_left = (self.soaked_left - dt).max(0.0);
            self.drip_left -= dt;
            if self.drip_left <= 0.0 {
                let r = rnd().clamp(0.0, 1.0);
                self.drip_left = cfg.drip_every.0 + (cfg.drip_every.1 - cfg.drip_every.0) * r;
                out.drip = Some(cfg.drip_gain_base + cfg.drip_gain_span * self.soaked_left / cfg.soaked_seconds.max(0.0001));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(depth: f32) -> SwimInput {
        SwimInput { depth, moving: true, ..Default::default() }
    }

    #[test]
    fn data_file_matches_standard() {
        let text = include_str!("../data/swim/effects.json");
        let c = SwimConfig::parse(&serde_json::from_str(text).unwrap());
        let d = SwimConfig::default();
        assert_eq!(c.soaked_seconds, 30.0);
        assert_eq!(c.noise_scale, 1.45);
        assert_eq!(c.drip_every, (1.1, 2.4));
        assert_eq!(c.tiring_depth, d.tiring_depth);
        assert_eq!(c.drop_chance, 0.0);
    }

    #[test]
    fn wading_soaks_to_half_and_expires() {
        let c = SwimConfig::default();
        let mut s = SwimState::default();
        s.update(&c, 0.1, input(0.2), &mut || 0.5);
        assert!(s.soaked_left > c.soaked_seconds * 0.4);
        for _ in 0..400 {
            s.update(&c, 0.1, SwimInput::default(), &mut || 0.5);
        }
        assert!(!s.is_soaked());
    }

    #[test]
    fn soaked_steps_are_louder() {
        let c = SwimConfig::default();
        let mut dry = SwimState::default();
        let mut wet = SwimState::default();
        wet.soak(&c);
        let a = dry.update(&c, 0.0, input(0.2), &mut || 0.5);
        let b = wet.update(&c, 0.0, input(0.2), &mut || 0.5);
        // Wading soaks at once (Godot _update_wading), so even the first "dry" wading step is already scaled.
        let base = c.wade_noise * (0.2 / c.max_depth).clamp(0.5, 1.0);
        assert!(a.noise > base);
        assert!(b.noise >= a.noise);
        let mut land = SwimState::default();
        assert_eq!(land.update(&c, 0.0, input(0.0), &mut || 0.5).noise, 0.0);
    }

    #[test]
    fn deep_water_tires_unless_god_mode() {
        let c = SwimConfig::default();
        let mut s = SwimState::default();
        assert_eq!(s.update(&c, 1.0, input(0.2), &mut || 0.5).stamina_drain, 0.0);
        assert!(s.update(&c, 1.0, input(1.0), &mut || 0.5).stamina_drain > 0.0);
        let god = SwimInput { god_mode: true, ..input(1.0) };
        assert_eq!(s.update(&c, 1.0, god, &mut || 0.5).stamina_drain, 0.0);
    }

    #[test]
    fn drip_interval_in_range() {
        let c = SwimConfig::default();
        let mut s = SwimState::default();
        s.soak(&c);
        let o = s.update(&c, 1.5, SwimInput::default(), &mut || 1.0);
        assert!(o.drip.is_some());
        assert!(s.drip_left <= c.drip_every.1 && s.drip_left >= c.drip_every.0);
    }

    #[test]
    fn item_drop_only_when_enabled_and_deep() {
        let mut c = SwimConfig::default();
        let mut s = SwimState::default();
        let held = SwimInput { holding_item: true, ..input(1.5) };
        assert!(!s.update(&c, 1.0, held, &mut || 0.0).drop_item);
        c.drop_chance = 1.0;
        assert!(s.update(&c, 1.0, held, &mut || 0.0).drop_item);
        let shallow = SwimInput { holding_item: true, ..input(0.5) };
        assert!(!s.update(&c, 1.0, shallow, &mut || 0.0).drop_item);
    }

    #[test]
    fn swimming_flickers_flashlight() {
        let c = SwimConfig::default();
        let mut s = SwimState::default();
        let i = SwimInput { flashlight_on: true, ..input(1.5) };
        assert!(s.update(&c, 1.0, i, &mut || 0.0).flicker_seconds > 0.0);
        assert_eq!(s.update(&c, 1.0, i, &mut || 0.99).flicker_seconds, 0.0);
    }
}
