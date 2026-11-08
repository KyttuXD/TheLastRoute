//! The original's `Sound` autoload as free functions (game/autoload/sound.gd 164-464): call these where the original
//! calls `Sound.<method>()`. Each renders the cue at the original's pan / intensity (cached by its cache key), applies its
//! throttle and plays it at its player gain on its bus. The trigger of every cue on the route is in cues/NOTES.md.
//! All of them are silent until `Sounds::new()` has started the engine.

#![allow(dead_code)]
use super::cues::rules::{self, Play};
use super::engine::with;

/// Plays any `rules::Play` (for the cues without a helper below: `play(rules::carnival("bell", 0.0, 1.0))`).
pub fn play(p: Play) {
    with(|e| e.play(&p));
}

/// Relay, battery, coin, tool pickup, locked cache opened (route_interactable_3d.gd 747-766, 1048).
pub fn collect() {
    play(rules::collect());
}
/// `Sound.recharge(amount)`: battery used 25 (player_controller_3d.gd 3376), safe zone 20 (ri 793), stim lands 100 (3339).
pub fn recharge(amount: f32) {
    play(rules::recharge(amount as f64));
}
/// Every UI button press (all BaseButtons are wired, sound.gd 804-812), sitting on a ride, a stim injection starting,
/// "play" furniture. At most one per 45 ms, UI bus.
pub fn menu_blip() {
    with(|e| {
        if !e.ready {
            return;
        }
        let now = e.now_usec();
        if let Some(p) = e.front.menu_blip(now) {
            e.play(&p);
        }
    });
}
/// Winding the music box (ri 818); Thessine mimicking Glyra.
pub fn music_box() {
    play(rules::music_box());
}
pub fn alert() {
    play(rules::alert());
}
/// The frame the runner becomes pursued (`world::update` plays this itself).
pub fn chase_start() {
    play(rules::chase_start());
}
pub fn taser() {
    play(rules::taser());
}
/// A room door toggled (ri 735) or the vent hatch: entering opens, leaving closes (pc 3811 / 3823).
pub fn door(open: bool) {
    play(rules::door(open));
}
pub fn door_breach() {
    play(rules::door_breach());
}
/// A refused or failed interaction (pc 3886-3890: any `_fail(...)`, e.g. INVENTORY FULL, the lift without its relays,
/// the music box with nothing to resolve; wrong tool 3392, no durability 3404, stim still active 3322).
pub fn interaction_fail() {
    play(rules::interaction_fail());
}
/// A tool's durability running out (pc 3132).
pub fn tool_break() {
    play(rules::tool_break());
}
pub fn axe_swing() {
    play(rules::axe_swing(0.0, 1.0));
}
pub fn axe_chop() {
    play(rules::axe_chop(0.0, 1.0));
}
/// The flashlight running dead (pc 2419), F on an empty flashlight (2442). A charged flashlight toggles silently.
pub fn light_switch() {
    play(rules::light_switch(0.0, 1.0));
}
pub fn capture() {
    play(rules::capture());
}
pub fn escape() {
    play(rules::escape());
}
/// Each escape press during a grab: `effort = escape_progress / escape_required` (main_3d.gd 1050).
pub fn struggle(effort: f32) {
    with(|e| {
        if !e.ready {
            return;
        }
        let now = e.now_usec();
        if let Some(p) = e.front.struggle(effort as f64, now) {
            e.play(&p);
        }
    });
}
pub fn nyxie_timer_wake() {
    play(rules::nyxie_timer_wake());
}
pub fn nyxie_empty_wake() {
    play(rules::nyxie_empty_wake());
}
/// Thessine's fallback mimic.
pub fn mimic(rare: bool) {
    play(rules::mimic(rare));
}
/// Thessine's mimic (main_3d.gd 2041-2062): `cueable` = ids of the other active hunters, `choice` in 0..1.
pub fn thessine_mimic(rare: bool, choice: f32, cueable: &[&str]) {
    play(rules::thessine_mimic(rare, choice as f64, cueable));
}
/// `Sound.heartbeat(danger)`: throttled (gap 1.08 s down to 0.32 s), silent under 0.04. `world::update` feeds the
/// danger every frame; call this only for extra beats (the stim crash uses `stim_crash`).
pub fn heartbeat(danger: f32) {
    with(|e| {
        if !e.ready {
            return;
        }
        let now = e.now_usec();
        if let Some(p) = e.front.heartbeat(danger as f64, now) {
            e.play(&p);
        }
    });
}
/// The stim crash heart (pc 3341-3353): call every frame with whether the crash is on; one `heartbeat(.65)` at once
/// and then every 0.8 s while it lasts.
pub fn stim_crash(dt: f32, crashing: bool) {
    let beat = with(|e| {
        if crashing && !e.stim_crashing {
            e.stim_heartbeat_in = 0.0;
        }
        e.stim_crashing = crashing;
        if !crashing {
            return false;
        }
        e.stim_heartbeat_in -= dt;
        if e.stim_heartbeat_in <= 0.0 {
            e.stim_heartbeat_in = 0.8;
            return true;
        }
        false
    });
    if beat {
        heartbeat(0.65);
    }
}
/// One of the runner's own steps: call EVERY frame the runner is moving on the floor (pc 1992-1993) with the gait
/// ("walk" / "sprint" / "crouch"), the floor surface ("" on the finite route; "metal" for the vent crawl) and the body's
/// real ground speed in m/s (< 0: the gait's fixed gap). The cadence (stride / speed), the 0.3 m/s floor and the
/// alternating feet are the original's.
pub fn footstep(mode: &str, surface: &str, speed_mps: f32) {
    with(|e| {
        if !e.ready {
            return;
        }
        let now = e.now_usec();
        if let Some(p) = e.front.footstep(mode, surface, speed_mps as f64, now) {
            e.play(&p);
        }
    });
}
/// `Sound.hunter_footstep(pan, volume)`: a hunter's heavy step, scaled (gain `volume / .05`, 0.1-2). `world::update`
/// plays the real hunter steps; this is for the fakes (Pink's, `rules::fake_step_volume`; the troll step).
pub fn hunter_footstep(pan: f32, volume: f32) {
    with(|e| {
        let p = e.front.hunter_footstep(pan as f64, volume as f64, "");
        e.play(&p);
    });
}

/// The swinging tools (held_item_poses.gd 65-66).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwingTool {
    Axe,
    Crowbar,
}

/// A tool swing (pc `_begin_tool_swing` 3447-3469): the axe whooshes at once (`axe_swing`), the crowbar is silent;
/// on a `hit` the impact lands after windup + strike (axe 0.24 s: `axe_chop`; crowbar 0.21 s: `door_breach`).
/// A swing at nothing is `hit = false`.
pub fn tool_swing(tool: SwingTool, hit: bool) {
    with(|e| {
        if tool == SwingTool::Axe {
            e.play(&rules::axe_swing(0.0, 1.0));
        }
        if hit {
            match tool {
                SwingTool::Axe => e.schedule(0.16 + 0.08, rules::axe_chop(0.0, 1.0)),
                SwingTool::Crowbar => e.schedule(0.12 + 0.09, rules::door_breach()),
            }
        }
    });
}

/// A hunter lingering at the runner's locker (hunter_view_3d.gd 1537-1558): one breath (Nyxie two, 1.1 s apart) when
/// she is within 3.5 m of the camera. `side` = metres of her offset along the camera's right, `gap` = her distance.
pub fn inspect_breath(side: f32, gap: f32, two_breaths: bool) {
    if gap > 3.5 {
        return;
    }
    let intensity = (1.0 - gap as f64 / 3.5 * 0.5) * if two_breaths { 1.15 } else { 1.0 };
    let p = rules::nw("breath", (side as f64).clamp(-1.0, 1.0), intensity);
    with(|e| {
        e.play(&p);
        if two_breaths {
            e.schedule(1.1, p.clone());
        }
    });
}

pub fn sleep_settle() {
    play(rules::sleep_settle());
}
pub fn sleep_yawn() {
    play(rules::sleep_yawn());
}
pub fn sleep_stir(hunter_distance: f32) {
    play(rules::sleep_stir(
        (1.0 - hunter_distance as f64 / 12.0).clamp(0.25, 1.0),
    ));
}
pub fn sleep_jolt() {
    play(rules::sleep_jolt());
}
pub fn hunter_laugh(pan: f32, volume: f32) {
    play(rules::hunter_laugh(pan as f64, volume as f64));
}
pub fn signal_node(progress: f32) {
    play(rules::signal_node(progress as f64));
}
pub fn radar_static() {
    play(rules::radar_static());
}
pub fn system_offline() {
    play(rules::system_offline());
}
pub fn xivvy_screech(pan: f32, intensity: f32) {
    play(rules::xivvy_screech(pan as f64, intensity as f64));
}
pub fn xivvy_wing(pan: f32, intensity: f32) {
    play(rules::xivvy_wing(pan as f64, intensity as f64));
}
pub fn nw(name: &str, pan: f32, intensity: f32) {
    play(rules::nw(name, pan as f64, intensity as f64));
}

/// The voice of each placed-tune id (one voice per tune: a pair shares its tune, so it is one voice).
const PLACED_TUNE_VOICES: [&str; 8] = [
    "music_box_placed_0",
    "music_box_placed_1",
    "music_box_placed_2",
    "music_box_placed_3",
    "music_box_placed_4",
    "music_box_placed_5",
    "music_box_placed_6",
    "music_box_placed_7",
];

/// The tune loop played `rate` times as fast (nearest-frame reads; the pitch moves with the speed).
fn tune_at_rate(rate: f32) -> Vec<[f32; 2]> {
    let base = super::cues::music_box::tune();
    if (rate - 1.0).abs() < 1e-4 || base.is_empty() {
        return base;
    }
    let n = (base.len() as f32 / rate) as usize;
    (0..n).map(|i| base[(((i as f32) * rate) as usize).min(base.len() - 1)]).collect()
}

/// Call every frame with `crate::game::music_box::tune_requests(..)` for the wound placed boxes: starts the loops
/// the requests name at their gains, retargets running ones, and stops every other placed-tune voice.
pub fn placed_music_boxes(requests: &[crate::game::music_box::TuneRequest]) {
    with(|e| {
        for (i, voice) in PLACED_TUNE_VOICES.iter().enumerate() {
            let req = requests.iter().find(|r| r.tune as usize % PLACED_TUNE_VOICES.len() == i);
            let tune = i as u32;
            e.set_loop(*voice, rules::Bus::Master, req.map(|r| r.gains), || {
                tune_at_rate(crate::game::music_box::tune_rate(tune))
            });
        }
    });
}
