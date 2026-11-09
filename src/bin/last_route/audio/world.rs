//! Per-frame world sound: main_3d.gd `_update_local_audio` (1682-1735) and the hunter views' footfall clocks and
//! Glyra's hum (hunter_view_3d.gd 1565-1588, 1676-1686). Call `update` once per game frame, ALSO while the runner is
//! hidden or struggling (the original runs it every frame; the heartbeat keeps going in a locker).

#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::HashMap;

use super::cues::music_box::{RunClock, RunRender};
use super::cues::AmbienceMixer;
use super::cues::positional::{Emitter3D, HUM};
use super::cues::rules::{self, Bus, HunterStepClock, HunterStepHearing};
use super::hunter_ambient::HunterAmbient;
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
    /// The rendered beds (a bed is a few seconds of synth maths): rendered a time-budgeted slice a frame, at most
    /// `render.cache_beds` kept (data/audio/ambience.json), least recently used evicted. Survives `reset`.
    static BEDS: RefCell<super::cues::ambience::BedStreamer> = RefCell::new(super::cues::ambience::BedStreamer::default());
}

fn bed_frames(id: &'static str) -> Vec<[f32; 2]> {
    BEDS.with(|c| c.borrow_mut().frames(id)).unwrap_or_default()
}

/// Renders the bed for `region` into the cache for up to `budget` (call it each frame of the loading screen with the
/// spawn region so the first ambience has no hitch). True when the bed is ready.
pub fn ambience_prewarm(region: &str, budget: std::time::Duration) -> bool {
    match ambience_bed(region) {
        Some(id) => BEDS.with(|c| c.borrow_mut().warm(id, budget)),
        None => true,
    }
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
            // render the wanted bed a slice a frame; the mixer keeps the previous bed until the new one is ready
            let want = region.and_then(ambience_bed);
            let shown = BEDS.with(|c| c.borrow_mut().update(want, super::cues::ambience::bed_budget()));
            let out = w.ambience.step_ducked(dt, shown, duck);
            for (n, (voice, slot)) in AMBIENCE_VOICES.iter().zip(out.iter()).enumerate() {
                if w.ambience_playing[n].is_some() && w.ambience_playing[n] != slot.bed {
                    e.set_loop(*voice, Bus::Ambience, None, Vec::new);
                }
                w.ambience_playing[n] = slot.bed;
                match slot.bed {
                    Some(id) => e.set_loop(*voice, Bus::Ambience, Some((slot.gain, slot.gain)), || bed_frames(id)),
                    None => e.set_loop(*voice, Bus::Ambience, None, Vec::new),
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

/// A hunter's step by the floor kind under her (a RefTextures floor kind such as "parquet0"; unknown kinds are
/// concrete): the same as `set_hunter_surface` with the surface that kind maps to. "" clears it.
pub fn set_hunter_floor(id: &str, floor_kind: &str) {
    let surface = if floor_kind.is_empty() { "" } else { super::cues::surface_for_floor(floor_kind) };
    set_hunter_surface(id, surface);
}

/// A one-shot hunter cue from data/audio/cues.json ("hunter_sniff", "hunter_knock") heard from where she stands: pan and
/// loudness from her offset to the runner, like a footstep. Silent when she is out of earshot.
pub fn hunter_event_at(name: &str, hunter_pos: (f32, f32), l: &Listener) {
    let offset = ((hunter_pos.0 - l.pos.0) as f64, (hunter_pos.1 - l.pos.1) as f64);
    // a sniff is a quiet idle sound: tuned by data/audio/hunter_ambient.json; any other hunter event keeps the step figure
    let hearing = if name == "hunter_sniff" {
        HunterAmbient::get().sniff_hearing(HunterStepHearing::get())
    } else {
        HunterStepHearing::get()
    };
    if let Some((pan, volume)) = rules::hunter_step_pan_volume_with(&hearing, offset, (l.facing.0 as f64, l.facing.1 as f64)) {
        super::sound::event(name, pan as f32, volume as f32);
    }
}

/// The voice of the wound music box's tune (one pre-rendered run, see `music_box`).
const BOX_VOICE: &str = "music_box_run";

/// The music box run is rendered in slices of `render.box_slice_frames` frames, until `render.box_budget_ms` of a game
/// frame is spent (data/audio/ambience.json): the render is a few million frames, which on one wasm thread would stall a
/// frame for tenths of a second if done at once. The finished run is the same samples either way (`RunRender`).
fn box_budget() -> std::time::Duration {
    std::time::Duration::from_secs_f64(super::cues::ambience::settings().box_budget_ms / 1000.0)
}

/// The wound music box's sound (music_box.rs): its tune, steady at normal speed while the wind is above the slowdown line
/// (70 % by default), then slower and lower as it drains, stopped when it is empty. The loops cannot change pitch while
/// they play, so the whole wound time is ONE pre-rendered run (`run_frames`, a pure function of the wound time) started
/// when the box is wound: the same samples, with no hand-over between a steady loop and a slowing one, on the browser
/// build (where loops decode asynchronously) as on desktop. The run starts again if the box is wound again, or the game
/// stood still while it played (paused). Not positional: the box is in the pack, so there is no source to hear it from.
/// Called every frame with the seconds left and the wound time (game/mod.rs `run_music_box`). On the MUSIC bus (MASTER x GAMEPLAY x
/// MUSIC), re-gained live when a slider moves.
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
                    e.set_loop(BOX_VOICE, Bus::Music, None, Vec::new);
                }
                return;
            }
            let now = e.now();
            let restart = mb::run_needs_restart(w.box_run.as_ref(), left, wound, now);
            w.box_run = Some(mb::next_clock(w.box_run.as_ref(), restart, left, wound, now));
            if restart {
                // the run is a few million frames: it is rendered a slice a frame (below), not in this one. What played
                // before no longer matches the box clock, so it stops now and the new run starts when it is ready.
                e.set_loop(BOX_VOICE, Bus::Music, None, Vec::new);
                w.box_render = Some(RunRender::new(wound, wound - left));
            }
            if let Some(render) = w.box_render.as_mut() {
                let t0 = crate::shared::clock::Instant::now();
                let mut done = false;
                let (budget, slice) = (box_budget(), super::cues::ambience::settings().box_slice_frames);
                while !done && t0.elapsed() < budget {
                    done = render.step(slice);
                }
                if done {
                    if let Some(render) = w.box_render.take() {
                        // the box clock ran on while the run rendered: join it where it is now, as a run started now would
                        let tune = crate::game::music_box::rules().tune_gain;
                        let frames = render.finish_at(wound - left);
                        e.play_frames(BOX_VOICE, Bus::Music, frames, (tune, tune), left);
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
                // Glyra's hum: positional loop on the HUNTERS bus while she is active and her box plays. The engine applies
                // the bus gain (and re-applies it on every slider change), so the base gains here are the positional ones,
                // toned down by data/audio/hunter_ambient.json (a shorter reach and a lower gain than Godot's HUM).
                if h.id == "glyra" {
                    let tune = HunterAmbient::get();
                    let hum = Emitter3D { max_distance: tune.hum_max_distance.unwrap_or(HUM.max_distance), ..HUM };
                    let gains = (h.active && h.glyra_humming).then(|| {
                        let g = hum.gains_from([h.pos.0, 0.0, h.pos.1], l.eye, l.right, l.up, l.forward);
                        (g.0 * tune.hum_gain, g.1 * tune.hum_gain)
                    });
                    e.set_loop("glyra_hum", Bus::Hunters, gains, super::cues::music_box::glyra_hum);
                }
            }
            if !hunters.iter().any(|h| h.id == "glyra") {
                e.set_loop("glyra_hum", Bus::Hunters, None, Vec::new);
            }
            if pursued && !w.was_pursued {
                e.play(&rules::chase_start());
                super::tts::note_cue("chase_start");
            }
            w.was_pursued = pursued;
            let danger = rules::heartbeat_danger(
                nearest,
                pursued,
                l.in_safe_zone,
            );
            if let Some(mut p) = e.front.heartbeat(danger, now) {
                // near but not after you: the calm heartbeat is toned down (data/audio/hunter_ambient.json)
                if !pursued {
                    p.gain *= HunterAmbient::get().calm_heartbeat_gain;
                }
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
            // a chasing hunter keeps the full figure (cues.json hunter_step); a wandering one is quieter and carries less far
            let hearing = if h.chasing {
                HunterStepHearing::get()
            } else {
                HunterAmbient::get().step_hearing(HunterStepHearing::get())
            };
            if let Some((pan, volume)) =
                rules::hunter_step_pan_volume_with(&hearing, offset, (l.facing.0 as f64, l.facing.1 as f64))
            {
                let surface = w.surfaces.get(h.id).map(String::as_str).unwrap_or("");
                let p = e.front.hunter_footstep(pan, volume, surface);
                e.play(&p);
            }
        });
    });
}

#[cfg(test)]
mod bed_stream_tests {
    use super::super::cues::ambience::{self, BedRender, BedStreamer};
    use std::time::Duration;

    #[test]
    fn synth_and_render_tunables_come_from_data() {
        let s = ambience::settings();
        // Standard values match the original synth
        assert_eq!(s.rate, 22050.0);
        assert_eq!(s.max_tone_hz, 1600.0);
        assert_eq!(s.dc_cutoff_hz, 30.0);
        assert_eq!(s.level_seconds, 0.03);
        assert!(s.cache_beds >= 2 && s.upsample_chunk >= 256 && s.box_slice_frames >= 256);
        assert!(s.bed_budget_ms >= 0.0 && s.box_budget_ms >= 0.0);
        assert_eq!(ambience::bed_budget(), Duration::from_secs_f64(s.bed_budget_ms / 1000.0));
    }

    #[test]
    fn stepping_a_bed_gives_the_same_samples_as_rendering_it_at_once() {
        let mut r = BedRender::new("lab", 1.0);
        while !r.step() {}
        assert_eq!(r.finish(), ambience::ambience("lab", 1.0));
    }

    #[test]
    fn no_frame_renders_more_than_one_stage_with_no_budget() {
        let chunk = ambience::settings().upsample_chunk;
        let seconds = 2.0f32;
        let mut r = BedRender::new("lab", seconds);
        let mut calls = 0;
        let mut last_frames = 0;
        while !r.is_done() {
            let before = r.stages_done;
            r.step_for(Duration::ZERO);
            calls += 1;
            assert_eq!(r.stages_done, before + 1, "a zero budget runs exactly one stage");
            assert!(r.frames_ready() - last_frames <= chunk, "the up-sample stays within its chunk");
            last_frames = r.frames_ready();
            assert!(calls < 10_000);
        }
        assert!(calls > 3, "a bed takes several frames");
        // the whole loop at the engine's output rate
        let rate = super::super::cues::SAMPLE_RATE as f64;
        assert_eq!(last_frames, (seconds as f64 * rate).round() as usize);
    }

    /// A step never works more than one chunk (`render.synth_chunk` synth samples of a layer or finishing pass, an
    /// up-sample step `render.upsample_chunk` output frames), so a frame's budget is passed by one chunk at most: every
    /// bed, every layer kind, a loop long enough that a whole layer is many chunks.
    #[test]
    fn no_step_works_more_than_one_chunk() {
        let s = ambience::settings();
        assert!(s.synth_chunk as f64 >= ambience::LONGEST_EVENT_SECONDS * s.rate, "a chunk holds the longest event");
        let mut ids: Vec<&String> = s.beds.keys().collect();
        ids.sort();
        let mut layer_steps = 0;
        for id in ids {
            let mut r = BedRender::new(id, s.loop_seconds);
            let mut steps = 0;
            loop {
                let up = r.upsampling();
                let done = r.step();
                steps += 1;
                let cap = if up { s.upsample_chunk } else { s.synth_chunk };
                assert!(r.last_work <= cap, "{id}: step {steps} worked {} samples (cap {cap})", r.last_work);
                if !up {
                    layer_steps += 1;
                }
                if done {
                    break;
                }
                assert!(steps < 100_000);
            }
            let layers = s.beds[id].layers.len();
            assert!(steps > layers + 6, "{id}: the render is cut into chunks ({steps} steps for {layers} layers)");
        }
        assert!(layer_steps > 0);
    }

    /// The chunked render gives the same samples as a render at once for every bed (`finish` steps too, so this holds a
    /// stepped render against the chunk-free walk of the same loop lengths).
    #[test]
    fn every_bed_steps_to_the_same_samples() {
        let s = ambience::settings();
        let mut ids: Vec<&String> = s.beds.keys().collect();
        ids.sort();
        for id in ids {
            let mut r = BedRender::new(id, 3.0);
            while !r.step_for(Duration::ZERO) {}
            assert_eq!(r.finish(), ambience::ambience(id, 3.0), "{id}");
        }
    }

    fn settle(st: &mut BedStreamer, want: &'static str) -> usize {
        let mut frames = 0;
        while st.update(Some(want), Duration::ZERO) != Some(want) {
            frames += 1;
            assert!(frames < 10_000);
        }
        frames
    }

    #[test]
    fn a_region_switch_keeps_the_old_bed_until_the_new_one_is_ready() {
        let a = ambience::bed_for("lab");
        let b = ambience::bed_for("sewer");
        assert_ne!(a, b);
        let mut st = BedStreamer::default();
        // the first bed has nothing before it: silence until ready
        assert_eq!(st.update(Some(a), Duration::ZERO), None);
        settle(&mut st, a);
        let mut waited = 0;
        loop {
            let shown = st.update(Some(b), Duration::ZERO);
            if shown == Some(b) {
                break;
            }
            assert_eq!(shown, Some(a), "the old bed plays while the new one renders");
            waited += 1;
            assert!(waited < 10_000);
        }
        assert!(waited >= 1, "the switch took more than one frame");
        assert!(st.is_cached(a) && st.is_cached(b));
        // leaving every region silences it and drops the half-rendered bed
        assert_eq!(st.update(None, Duration::ZERO), None);
    }

    #[test]
    fn the_cache_keeps_the_most_recent_beds() {
        let cap = ambience::settings().cache_beds;
        let ids: [&'static str; 8] = ["b0", "b1", "b2", "b3", "b4", "b5", "b6", "b7"];
        let mut st = BedStreamer::default();
        for id in ids.iter().take(cap) {
            st.insert(id, vec![[0.0, 0.0]]);
        }
        assert!(st.frames(ids[0]).is_some(), "using the oldest makes it the newest");
        st.insert(ids[cap], vec![[0.0, 0.0]]);
        assert!(st.is_cached(ids[0]), "the recently used bed survives");
        assert!(!st.is_cached(ids[1]), "the least recently used bed was evicted");
        assert_eq!(st.cached_ids().len(), cap);
        assert_eq!(*st.cached_ids().last().unwrap(), ids[cap]);
    }

    #[test]
    fn warming_fills_the_cache_without_showing_the_bed() {
        let id = ambience::bed_for("lab");
        let mut st = BedStreamer::default();
        let mut n = 0;
        while !st.warm(id, Duration::ZERO) {
            n += 1;
            assert!(n < 10_000);
        }
        assert!(st.is_cached(id));
        assert_eq!(st.shown(), None);
        assert_eq!(st.update(Some(id), Duration::ZERO), Some(id), "a warmed bed plays at once");
    }

    #[test]
    fn a_hunter_floor_kind_maps_to_a_surface_step() {
        super::set_hunter_floor("tester_h", "parquet0");
        super::WORLD.with(|w| assert_eq!(w.borrow().surfaces.get("tester_h").map(String::as_str), Some("wood")));
        super::set_hunter_floor("tester_h", "");
        super::WORLD.with(|w| assert!(w.borrow().surfaces.get("tester_h").is_none()));
    }
}

#[cfg(test)]
mod chase_subtitle_tests {
    /// Regression: the chase_start cue played through the engine but never noted its subtitle, so "*FOOTSTEPS CLOSE IN*"
    /// never showed. The cue has a subtitle, and the pursuit edge notes it right after playing it.
    #[test]
    fn chase_start_notes_its_subtitle() {
        assert!(super::super::tts::subtitle_for("chase_start", true).is_some(), "chase_start has no subtitle in cues.json");
        let src = include_str!("world.rs");
        let play = src.find("e.play(&rules::chase_start());").expect("chase_start play");
        let rest = &src[play..];
        let next_line = rest.lines().nth(1).unwrap_or_default().trim();
        assert_eq!(next_line, "super::tts::note_cue(\"chase_start\");");
    }
}
