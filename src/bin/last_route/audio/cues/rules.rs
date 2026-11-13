//! The front end of the original's sound system: which cue each `Sound.<method>()` plays, at what pan / intensity /
//! player volume, on which bus, with which throttle (game/autoload/sound.gd 164-464), plus the call-site rules of the
//! finite route that decide pan and volume (game/main_3d.gd `_update_local_audio` 1682-1735, `_play_thessine_mimic`
//! 2041-2062, game/player/void_player_effects.gd). Everything here is pure: the game passes the clock in.
//!
//! A `Play` is what to render (`Play::frames`, cacheable by `Play::key`) and how loud to play it (`Play::gain`, the
//! player's linear volume; the bus gain comes from the settings, see `bus_gain`).

use super::render::{cache_key, snapped};

/// The volume bus a sound plays on. Every sound has one (there is no bus that skips the sliders): `bus_for_cue` picks it
/// from data/audio/buses.json, the loops name theirs, and `bus_gain` turns it into the gain. `Sfx` is GAMEPLAY.
/// `Hunters`, `Ambience` and `Music` sit under GAMEPLAY; `Ui` is MASTER only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bus {
    Sfx,
    Hunters,
    Ambience,
    Music,
    Ui,
}

impl Bus {
    pub const ALL: [Bus; 5] = [Bus::Sfx, Bus::Hunters, Bus::Ambience, Bus::Music, Bus::Ui];

    /// The key buses.json uses for this bus.
    pub fn key(self) -> &'static str {
        match self {
            Bus::Sfx => "sfx",
            Bus::Hunters => "hunters",
            Bus::Ambience => "ambience",
            Bus::Music => "music",
            Bus::Ui => "ui",
        }
    }

    pub fn from_key(key: &str) -> Option<Bus> {
        Bus::ALL.into_iter().find(|b| b.key() == key)
    }
}

/// The settings' volume sliders as linear gains 0..1. MASTER scales everything; GAMEPLAY scales `Sfx` and the three buses
/// under it (HUNTERS, AMBIENCE, MUSIC); MENU / UI scales `Ui` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub master: f32,
    pub gameplay: f32,
    pub hunters: f32,
    pub ambience: f32,
    pub music: f32,
    pub ui: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Volumes { master: 1.0, gameplay: 1.0, hunters: 1.0, ambience: 1.0, music: 1.0, ui: 1.0 }
    }
}

/// The gain of `bus`: MASTER x its slider (x GAMEPLAY for the buses under it). The only place this is computed.
pub fn bus_gain(bus: Bus, v: &Volumes) -> f32 {
    let c = |x: f32| x.clamp(0.0, 1.0);
    c(v.master)
        * match bus {
            Bus::Sfx => c(v.gameplay),
            Bus::Hunters => c(v.gameplay) * c(v.hunters),
            Bus::Ambience => c(v.gameplay) * c(v.ambience),
            Bus::Music => c(v.gameplay) * c(v.music),
            Bus::Ui => c(v.ui),
        }
}

struct BusMap {
    default: Bus,
    exact: std::collections::HashMap<String, Bus>,
    /// Longest prefix first.
    prefix: Vec<(String, Bus)>,
    labels: [String; 5],
}

fn parse_bus_map(text: &str) -> BusMap {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    let bus_of = |x: &serde_json::Value| x.as_str().and_then(Bus::from_key);
    let mut exact = std::collections::HashMap::new();
    if let Some(o) = v["exact"].as_object() {
        for (k, b) in o {
            if let Some(b) = bus_of(b) {
                exact.insert(k.clone(), b);
            }
        }
    }
    let mut prefix: Vec<(String, Bus)> = vec![];
    if let Some(o) = v["prefix"].as_object() {
        for (k, b) in o {
            if let Some(b) = bus_of(b) {
                prefix.push((k.clone(), b));
            }
        }
    }
    prefix.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    let labels = Bus::ALL.map(|b| v["labels"][b.key()].as_str().unwrap_or(b.key()).to_uppercase());
    BusMap { default: bus_of(&v["default"]).unwrap_or(Bus::Sfx), exact, prefix, labels }
}

fn bus_map() -> &'static BusMap {
    static M: std::sync::OnceLock<BusMap> = std::sync::OnceLock::new();
    M.get_or_init(|| parse_bus_map(include_str!("../../data/audio/buses.json")))
}

/// The bus a cue plays on (data/audio/buses.json): exact name, else the longest prefix, else the default (GAMEPLAY).
pub fn bus_for_cue(cue: &str) -> Bus {
    let m = bus_map();
    if let Some(b) = m.exact.get(cue) {
        return *b;
    }
    m.prefix.iter().find(|(p, _)| cue.starts_with(p.as_str())).map(|(_, b)| *b).unwrap_or(m.default)
}

/// The settings caption of a bus, upper case (data/audio/buses.json `labels`).
pub fn bus_label(bus: Bus) -> &'static str {
    let i = Bus::ALL.iter().position(|b| *b == bus).unwrap_or(0);
    bus_map().labels[i].as_str()
}

/// The raw buses.json text and its parse, for the regression tests.
#[cfg(test)]
pub(crate) fn bus_map_entries() -> (Vec<(String, Bus)>, Vec<(String, Bus)>) {
    let m = bus_map();
    (m.exact.iter().map(|(k, b)| (k.clone(), *b)).collect(), m.prefix.clone())
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
        bus: bus_for_cue(cue),
    }
}

/// `_play_scaled` (sound.gd 751): one render at intensity 1 with the pan snapped to 0.25, the loudness on the player.
fn scaled(cue: &str, pan: f64, intensity: f64) -> Play {
    Play {
        cue: cue.to_string(),
        pan: snapped(pan.clamp(-1.0, 1.0), SCALED_PAN_STEP),
        intensity: 1.0,
        gain: intensity.clamp(0.05, 4.0) as f32,
        bus: bus_for_cue(cue),
    }
}

pub const SCALED_PAN_STEP: f64 = 0.25;

/// A pan the way `scaled` snaps it (data events that play scaled).
pub fn snap_scaled_pan(pan: f64) -> f64 {
    snapped(pan.clamp(-1.0, 1.0), SCALED_PAN_STEP)
}

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
            footstep_bus: bus_for_cue("footstep_walk"),
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
        Some(named("menu", 0.0, 1.0))
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

/// When the `chase_start` cue may play again, from data/audio/cues.json `chase_start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseCueRules {
    /// Seconds a hunter must have gone without pursuing the runner before its next pursuit is a new chase.
    pub rearm_seconds: f64,
    /// Cells within which a hunter counts as touching a runner it cannot capture (admin god mode).
    pub immune_touch_range: f64,
}

impl Default for ChaseCueRules {
    fn default() -> Self {
        ChaseCueRules { rearm_seconds: 6.0, immune_touch_range: 2.0 }
    }
}

impl ChaseCueRules {
    /// The `chase_start` block of a cues.json text; a missing or bad value keeps the default.
    pub fn parse(text: &str) -> ChaseCueRules {
        let d = ChaseCueRules::default();
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let c = &v["chase_start"];
        let num = |key: &str, d: f64| c[key].as_f64().filter(|x| x.is_finite() && *x >= 0.0).unwrap_or(d);
        ChaseCueRules {
            rearm_seconds: num("rearm_seconds", d.rearm_seconds),
            immune_touch_range: num("immune_touch_range", d.immune_touch_range),
        }
    }

    /// The embedded data's rules.
    pub fn get() -> ChaseCueRules {
        static R: std::sync::OnceLock<ChaseCueRules> = std::sync::OnceLock::new();
        *R.get_or_init(|| {
            let text = crate::content::embedded::data_under("audio/cues.json").next().map(|x| x.1).unwrap_or("{}");
            ChaseCueRules::parse(text)
        })
    }
}

/// Decides when `chase_start` plays. The cue is the frame the runner BECOMES pursued (no hunter pursued it the frame
/// before), and only when one of the hunters now pursuing has not pursued the runner for `rearm_seconds` (or never
/// has): a hunter that drops the runner and picks it up again sooner is still the same chase. A hunter touching a
/// runner it cannot capture never starts the cue, but its pursuit is remembered, so it stays silent afterwards too.
///
/// One frame is `begin(dt)`, one `note` per hunter, then `end()`. The clock is the game's own (the sum of `dt`), so a
/// paused game does not re-arm the cue. A hunter's key is allocated the first time it pursues, never per frame.
#[derive(Debug, Default)]
pub struct ChaseCueGate {
    clock: f64,
    /// Hunter key -> the clock at the last frame it pursued the runner.
    last_pursuit: std::collections::HashMap<String, f64>,
    was_pursued: bool,
    any: bool,
    fresh: bool,
}

impl ChaseCueGate {
    /// Starts a frame `dt` seconds after the last one.
    pub fn begin(&mut self, dt: f64) {
        self.clock += dt.max(0.0);
        self.any = false;
        self.fresh = false;
    }

    /// One hunter this frame: whether it pursues the runner, and whether it touches a runner it cannot capture.
    pub fn note(&mut self, rules: &ChaseCueRules, key: &str, pursuing: bool, touching_immune: bool) {
        if !pursuing {
            return;
        }
        self.any = true;
        match self.last_pursuit.get_mut(key) {
            Some(last) => {
                if self.clock - *last >= rules.rearm_seconds && !touching_immune {
                    self.fresh = true;
                }
                *last = self.clock;
            }
            None => {
                if !touching_immune {
                    self.fresh = true;
                }
                self.last_pursuit.insert(key.to_string(), self.clock);
            }
        }
    }

    /// Ends the frame: (the runner is pursued, the cue plays now).
    pub fn end(&mut self) -> (bool, bool) {
        let fire = self.any && !self.was_pursued && self.fresh;
        self.was_pursued = self.any;
        (self.any, fire)
    }
}

#[cfg(test)]
mod chase_cue_gate_tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    /// Runs `seconds` of frames with one hunter; `pursuing(frame)` says whether it pursues. Returns the cues played.
    fn run(gate: &mut ChaseCueGate, rules: &ChaseCueRules, seconds: f64, touching: bool, pursuing: impl Fn(usize) -> bool) -> usize {
        let mut fired = 0;
        for f in 0..(seconds / DT).round() as usize {
            gate.begin(DT);
            gate.note(rules, "h", pursuing(f), touching);
            if gate.end().1 {
                fired += 1;
            }
        }
        fired
    }

    #[test]
    fn the_data_has_a_rearm_time_of_a_few_seconds() {
        let r = ChaseCueRules::get();
        assert!(r.rearm_seconds >= 2.0 && r.rearm_seconds <= 30.0, "rearm_seconds {}", r.rearm_seconds);
        assert!(r.immune_touch_range > 0.0);
        assert_eq!(ChaseCueRules::parse("{}"), ChaseCueRules::default());
        assert_eq!(ChaseCueRules::parse(r#"{"chase_start":{"rearm_seconds":"x","immune_touch_range":-1}}"#), ChaseCueRules::default());
        assert_eq!(ChaseCueRules::parse(r#"{"chase_start":{"rearm_seconds":3.5}}"#).rearm_seconds, 3.5);
    }

    #[test]
    fn a_steady_chase_plays_the_cue_once() {
        let (mut g, r) = (ChaseCueGate::default(), ChaseCueRules::default());
        assert_eq!(run(&mut g, &r, 10.0, false, |_| true), 1);
    }

    #[test]
    fn losing_and_regaining_the_same_target_within_the_cooldown_stays_silent() {
        let (mut g, r) = (ChaseCueGate::default(), ChaseCueRules::default());
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 1, "the chase starts");
        // the state flaps: six frames chasing, six frames not, for ten seconds
        assert_eq!(run(&mut g, &r, 10.0, false, |f| (f / 6) % 2 == 0), 0, "a flapping chase is one chase");
        // lost for half the rearm time, then found again
        assert_eq!(run(&mut g, &r, r.rearm_seconds * 0.5, false, |_| false), 0);
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 0, "found again inside the cooldown: no cue");
    }

    #[test]
    fn a_new_chase_after_the_cooldown_plays_the_cue_again() {
        let (mut g, r) = (ChaseCueGate::default(), ChaseCueRules::default());
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 1);
        assert_eq!(run(&mut g, &r, r.rearm_seconds + 0.5, false, |_| false), 0);
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 1, "a chase after the cooldown is a new chase");
    }

    #[test]
    fn a_hunter_touching_an_immune_runner_never_starts_the_cue() {
        let (mut g, r) = (ChaseCueGate::default(), ChaseCueRules::default());
        // ten seconds of a chase that flaps every sense tick against a god-mode runner
        assert_eq!(run(&mut g, &r, 10.0, true, |f| (f / 6) % 2 == 0), 0);
        // the touch ends and the hunter keeps chasing: still the same chase
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 0);
    }

    #[test]
    fn another_hunter_starting_its_own_chase_is_not_silenced_by_the_first() {
        let (mut g, r) = (ChaseCueGate::default(), ChaseCueRules::default());
        assert_eq!(run(&mut g, &r, 1.0, false, |_| true), 1);
        assert_eq!(run(&mut g, &r, 1.0, false, |_| false), 0);
        g.begin(DT);
        g.note(&r, "other", true, false);
        assert_eq!(g.end(), (true, true), "a hunter that never chased the runner starts a new chase");
    }
}

/// How far a hunter's step (and the hunter cues heard like one: sniff, knock) carries, from data/audio/cues.json
/// `hunter_step` (Godot main_3d.gd 1729-1735: 9 cells, panned over 4.5, 0.09 at her feet).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HunterStepHearing {
    /// Cells past which the step is silent.
    pub range: f64,
    /// Cells of sideways offset that pan the step fully to one side.
    pub pan_span: f64,
    /// Volume at the hunter's feet; it falls linearly to 0 at `range`.
    pub volume: f64,
}

impl Default for HunterStepHearing {
    fn default() -> Self {
        HunterStepHearing { range: 9.0, pan_span: 4.5, volume: 0.09 }
    }
}

impl HunterStepHearing {
    /// The `hunter_step` block of a cues.json text; a missing or bad value keeps the Godot figure.
    pub fn parse(text: &str) -> HunterStepHearing {
        let d = HunterStepHearing::default();
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let h = &v["hunter_step"];
        let pos = |key: &str, d: f64| h[key].as_f64().filter(|x| x.is_finite() && *x > 0.0).unwrap_or(d);
        HunterStepHearing { range: pos("range", d.range), pan_span: pos("pan_span", d.pan_span), volume: pos("volume", d.volume) }
    }

    /// The embedded data's hearing.
    pub fn get() -> HunterStepHearing {
        static H: std::sync::OnceLock<HunterStepHearing> = std::sync::OnceLock::new();
        *H.get_or_init(|| {
            let text = crate::content::embedded::data_under("audio/cues.json").next().map(|x| x.1).unwrap_or("{}");
            HunterStepHearing::parse(text)
        })
    }

    /// The step volume `d` cells away: `volume * (1 - d / range)`, 0 at and past `range`.
    pub fn volume_at(&self, d: f64) -> f64 {
        self.volume * (1.0 - d.clamp(0.0, self.range) / self.range)
    }
}

/// Hunter footstep pan and volume (main_3d.gd 1729-1735): only within `hunter_step.range` cells (9); pan is the offset
/// along the player's right (facing rotated, `right = (-facing.y, facing.x)`) over `pan_span` cells (4.5); volume
/// `volume * (1 - d / range)` (.09). `offset` = hunter grid position - player grid position, `facing` = the player's 2D
/// facing (x, z). The figures are data (`HunterStepHearing`).
pub fn hunter_step_pan_volume(offset: (f64, f64), facing: (f64, f64)) -> Option<(f64, f64)> {
    hunter_step_pan_volume_with(&HunterStepHearing::get(), offset, facing)
}

/// [`hunter_step_pan_volume`] with the hearing given.
pub fn hunter_step_pan_volume_with(h: &HunterStepHearing, offset: (f64, f64), facing: (f64, f64)) -> Option<(f64, f64)> {
    let distance = (offset.0 * offset.0 + offset.1 * offset.1).sqrt();
    if distance > h.range {
        return None;
    }
    let right = (-facing.1, facing.0);
    let pan = ((offset.0 * right.0 + offset.1 * right.1) / h.pan_span).clamp(-1.0, 1.0);
    Some((pan, h.volume_at(distance)))
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
    (HunterStepHearing::get().volume_at(distance) * 0.55).max(0.005)
}

#[cfg(test)]
mod hunter_step_hearing_tests {
    use super::*;

    #[test]
    fn the_embedded_hunter_step_holds_the_godot_figures() {
        assert_eq!(HunterStepHearing::get(), HunterStepHearing::default());
        let (pan, vol) = hunter_step_pan_volume((4.5, 0.0), (0.0, -1.0)).unwrap();
        assert!((pan.abs() - 1.0).abs() < 1e-9 && (vol - 0.045).abs() < 1e-9, "{pan} {vol}");
        assert!(hunter_step_pan_volume((9.01, 0.0), (0.0, 1.0)).is_none());
        assert!((fake_step_volume(0.0) - 0.09 * 0.55).abs() < 1e-9);
    }

    #[test]
    fn changed_data_changes_the_reach_pan_and_volume() {
        let h = HunterStepHearing::parse(r#"{"hunter_step": {"range": 18.0, "pan_span": 9.0, "volume": 0.18}}"#);
        assert_eq!(h, HunterStepHearing { range: 18.0, pan_span: 9.0, volume: 0.18 });
        let (pan, vol) = hunter_step_pan_volume_with(&h, (0.0, 12.0), (1.0, 0.0)).expect("heard at 12 m");
        assert!((pan - 1.0).abs() < 1e-9 && (vol - 0.06).abs() < 1e-9, "{pan} {vol}");
        // a bad value keeps the Godot figure
        assert_eq!(HunterStepHearing::parse(r#"{"hunter_step": {"range": -1}}"#).range, 9.0);
    }
}
