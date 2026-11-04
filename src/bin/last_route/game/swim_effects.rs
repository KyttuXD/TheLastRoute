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

    /// Soak fully (after a landing in the pool).
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

// ---------------------------------------------------------------------------------------------------------------
// The running game: the real runner's water depth, wading, and the soaked state.
// Only the real player's runner comes through here, so bots and chased fake players never raise the danger readout,
// heartbeat or chase cue from water.

/// The sewer chunk the runner stands in, kept so its grid is generated once per chunk, not per frame.
pub type WaterChunk = ((i32, i32, i32), Option<Box<crate::world::route::sewer::SewerChunk>>);

use super::{swim, Game};
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::route::sewer::{self, core as sewer_core, water};

impl Game {
    fn sewer_level(&self) -> Option<i32> {
        self.inf
            .as_ref()
            .map(|i| i.level)
            .filter(|l| *l == sewer_core::LEVEL || *l == sewer_core::LOWER_LEVEL)
    }

    /// Loads (or reuses) the chunk of sewer storey `level` under window position `pos` and returns the cell inside it.
    /// None when `level` is not a sewer storey.
    fn load_water_chunk(&mut self, level: i32, pos: (f32, f32)) -> Option<(i32, i32)> {
        if level != sewer_core::LEVEL && level != sewer_core::LOWER_LEVEL {
            return None;
        }
        let o = self.window_origin();
        let abs = (o.0 + pos.0.floor() as i32, o.1 + pos.1.floor() as i32);
        let (chunk, local) = swim::split_cell(abs, sewer_core::CHUNK);
        let key = (chunk.0, level, chunk.1);
        if self.water_cache.as_ref().map(|c| c.0) != Some(key) {
            let seed = self.inf.as_ref()?.seed.clone();
            let ch = sewer::chunk(
                crate::world::route::layout::Coord::new(key.0, key.1, key.2),
                &sewer::Inputs { world: &seed },
            );
            self.water_cache = Some((key, ch.map(Box::new)));
        }
        Some(local)
    }

    /// Metres of water at the runner's feet: the sewer's water cells, or the pipe flood a valve opened
    /// (`flood_depth`, set each frame by the endless update), whichever is deeper. 0 off the sewer.
    pub(super) fn runner_water_depth(&mut self) -> f32 {
        let mut d = self.flood_depth.max(0.0);
        if let Some(level) = self.sewer_level() {
            if let Some(local) = self.load_water_chunk(level, self.p.pos) {
                if let Some((_, Some(ch))) = &self.water_cache {
                    d = d.max(water::water_depth_at(&ch.sewer_grid, local.0, local.1));
                }
            }
        }
        d
    }

    /// Metres the runner's eye should be lowered: over a sewer WATER cell the eye drops by the channel depth (the
    /// render bed, the same for every sewer storey) as if the body sank into the water, capped and eased by
    /// data/swim/camera.json. Dry floor and a pipe flood over it give 0 with the shipped data.
    pub(super) fn runner_water_sink_target(&mut self) -> f32 {
        let mut drop = 0.0;
        if let Some(level) = self.sewer_level() {
            if self.water_depth_on(level, self.p.pos) > 0.0 {
                drop = water::params().channel_depth;
            }
        }
        let flood = self.flood_depth.max(0.0);
        swim::sink_target(swim::camera_rules(), drop, flood, self.p.eye_height(), water::params().water_surface)
    }

    /// One frame of the water camera: eases `Player::water_sink` to its target (0 when `wet` is false, a ladder or a
    /// hiding spot must not keep a sunk eye).
    pub(super) fn update_water_sink(&mut self, dt: f32, wet: bool) {
        let target = if wet { self.runner_water_sink_target() } else { 0.0 };
        self.p.water_sink = swim::ease_sink(swim::camera_rules(), self.p.water_sink, target, dt);
    }

    /// Metres of sewer water on storey `level` at window position `pos` (0 off the sewer or over dry floor).
    pub(super) fn water_depth_on(&mut self, level: i32, pos: (f32, f32)) -> f32 {
        let Some(local) = self.load_water_chunk(level, pos) else { return 0.0 };
        match &self.water_cache {
            Some((_, Some(ch))) => water::water_depth_at_level(&ch.sewer_grid, local.0, local.1, level),
            _ => 0.0,
        }
    }

    /// The touchdown of a fall (`Player::take_landing`). The storey is the one that owns the landing height, not the
    /// active label, which may lag a frame. A fall below `min_height` does nothing; water deep enough takes the body
    /// (splash, flicker, noise, soak, no stun); anything else is the ground rule: stun, stamina, exposure, noise.
    /// Never changes the storey or moves the runner. In the pool the runner then swims by the normal wade rules.
    pub(super) fn apply_landing(&mut self, l: super::player::Landing, _content: &Content, _catalog: &Catalog) {
        let rules = swim::fall_rules();
        if !(l.drop_height >= rules.min_height) {
            return;
        }
        let level = super::level_stack::owning_level(l.y);
        let depth = self.water_depth_on(level, l.pos);
        if depth > rules.pool.min_depth {
            let p = &rules.pool;
            crate::audio::sound::play(crate::audio::cues::rules::splash_big(0.0, p.splash_gain as f64));
            self.p.force_flashlight_flicker(p.flicker_seconds);
            self.p.noise_burst = p.noise;
            self.p.noise_burst_timer = p.noise_seconds;
            self.push_noise_at(crate::hunters::noise_events::NoiseKind::PlungeSplash, l.pos, level);
            if p.soak {
                self.swim.soak(&self.swim_cfg);
            }
            return;
        }
        let g = &rules.ground;
        let k = rules.ground_scale(l.drop_height);
        self.p.stun = self.p.stun.max(g.stun_seconds * k);
        self.p.stamina = (self.p.stamina - g.stamina_loss * k).max(0.0);
        if self.p.stamina <= 0.0 {
            self.p.exhausted = true;
        }
        self.meters.add_exposure(g.exposure_add * k);
        self.p.noise_burst = g.noise * k;
        self.p.noise_burst_timer = g.noise_seconds;
        self.p.force_flashlight_flicker(g.flicker_seconds);
        self.push_noise_scaled(crate::hunters::noise_events::NoiseKind::HardLanding, l.pos, level, (l.drop_height / rules.min_height).clamp(1.0, 2.0));
        self.say("You hit the floor hard.");
    }

    /// The wading speed multiplier for `depth` (1 when dry).
    pub(super) fn wade_scale(depth: f32) -> f32 {
        swim::wade_speed_scale(&swim::rules().wade, depth)
    }

    /// Once per frame after the runner moved: wading noise, stamina, the soaked state and its sounds.
    pub(super) fn update_swim(&mut self, dt: f32, depth: f32, god: bool) {
        let input = SwimInput {
            depth,
            moving: self.p.moving,
            god_mode: god,
            holding_item: self.slots[self.selected].is_some(),
            flashlight_on: self.p.flashlight_on,
        };
        let o = self.swim.update(&self.swim_cfg, dt, input, &mut || macroquad::rand::gen_range(0.0f32, 1.0));
        self.apply_swim_output(o);
    }

    pub(super) fn apply_swim_output(&mut self, o: SwimOutput) {
        let scale = self.swim.noise_scale(&self.swim_cfg);
        self.p.noise = swim::merge_water_noise(self.p.noise, o.noise, scale);
        if o.stamina_drain > 0.0 {
            self.p.stamina = (self.p.stamina - o.stamina_drain).max(0.0);
            if self.p.stamina <= 0.0 {
                self.p.exhausted = true;
            }
        }
        if let Some(g) = o.splash {
            crate::audio::sound::play(crate::audio::cues::rules::splash_big(0.0, g as f64));
        }
        if let Some(g) = o.drip {
            crate::audio::sound::play(crate::audio::cues::rules::water_drip(0.0, g as f64));
        }
        if o.flicker_seconds > 0.0 {
            self.p.force_flashlight_flicker(o.flicker_seconds);
        }
        if o.flashlight_drain > 0.0 {
            self.p.lamp = (self.p.lamp - o.flashlight_drain).max(0.0);
        }
        if o.drop_item {
            self.drop_selected();
        }
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

    // --- the running game (the call sites in Game::step reach these) ---

    fn sewer_game(level: i32) -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-SWIM-7", level, &content, &catalog);
        (g, content, catalog)
    }

    #[test]
    fn the_runner_reads_the_water_depth_of_the_sewer_cell_under_it() {
        let (mut g, _, _) = sewer_game(sewer_core::LEVEL);
        let center = g.inf.as_ref().unwrap().center;
        let seed = g.inf.as_ref().unwrap().seed.clone();
        let ch = sewer::chunk(crate::world::route::layout::Coord::new(center.0, sewer_core::LEVEL, center.1), &sewer::Inputs { world: &seed })
            .expect("a sewer chunk");
        let mut wet = None;
        let mut dry = None;
        for z in 0..sewer_core::N {
            for x in 0..sewer_core::N {
                match ch.sewer_grid[z][x] {
                    sewer_core::WATER if wet.is_none() => wet = Some((x, z)),
                    sewer_core::DRY if dry.is_none() => dry = Some((x, z)),
                    _ => {}
                }
            }
        }
        let o = g.window_origin();
        let place = |g: &mut Game, c: (usize, usize)| {
            let abs = (center.0 * sewer_core::CHUNK + c.0 as i32, center.1 * sewer_core::CHUNK + c.1 as i32);
            g.p.pos = ((abs.0 - o.0) as f32 + 0.5, (abs.1 - o.1) as f32 + 0.5);
        };
        place(&mut g, wet.expect("the main cross carries water"));
        assert!((g.runner_water_depth() - water::params().full_depth()).abs() < 1e-4);
        place(&mut g, dry.expect("a dry cell"));
        assert_eq!(g.runner_water_depth(), 0.0);
        g.flood_depth = 0.5;
        assert!((g.runner_water_depth() - 0.5).abs() < 1e-4, "a pipe flood wets a dry cell");
    }

    #[test]
    fn a_dry_floor_has_no_water_and_no_slowdown() {
        let (mut g, _, _) = sewer_game(0);
        assert_eq!(g.runner_water_depth(), 0.0);
        assert_eq!(Game::wade_scale(0.0), 1.0);
    }

    #[test]
    fn deeper_water_slows_the_runner_by_the_swim_data() {
        let w = swim::rules().wade;
        assert!(Game::wade_scale(0.3) < 1.0);
        assert!(Game::wade_scale(0.6) < Game::wade_scale(0.3));
        assert!((Game::wade_scale(w.deep_depth) - w.speed_scale).abs() < 1e-5);
    }

    #[test]
    fn wading_makes_noise_hunters_hear_once_costs_stamina_and_soaks() {
        let (mut g, _, _) = sewer_game(0);
        g.p.moving = true;
        g.p.noise = 0.0;
        g.p.stamina = crate::config::MAX_STAMINA;
        g.update_swim(0.1, 0.8, false);
        assert!(g.swim.is_soaked());
        assert!(g.p.noise > 0.0);
        assert!(g.p.stamina < crate::config::MAX_STAMINA, "deep wading tires");
        let heard = g.p.noise * g.swim.noise_scale(&g.swim_cfg);
        assert!(heard >= g.swim_cfg.wade_noise * 0.5 && heard <= g.swim_cfg.wade_deep_noise * g.swim_cfg.noise_scale + 1e-3);
        // god mode never tires
        g.p.stamina = crate::config::MAX_STAMINA;
        g.update_swim(0.1, 0.8, true);
        assert_eq!(g.p.stamina, crate::config::MAX_STAMINA);
    }

    #[test]
    fn standing_in_water_makes_no_noise() {
        let (mut g, _, _) = sewer_game(0);
        g.p.moving = false;
        g.p.noise = 0.0;
        g.update_swim(0.1, 0.8, false);
        assert_eq!(g.p.noise, 0.0);
    }

    #[test]
    fn a_dry_runner_stays_dry_and_a_soaked_one_dries_out() {
        let (mut g, _, _) = sewer_game(0);
        g.update_swim(0.1, 0.0, false);
        assert!(!g.swim.is_soaked());
        g.swim.soak(&g.swim_cfg);
        for _ in 0..((g.swim_cfg.soaked_seconds / 0.5) as usize + 4) {
            g.update_swim(0.5, 0.0, false);
        }
        assert!(!g.swim.is_soaked());
    }

    // --- the water camera ---

    #[test]
    fn the_eye_sinks_in_sewer_water_and_rises_on_dry_floor() {
        let (mut g, _, _) = sewer_game(sewer_core::LEVEL);
        let wet = sewer_cell(&g, sewer_core::WATER);
        let dry = sewer_cell(&g, sewer_core::DRY);
        let rules = swim::camera_rules();
        g.p.pos = wet;
        let standing = g.p.eye_height();
        for _ in 0..60 {
            g.update_water_sink(1.0 / 60.0, true);
            assert!(g.p.eye_y() - g.p.y() >= rules.min_eye_above_floor - 1e-4, "never under the floor limit");
        }
        assert!((g.p.water_sink - 1.0).abs() < 0.02, "sink {}", g.p.water_sink);
        assert!((g.p.eye_y() - g.p.y() - (standing - 1.0)).abs() < 0.02);
        assert!(g.p.eye_y() - g.p.y() - water::params().water_surface >= rules.min_eye_above_surface - 1e-3);
        g.p.pos = dry;
        for _ in 0..200 {
            g.update_water_sink(1.0 / 60.0, true);
        }
        assert_eq!(g.p.water_sink, 0.0, "back on dry floor the eye is up again");
        // a hiding spot or a ladder lets a sunk eye rise too
        g.p.pos = wet;
        for _ in 0..60 {
            g.update_water_sink(1.0 / 60.0, true);
        }
        for _ in 0..200 {
            g.update_water_sink(1.0 / 60.0, false);
        }
        assert_eq!(g.p.water_sink, 0.0);
    }

    #[test]
    fn a_pipe_flood_over_dry_floor_does_not_sink_the_eye() {
        let (mut g, _, _) = sewer_game(sewer_core::LEVEL);
        g.p.pos = sewer_cell(&g, sewer_core::DRY);
        g.flood_depth = 0.5;
        assert_eq!(g.runner_water_sink_target(), 0.0);
        let mut off = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-SWIM-7", 0, &Content::load(), &Catalog::load());
        assert_eq!(off.runner_water_sink_target(), 0.0, "no sewer, no sink");
    }

    // --- the landing rule of a fall (data/swim/fall.json) ---

    use crate::game::level_stack::{floor_y_of, owning_level};
    use crate::game::player::Landing;
    use crate::world::vertical::STOREY;

    /// A landing on storey `level` at window position `pos` after falling `drop_height` metres.
    fn landing(level: i32, pos: (f32, f32), drop_height: f32) -> Landing {
        Landing { y: floor_y_of(level) + 0.01, drop_height, speed: (2.0 * crate::config::PLAYER_GRAVITY * drop_height).sqrt(), pos }
    }

    /// Window position of a cell of the centre sewer chunk with the given grid code.
    fn sewer_cell(g: &Game, code: u8) -> (f32, f32) {
        let center = g.inf.as_ref().unwrap().center;
        let seed = g.inf.as_ref().unwrap().seed.clone();
        let ch = sewer::chunk(crate::world::route::layout::Coord::new(center.0, sewer_core::LEVEL, center.1), &sewer::Inputs { world: &seed })
            .expect("a sewer chunk");
        let o = g.window_origin();
        for z in 0..sewer_core::N {
            for x in 0..sewer_core::N {
                if ch.sewer_grid[z][x] == code {
                    let abs = (center.0 * sewer_core::CHUNK + x as i32, center.1 * sewer_core::CHUNK + z as i32);
                    return ((abs.0 - o.0) as f32 + 0.5, (abs.1 - o.1) as f32 + 0.5);
                }
            }
        }
        panic!("no cell with code {code}");
    }

    #[test]
    fn landing_in_the_pool_splashes_and_soaks_without_stun() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let pos = sewer_cell(&g, sewer_core::WATER);
        let rules = swim::fall_rules();
        g.p.stun = 0.0;
        g.p.stamina = crate::config::MAX_STAMINA;
        let exposure = g.meters.exposure;
        assert!(!g.swim.is_soaked());
        g.apply_landing(landing(sewer_core::LEVEL, pos, STOREY), &content, &catalog);
        assert!(g.swim.is_soaked(), "the pool soaks the runner");
        assert_eq!(g.p.stun, 0.0, "water takes the body: no stun");
        assert_eq!(g.p.stamina, crate::config::MAX_STAMINA, "no stamina loss");
        assert_eq!(g.meters.exposure, exposure, "no exposure");
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (rules.pool.noise, rules.pool.noise_seconds));
    }

    #[test]
    fn landing_on_dry_floor_stuns_and_costs_stamina() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let pos = sewer_cell(&g, sewer_core::DRY);
        let rules = swim::fall_rules();
        g.p.stun = 0.0;
        g.p.stamina = crate::config::MAX_STAMINA;
        g.apply_landing(landing(sewer_core::LEVEL, pos, STOREY), &content, &catalog);
        assert!(g.p.stun > 0.0 && g.p.stamina < crate::config::MAX_STAMINA);
        assert!(g.meters.exposure > 0.0, "exposure rises");
        assert!(g.p.noise_burst > 0.0 && g.p.noise_burst_timer == rules.ground.noise_seconds, "hunters hear it");
        assert!(!g.swim.is_soaked(), "dry floor does not soak");
        // a landing that empties the stamina exhausts the runner
        g.p.stamina = 1.0;
        g.p.exhausted = false;
        g.apply_landing(landing(sewer_core::LEVEL, pos, STOREY), &content, &catalog);
        assert!(g.p.stamina == 0.0 && g.p.exhausted);
    }

    #[test]
    fn a_short_fall_has_no_effect() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let rules = swim::fall_rules();
        for pos in [sewer_cell(&g, sewer_core::WATER), sewer_cell(&g, sewer_core::DRY)] {
            g.p.stun = 0.0;
            g.p.stamina = crate::config::MAX_STAMINA;
            g.p.noise_burst = 0.0;
            g.apply_landing(landing(sewer_core::LEVEL, pos, rules.min_height * 0.9), &content, &catalog);
            assert_eq!(g.p.stun, 0.0);
            assert_eq!(g.p.stamina, crate::config::MAX_STAMINA);
            assert_eq!(g.p.noise_burst, 0.0);
            assert_eq!(g.meters.exposure, 0.0);
            assert!(!g.swim.is_soaked());
        }
    }

    #[test]
    fn ground_effects_scale_with_drop_height_between_the_clamps() {
        let (mut g, content, catalog) = sewer_game(0);
        let rules = swim::fall_rules();
        let mut stun_for = |drop: f32| {
            g.p.stun = 0.0;
            g.apply_landing(landing(0, (3.5, 3.5), drop), &content, &catalog);
            g.p.stun
        };
        let (short, one, tall, taller) = (stun_for(rules.min_height), stun_for(STOREY), stun_for(10.0 * STOREY), stun_for(20.0 * STOREY));
        assert!(short >= rules.ground.stun_seconds * rules.ground.scale_min - 1e-4, "never below the lower clamp");
        assert!(short <= one && one < tall, "a longer fall stuns longer: {short} {one} {tall}");
        assert!((tall - rules.ground.stun_seconds * rules.ground.scale_max).abs() < 1e-4, "capped by scale_max");
        assert_eq!(tall, taller, "past the cap the stun stops growing");
    }

    #[test]
    fn the_test_landing_speed_is_the_speed_of_the_simulated_fall() {
        // the reported touchdown speed of a free fall of h metres under the game's gravity is sqrt(2 g h)
        let l = landing(0, (3.5, 3.5), STOREY);
        let g = crate::config::PLAYER_GRAVITY;
        let simulated = g * (2.0 * STOREY / g).sqrt();
        assert!((l.speed - simulated).abs() < 1e-3, "{} m/s against the simulated {simulated} m/s", l.speed);
    }

    #[test]
    fn the_pool_surface_y_is_minus_storey_plus_the_water_surface() {
        let surface_in_level_frame = water::surface_y(floor_y_of(sewer_core::LEVEL));
        assert!((surface_in_level_frame - (-STOREY + water::surface_y(0.0))).abs() < 1e-4);
        assert_eq!(owning_level(surface_in_level_frame - water::surface_y(0.0) + 0.01), sewer_core::LEVEL, "the surface sits in the -1 storey");
    }

    #[test]
    fn no_change_storey_is_called_by_a_landing() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let pos = sewer_cell(&g, sewer_core::WATER);
        g.p.pos = pos;
        let (builds, level) = (g.inf.as_ref().unwrap().builds, g.inf.as_ref().unwrap().level);
        for drop in [swim::fall_rules().min_height * 0.5, STOREY, 3.0 * STOREY] {
            g.apply_landing(landing(sewer_core::LEVEL, pos, drop), &content, &catalog);
        }
        assert_eq!(g.inf.as_ref().unwrap().builds, builds, "no window was built");
        assert_eq!(g.inf.as_ref().unwrap().level, level, "the storey label is untouched");
        assert_eq!(g.p.pos, pos, "the landing never moves the runner");
    }

    // --- the landing sounds (hunters hear them on the storey where they happen) ---

    use crate::hunters::brain::{Brain, State};
    use crate::hunters::noise_events::{self, NoiseKind};

    #[test]
    fn a_plunge_splash_is_investigated() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let pos = sewer_cell(&g, sewer_core::WATER);
        g.hunters.clear();
        let mut h = Brain::new(content.hunters[0].clone(), (pos.0 + 6.0, pos.1), "T", crate::config::Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        g.hunters.push(h);
        g.noise_events.clear();
        let drop = swim::fall_rules().min_height * 2.0;
        g.apply_landing(landing(sewer_core::LEVEL, pos, drop), &content, &catalog);
        let events = std::mem::take(&mut g.noise_events);
        let splash = events.iter().find(|e| e.kind == NoiseKind::PlungeSplash).expect("a splash event");
        assert_eq!(splash.level, sewer_core::LEVEL);
        assert_eq!(g.hunters_hear_noise(&events, 1.0), 1);
        assert!(matches!(g.hunters[0].state, State::Search | State::Investigate), "{:?}", g.hunters[0].state);
        // a hunter on another storey hears nothing
        g.hunters[0].state = State::Wander;
        let mut elsewhere = splash.clone();
        elsewhere.level = sewer_core::LEVEL + 5;
        assert_eq!(g.hunters_hear_noise(&[elsewhere], 1.0), 0);
        assert_eq!(g.hunters[0].state, State::Wander);
    }

    #[test]
    fn a_hard_landing_event_carries_the_landing_level_not_the_label() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        assert_eq!(g.runner_level(), sewer_core::LEVEL);
        g.noise_events.clear();
        g.apply_landing(landing(0, (3.5, 3.5), STOREY), &content, &catalog);
        let e = g.noise_events.iter().find(|e| e.kind == NoiseKind::HardLanding).expect("a hard landing event");
        assert_eq!(e.level, 0, "the storey that owns the landing height, not the runner's label");
    }

    #[test]
    fn a_taller_fall_is_heard_farther_but_never_past_the_cap() {
        let (mut g, content, catalog) = sewer_game(0);
        let rules = swim::fall_rules();
        let nr = noise_events::rules();
        let mut radius_for = |drop: f32| {
            g.noise_events.clear();
            g.apply_landing(landing(0, (3.5, 3.5), drop), &content, &catalog);
            let e = g.noise_events.iter().find(|e| e.kind == NoiseKind::HardLanding).expect("event").clone();
            nr.radius_of(&e, 1.0)
        };
        let (short, tall, huge) = (radius_for(rules.min_height), radius_for(rules.min_height * 2.0), radius_for(rules.min_height * 20.0));
        assert!(tall >= short, "a taller fall carries at least as far: {short} {tall}");
        assert!(huge <= nr.max_radius + 1e-4, "never past the cap");
        assert!(huge >= tall);
    }

    #[test]
    fn a_fall_below_the_minimum_emits_no_event() {
        let (mut g, content, catalog) = sewer_game(0);
        g.noise_events.clear();
        g.apply_landing(landing(0, (3.5, 3.5), swim::fall_rules().min_height * 0.5), &content, &catalog);
        assert!(g.noise_events.is_empty());
    }
}
