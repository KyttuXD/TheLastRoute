//! Per-frame world sound: main_3d.gd `_update_local_audio` (1682-1735) and the hunter views' footfall clocks and
//! Glyra's hum (hunter_view_3d.gd 1565-1588, 1676-1686). Call `update` once per game frame, ALSO while the runner is
//! hidden or struggling (the original runs it every frame; the heartbeat keeps going in a locker).

#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::HashMap;

use super::cues::music_box::{RunClock, RunRender};
use super::cues::AmbienceMixer;
use super::cues::positional::HUM;
use super::cues::rules::{self, Bus, HunterStepClock};
use super::cues::stride::{preferred_clip, HunterGait};
use super::engine::with;

/// The runner as the sound sees them.
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    /// Grid position (cells = metres) and the 2D facing (x, z): `p.pos`, `forward2()`.
    pub pos: (f32, f32),
    pub facing: (f32, f32),
    /// The camera (the original's listener): eye position and basis, from `eye()` / `camera_basis()`.
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub hidden: bool,
    pub in_safe_zone: bool,
}

/// One hunter as the sound sees her.
#[derive(Clone, Copy, Debug)]
pub struct HunterAudio<'a> {
    /// HunterDef id ("glyra", "nyxie", ...): picks her stride table and her special cues.
    pub id: &'a str,
    pub pos: (f32, f32),
    pub active: bool,
    /// brain state: CHASE / WANDER-INVESTIGATE-SEARCH / IDLE.
    pub chasing: bool,
    pub idle: bool,
    pub direct_visible: bool,
    pub winded: bool,
    /// stun > 0, in the vents or climbing: no footfalls (and a stunned chaser is no pursuit cue).
    pub stunned: bool,
    pub in_vent: bool,
    /// Nyxie: woken by the empty flashlight (else the timer). Ignored for the others.
    pub nyxie_empty_wake: bool,
    /// Glyra: her box is PLAYING (`Glyra::playing()`). Ignored for the others.
    pub glyra_humming: bool,
    /// The hunter's target is the runner (not a bot or fake player). Only these feed the danger readout, the
    /// heartbeat and the chase cue.
    pub targets_runner: bool,
}

/// (nearest runner-targeting active hunter distance, pursued) for the heartbeat and the chase cue.
pub fn runner_danger_inputs(pos: (f32, f32), hunters: &[HunterAudio]) -> (Option<f64>, bool) {
    let mut nearest = f32::INFINITY;
    let mut pursued = false;
    for h in crate::hunters::target_filter::runner_targeting(hunters, |h| h.targets_runner) {
        if !h.active {
            continue;
        }
        let d = ((h.pos.0 - pos.0).powi(2) + (h.pos.1 - pos.1).powi(2)).sqrt();
        nearest = nearest.min(d);
        if h.chasing && !h.stunned && (h.direct_visible || d <= CHASE_CUE_RANGE) {
            pursued = true;
        }
    }
    (nearest.is_finite().then_some(nearest as f64), pursued)
}

#[cfg(test)]
mod runner_only_tests {
    use super::*;

    fn h(chasing: bool, targets_runner: bool) -> HunterAudio<'static> {
        HunterAudio {
            id: "glyra", pos: (3.0, 0.0), active: true, chasing, idle: false, direct_visible: true,
            winded: false, stunned: false, in_vent: false, nyxie_empty_wake: false,
            glyra_humming: false, targets_runner,
        }
    }

    #[test]
    fn hunter_chasing_a_bot_raises_no_danger_or_pursuit() {
        let (near, pursued) = runner_danger_inputs((0.0, 0.0), &[h(true, false)]);
        assert!(near.is_none());
        assert!(!pursued);
    }

    #[test]
    fn hunter_chasing_the_runner_raises_both() {
        let (near, pursued) = runner_danger_inputs((0.0, 0.0), &[h(true, false), h(true, true)]);
        assert_eq!(near, Some(3.0));
        assert!(pursued);
    }
}

#[derive(Default)]
struct WorldState {
    gaits: Vec<HunterGait>,
    was_active: HashMap<String, bool>,
    was_pursued: bool,
    clock: HunterStepClock,
    /// The music box tune that is playing: what the run was started for, and the box clock at the last frame.
    box_run: Option<RunClock>,
    /// The music box run being rendered a slice a frame (`music_box`).
    box_render: Option<RunRender>,
    /// The ambience crossfade between regions (`ambience`).
    ambience: AmbienceMixer,
    /// The bed each ambience voice really plays (a voice that changes bed is stopped first).
    ambience_playing: [Option<&'static str>; 2],
    /// The floor surface under each hunter id (`set_hunter_surface`), for her footsteps. Absent: the heavy step.
    surfaces: HashMap<String, String>,
}

/// The two ambience loop voices: a region change fades the new bed in on the idle one while the old one fades out.
const AMBIENCE_VOICES: [&str; 2] = ["ambience_a", "ambience_b"];

thread_local! {
    /// The beds already rendered (a bed is a few seconds of synth maths): at most `AMBIENCE_CACHE` are kept.
    static BEDS: RefCell<Vec<(&'static str, Vec<[f32; 2]>)>> = RefCell::new(Vec::new());
}
const AMBIENCE_CACHE: usize = 4;

fn bed_frames(id: &'static str) -> Vec<[f32; 2]> {
    BEDS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, f)) = c.iter().find(|(k, _)| *k == id) {
            return f.clone();
        }
        let frames = super::cues::ambience(id, super::cues::ambience::settings().loop_seconds);
        if c.len() >= AMBIENCE_CACHE {
            c.remove(0);
        }
        c.push((id, frames.clone()));
        frames
    })
}

/// The region ambience: call once per game frame with the id of the region the runner stands in (`None` in menus or
/// nowhere), and `duck` 0..1 to turn the beds down (the original's `set_ambience_duck`). The bed for the region is
/// looked up in data/audio/ambience.json (own bed, alias, else the fallback), crossfaded over `region_fade_seconds`
/// when it changes, and played on the SFX bus so the sliders reach it.
pub fn ambience(dt: f32, region: Option<&str>, duck: f32) {
    WORLD.with(|w| {
        let w = &mut *w.borrow_mut();
        with(|e| {
            if !e.ready {
                return;
            }
            let out = w.ambience.step_ducked(dt, region, duck);
            for (n, (voice, slot)) in AMBIENCE_VOICES.iter().zip(out.iter()).enumerate() {
                if w.ambience_playing[n].is_some() && w.ambience_playing[n] != slot.bed {
                    e.set_loop(*voice, Bus::Sfx, None, Vec::new);
                }
                w.ambience_playing[n] = slot.bed;
                match slot.bed {
                    Some(id) => e.set_loop(*voice, Bus::Sfx, Some((slot.gain, slot.gain)), || bed_frames(id)),
                    None => e.set_loop(*voice, Bus::Sfx, None, Vec::new),
                }
            }
        });
    });
}

/// The bed a region id plays (`None` when the data has no bed at all): for the debug overlay and the tests.
pub fn ambience_bed(region: &str) -> Option<&'static str> {
    Some(super::cues::ambience::bed_for(region)).filter(|b| !b.is_empty())
}

/// The floor surface a hunter is walking on ("carpet", "wood", "tile", "concrete", "metal", "stone", "grass", "water";
/// "" clears it): her footsteps then use that surface's step, like `Sound.hunter_footstep(.., surface)`. Call it when
/// the hunter's floor changes, from the side that simulates the hunters.
pub fn set_hunter_surface(id: &str, surface: &str) {
    WORLD.with(|w| {
        let w = &mut *w.borrow_mut();
        if surface.is_empty() {
            w.surfaces.remove(id);
        } else if w.surfaces.get(id).map(String::as_str) != Some(surface) {
            w.surfaces.insert(id.to_string(), surface.to_string());
        }
    });
}

/// The voice of the wound music box's tune (one pre-rendered run, see `music_box`).
const BOX_VOICE: &str = "music_box_run";

/// The music box run is rendered in slices of this many frames, until this much time of a game frame is spent: the render
/// is a few million frames, which on one wasm thread would stall a frame for tenths of a second if done at once. The
/// finished run is the same samples either way (`RunRender`).
const BOX_RENDER_SLICE: usize = 4096;
const BOX_RENDER_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);

/// The wound music box's sound (music_box.rs): its tune, steady at normal speed while the wind is above the slowdown line
/// (70 % by default), then slower and lower as it drains, stopped when it is empty. The loops cannot change pitch while
/// they play, so the whole wound time is ONE pre-rendered run (`run_frames`, a pure function of the wound time) started
/// when the box is wound: the same samples, with no hand-over between a steady loop and a slowing one, on the browser
/// build (where loops decode asynchronously) as on desktop. The run starts again if the box is wound again, or the game
/// stood still while it played (paused). Not positional: the box is in the pack, so there is no source to hear it from.
/// Called every frame with the seconds left and the wound time (game/mod.rs `run_music_box`). On the Master bus (the tune
/// is an AudioStreamPlayer3D in the original), so Master reaches it live.
pub fn music_box(left: f32, wound: f32) {
    use super::cues::music_box as mb;
    WORLD.with(|w| {
        let w = &mut *w.borrow_mut();
        with(|e| {
            if !e.ready {
                return;
            }
            if left <= 0.0 {
                // empty: the tune stops (the run is silent from here anyway)
                w.box_render = None;
                if w.box_run.take().is_some() {
                    e.set_loop(BOX_VOICE, Bus::Master, None, Vec::new);
                }
                return;
            }
            let now = e.now();
            let restart = mb::run_needs_restart(w.box_run.as_ref(), left, wound, now);
            w.box_run = Some(mb::next_clock(w.box_run.as_ref(), restart, left, wound, now));
            if restart {
                // the run is a few million frames: it is rendered a slice a frame (below), not in this one. What played
                // before no longer matches the box clock, so it stops now and the new run starts when it is ready.
                e.set_loop(BOX_VOICE, Bus::Master, None, Vec::new);
                w.box_render = Some(RunRender::new(wound, wound - left));
            }
            if let Some(render) = w.box_render.as_mut() {
                let t0 = crate::shared::clock::Instant::now();
                let mut done = false;
                while !done && t0.elapsed() < BOX_RENDER_BUDGET {
                    done = render.step(BOX_RENDER_SLICE);
                }
                if done {
                    if let Some(render) = w.box_render.take() {
                        // the box clock ran on while the run rendered: join it where it is now, as a run started now would
                        let tune = crate::game::music_box::rules().tune_gain;
                        let frames = render.finish_at(wound - left);
                        e.play_frames(BOX_VOICE, Bus::Master, frames, (tune, tune), left);
                    }
                }
            }
        });
    });
}

thread_local! {
    static WORLD: RefCell<WorldState> = RefCell::new(WorldState::default());
}

/// Forget the floor's hunters (call when a floor starts or a run ends): their gait clocks, wake edges, the pursuit
/// edge, and stop the loops and queued cues.
pub fn reset() {
    WORLD.with(|w| *w.borrow_mut() = WorldState::default());
    super::creature::reset();
    super::stop_all();
}

/// CHASE_CUE_RANGE (hunter_brain.gd 520): a chaser this close is a pursuit cue even out of sight.
const CHASE_CUE_RANGE: f32 = 18.0;

pub fn update(dt: f32, l: &Listener, hunters: &[HunterAudio]) {
    WORLD.with(|w| {
        let w = &mut *w.borrow_mut();
        if w.gaits.len() != hunters.len() {
            w.gaits = vec![HunterGait::new(); hunters.len()];
        }
        with(|e| {
            if !e.ready {
                return;
            }
            e.pump();
            let now = e.now_usec();
            let mut stepping: Option<(usize, f32)> = None;
            let (nearest, pursued) = runner_danger_inputs(l.pos, hunters);
            for (i, h) in hunters.iter().enumerate() {
                let d = ((h.pos.0 - l.pos.0).powi(2) + (h.pos.1 - l.pos.1).powi(2)).sqrt();
                let was = w.was_active.get(h.id).copied().unwrap_or(false);
                if h.active {
                    // the wolf spiders skitter (audio/creature.rs) instead of walking in a humanoid's boots
                    if !h.idle && !crate::hunters::secret_creatures::is_secret(h.id) && stepping.map(|s| d < s.1).unwrap_or(true) {
                        stepping = Some((i, d));
                    }
                    if h.id == "nyxie" && !was {
                        e.play(&if h.nyxie_empty_wake {
                            rules::nyxie_empty_wake()
                        } else {
                            rules::nyxie_timer_wake()
                        });
                    }
                }
                // no String per hunter per frame: allocate the key only the first time a hunter is seen
                match w.was_active.get_mut(h.id) {
                    Some(v) => *v = h.active,
                    None => {
                        w.was_active.insert(h.id.to_string(), h.active);
                    }
                }
                // every view runs its gait (so the eased speed is right when she becomes the stepping one)
                let clip =
                    preferred_clip(h.chasing, !h.chasing && !h.idle, h.direct_visible, h.winded);
                w.gaits[i].update(h.id, h.pos, dt, clip, h.active && !h.stunned && !h.in_vent);
                // Glyra's hum: positional loop on Master while she is active and her box plays. The engine applies the
                // Master gain (and re-applies it on every master change), so the base gains here are the positional ones.
                if h.id == "glyra" {
                    let gains = (h.active && h.glyra_humming).then(|| {
                        HUM.gains_from([h.pos.0, 0.0, h.pos.1], l.eye, l.right, l.up, l.forward)
                    });
                    e.set_loop("glyra_hum", Bus::Master, gains, super::cues::music_box::glyra_hum);
                }
            }
            if !hunters.iter().any(|h| h.id == "glyra") {
                e.set_loop("glyra_hum", Bus::Master, None, Vec::new);
            }
            if pursued && !w.was_pursued {
                e.play(&rules::chase_start());
            }
            w.was_pursued = pursued;
            let danger = rules::heartbeat_danger(
                nearest,
                pursued,
                l.in_safe_zone,
            );
            if let Some(p) = e.front.heartbeat(danger, now) {
                e.play(&p);
            }
            let rate = stepping
                .map(|(i, _)| w.gaits[i].footfall_rate)
                .unwrap_or(0.0);
            if !w
                .clock
                .tick(dt as f64, rate as f64, stepping.is_some() && !l.hidden)
            {
                return;
            }
            let (i, _) = stepping.unwrap();
            let h = &hunters[i];
            let offset = ((h.pos.0 - l.pos.0) as f64, (h.pos.1 - l.pos.1) as f64);
            if let Some((pan, volume)) =
                rules::hunter_step_pan_volume(offset, (l.facing.0 as f64, l.facing.1 as f64))
            {
                let surface = w.surfaces.get(h.id).map(String::as_str).unwrap_or("");
                let p = e.front.hunter_footstep(pan, volume, surface);
                e.play(&p);
            }
        });
    });
}
