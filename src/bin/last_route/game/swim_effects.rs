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

// ---------------------------------------------------------------------------------------------------------------
// The running game: the real runner's water depth, wading, the soaked state and the waterfall plunge.
// Only the real player's runner comes through here, so bots and chased fake players never raise the danger readout,
// heartbeat or chase cue from water.

/// The sewer chunk the runner stands in, kept so its grid is generated once per chunk, not per frame.
pub type WaterChunk = ((i32, i32, i32), Option<Box<crate::world::route::sewer::SewerChunk>>);

/// A waterfall plunge in progress. Its positions are absolute cells (so the window may change under it).
#[derive(Clone, Copy, Debug)]
pub struct ActivePlunge {
    pub plunge: super::swim::Plunge,
    /// The storey already changed to the collector (done on the splash).
    pub switched: bool,
}

use super::{swim, Controls, Game};
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

    /// Loads (or reuses) the sewer chunk under the runner and returns the runner's cell inside it. None off the sewer.
    fn load_water_chunk(&mut self) -> Option<(i32, i32)> {
        let level = self.sewer_level()?;
        let o = self.window_origin();
        let abs = (o.0 + self.p.pos.0.floor() as i32, o.1 + self.p.pos.1.floor() as i32);
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
        if let Some(local) = self.load_water_chunk() {
            if let Some((_, Some(ch))) = &self.water_cache {
                d = d.max(water::water_depth_at(&ch.sewer_grid, local.0, local.1));
            }
        }
        d
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
            plunging: self.plunge.is_some(),
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

    /// The runner presses jump on the waterfall ledge of a first-sewer chunk: the deliberate plunge to the collector.
    pub(super) fn try_begin_plunge(&mut self, c: &Controls) {
        if !c.jump || self.plunge.is_some() || self.sewer_level() != Some(sewer_core::LEVEL) {
            return;
        }
        let Some(local) = self.load_water_chunk() else { return };
        let Some((key, Some(ch))) = &self.water_cache else { return };
        let Some(falls) = &ch.falls else { return };
        let j = &falls.jump;
        if !swim::at_jump_ledge(local, (j.start.x, j.start.y), (j.cell.x, j.cell.y)) {
            return;
        }
        let rules = swim::rules().plunge;
        let base = ((key.0 * sewer_core::CHUNK) as f32, (key.2 * sewer_core::CHUNK) as f32);
        let o = self.window_origin();
        let surface = water::surface_y(0.0);
        let from = [o.0 as f32 + self.p.pos.0, surface + rules.min_drop, o.1 as f32 + self.p.pos.1];
        let land = [base.0 + j.land[0] as f32, surface, base.1 + j.land[1] as f32];
        let exit = [base.0 + j.exit[0] as f32, 0.0, base.1 + j.exit[1] as f32];
        self.plunge = Some(ActivePlunge { plunge: swim::Plunge::begin(&rules, from, land, exit, surface), switched: false });
    }

    /// One frame of a plunge: input is ignored, the body follows the arc with collision off, the splash changes the
    /// storey, and the finish soaks the runner.
    pub(super) fn advance_plunge(&mut self, dt: f32, content: &Content, catalog: &Catalog) {
        let Some(mut a) = self.plunge.take() else { return };
        let rules = swim::rules().plunge;
        let f = a.plunge.step(dt);
        self.p.moving = false;
        self.p.sprinting = false;
        self.p.noise = f.noise;
        if f.splashed {
            crate::audio::sound::play(crate::audio::cues::rules::splash_big(0.0, 1.0));
            self.p.force_flashlight_flicker(self.swim_cfg.flicker_seconds);
            self.p.noise_burst = rules.splash_noise;
            self.p.noise_burst_timer = rules.splash_noise_seconds;
            if !a.switched {
                a.switched = true;
                if self.sewer_level() == Some(sewer_core::LEVEL) {
                    self.change_storey(sewer_core::LOWER_LEVEL, content, catalog, |_| {});
                }
            }
        }
        let o = self.window_origin();
        self.p.pos = (f.pos[0] - o.0 as f32, f.pos[2] - o.1 as f32);
        self.p.feet_y = if f.phase == swim::PlungePhase::Fall { (f.pos[1] - water::surface_y(0.0)).max(0.0) } else { 0.0 };
        if f.finished {
            self.swim.soak(&self.swim_cfg);
            self.p.feet_y = 0.0;
            self.p.noise = 0.0;
        } else {
            self.plunge = Some(a);
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

    #[test]
    fn a_finished_plunge_soaks_the_runner_and_returns_control() {
        let (mut g, content, catalog) = sewer_game(sewer_core::LEVEL);
        let rules = swim::rules().plunge;
        let p0 = g.p.pos;
        let o = g.window_origin();
        let from = [o.0 as f32 + p0.0, rules.min_drop, o.1 as f32 + p0.1];
        let land = [from[0] + 2.0, 0.0, from[2]];
        let exit = [from[0] + 4.0, 0.0, from[2]];
        g.plunge = Some(ActivePlunge { plunge: swim::Plunge::begin(&rules, from, land, exit, 0.0), switched: false });
        assert!(!g.swim.is_soaked());
        let mut frames = 0;
        while g.plunge.is_some() && frames < 4000 {
            g.advance_plunge(0.05, &content, &catalog);
            frames += 1;
        }
        assert!(g.plunge.is_none(), "the plunge finishes");
        assert_eq!(g.inf.as_ref().unwrap().level, sewer_core::LOWER_LEVEL, "the splash changed the storey");
        assert!(g.swim.is_soaked(), "the finished plunge soaks the runner");
    }

    #[test]
    fn jump_off_the_ledge_is_the_only_way_to_begin_a_plunge() {
        let (mut g, _, _) = sewer_game(0);
        let c = Controls { jump: true, ..Default::default() };
        g.try_begin_plunge(&c);
        assert!(g.plunge.is_none(), "not on a sewer storey");
    }
}
