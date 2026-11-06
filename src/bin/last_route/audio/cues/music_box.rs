//! The wound music box and Glyra's hum: looping, positional, pitch-ridden playback of two rendered cues.
//!
//! The box's tune is Night Watch's Masked Hours tune (games/night-watch/src/modes/mask/mask-data.js `TUNE`, played by
//! mask.js): thirteen sine notes, 380 ms apart at full wind, each ringing 0.5 s, the phrase starting again straight after
//! the last note. The clock is `Game::music_box_left` (game/music_box.rs), as in game/world/interactables/music_boxes.gd.
//!
//! IMPORTANT for the finite route: MusicBoxTune is only attached when the interactable has a `music_boxes` clock, which
//! only the streamed (infinite) route view sets (streamed_facility_view_3d.gd 4734). The finite floor's FacilityView3D
//! never sets it, so in main_3d winding the box plays the one-shot "music_box" phrase (route_interactable_3d.gd 818)
//! and the only continuous music is Glyra's own hum while her box PLAYS. This port plays the tune on the finite route
//! anyway (requested); NOTES.md says the same.
//!
//! Godot changes a looping stream's pitch with `pitch_scale` (resampling: slower AND lower). macroquad 0.4 cannot
//! change the pitch of a playing sound, so `run_frames` resamples in software: the whole wound time is rendered as one
//! buffer (the loop read back at the slowing rate) and played once from the moment the box is wound.

use super::render::SAMPLE_RATE;

/// The tune, note by note, in Hz: Night Watch's Masked Hours phrase (`TUNE` in
/// games/night-watch/src/modes/mask/mask-data.js), E5 D5 C5 D5 E5 E5 E5 D5 D5 D5 E5 G5 G5. The notes are in that
/// game's rounded Hz; they play in order and the phrase starts again straight after the last note.
pub const TUNE_NOTES: [f64; 13] = [
    659.0, 587.0, 523.0, 587.0, 659.0, 659.0, 659.0, 587.0, 587.0, 587.0, 659.0, 784.0, 784.0,
];

/// The gap between two notes at full wind, in frames: 380 ms (mask.js `noteAt = 380 + ...` at 100 %). A whole number of
/// frames, so every note of every round lands the same distance from the last (0.38 s x 44100 = 16758).
pub const NOTE_STEP_FRAMES: usize = 16758;

/// How long one note rings, in seconds (mask.js `blip(.., 0.5, "sine", 0.04, ..)`).
pub const NOTE_RING_SECONDS: f64 = 0.5;

/// The loop's length in frames: the notes at their spacing, no breath. The last note rings over the seam into the
/// first slot (see `tune`).
pub const LOOP_FRAMES: usize = TUNE_NOTES.len() * NOTE_STEP_FRAMES;

/// The last stretch of a wound box, in seconds (data/music_box.json `winddown_seconds`). It no longer drives the
/// tune's slowing (that starts at `slowdown_from_fraction`); it is the threshold of Glyra's `Mood` and the HUD colour.
pub fn winddown_seconds() -> f32 {
    crate::game::music_box::rules().winddown_seconds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Playing,
    WindingDown,
    Stopped,
}

/// MusicBoxes.mood_for (music_boxes.gd 31).
pub fn mood_for(seconds_left: f32) -> Mood {
    if seconds_left <= 0.0 {
        Mood::Stopped
    } else if seconds_left <= winddown_seconds() {
        Mood::WindingDown
    } else {
        Mood::Playing
    }
}

/// How fast the tune plays for the wind left in the box, as a factor of its normal speed: 1 from a full wind down to
/// `from` (a fraction of the wound time, 0.70 = 70 %), then it eases down to `floor` as the box drains (smoothstep, so
/// there is no kink at `from`), and 0 (stopped) once the box is empty. Tempo and pitch both follow it, like a tape
/// slowing, because the tune is read back at this rate. `fraction` is `music_box_left / wound_seconds`.
pub fn slowdown_factor(fraction: f32, from: f32, floor: f32) -> f32 {
    if fraction.is_nan() || fraction <= 0.0 {
        return 0.0;
    }
    if from <= 0.0 || fraction >= from {
        return 1.0;
    }
    let x = (fraction / from).clamp(0.0, 1.0);
    let ease = x * x * (3.0 - 2.0 * x);
    let floor = floor.clamp(0.0, 1.0);
    floor + (1.0 - floor) * ease
}

/// [`slowdown_factor`] with the shipped numbers (data/music_box.json): the tune's speed with `seconds_left` of a box
/// wound for `wound_seconds`. The one formula the audio follows, driven by the box clock `Game::music_box_left`.
pub fn playback_rate(seconds_left: f32, wound_seconds: f32) -> f32 {
    if wound_seconds <= 0.0 {
        return 0.0;
    }
    let r = crate::game::music_box::rules();
    slowdown_factor(seconds_left / wound_seconds, r.slowdown_from_fraction, r.slowdown_floor)
}

/// The tune loop (`render_cue("music_box_tune")`) cut to exactly [`LOOP_FRAMES`] frames. The cue itself runs a little
/// past that (the last note rings after its slot); the part past the end is added onto the start, so the last note
/// keeps ringing into the first slot of the next round and the seam has neither a click nor a gap. Every round is the
/// same samples.
pub fn tune() -> Vec<[f32; 2]> {
    let mut frames = super::render("music_box_tune", 0.0, 1.0).unwrap_or_default();
    if frames.len() > LOOP_FRAMES {
        let over: Vec<[f32; 2]> = frames.split_off(LOOP_FRAMES);
        for (k, f) in over.iter().enumerate() {
            let at = k % LOOP_FRAMES;
            frames[at][0] += f[0];
            frames[at][1] += f[1];
        }
    }
    frames.resize(LOOP_FRAMES, [0.0, 0.0]);
    frames
}

/// The whole of one wound box's tune as one buffer: the loop read at [`playback_rate`] frame by frame, from
/// `from_elapsed` seconds after winding (0 = a box just wound; more joins a box part-way through). A full wind plays the
/// loop exactly (rate 1 reads whole frames), then the tune slows and sinks smoothly as the wind drains, and the buffer
/// ends when the box is empty. It depends only on the wound time and the data, so it is the same on every platform, and
/// a sound that cannot change pitch while it plays (macroquad 0.4) needs no more than this one pre-rendered stretch.
pub fn run_frames(wound_seconds: f32, from_elapsed: f32) -> Vec<[f32; 2]> {
    let mut run = RunRender::new(wound_seconds, from_elapsed);
    while !run.step(usize::MAX) {}
    run.finish()
}

/// [`run_frames`] rendered a slice at a time, so the game can spread the (long: a 45 s box is two million frames) render
/// over several game frames instead of stalling one. The output is exactly `run_frames`, whatever the slice sizes: the same
/// loop, paused between iterations. Build it with [`RunRender::new`], call [`RunRender::step`] until it says it is done, then
/// take the frames with [`RunRender::finish`] (or [`RunRender::finish_at`] when the box clock has moved on meanwhile).
pub struct RunRender {
    wound_seconds: f32,
    total: usize,
    skip: usize,
    /// The next frame index to render.
    k: usize,
    /// The tune loop, rendered by the first `step` (it is itself a few thousand sine notes, worth a game frame of its own).
    player: Option<LoopPlayer>,
    out: Vec<[f32; 2]>,
    done: bool,
}

impl RunRender {
    pub fn new(wound_seconds: f32, from_elapsed: f32) -> RunRender {
        let (total, skip) = run_extent(wound_seconds, from_elapsed);
        RunRender { wound_seconds, total, skip, k: 0, player: None, out: Vec::with_capacity(total - skip), done: total == 0 }
    }

    /// Renders up to `max_frames` more frames (the first call renders only the tune loop and no frames). True once the
    /// whole run is rendered.
    pub fn step(&mut self, max_frames: usize) -> bool {
        if self.done {
            return true;
        }
        let Some(player) = self.player.as_mut() else {
            self.player = Some(LoopPlayer::new(tune()));
            return false;
        };
        let sr = f64::from(SAMPLE_RATE);
        let end = self.k.saturating_add(max_frames).min(self.total);
        while self.k < end {
            let left = ((self.total - self.k) as f64 / sr) as f32; // seconds left, always above 0 until the last frame is out
            let rate = playback_rate(left, self.wound_seconds);
            if rate <= 0.0 {
                self.done = true;
                return true;
            }
            let frame = player.next(rate);
            if self.k >= self.skip {
                self.out.push(frame);
            }
            self.k += 1;
        }
        self.done = self.k >= self.total;
        self.done
    }

    /// The frames rendered so far; the whole run once `step` has said it is done.
    pub fn finish(self) -> Vec<[f32; 2]> {
        self.out
    }

    /// The finished run as it would have been rendered to start `from_elapsed` seconds after winding: the frames that have
    /// already been played by then are dropped. Exactly `run_frames(wound, from_elapsed)` when `from_elapsed` is no earlier
    /// than the one the render began with (an earlier one cannot bring frames back, and leaves the run as rendered).
    pub fn finish_at(mut self, from_elapsed: f32) -> Vec<[f32; 2]> {
        let (_, skip) = run_extent(self.wound_seconds, from_elapsed);
        let drop = skip.saturating_sub(self.skip).min(self.out.len());
        self.out.drain(..drop);
        self.out
    }

    /// Frames of the whole run still to render (for tests).
    pub fn remaining(&self) -> usize {
        if self.done {
            0
        } else {
            self.total - self.k
        }
    }
}

/// (total frames of a run, frames to skip) for a wound time and a start `from_elapsed` seconds in.
fn run_extent(wound_seconds: f32, from_elapsed: f32) -> (usize, usize) {
    let sr = f64::from(SAMPLE_RATE);
    let total = (f64::from(wound_seconds.max(0.0)) * sr) as usize;
    let skip = ((f64::from(from_elapsed.max(0.0)) * sr) as usize).min(total);
    (total, skip)
}

/// What the world keeps about the tune it started, to know when the pre-rendered run no longer matches the box clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunClock {
    pub wound: f32,
    pub last_left: f32,
    /// The wall clock at the last frame the box clock was read.
    pub last_now: f64,
}

/// A frame gap longer than this (seconds) means the game stood still (paused, loading, the tab hidden) while the
/// run kept playing on the wall clock, so the run is started again from the box clock.
pub const RESYNC_GAP_SECONDS: f64 = 0.4;

/// True when the run must be (re)started for `left` now: none is playing, the box was wound again (the clock went up),
/// the wound time changed, or the game stood still for longer than [`RESYNC_GAP_SECONDS`].
pub fn run_needs_restart(prev: Option<&RunClock>, left: f32, wound: f32, now: f64) -> bool {
    match prev {
        None => true,
        Some(p) => p.wound != wound || left > p.last_left + 1e-3 || now - p.last_now > RESYNC_GAP_SECONDS,
    }
}

/// Glyra's hum loop (`render_cue("glyra_hum")`, looped), played while her box is PLAYING (GlyraGimmick.humming()).
pub fn glyra_hum() -> Vec<[f32; 2]> {
    super::render("glyra_hum", 0.0, 1.0).unwrap_or_default()
}

/// Positional settings of the two loops (AudioStreamPlayer3D, inverse-distance attenuation, Godot's default max_db 3).
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub unit_size: f32,
    pub max_distance: f32,
    pub volume_db: f32,
    /// Height of the emitter above its node (m).
    pub height: f32,
}

/// MusicBoxTune: heard to 12 m, +6 dB, 0.4 m above the box.
pub const TUNE_EMITTER: Emitter = Emitter {
    unit_size: 5.0,
    max_distance: 12.0,
    volume_db: 6.0,
    height: 0.4,
};
/// GlyraHum: heard to 14 m, +3 dB, 1.5 m up (her head).
pub const HUM_EMITTER: Emitter = Emitter {
    unit_size: 4.0,
    max_distance: 14.0,
    volume_db: 3.0,
    height: 1.5,
};

impl Emitter {
    /// Godot's ATTENUATION_INVERSE_DISTANCE: `linear_to_db(1 / (d / unit_size + eps)) + volume_db`, capped at max_db
    /// (3 dB), silent past max_distance. Returned as a linear gain. (Godot also low-passes distant 3D sources, default
    /// cutoff 5 kHz at -24 dB attenuation; not modelled.)
    pub fn gain(&self, distance: f32) -> f32 {
        if distance > self.max_distance {
            return 0.0;
        }
        let db =
            (20.0 * (1.0 / (distance / self.unit_size + 1e-5)).log10() + self.volume_db).min(3.0);
        10f32.powf(db / 20.0)
    }
}

/// A looping buffer read at a variable rate (linear interpolation between frames, like Godot's WAV playback).
#[derive(Clone, Debug)]
pub struct LoopPlayer {
    pub frames: Vec<[f32; 2]>,
    /// Read position in source frames.
    pub pos: f64,
}

impl LoopPlayer {
    pub fn new(frames: Vec<[f32; 2]>) -> Self {
        LoopPlayer { frames, pos: 0.0 }
    }

    /// The next output frame, advancing `pitch` source frames. The position wraps by subtraction (no float modulo, which
    /// is slow on the browser build); a whole-frame position reads the source frame exactly.
    pub fn next(&mut self, pitch: f32) -> [f32; 2] {
        let n = self.frames.len();
        if n == 0 {
            return [0.0, 0.0];
        }
        let i = (self.pos as usize).min(n - 1);
        let j = if i + 1 == n { 0 } else { i + 1 };
        let frac = (self.pos - i as f64) as f32;
        let (a, b) = (self.frames[i], self.frames[j]);
        let out = [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac];
        self.pos += f64::from(pitch);
        let len = n as f64;
        while self.pos >= len {
            self.pos -= len;
        }
        out
    }

    /// Fills `out` (at SAMPLE_RATE) advancing `pitch` source frames per output frame.
    pub fn fill(&mut self, out: &mut [[f32; 2]], pitch: f32) {
        for o in out.iter_mut() {
            *o = self.next(pitch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::cues::{components, SAMPLE_RATE};

    const FROM: f32 = 0.70;
    const FLOOR: f32 = 0.40;

    #[test]
    fn full_wind_and_seventy_percent_play_at_normal_speed() {
        assert_eq!(slowdown_factor(1.0, FROM, FLOOR), 1.0);
        assert_eq!(slowdown_factor(0.85, FROM, FLOOR), 1.0);
        assert_eq!(slowdown_factor(0.70, FROM, FLOOR), 1.0);
    }

    #[test]
    fn thirty_five_percent_is_clearly_slower_and_empty_is_stopped() {
        let half = slowdown_factor(0.35, FROM, FLOOR);
        assert!(half < 0.8 && half > FLOOR, "35 % should be clearly slower, not stopped: {half}");
        assert_eq!(slowdown_factor(0.0, FROM, FLOOR), 0.0);
        assert_eq!(slowdown_factor(-0.1, FROM, FLOOR), 0.0);
        assert_eq!(slowdown_factor(f32::NAN, FROM, FLOOR), 0.0);
    }

    #[test]
    fn the_slowdown_eases_in_smoothly_and_never_speeds_up() {
        // no kink at 70 %: just under it the factor is still about 1
        assert!(slowdown_factor(0.699, FROM, FLOOR) > 0.999);
        let mut last = 1.0;
        for step in (1..=1000).rev() {
            let f = slowdown_factor(step as f32 / 1000.0, FROM, FLOOR);
            assert!(f <= last + 1e-6, "the tune sped up at {step}/1000");
            assert!(f >= FLOOR - 1e-6);
            assert!(last - f < 0.01, "a jump of {} at {step}/1000", last - f);
            last = f;
        }
        assert!((slowdown_factor(0.001, FROM, FLOOR) - FLOOR).abs() < 0.01, "it runs down towards the floor");
    }

    #[test]
    fn the_shipped_numbers_drive_the_rate_by_the_box_clock() {
        // 45 s standard box: full wind and 31.5 s left are normal; 15.75 s left (35 %) is slower; empty is stopped
        assert_eq!(playback_rate(45.0, 45.0), 1.0);
        assert_eq!(playback_rate(31.5, 45.0), 1.0);
        assert!(playback_rate(15.75, 45.0) < 0.8);
        assert_eq!(playback_rate(0.0, 45.0), 0.0);
        // the same fractions on a nightmare box (25 s) give the same speeds
        assert_eq!(playback_rate(17.5, 25.0), 1.0);
        assert!((playback_rate(8.75, 25.0) - playback_rate(15.75, 45.0)).abs() < 1e-5);
    }

    #[test]
    fn the_tune_is_night_watchs_notes_at_a_constant_spacing() {
        let parts = components("music_box_tune", 0.0, 1.0).expect("the cue exists");
        let notes: Vec<_> = parts.iter().filter(|c| c.volume > 0.0).collect();
        assert_eq!(notes.len(), TUNE_NOTES.len());
        for (k, c) in notes.iter().enumerate() {
            assert_eq!(c.frequency, TUNE_NOTES[k], "note {k}");
            assert_eq!((c.delay * f64::from(SAMPLE_RATE)) as usize, k * NOTE_STEP_FRAMES, "note {k} is on the grid");
            assert!((c.duration - NOTE_RING_SECONDS).abs() < 1e-9);
        }
        assert_eq!(NOTE_STEP_FRAMES as f64 / f64::from(SAMPLE_RATE), 0.38);
    }

    #[test]
    fn the_loop_seam_has_no_click_and_no_gap() {
        let t = tune();
        assert_eq!(t.len(), LOOP_FRAMES);
        let step = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs().max((a[1] - b[1]).abs());
        let across = step(t[LOOP_FRAMES - 1], t[0]);
        let inside = (1..LOOP_FRAMES).map(|i| step(t[i - 1], t[i])).fold(0.0f32, f32::max);
        assert!(across <= inside * 1.01, "the seam jumps {across}, the loudest step inside is {inside}");
        // the last note still rings at the seam, so the loop never goes quiet there
        let tail: f32 = t[LOOP_FRAMES - 400..].iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(tail > 0.002, "the last note was cut short at the seam");
        // and the first note starts from nothing (its attack), so what rings in is the last note's tail alone
        assert!(t[0][0].abs() < 0.01);
    }

    #[test]
    fn every_round_of_the_run_is_the_same_samples_until_the_slowdown() {
        let wound = 45.0;
        let run = run_frames(wound, 0.0);
        assert_eq!(run.len(), (wound * SAMPLE_RATE as f32) as usize);
        // a 45 s box is at normal speed for its first 13.5 s (down to 70 %), so the first two rounds (4.94 s each)
        // are the loop, exactly, every time
        assert!(run[..LOOP_FRAMES] == run[LOOP_FRAMES..2 * LOOP_FRAMES], "round two differs from round one");
        assert!(run[..LOOP_FRAMES] == tune()[..], "the first round is not the loop");
    }

    #[test]
    fn the_run_slows_and_sinks_as_the_box_drains_and_joins_part_way() {
        let wound = 20.0;
        let sr = SAMPLE_RATE as usize;
        let run = run_frames(wound, 0.0);
        let crossings = |from: usize, to: usize| -> usize {
            run[from..to].windows(2).filter(|w| (w[0][0] < 0.0) != (w[1][0] < 0.0)).count()
        };
        let normal = crossings(sr, 2 * sr);
        let late = crossings(19 * sr - sr / 2, 20 * sr - sr / 2 - 1000);
        assert!(normal > 300, "the first second is the tune at its pitch: {normal}");
        assert!((late as f32) < normal as f32 * 0.7, "pitch should have sunk: {late} against {normal}");
        // part-way: the same samples as the tail of the full run
        let joined = run_frames(wound, 6.0);
        assert_eq!(joined.len(), run.len() - 6 * sr);
        assert!(joined[..] == run[6 * sr..], "joining part-way gives different samples");
        // an empty or spent box is no tune
        assert!(run_frames(wound, wound).is_empty());
        assert!(run_frames(0.0, 0.0).is_empty());
    }

    #[test]
    fn a_run_rendered_in_slices_is_the_run_rendered_whole() {
        let (wound, from) = (6.0f32, 1.25f32);
        let whole = run_frames(wound, from);
        assert!(!whole.is_empty());
        for slice in [1usize, 777, 4096, 100_000, usize::MAX] {
            if slice == 1 {
                continue; // one call per frame is too slow for a debug run; 777 covers the odd-size case
            }
            let mut run = RunRender::new(wound, from);
            let mut calls = 0usize;
            while !run.step(slice) {
                calls += 1;
                assert!(calls < 2_000_000, "never finishes at slice {slice}");
            }
            // the first call renders the tune loop only
            if slice == 777 {
                assert!(calls > whole.len() / 777, "{calls} calls for {} frames", whole.len());
            }
            assert!(run.remaining() == 0 && run.step(10), "done stays done");
            let frames = run.finish();
            assert!(frames == whole, "slice {slice}: {} frames against {}", frames.len(), whole.len());
        }
    }

    #[test]
    fn a_slice_never_renders_more_than_it_was_given() {
        let mut run = RunRender::new(2.0, 0.0);
        assert!(!run.step(100_000), "the first call is the tune loop");
        let start = run.remaining();
        assert_eq!(start, 2 * SAMPLE_RATE as usize);
        assert!(!run.step(10_000));
        assert_eq!(start - run.remaining(), 10_000);
        assert!(!run.step(0), "a slice of nothing moves nothing");
        assert_eq!(start - run.remaining(), 10_000);
    }

    #[test]
    fn a_run_finished_late_drops_the_frames_the_box_clock_has_passed() {
        let wound = 8.0f32;
        let started_at = 0.5f32;
        let mut run = RunRender::new(wound, started_at);
        while !run.step(50_000) {}
        // the render took 0.75 s of box time: it joins as if it had been started then, sample for sample
        let late = run.finish_at(1.25);
        assert!(late == run_frames(wound, 1.25), "a late finish differs from a late start");
        // an earlier time than the start cannot bring frames back
        let mut run = RunRender::new(wound, 1.0);
        while !run.step(50_000) {}
        let n = run.out.len();
        assert_eq!(run.finish_at(0.2).len(), n);
        // past the end it is empty, not a panic
        let mut run = RunRender::new(1.0, 0.0);
        while !run.step(50_000) {}
        assert!(run.finish_at(99.0).is_empty());
        // an empty run is done at once and renders nothing
        let mut none = RunRender::new(0.0, 0.0);
        assert!(none.step(10) && none.finish().is_empty());
    }

    #[test]
    fn the_run_restarts_only_when_it_stops_matching_the_clock() {
        let clock = RunClock { wound: 45.0, last_left: 40.0, last_now: 10.0 };
        assert!(run_needs_restart(None, 45.0, 45.0, 0.0));
        assert!(!run_needs_restart(Some(&clock), 39.98, 45.0, 10.02), "an ordinary frame");
        assert!(run_needs_restart(Some(&clock), 45.0, 45.0, 10.02), "wound again");
        assert!(run_needs_restart(Some(&clock), 39.98, 35.0, 10.02), "another difficulty");
        assert!(run_needs_restart(Some(&clock), 40.0, 45.0, 14.0), "the game stood still");
    }
}
