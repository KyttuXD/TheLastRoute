//! The front end of the original's sound system: which cue each `Sound.<method>()` plays, at what pan / intensity /
//! player volume, on which bus, with which throttle (game/autoload/sound.gd 164-464), plus the call-site rules of the
//! finite route that decide pan and volume (game/main_3d.gd `_update_local_audio` 1682-1735, `_play_thessine_mimic`
//! 2041-2062, game/player/void_player_effects.gd). Everything here is pure: the game passes the clock in.
//!
//! A `Play` is what to render (`Play::frames`, cacheable by `Play::key`) and how loud to play it (`Play::gain`, the
//! player's linear volume; the bus gain comes from the settings, see `bus_gain`).

use super::render::{cache_key, snapped};

/// The audio bus a cue plays on (default_bus_layout.tres). Every `Sound.<method>()` cue is SFX except the menu blip, which
/// is UI. `Master` is for the positional node players (the music box tune and its wind-down, Glyra's hum, the falls):
/// Godot's AudioStreamPlayer3D defaults to the Master bus, so only the Master slider applies to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Master,
    Sfx,
    Ui,
}

/// The settings' bus gains, as the engine applies them: `Master` x the bus slider (`Sfx` or `Ui`), each 0..1.
/// `Engine::bus_gain` is the only place this is computed at run time.
pub fn bus_gain(bus: Bus, master: f32, sfx: f32, ui: f32) -> f32 {
    master
        * match bus {
            Bus::Master => 1.0,
            Bus::Sfx => sfx,
            Bus::Ui => ui,
        }
}

pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// One cue to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    pub cue: String,
    /// Baked into the render (-1 left .. 1 right), not applied at play time.
    pub pan: f64,
    /// Baked into the render (0..4).
    pub intensity: f64,
    /// The player's linear volume (1 for most cues; hunter/teammate steps carry their loudness here).
    pub gain: f32,
    pub bus: Bus,
}

impl Play {
    /// The original's cache key: plays with the same key are the same bytes.
    pub fn key(&self) -> String {
        cache_key(&self.cue, self.pan, self.intensity)
    }

    pub fn frames(&self) -> Option<Vec<[f32; 2]>> {
        super::render(&self.cue, self.pan, self.intensity)
    }
}

/// `_play_named`: the render carries pan and intensity, the player plays at 0 dB.
fn named(cue: &str, pan: f64, intensity: f64) -> Play {
    Play {
        cue: cue.to_string(),
        pan,
        intensity,
        gain: 1.0,
        bus: Bus::Sfx,
    }
}

/// `_play_scaled` (sound.gd 751): one render at intensity 1 with the pan snapped to 0.25, the loudness on the player.
fn scaled(cue: &str, pan: f64, intensity: f64) -> Play {
    Play {
        cue: cue.to_string(),
        pan: snapped(pan.clamp(-1.0, 1.0), SCALED_PAN_STEP),
        intensity: 1.0,
        gain: intensity.clamp(0.05, 4.0) as f32,
        bus: Bus::Sfx,
    }
}

pub const SCALED_PAN_STEP: f64 = 0.25;

// ---- stateless cues (one per Sound method) ----

/// Relay, battery, coin, tool pickup and locked-cache open (route_interactable_3d.gd 747-766, 1048; main_3d.gd 1567).
pub fn collect() -> Play {
    named("collect", 0.0, 1.0)
}
/// `Sound.recharge(amount)`: battery used (amount 25), safe zone (20), stim pack landing (100, clamps to 4).
pub fn recharge(amount: f64) -> Play {
    named("recharge", 0.0, (amount / 10.0).clamp(0.0, 4.0))
}
pub fn music_box() -> Play {
    named("music_box", 0.0, 1.0)
}
pub fn alert() -> Play {
    named("alert", 0.0, 1.0)
}
pub fn chase_start() -> Play {
    named("chase_start", 0.0, 1.0)
}
pub fn taser() -> Play {
    named("taser", 0.0, 1.0)
}
pub fn door(open: bool) -> Play {
    named(if open { "door_open" } else { "door_close" }, 0.0, 1.0)
}
pub fn door_breach() -> Play {
    named("door_breach", 0.0, 1.0)
}
pub fn interaction_fail() -> Play {
    named("interaction_fail", 0.0, 1.0)
}
pub fn tool_break() -> Play {
    named("tool_break", 0.0, 1.0)
}
pub fn axe_swing(pan: f64, intensity: f64) -> Play {
    named("axe_swing", pan, intensity)
}
pub fn axe_chop(pan: f64, intensity: f64) -> Play {
    named("axe_chop", pan, intensity)
}
pub fn hide_search(pan: f64, intensity: f64) -> Play {
    named("hide_search", pan, intensity)
}
pub fn light_switch(pan: f64, intensity: f64) -> Play {
    named("light_switch", pan, intensity)
}
pub fn sleep_settle() -> Play {
    named("sleep_settle", 0.0, 1.0)
}
pub fn sleep_yawn() -> Play {
    named("sleep_yawn", 0.0, 1.0)
}
/// `Sound.sleep_stir(0, clamp(1 - hunter_distance / 12, .25, 1))` (player_controller_3d.gd 647).
pub fn sleep_stir(intensity: f64) -> Play {
    named("sleep_stir", 0.0, intensity)
}
pub fn sleep_jolt() -> Play {
    named("sleep_jolt", 0.0, 1.0)
}
pub fn hunter_snore(pan: f64, intensity: f64) -> Play {
    named("hunter_snore", pan, intensity)
}
pub fn nyxie_timer_wake() -> Play {
    named("nyxie_timer", 0.0, 1.0)
}
pub fn nyxie_empty_wake() -> Play {
    named("nyxie_empty", 0.0, 1.0)
}
/// `Sound.hunter_laugh(pan, volume)`: a noisy emote, scaled by distance (hunter_view_3d.gd 1470).
pub fn hunter_laugh(pan: f64, volume: f64) -> Play {
    named("hunter_laugh", pan, (volume / 0.05).clamp(0.1, 2.0))
}
pub fn spider_bark(pan: f64, intensity: f64) -> Play {
    named("spider_bark", pan, intensity.clamp(0.1, 2.0))
}
pub fn spider_hiss(pan: f64, intensity: f64) -> Play {
    named("spider_hiss", pan, intensity.clamp(0.1, 2.0))
}
pub fn spider_skitter(pan: f64, intensity: f64) -> Play {
    named("spider_skitter", pan, intensity.clamp(0.1, 2.0))
}
pub fn xivvy_screech(pan: f64, intensity: f64) -> Play {
    named("xivvy_screech", pan, intensity.clamp(0.1, 2.0))
}
pub fn xivvy_wing(pan: f64, intensity: f64) -> Play {
    named("xivvy_wing", pan, intensity.clamp(0.1, 2.0))
}
pub fn radar_static() -> Play {
    named("radar_static", 0.0, 1.0)
}
pub fn system_offline() -> Play {
    named("system_offline", 0.0, 1.0)
}
/// One press of Gray's signal route: `progress` = 1 - nodes_left / nodes (void_player_effects.gd 148).
pub fn signal_node(progress: f64) -> Play {
    named("signal_node", 0.0, (0.7 + progress).clamp(0.5, 1.8))
}
pub fn splash_big(pan: f64, intensity: f64) -> Play {
    named("splash_big", pan, intensity.clamp(0.1, 2.0))
}
pub fn water_drip(pan: f64, intensity: f64) -> Play {
    named("water_drip", pan, intensity.clamp(0.1, 2.0))
}
pub fn inferno_crackle(pan: f64, intensity: f64) -> Play {
    named("inferno_crackle", pan, intensity.clamp(0.1, 2.0))
}
pub fn pit_rumble(pan: f64, intensity: f64) -> Play {
    named("pit_rumble", pan, intensity.clamp(0.1, 2.0))
}
/// A music-wing prop: "mic", "podium", "drums", "electric", "acoustic", "bass", "keys", "piano".
pub fn music(kind: &str, pan: f64, intensity: f64) -> Play {
    named(&format!("music_{}", kind), pan, intensity.clamp(0.1, 2.0))
}
pub fn ladder_rung(pan: f64, intensity: f64) -> Play {
    named("ladder_rung", pan, intensity.clamp(0.1, 2.0))
}
pub fn carnival(kind: &str, pan: f64, intensity: f64) -> Play {
    named(&format!("carnival_{}", kind), pan, intensity)
}
pub fn nw(name: &str, pan: f64, intensity: f64) -> Play {
    named(&format!("nw_{}", name), pan, intensity)
}
/// Thessine's fallback mimic (main_3d.gd 2050).
pub fn mimic(rare: bool) -> Play {
    named(if rare { "mimic_rare" } else { "mimic" }, 0.0, 1.0)
}
pub fn capture() -> Play {
    named("capture", 0.0, 1.0)
}
pub fn escape() -> Play {
    named("escape", 0.0, 1.0)
}
/// A co-op teammate's step (remote_players.gd): scaled like a hunter step.
pub fn remote_player_footstep(mode: &str, pan: f64, volume: f64) -> Play {
    let resolved = resolve_gait(mode);
    scaled(
        &format!("teammate_footstep_{}", resolved),
        pan,
        (volume / 0.04).clamp(0.08, 1.5),
    )
}
/// `Sound.office_jumpscare(hunter_id)`: a pitch tint from the id (sound.gd 485). Disabled on the route
/// (HunterJumpscare.ENABLED = false).
pub fn office_jumpscare(hunter_id: &str) -> Play {
    let tint = (super::godot_string_hash(hunter_id) as i64).abs() % 90;
    named("office_jumpscare", 0.0, 1.0 + tint as f64 / 900.0)
}
/// The Night Watch office queue's names (`queue_office_cue`, sound.gd 469): anything else plays "control".
pub fn office_cue(cue: &str, pan: f64, intensity: f64) -> Play {
    const KNOWN: [&str; 11] = [
        "camera",
        "warning",
        "crimson_door",
        "presence",
        "scrape",
        "static",
        "drain",
        "footsteps",
        "breath",
        "shift",
        "control",
    ];
    let resolved = if KNOWN.contains(&cue) { cue } else { "control" };
    named(&format!("office_{}", resolved), pan, intensity)
}

// ---- gaits ----

/// Metres per footfall per gait (sound.gd 411) and the fixed gaps used without a speed (413).
pub const STEP_STRIDE_M: [(&str, f64); 3] = [("sprint", 1.28), ("walk", 1.34), ("crouch", 1.15)];
pub const STEP_GAP_FALLBACK: [(&str, f64); 3] =
    [("sprint", 0.205), ("walk", 0.335), ("crouch", 0.51)];
pub const STEP_GAP_MIN: f64 = 0.12;
pub const STEP_GAP_MAX: f64 = 0.9;
/// Below this ground speed (m/s) no steps (walking into a wall).
pub const STEP_MIN_SPEED: f64 = 0.3;

fn resolve_gait(mode: &str) -> &'static str {
    match mode {
        "sprint" => "sprint",
        "crouch" => "crouch",
        _ => "walk",
    }
}

fn lookup(table: &[(&str, f64); 3], gait: &str) -> f64 {
    table
        .iter()
        .find(|(g, _)| *g == gait)
        .map(|(_, v)| *v)
        .unwrap_or(table[1].1)
}

/// `Sound.footstep_gap(mode, speed_mps)` (sound.gd 421): seconds between footfalls; speed < 0 = the gait's fixed gap.
pub fn footstep_gap(mode: &str, speed_mps: f64) -> f64 {
    let gait = resolve_gait(mode);
    if speed_mps < 0.0 {
        return lookup(&STEP_GAP_FALLBACK, gait);
    }
    (lookup(&STEP_STRIDE_M, gait) / speed_mps.max(0.01)).clamp(STEP_GAP_MIN, STEP_GAP_MAX)
}

// ---- stateful front end ----

/// The throttles and the shared step counter of the Sound autoload. Times are microseconds of any monotonic clock.
#[derive(Clone, Debug)]
pub struct SoundFront {
    last_heartbeat_usec: i64,
    last_footstep_usec: i64,
    last_struggle_usec: i64,
    last_menu_usec: i64,
    /// `_footstep_index`: bumped by every runner step and every surface hunter step; its parity also sides the struggle.
    pub footstep_index: i64,
    /// The bus runner steps play on. "SFX" on the finite route (only LevelAmbience, infinite route, moves it to "Level").
    pub footstep_bus: Bus,
}

impl Default for SoundFront {
    fn default() -> Self {
        SoundFront {
            last_heartbeat_usec: -2_000_000,
            last_footstep_usec: -2_000_000,
            last_struggle_usec: -2_000_000,
            last_menu_usec: -2_000_000,
            footstep_index: 0,
            footstep_bus: Bus::Sfx,
        }
    }
}

impl SoundFront {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Sound.menu_blip()`: every button press in every menu (all BaseButtons are wired, except `silent_ui` touch
    /// controls), also sitting on a ride, the stim injection start and a "play" furniture. At most one per 45 ms, UI bus.
    pub fn menu_blip(&mut self, now_usec: i64) -> Option<Play> {
        if now_usec - self.last_menu_usec < 45_000 {
            return None;
        }
        self.last_menu_usec = now_usec;
        Some(Play {
            bus: Bus::Ui,
            ..named("menu", 0.0, 1.0)
        })
    }

    /// `Sound.heartbeat(danger)` (sound.gd 395): silent under 0.04; the gap shrinks from 1.08 s to 0.32 s with danger.
    pub fn heartbeat(&mut self, intensity: f64, now_usec: i64) -> Option<Play> {
        let danger = intensity.clamp(0.0, 1.0);
        if danger < 0.04 {
            return None;
        }
        let gap_usec = ((0.32f64).max(1.08 - danger * 0.73) * 1_000_000.0) as i64;
        if now_usec - self.last_heartbeat_usec < gap_usec {
            return None;
        }
        self.last_heartbeat_usec = now_usec;
        Some(named("heartbeat", 0.0, danger))
    }

    /// `Sound.footstep(mode, surface, speed_mps)` (sound.gd 430): one of the runner's own steps, throttled to the gait's
    /// cadence at the body's real ground speed (-1 = fixed gap). Alternates feet: pan +-0.045. `surface` "" is the
    /// generic step (the finite route never sets a surface; only the vent crawl calls footstep("crouch", "metal")).
    pub fn footstep(
        &mut self,
        mode: &str,
        surface: &str,
        speed_mps: f64,
        now_usec: i64,
    ) -> Option<Play> {
        let gait = resolve_gait(mode);
        if (0.0..STEP_MIN_SPEED).contains(&speed_mps) {
            return None;
        }
        if now_usec - self.last_footstep_usec < (footstep_gap(gait, speed_mps) * 1_000_000.0) as i64
        {
            return None;
        }
        self.last_footstep_usec = now_usec;
        self.footstep_index += 1;
        let side = if self.footstep_index % 2 != 0 {
            1.0
        } else {
            -1.0
        };
        let cue = if surface.is_empty() {
            format!("footstep_{}", gait)
        } else {
            format!("step:{}:{}:{}", gait, surface, self.footstep_index % 4)
        };
        Some(Play {
            bus: self.footstep_bus,
            ..named(&cue, side * 0.045, 1.0)
        })
    }

    /// `Sound.hunter_footstep(pan, volume, surface)` (sound.gd 273): the heavy two-thump step, or a surface step for a
    /// hunter when `surface` is given. Played scaled: player gain = clamp(volume / 0.05, 0.1, 2.0).
    pub fn hunter_footstep(&mut self, pan: f64, volume: f64, surface: &str) -> Play {
        let level = (volume / 0.05).clamp(0.1, 2.0);
        if surface.is_empty() {
            return scaled("hunter_footstep", pan, level);
        }
        self.footstep_index += 1;
        scaled(
            &format!("step:hunter:{}:{}", surface, self.footstep_index % 4),
            pan,
            level,
        )
    }

    /// `Sound.struggle(effort)` (sound.gd 451): each escape press during a grab, effort = escape_progress / required.
    /// Gap 130 ms at effort 0 down to 85 ms at 1.
    pub fn struggle(&mut self, intensity: f64, now_usec: i64) -> Option<Play> {
        let effort = intensity.clamp(0.0, 1.0);
        let gap_usec = ((0.13 - effort * 0.045) * 1_000_000.0) as i64;
        if now_usec - self.last_struggle_usec < gap_usec {
            return None;
        }
        self.last_struggle_usec = now_usec;
        let pan = if self.footstep_index % 2 != 0 {
            -0.035
        } else {
            0.035
        };
        Some(named("struggle", pan, effort))
    }
}

// ---- call-site rules of the finite route (main_3d.gd) ----

/// The heartbeat danger fed to `heartbeat` every frame (main_3d.gd 1715-1720): from the nearest ACTIVE hunter's grid
/// distance (cells), at least 0.48 while a hunter chases this player, 0 in a safe zone.
pub fn heartbeat_danger(
    nearest_active_hunter_cells: Option<f64>,
    locally_pursued: bool,
    in_safe_zone: bool,
) -> f64 {
    let mut danger = match nearest_active_hunter_cells {
        None => 0.0,
        Some(d) => (1.0 - (d - 0.5) / 9.0).clamp(0.0, 1.0),
    };
    if locally_pursued {
        danger = danger.max(0.48);
    }
    if in_safe_zone {
        danger = 0.0;
    }
    danger
}

/// Hunter footstep pan and volume (main_3d.gd 1729-1735): only within 9 cells; pan is the offset along the player's
/// right (facing rotated, `right = (-facing.y, facing.x)`) over 4.5 cells; volume `.09 * (1 - d / 9)`.
/// `offset` = hunter grid position - player grid position, `facing` = the player's 2D facing (x, z).
pub fn hunter_step_pan_volume(offset: (f64, f64), facing: (f64, f64)) -> Option<(f64, f64)> {
    let distance = (offset.0 * offset.0 + offset.1 * offset.1).sqrt();
    if distance > 9.0 {
        return None;
    }
    let right = (-facing.1, facing.0);
    let pan = ((offset.0 * right.0 + offset.1 * right.1) / 4.5).clamp(-1.0, 1.0);
    Some((pan, 0.09 * (1.0 - distance / 9.0)))
}

/// The hunter step clock (main_3d.gd 1721-1728): one step per footfall of the nearest non-idle active hunter's gait.
/// `footfall_rate` (steps/s) is HunterView3D.footfall_rate: `2 * |anim speed_scale| / clip_length` while a
/// walk/chase/sprint clip plays, `move_speed / 0.75` for creature rigs, 0 when stunned / in vents / climbing / idle.
#[derive(Clone, Copy, Debug, Default)]
pub struct HunterStepClock {
    elapsed: f64,
}

impl HunterStepClock {
    /// True when a step is due this frame. Pass `stepping = false` when there is no stepping hunter or the player is
    /// hidden (the clock resets, as the original's does).
    pub fn tick(&mut self, delta: f64, footfall_rate: f64, stepping: bool) -> bool {
        if !stepping {
            self.elapsed = 0.0;
            return false;
        }
        self.elapsed += delta * footfall_rate;
        if self.elapsed < 1.0 {
            return false;
        }
        self.elapsed %= 1.0;
        true
    }
}

/// Thessine's mimic (main_3d.gd 2041-2062), every 10 s a 50 % roll; 8 % of those are "rare". `cueable` = the ids of
/// the OTHER active hunters among nyxie, glyra, ferryn, briselle, loxelle, ravelle, haelith; `choice` in 0..1.
pub fn thessine_mimic(rare: bool, choice: f64, cueable: &[&str]) -> Play {
    const CUEABLE: [&str; 7] = [
        "nyxie", "glyra", "ferryn", "briselle", "loxelle", "ravelle", "haelith",
    ];
    let mut ids: Vec<&str> = cueable
        .iter()
        .copied()
        .filter(|id| CUEABLE.contains(id))
        .collect();
    ids.sort();
    if rare || ids.is_empty() {
        return mimic(rare);
    }
    let choice = choice.clamp(0.0, 0.999999);
    match ids[(choice * ids.len() as f64) as usize] {
        "nyxie" => nyxie_timer_wake(),
        "glyra" => music_box(),
        "haelith" => SoundFront::new().hunter_footstep(0.0, 0.09, ""),
        _ => alert(),
    }
}

/// Pink's false footsteps (void_player_effects.gd 88): the real-hunter curve x 0.55, never below 0.005.
pub fn fake_step_volume(distance: f64) -> f64 {
    (0.09 * (1.0 - distance.clamp(0.0, 9.0) / 9.0) * 0.55).max(0.005)
}
