//! Mouse look: the numbers (`data/player/look.json`) and the pure maths behind the camera turn. The Godot original
//! turns `relative * sensitivity` per mouse event with no smoothing, no freeze and no clamp except the pitch, so
//! everything here exists to keep a real mouse from being dropped or amplified on this port's windowing layer:
//! a short settle after the cursor is captured, rejection of the one-frame warp spike, a per-platform multiplier, and
//! soft edges (not walls) where a view limit exists.

use macroquad::prelude::Vec2;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Cfg {
    /// Pitch limit in radians (Godot `clamp(_pitch, -1.4, 1.4)`).
    pub pitch_limit_rad: f32,
    /// Frames of look dropped after the cursor is captured again.
    pub relock_drop_frames: u32,
    /// And at least this many seconds, so a 144 Hz display still waits out the warp.
    pub relock_drop_min_s: f32,
    /// A frame longer than this (a hitch, a background pause) counts as a re-lock.
    pub gap_relock_s: f32,
    /// Frames after a re-lock in which a single huge move is a warp spike and is dropped.
    pub spike_window_frames: u32,
    pub spike_reject_px: f32,
    /// After a re-lock the FIRST move that arrives is dropped when it is larger than this: the pointer may have been
    /// moved while the cursor was free (Escape, a menu, another window), and the first absolute position event after the
    /// capture reports that whole trip as one turn, however late the mouse is touched.
    pub first_motion_max_px: f32,
    /// Hard cap on one frame's move at any time (guards junk values, never scales a normal move).
    pub per_frame_max_px: f32,
    pub mouse_multiplier_macos: f32,
    pub mouse_multiplier_windows: f32,
    pub mouse_multiplier_other: f32,
    /// The browser build (wasm32): its own feel, since the other platforms' values made the web look too slow.
    pub mouse_multiplier_web: f32,
    /// Width of the eased zone at the edge of the locker window's look cone.
    pub hide_soft_zone_deg: f32,
    /// Free horizontal look around the rails' direction while climbing, and the eased zone at its edge.
    pub ladder_free_yaw_deg: f32,
    pub ladder_soft_zone_deg: f32,
}

type Json = serde_json::Value;

fn num(v: &Json, key: &str) -> Result<f32, String> {
    let n = v
        .get(key)
        .ok_or_else(|| format!("look.json: \"{key}\" is missing"))?
        .as_f64()
        .ok_or_else(|| format!("look.json: \"{key}\" must be a number"))?;
    if !n.is_finite() {
        return Err(format!("look.json: \"{key}\" must be finite"));
    }
    Ok(n as f32)
}

fn positive(v: &Json, key: &str) -> Result<f32, String> {
    let n = num(v, key)?;
    if n <= 0.0 {
        return Err(format!("look.json: \"{key}\" must be above 0"));
    }
    Ok(n)
}

fn count(v: &Json, key: &str) -> Result<u32, String> {
    let n = v
        .get(key)
        .ok_or_else(|| format!("look.json: \"{key}\" is missing"))?
        .as_u64()
        .ok_or_else(|| format!("look.json: \"{key}\" must be a whole number of 0 or more"))?;
    u32::try_from(n).map_err(|_| format!("look.json: \"{key}\" is too large"))
}

impl Cfg {
    /// Parse and validate the table; a bad one is refused with a message naming the key.
    pub fn parse(text: &str) -> Result<Cfg, String> {
        let v: Json = serde_json::from_str(text).map_err(|e| format!("look.json: not valid JSON: {e}"))?;
        let c = Cfg {
            pitch_limit_rad: positive(&v, "pitch_limit_rad")?,
            relock_drop_frames: count(&v, "relock_drop_frames")?,
            relock_drop_min_s: num(&v, "relock_drop_min_s")?,
            gap_relock_s: positive(&v, "gap_relock_s")?,
            spike_window_frames: count(&v, "spike_window_frames")?,
            spike_reject_px: positive(&v, "spike_reject_px")?,
            first_motion_max_px: positive(&v, "first_motion_max_px")?,
            per_frame_max_px: positive(&v, "per_frame_max_px")?,
            mouse_multiplier_macos: positive(&v, "mouse_multiplier_macos")?,
            mouse_multiplier_windows: positive(&v, "mouse_multiplier_windows")?,
            mouse_multiplier_other: positive(&v, "mouse_multiplier_other")?,
            mouse_multiplier_web: positive(&v, "mouse_multiplier_web")?,
            hide_soft_zone_deg: positive(&v, "hide_soft_zone_deg")?,
            ladder_free_yaw_deg: positive(&v, "ladder_free_yaw_deg")?,
            ladder_soft_zone_deg: positive(&v, "ladder_soft_zone_deg")?,
        };
        if c.relock_drop_min_s < 0.0 {
            return Err("look.json: \"relock_drop_min_s\" must not be negative".into());
        }
        if c.pitch_limit_rad >= std::f32::consts::FRAC_PI_2 {
            return Err("look.json: \"pitch_limit_rad\" must stay below a quarter turn".into());
        }
        if c.spike_reject_px > c.per_frame_max_px {
            return Err("look.json: \"spike_reject_px\" must not exceed \"per_frame_max_px\"".into());
        }
        Ok(c)
    }

    /// The multiplier for the platform this build runs on.
    pub fn multiplier(&self) -> f32 {
        if cfg!(target_arch = "wasm32") {
            self.mouse_multiplier_web
        } else if cfg!(target_os = "macos") {
            self.mouse_multiplier_macos
        } else if cfg!(target_os = "windows") {
            self.mouse_multiplier_windows
        } else {
            self.mouse_multiplier_other
        }
    }
}

pub fn cfg() -> &'static Cfg {
    static C: OnceLock<Cfg> = OnceLock::new();
    C.get_or_init(|| Cfg::parse(include_str!("../data/player/look.json")).unwrap_or_else(|e| panic!("{e}")))
}

/// Mouse move for one frame in pixels. `delta` is macroquad's `mouse_delta_position` (last - current, in [-1, 1]
/// screen units, a full-width sweep is 2); `screen` is the logical window size; `dpi` the window's dpi scale (raw
/// deltas stay in points while macroquad divides positions by it). `fallback_px` is the pixel move since the last
/// frame, used when the platform reports no delta. Both sources return the Godot signs (right turns right, down looks
/// down) so the first turn after capture goes the same way as the rest.
pub fn delta_px(delta: Vec2, screen: Vec2, dpi: f32, fallback_px: Option<Vec2>) -> Vec2 {
    if delta.length_squared() > 0.0 {
        Vec2::new(-delta.x, -delta.y) * screen * 0.5 * dpi.max(0.01)
    } else {
        fallback_px.unwrap_or(Vec2::ZERO) * dpi.max(0.01)
    }
}

/// Radians from a pixel move: `sens` radians per Godot viewport pixel times the platform multiplier.
pub fn px_to_radians(px: Vec2, sens: f32, multiplier: f32) -> Vec2 {
    px * sens * multiplier
}

/// What to do with the cursor grab on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelockAction {
    /// Drop look while the capture settles (a real grab change only, never a click).
    pub settle: bool,
    /// Forget the last cursor position (the position-delta fallback restarts).
    pub clear_last: bool,
}

/// A real grab change settles and restarts the fallback; a click while already grabbed only re-issues the capture (a
/// Windows backend can reject an early one) and never freezes the look. macOS ignores a repeated grab, so there the
/// fallback position stays too.
pub fn relock_action(grab_changed: bool, click_while_grabbed: bool, mac: bool) -> RelockAction {
    if grab_changed {
        RelockAction { settle: true, clear_last: true }
    } else if click_while_grabbed {
        RelockAction { settle: false, clear_last: !mac }
    } else {
        RelockAction { settle: false, clear_last: false }
    }
}

/// Frame filter between the raw mouse move and the camera: a short time-based settle after a capture, then the spike
/// rejection and the per-frame cap.
#[derive(Clone, Debug, Default)]
pub struct LookFilter {
    frames: u32,
    clock: f32,
    since: u32,
    /// The first move after a re-lock has not arrived yet.
    first_pending: bool,
}

impl LookFilter {
    /// The cursor was just captured again (or a long gap passed): drop the next frames and watch for the warp spike.
    pub fn relock(&mut self, c: &Cfg) {
        self.frames = c.relock_drop_frames;
        self.clock = c.relock_drop_min_s;
        self.since = 0;
        self.first_pending = true;
    }

    /// Whether the settle is still running.
    pub fn settling(&self) -> bool {
        self.frames > 0 || self.clock > 0.0
    }

    /// One frame's pixel move through the filter, `dt` the frame's seconds.
    pub fn apply(&mut self, px: Vec2, dt: f32, c: &Cfg) -> Vec2 {
        if self.settling() {
            self.frames = self.frames.saturating_sub(1);
            self.clock -= dt.max(0.0);
            return Vec2::ZERO;
        }
        self.since = self.since.saturating_add(1);
        let len = px.length();
        if !len.is_finite() {
            return Vec2::ZERO;
        }
        if self.first_pending && len > 0.0 {
            self.first_pending = false;
            if len > c.first_motion_max_px {
                return Vec2::ZERO;
            }
        }
        if self.since <= c.spike_window_frames && len > c.spike_reject_px {
            return Vec2::ZERO;
        }
        if len > c.per_frame_max_px {
            return px * (c.per_frame_max_px / len);
        }
        px
    }
}

/// A smooth 0..1 curve (smoothstep).
pub fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Shortest signed turn from `from` to `to` in radians.
pub fn turn(from: f32, to: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (to - from + PI).rem_euclid(TAU) - PI
}

const SOFT_EPS: f32 = 1e-4;

/// How many zone widths of mouse travel past the start of the soft zone are remembered. Pushing harder than this
/// against the edge is discarded, so turning back never has to unwind a long push (the edge would feel sticky); the
/// look ends `zone * exp(-SOFT_REACH)` short of the limit.
const SOFT_REACH: f32 = 2.5;

/// Maps a signed offset inside a soft limit back to the unbounded "virtual" travel the mouse has made: 1:1 up to
/// `limit - zone`, then stretched so the real offset only approaches `limit`.
fn soft_to_virtual(o: f32, limit: f32, zone: f32) -> f32 {
    let inner = limit - zone;
    let a = o.abs();
    if a <= inner {
        return o;
    }
    let x = ((a - inner) / zone).min(1.0 - SOFT_EPS);
    o.signum() * (inner - zone * (1.0 - x).ln()).min(inner + zone * SOFT_REACH)
}

fn soft_from_virtual(r: f32, limit: f32, zone: f32) -> f32 {
    let inner = limit - zone;
    let a = r.abs().min(inner + zone * SOFT_REACH);
    if a <= inner {
        return r;
    }
    r.signum() * (inner + zone * (1.0 - (-(a - inner) / zone).exp()))
}

/// One step of a soft-limited offset: the offset `o` from the centre moved by `delta` (same units as `limit`). Inside
/// `limit - zone` it follows the mouse 1:1; in the last `zone` it slows smoothly, never passes `limit`, and turning back
/// moves the view at once (the push into the edge is remembered only up to `SOFT_REACH` zones). Exact for any frame length, so a big single-frame move cannot overshoot.
pub fn soft_step(o: f32, delta: f32, limit: f32, zone: f32) -> f32 {
    let limit = limit.max(SOFT_EPS);
    let zone = zone.clamp(SOFT_EPS, limit);
    let v = soft_to_virtual(o.clamp(-limit, limit), limit, zone) + delta;
    soft_from_virtual(v, limit, zone).clamp(-limit, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec2;

    fn c() -> &'static Cfg {
        cfg()
    }

    #[test]
    fn json_validates_and_names_the_bad_key() {
        let good = include_str!("../data/player/look.json");
        assert!(Cfg::parse(good).is_ok());
        let bad = good.replace("\"spike_reject_px\"", "\"spike\"");
        assert!(Cfg::parse(&bad).unwrap_err().contains("spike_reject_px"));
        let bad = good.replace("\"relock_drop_frames\": 2", "\"relock_drop_frames\": -1");
        assert!(Cfg::parse(&bad).unwrap_err().contains("relock_drop_frames"));
        let bad = good.replace("\"pitch_limit_rad\": 1.4", "\"pitch_limit_rad\": 1.7");
        assert!(Cfg::parse(&bad).unwrap_err().contains("pitch_limit_rad"));
        let bad = good.replace("\"spike_reject_px\": 500.0", "\"spike_reject_px\": 900.0");
        assert!(Cfg::parse(&bad).unwrap_err().contains("spike_reject_px"));
    }

    #[test]
    fn multipliers_match_their_derivations() {
        use crate::config::{MOUSE_LOOK_DISPLAY_SCALE, MOUSE_LOOK_GODOT_WINDOW_SCALE};
        let k = MOUSE_LOOK_GODOT_WINDOW_SCALE;
        assert!((c().mouse_multiplier_macos - 1.0 / k).abs() < 1e-5, "macOS points -> 1/k like Godot");
        assert!((c().mouse_multiplier_windows - MOUSE_LOOK_DISPLAY_SCALE / k).abs() < 1e-5, "Windows calibration kept");
    }

    #[test]
    fn delta_to_radians_matches_godot_signs_and_scale() {
        let screen = vec2(1920.0, 1080.0);
        let px = delta_px(vec2(-100.0 * 2.0 / 1920.0, -100.0 * 2.0 / 1080.0), screen, 1.0, None);
        assert!((px.x - 100.0).abs() < 1e-3 && (px.y - 100.0).abs() < 1e-3, "{px:?}");
        let r = px_to_radians(px, 0.0075, 1.0);
        assert!((r.x - 0.75).abs() < 1e-4 && (r.y - 0.75).abs() < 1e-4);
        let fb = delta_px(Vec2::ZERO, screen, 1.0, Some(vec2(100.0, 100.0)));
        assert_eq!(fb, vec2(100.0, 100.0), "the fallback turns the same way");
    }

    #[test]
    fn dpi_scale_keeps_the_turn() {
        // 100 raw points to the right: macroquad divides positions by the dpi scale (and the logical screen is that
        // much smaller), so the delta shows 100 / dpi px; the dpi factor restores the points
        let one = delta_px(vec2(-100.0 * 2.0 / 1000.0, 0.0), vec2(1000.0, 600.0), 1.0, None);
        let two = delta_px(vec2(-(100.0 / 2.0) * 2.0 / 500.0, 0.0), vec2(500.0, 300.0), 2.0, None);
        assert!((one.x - two.x).abs() < 1e-3, "{one:?} {two:?}");
    }

    #[test]
    fn click_while_grabbed_never_arms_the_settle() {
        for mac in [true, false] {
            assert!(!relock_action(false, true, mac).settle);
            assert!(!relock_action(false, false, mac).settle);
            assert!(relock_action(true, false, mac).settle);
        }
        assert!(!relock_action(false, true, true).clear_last, "macOS keeps tracking on a click");
        assert!(relock_action(false, true, false).clear_last);
    }

    #[test]
    fn settle_drops_two_frames_then_passes_everything() {
        let c = c();
        let mut f = LookFilter::default();
        f.relock(c);
        let dt = 1.0 / 60.0;
        let out: Vec<f32> = (0..10).map(|_| f.apply(vec2(20.0, 0.0), dt, c).x).collect();
        let dropped = out.iter().take_while(|x| **x == 0.0).count();
        assert_eq!(dropped as u32, c.relock_drop_frames.max((c.relock_drop_min_s / dt).ceil() as u32), "{out:?}");
        assert!(dropped <= 3, "a 60 fps settle is a couple of frames, not a freeze: {out:?}");
        assert!(out[dropped..].iter().all(|x| *x == 20.0), "{out:?}");
    }

    #[test]
    fn settle_is_time_based_on_a_fast_display() {
        let c = c();
        let mut f = LookFilter::default();
        f.relock(c);
        let dt = 1.0 / 240.0;
        let mut dropped = 0.0;
        while f.apply(vec2(10.0, 0.0), dt, c).x == 0.0 {
            dropped += dt;
            assert!(dropped < 0.2, "never freezes for long");
        }
        assert!(dropped >= c.relock_drop_min_s - dt, "waits out at least the minimum time: {dropped}");
    }

    #[test]
    fn spikes_after_a_grab_are_dropped_and_real_flicks_pass() {
        let c = c();
        let mut f = LookFilter::default();
        f.relock(c);
        while f.settling() {
            f.apply(Vec2::ZERO, 0.016, c);
        }
        assert_eq!(f.apply(vec2(900.0, 0.0), 0.016, c), Vec2::ZERO, "the warp spike is rejected");
        assert_eq!(f.apply(vec2(30.0, 0.0), 0.016, c), vec2(30.0, 0.0), "the next normal frame passes");
        for _ in 0..c.spike_window_frames + 2 {
            f.apply(Vec2::ZERO, 0.016, c);
        }
        let flick = f.apply(vec2(c.spike_reject_px + 100.0, 0.0), 0.016, c);
        assert!((flick.x - (c.spike_reject_px + 100.0).min(c.per_frame_max_px)).abs() < 1e-3, "a real flick is kept: {flick:?}");
        let junk = f.apply(vec2(9000.0, 0.0), 0.016, c);
        assert!((junk.length() - c.per_frame_max_px).abs() < 1e-2, "junk is capped, not scaled further");
        assert_eq!(f.apply(vec2(f32::NAN, 0.0), 0.016, c), Vec2::ZERO);
    }

    #[test]
    fn look_is_independent_of_frame_time() {
        // no dt anywhere in the mouse path: the same total move turns the same however it is split
        let c = c();
        let total = |frames: u32| {
            let mut f = LookFilter::default();
            let mut sum = Vec2::ZERO;
            for _ in 0..frames {
                sum += px_to_radians(f.apply(vec2(120.0 / frames as f32, 0.0), 0.008 * frames as f32, c), 0.0075, c.multiplier());
            }
            sum.x
        };
        assert!((total(1) - total(8)).abs() < 1e-6 && (total(2) - total(8)).abs() < 1e-6);
    }

    #[test]
    fn soft_step_is_identity_inside_bounded_and_monotonic() {
        let (limit, zone) = (0.96_f32, 0.24_f32);
        for i in 0..40 {
            let o = i as f32 * 0.02;
            if o <= limit - zone - 0.02 {
                assert!((soft_step(o, 0.0, limit, zone) - o).abs() < 1e-6);
                assert!((soft_step(o, 0.01, limit, zone) - (o + 0.01)).abs() < 1e-5, "1:1 inside");
            }
        }
        // pushing outward in small steps is monotonic, always below the limit, and slows down
        let mut o = 0.0;
        let mut last_gain = f32::MAX;
        for _ in 0..400 {
            let n = soft_step(o, 0.02, limit, zone);
            assert!(n >= o && n <= limit, "{o} -> {n}");
            if o > limit - zone {
                let gain = n - o;
                assert!(gain <= last_gain + 1e-6, "slows into the edge");
                last_gain = gain;
            }
            o = n;
        }
        assert!(o > limit - zone * 0.1, "it does get to the edge: {o}");
        // one huge frame cannot overshoot, and turning back moves at once
        assert!(soft_step(0.0, 50.0, limit, zone) <= limit);
        assert!(soft_step(-0.5, -50.0, limit, zone) >= -limit);
        let back = soft_step(o, -0.1, limit, zone);
        assert!(back < o - 0.005, "turning back starts moving at once");
        // symmetric
        assert!((soft_step(0.3, 0.9, limit, zone) + soft_step(-0.3, -0.9, limit, zone)).abs() < 1e-6);
    }

    #[test]
    fn turn_takes_the_short_way() {
        assert!((turn(3.0, -3.0) - (std::f32::consts::TAU - 6.0)).abs() < 1e-5);
        assert!(turn(0.5, 0.5).abs() < 1e-6);
    }
}

#[cfg(test)]
mod first_motion_tests {
    use super::*;
    use macroquad::prelude::vec2;

    fn cfg() -> Cfg {
        Cfg::parse(include_str!("../data/player/look.json")).unwrap()
    }

    /// The pointer was moved far while the cursor was free; the first move after the re-lock reports the whole trip.
    #[test]
    fn the_first_big_move_after_a_relock_is_dropped_however_late_it_comes() {
        let c = cfg();
        let mut f = LookFilter::default();
        f.relock(&c);
        for _ in 0..40 {
            assert_eq!(f.apply(Vec2::ZERO, 0.016, &c), Vec2::ZERO);
        }
        // 300 px is under the spike limit: it used to turn the camera by the whole trip
        assert_eq!(f.apply(vec2(300.0, 0.0), 0.016, &c), Vec2::ZERO);
        // and the next real move is kept
        assert_eq!(f.apply(vec2(12.0, 3.0), 0.016, &c), vec2(12.0, 3.0));
    }

    #[test]
    fn a_small_first_move_is_kept() {
        let c = cfg();
        let mut f = LookFilter::default();
        f.relock(&c);
        for _ in 0..10 {
            f.apply(Vec2::ZERO, 0.016, &c);
        }
        assert_eq!(f.apply(vec2(8.0, -2.0), 0.016, &c), vec2(8.0, -2.0));
        assert_eq!(f.apply(vec2(200.0, 0.0), 0.016, &c), vec2(200.0, 0.0), "after the first move a flick is a flick");
    }

    #[test]
    fn every_relock_arms_the_guard_again() {
        let c = cfg();
        let mut f = LookFilter::default();
        f.relock(&c);
        for _ in 0..10 {
            f.apply(Vec2::ZERO, 0.016, &c);
        }
        f.apply(vec2(5.0, 0.0), 0.016, &c);
        f.relock(&c);
        for _ in 0..10 {
            f.apply(Vec2::ZERO, 0.016, &c);
        }
        assert_eq!(f.apply(vec2(250.0, 0.0), 0.016, &c), Vec2::ZERO);
    }
}
