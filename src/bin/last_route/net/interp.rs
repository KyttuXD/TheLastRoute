//! Remote runner smoothing (`data/net/sync.json`, `lr.net_sync/1`). The last sample of a runner arrives 20 to 30
//! times a second, so drawing it as it is steps the figure. Instead each runner keeps a short buffer of its samples and
//! is drawn a little in the past, between the two samples around that moment ([`Track::shown`]); past the newest
//! sample it carries on at its measured speed for a short while, then holds. The delay follows the measured arrival
//! interval and jitter. Everything here takes plain seconds, so it is testable without a clock.
//!
//! The sending side's fixed-step clock is [`SendClock`].

use super::world_sync::{newer, Sample};
use serde_json::Value;
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::OnceLock;
use crate::shared::clock::Instant;

pub const SCHEMA: &str = "lr.net_sync/1";
const TEXT: &str = include_str!("../data/net/sync.json");

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub send_hz: f32,
    /// An early send (a big move, a turn or a pose change) waits at least this many send intervals after the last one.
    pub send_early_min_gap_factor: f32,
    pub send_early_move_m: f32,
    pub send_early_yaw_rad: f32,
    pub interp_delay_min_s: f32,
    pub interp_delay_max_s: f32,
    /// Delay = `interval_factor` x arrival interval + `jitter_factor` x jitter, clamped to the two limits.
    pub interval_factor: f32,
    pub jitter_factor: f32,
    pub max_extrapolation_s: f32,
    /// A jump longer than this (metres), or onto another floor, is a teleport and is not glided.
    pub snap_distance_m: f32,
    /// Two samples further apart than this are not glided between: the figure holds, then appears at the later one.
    pub gap_hold_s: f32,
    pub buffer_len: usize,
    pub remote_timeout_s: f32,
    /// No runner moves faster than this (m/s). A sent or received velocity above it is zeroed (sender) or scaled down
    /// (receiver), and a step faster than it is a jump, not a run.
    pub max_speed_mps: f32,
    /// A sample arriving less than this many send intervals after the one before it came in a burst (a frame hitch
    /// handed several packets over at once): it is placed one send interval after the previous one instead.
    pub burst_gap_factor: f32,
    /// A client that hosts a room (the public route) runs up to this many extra fixed steps after a long frame (a hitch,
    /// or a background tab that was just brought back), so the hunters it simulates for the guests catch up instead of
    /// losing the stall. 0 keeps one capped step per frame.
    pub host_catch_up_steps: u32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            send_hz: 30.0,
            send_early_min_gap_factor: 0.5,
            send_early_move_m: 0.6,
            send_early_yaw_rad: 0.6,
            interp_delay_min_s: 0.08,
            interp_delay_max_s: 0.25,
            interval_factor: 2.0,
            jitter_factor: 1.5,
            max_extrapolation_s: 0.15,
            snap_distance_m: 8.0,
            gap_hold_s: 0.5,
            buffer_len: 8,
            remote_timeout_s: 2.0,
            max_speed_mps: 12.5,
            burst_gap_factor: 0.25,
            host_catch_up_steps: 4,
        }
    }
}

impl Rules {
    /// Parses the table. Every key but `schema` is optional; a present key must be a number in range.
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("sync.json: {e}"))?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("sync.json: key 'schema' must be {SCHEMA}"));
        }
        let d = Rules::default();
        let num = |key: &str, default: f32, min: f32| -> Result<f32, String> {
            match v.get(key) {
                None => Ok(default),
                Some(x) => match x.as_f64() {
                    Some(n) if n.is_finite() && n as f32 >= min => Ok(n as f32),
                    _ => Err(format!("sync.json: key '{key}' must be a number of at least {min}")),
                },
            }
        };
        let r = Rules {
            send_hz: num("send_hz", d.send_hz, 1.0)?,
            send_early_min_gap_factor: num("send_early_min_gap_factor", d.send_early_min_gap_factor, 0.0)?,
            send_early_move_m: num("send_early_move_m", d.send_early_move_m, 0.0)?,
            send_early_yaw_rad: num("send_early_yaw_rad", d.send_early_yaw_rad, 0.0)?,
            interp_delay_min_s: num("interp_delay_min_s", d.interp_delay_min_s, 0.0)?,
            interp_delay_max_s: num("interp_delay_max_s", d.interp_delay_max_s, 0.0)?,
            interval_factor: num("interval_factor", d.interval_factor, 0.0)?,
            jitter_factor: num("jitter_factor", d.jitter_factor, 0.0)?,
            max_extrapolation_s: num("max_extrapolation_s", d.max_extrapolation_s, 0.0)?,
            snap_distance_m: num("snap_distance_m", d.snap_distance_m, 0.0)?,
            gap_hold_s: num("gap_hold_s", d.gap_hold_s, 0.0)?,
            buffer_len: num("buffer_len", d.buffer_len as f32, 2.0)? as usize,
            remote_timeout_s: num("remote_timeout_s", d.remote_timeout_s, 0.1)?,
            max_speed_mps: num("max_speed_mps", d.max_speed_mps, 0.1)?,
            burst_gap_factor: num("burst_gap_factor", d.burst_gap_factor, 0.0)?,
            host_catch_up_steps: num("host_catch_up_steps", d.host_catch_up_steps as f32, 0.0)?.min(20.0) as u32,
        };
        if r.interp_delay_min_s > r.interp_delay_max_s {
            return Err("sync.json: key 'interp_delay_min_s' must not exceed 'interp_delay_max_s'".into());
        }
        Ok(r)
    }

    pub fn send_interval(&self) -> f64 {
        1.0 / self.send_hz as f64
    }
}

pub fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules::parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// Scales a ground velocity down to at most `max` m/s. Non-finite or overflowing input is zeroed.
pub fn clamp_velocity(vx: f32, vz: f32, max: f32) -> (f32, f32) {
    let (x, z) = (vx as f64, vz as f64);
    let len = x.hypot(z);
    if !len.is_finite() {
        return (0.0, 0.0);
    }
    if len <= max as f64 {
        return (vx, vz);
    }
    let k = max as f64 / len;
    ((x * k) as f32, (z * k) as f32)
}

/// Seconds since the first call: the one timeline all tracks share.
pub fn secs(now: Instant) -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    match now.checked_duration_since(epoch) {
        Some(d) => d.as_secs_f64(),
        None => -epoch.duration_since(now).as_secs_f64(),
    }
}

/// One runner's received samples with their arrival times, oldest first.
#[derive(Clone, Debug)]
pub struct Track {
    buf: VecDeque<(f64, Sample)>,
    /// Smoothed gap between arrivals and its smoothed deviation (the jitter), seconds.
    interval: f32,
    jitter: f32,
    /// When the last accepted sample really arrived (the buffer times can run ahead of it).
    last_arrival: f64,
    /// The latest render time handed out: the picture never plays backwards when the delay grows.
    last_render: Cell<f64>,
}

impl Track {
    pub fn new(rules: &Rules) -> Track {
        Track { buf: VecDeque::new(), interval: 1.0 / rules.send_hz, jitter: 0.0, last_arrival: f64::NEG_INFINITY, last_render: Cell::new(f64::NEG_INFINITY) }
    }

    /// Adds a sample that arrived at `t`. A sample that is not newer than the newest kept (a reordered or duplicate
    /// packet) is refused. Arrival times are kept strictly increasing: a burst (samples that arrive together, as after
    /// a frame hitch) is spaced one send interval apart, at most `interp_delay_max_s` ahead of the clock, so it plays
    /// back as the smooth walk it was and does not inflate the jitter estimate. Ordinary arrivals after a burst are
    /// placed half an interval apart until the lead over the clock is gone, so the extra latency drains.
    pub fn push(&mut self, t: f64, sample: Sample, rules: &Rules) -> bool {
        let real = t;
        let mut t = t;
        if let Some((last_t, last)) = self.buf.back() {
            if !newer(sample.seq, last.seq) {
                return false;
            }
            let send = rules.send_interval();
            let real_gap = real - self.last_arrival;
            if real_gap < send * rules.burst_gap_factor as f64 {
                t = (last_t + send).min(real + rules.interp_delay_max_s as f64);
            } else if *last_t > real {
                t = real.max(last_t + send * 0.5);
            }
            let gap = real_gap.max(0.0) as f32;
            // only ordinary gaps teach the estimate: a stall or a keep-alive silence must not inflate the delay for good,
            // and an early send (a gap under half an interval) says nothing about the rate
            if gap < 0.5 && gap as f64 >= send * 0.5 {
                self.jitter += ((gap - self.interval).abs() - self.jitter) * 0.1;
                self.interval += (gap - self.interval) * 0.1;
            }
        }
        self.last_arrival = real;
        let t = match self.buf.back() {
            Some((last_t, _)) if t <= *last_t => last_t + 1e-3,
            _ => t,
        };
        self.buf.push_back((t, sample));
        while self.buf.len() > rules.buffer_len.max(2) {
            self.buf.pop_front();
        }
        true
    }

    /// How far behind the newest data a runner is drawn.
    pub fn delay(&self, rules: &Rules) -> f32 {
        (rules.interval_factor * self.interval + rules.jitter_factor * self.jitter).clamp(rules.interp_delay_min_s, rules.interp_delay_max_s)
    }

    pub fn newest(&self) -> Option<&Sample> {
        self.buf.back().map(|(_, s)| s)
    }

    /// The runner as drawn at time `now`: the buffer played back `delay` seconds late. Position, yaw and height are
    /// interpolated; the other fields come from the earlier sample of the pair.
    pub fn shown(&self, now: f64, rules: &Rules) -> Option<Sample> {
        let render = (now - self.delay(rules) as f64).max(self.last_render.get());
        self.last_render.set(render);
        let (first_t, first) = self.buf.front()?;
        if render <= *first_t {
            return Some(*first);
        }
        for i in 0..self.buf.len() - 1 {
            let (ta, a) = &self.buf[i];
            let (tb, b) = &self.buf[i + 1];
            if render >= *tb {
                continue;
            }
            let teleport = a.depth != b.depth || (b.x - a.x).hypot(b.y - a.y) > rules.snap_distance_m;
            if teleport || (tb - ta) as f32 > rules.gap_hold_s {
                // a jump or a long silence is not a walk: hold, then appear at the later sample
                return Some(if teleport { *b } else { *a });
            }
            let k = ((render - ta) / (tb - ta)) as f32;
            return Some(blend(a, b, k));
        }
        // past the newest sample: carry on at the measured speed for a short while, then hold
        let (tn, n) = self.buf.back()?;
        let ahead = ((render - tn) as f32).min(rules.max_extrapolation_s).max(0.0);
        let (vx, vz) = clamp_velocity_pair(self.velocity(), rules.max_speed_mps);
        let mut s = *n;
        s.x += vx * ahead;
        s.y += vz * ahead;
        Some(s)
    }

    /// The newest sample's velocity: what it says, else (an older client) the step from the sample before it.
    fn velocity(&self) -> (f32, f32) {
        let Some((tn, n)) = self.buf.back() else { return (0.0, 0.0) };
        if !n.moving {
            return (0.0, 0.0);
        }
        if n.vx != 0.0 || n.vz != 0.0 {
            return (n.vx, n.vz);
        }
        match self.buf.iter().rev().nth(1) {
            Some((tp, p)) if p.depth == n.depth && tn - tp > 1e-3 && tn - tp < 0.5 => {
                let dt = (tn - tp) as f32;
                ((n.x - p.x) / dt, (n.y - p.y) / dt)
            }
            _ => (0.0, 0.0),
        }
    }
}

fn clamp_velocity_pair(v: (f32, f32), max: f32) -> (f32, f32) {
    clamp_velocity(v.0, v.1, max)
}

fn blend(a: &Sample, b: &Sample, k: f32) -> Sample {
    let mut s = *a;
    s.x = a.x + (b.x - a.x) * k;
    s.y = a.y + (b.y - a.y) * k;
    s.yaw = lerp_angle(a.yaw, b.yaw, k);
    s.h = match (a.h, b.h) {
        (Some(x), Some(y)) => Some(x + (y - x) * k),
        (x, _) => x,
    };
    s.vx = a.vx + (b.vx - a.vx) * k;
    s.vz = a.vz + (b.vz - a.vz) * k;
    s
}

/// Interpolates an angle the short way round.
pub fn lerp_angle(a: f32, b: f32, k: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    a + d * k
}

/// The sender's fixed-step clock: `next += interval`, so the rate does not depend on the frame length.
#[derive(Clone, Debug, Default)]
pub struct SendClock {
    next: f64,
    last: f64,
}

impl SendClock {
    /// Whether a sample goes out at `now` (seconds). `early` asks for one before the step is due (a big move or a
    /// pose change); it is honoured once the minimum gap since the last send has passed.
    pub fn due(&mut self, now: f64, early: bool, rules: &Rules) -> bool {
        let interval = rules.send_interval();
        let step_due = now + 0.001 >= self.next;
        let early_ok = early && now - self.last >= interval * rules.send_early_min_gap_factor as f64;
        if !step_due && !early_ok {
            return false;
        }
        self.last = now;
        self.next = if step_due && now - self.next <= interval { self.next + interval } else { now + interval };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seq: u32, x: f32, y: f32) -> Sample {
        Sample {
            seq,
            depth: 1,
            h: Some(5.0),
            x,
            y,
            yaw: 0.0,
            moving: true,
            sprinting: false,
            crouching: false,
            spectating: false,
            game_over: false,
            escape_active: false,
            clip: None,
            vy: 0.0,
            vx: 0.0,
            vz: 0.0,
            slim: false,
            hide: None,
            held: None,
            flash_pitch: None,
        }
    }

    fn track_with(points: &[(f64, u32, f32)]) -> (Track, Rules) {
        let r = Rules::default();
        let mut t = Track::new(&r);
        for &(time, seq, x) in points {
            assert!(t.push(time, at(seq, x, 0.0), &r));
        }
        (t, r)
    }

    #[test]
    fn the_shipped_table_parses_and_a_missing_key_keeps_its_default() {
        assert_eq!(Rules::parse(TEXT).unwrap(), Rules::default());
        let r = Rules::parse(r#"{"schema":"lr.net_sync/1","send_hz":20}"#).unwrap();
        assert_eq!(r.send_hz, 20.0);
        assert_eq!(r.buffer_len, 8);
        assert!(Rules::parse(r#"{"schema":"lr.net_sync/1","send_hz":"fast"}"#).is_err());
        assert!(Rules::parse(r#"{"schema":"lr.net_sync/1","interp_delay_min_s":1,"interp_delay_max_s":0.1}"#).is_err());
        assert!(Rules::parse(r#"{"schema":"nope"}"#).is_err());
    }

    #[test]
    fn a_time_between_two_samples_is_interpolated() {
        let (t, r) = track_with(&[(10.0, 1, 0.0), (10.05, 2, 1.0)]);
        let d = t.delay(&r) as f64;
        let s = t.shown(10.0 + d + 0.025, &r).unwrap();
        assert!((s.x - 0.5).abs() < 1e-3, "{}", s.x);
    }

    #[test]
    fn the_picture_depends_on_time_not_on_how_often_it_is_drawn() {
        // 5 m/s, a sample every 50 ms
        let pts: Vec<(f64, u32, f32)> = (0..8).map(|i| (10.0 + i as f64 * 0.05, i + 1, i as f32 * 0.25)).collect();
        let (t, r) = track_with(&pts);
        let d = t.delay(&r) as f64;
        for fps in [24.0, 60.0, 144.0] {
            let t = t.clone();
            let mut now = 10.0 + d + 0.05;
            while now < 10.0 + d + 0.3 {
                let want = 5.0 * (now - d - 10.0) as f32;
                let got = t.shown(now, &r).unwrap().x;
                assert!((got - want).abs() < 1e-3, "{fps} fps at {now}: {got} vs {want}");
                now += 1.0 / fps;
            }
        }
    }

    #[test]
    fn a_held_item_and_a_hide_change_with_the_sample_that_carries_them() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        t.push(10.0, at(1, 0.0, 0.0), &r);
        let mut b = at(2, 0.2, 0.0);
        b.held = Some(crate::game::Item::Axe);
        t.push(10.05, b, &r);
        let d = t.delay(&r) as f64;
        assert_eq!(t.shown(10.0 + d + 0.025, &r).unwrap().held, None, "between the two, the earlier sample's item");
        assert_eq!(t.shown(10.05 + d + 0.01, &r).unwrap().held, Some(crate::game::Item::Axe), "then the later one's");
        assert_eq!(t.newest().unwrap().held, Some(crate::game::Item::Axe));
    }

    #[test]
    fn a_reordered_or_repeated_packet_is_refused() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        assert!(t.push(1.0, at(5, 5.0, 0.0), &r));
        assert!(!t.push(1.01, at(4, 4.0, 0.0), &r), "older");
        assert!(!t.push(1.02, at(5, 9.0, 0.0), &r), "repeat");
        assert!(t.push(1.03, at(7, 7.0, 0.0), &r), "a lost packet is only a gap");
        assert_eq!(t.newest().unwrap().seq, 7);
    }

    #[test]
    fn the_buffer_is_capped_and_arrival_times_stay_increasing() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        for i in 0..40u32 {
            t.push(5.0, at(i + 1, i as f32, 0.0), &r);
        }
        assert_eq!(t.buf.len(), r.buffer_len);
        assert!(t.buf.iter().zip(t.buf.iter().skip(1)).all(|(a, b)| a.0 < b.0));
    }

    #[test]
    fn past_the_newest_sample_the_runner_carries_on_for_a_short_while_then_holds() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        let mut a = at(1, 0.0, 0.0);
        a.vx = 4.0;
        t.push(20.0, a, &r);
        let mut b = at(2, 0.2, 0.0);
        b.vx = 4.0;
        t.push(20.05, b, &r);
        let d = t.delay(&r) as f64;
        let s = t.shown(20.05 + d + 0.05, &r).unwrap();
        assert!((s.x - (0.2 + 4.0 * 0.05)).abs() < 1e-3);
        let far = t.shown(20.05 + d + 5.0, &r).unwrap();
        assert!((far.x - (0.2 + 4.0 * r.max_extrapolation_s)).abs() < 1e-3, "capped, got {}", far.x);
        // a standing sample is not extrapolated
        let mut stand = at(3, 0.5, 0.0);
        stand.moving = false;
        stand.vx = 4.0;
        t.push(20.1, stand, &r);
        assert!((t.shown(20.1 + d + 1.0, &r).unwrap().x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_sample_without_velocity_is_extrapolated_from_the_step_before() {
        let (t, r) = track_with(&[(1.0, 1, 0.0), (1.05, 2, 0.25)]);
        let s = t.shown(1.05 + t.delay(&r) as f64 + 0.05, &r).unwrap();
        assert!((s.x - 0.5).abs() < 1e-3, "{}", s.x);
    }

    #[test]
    fn a_teleport_snaps_and_a_long_silence_is_not_glided() {
        let (t, r) = track_with(&[(1.0, 1, 0.0), (1.05, 2, 50.0)]);
        let d = t.delay(&r) as f64;
        assert_eq!(t.shown(1.0 + d + 0.01, &r).unwrap().x, 50.0, "no glide over 50 m");
        let (t, r) = track_with(&[(1.0, 1, 0.0), (2.0, 2, 3.0)]);
        let d = t.delay(&r) as f64;
        assert_eq!(t.shown(1.0 + d + 0.5, &r).unwrap().x, 0.0, "holds through the gap");
        assert_eq!(t.shown(2.0 + d + 0.001, &r).unwrap().x.round(), 3.0, "then appears at the next sample");
        // another floor snaps too
        let r = Rules::default();
        let mut t = Track::new(&r);
        t.push(1.0, at(1, 0.0, 0.0), &r);
        let mut up = at(2, 0.5, 0.0);
        up.depth = 2;
        t.push(1.05, up, &r);
        assert_eq!(t.shown(1.0 + t.delay(&r) as f64 + 0.01, &r).unwrap().depth, 2);
    }

    #[test]
    fn yaw_goes_the_short_way_round() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        let mut a = at(1, 0.0, 0.0);
        a.yaw = 3.0;
        let mut b = at(2, 0.0, 0.0);
        b.yaw = -3.0;
        t.push(1.0, a, &r);
        t.push(1.05, b, &r);
        let y = t.shown(1.0 + t.delay(&r) as f64 + 0.025, &r).unwrap().yaw;
        assert!((y.abs() - std::f32::consts::PI).abs() < 0.2, "through pi, not through 0: {y}");
        assert!((lerp_angle(0.1, 6.2, 0.5) - (0.1 - 0.0916)).abs() < 0.01);
    }

    #[test]
    fn the_delay_follows_jitter_within_its_limits() {
        let r = Rules::default();
        let mut steady = Track::new(&r);
        let mut ragged = Track::new(&r);
        let mut t = 0.0;
        for i in 0..60u32 {
            steady.push(i as f64 / 30.0, at(i + 1, 0.0, 0.0), &r);
            t += if i % 2 == 0 { 0.01 } else { 0.07 };
            ragged.push(t, at(i + 1, 0.0, 0.0), &r);
        }
        assert!(ragged.delay(&r) > steady.delay(&r));
        assert!(steady.delay(&r) >= r.interp_delay_min_s);
        assert!(ragged.delay(&r) <= r.interp_delay_max_s);
    }

    #[test]
    fn the_send_clock_gives_the_rate_at_any_frame_rate() {
        let r = Rules::default();
        for fps in [30.0, 60.0, 144.0, 240.0] {
            let mut c = SendClock::default();
            let sent = (0..(fps as usize * 10)).filter(|i| c.due(*i as f64 / fps, false, &r)).count();
            let hz = sent as f32 / 10.0;
            assert!((hz - 30.0).abs() <= 1.0 + if fps == 30.0 { 0.1 } else { 0.0 }, "{fps} fps gave {hz} Hz");
        }
    }

    #[test]
    fn the_send_clock_resyncs_after_a_stall_and_allows_a_spaced_early_send() {
        let r = Rules::default();
        let mut c = SendClock::default();
        assert!(c.due(0.0, false, &r));
        assert!(c.due(1.0, false, &r), "after a stall");
        assert!(!c.due(1.001, false, &r), "no burst of catch-up sends");
        assert!(!c.due(1.005, true, &r), "early sends keep the minimum gap");
        assert!(c.due(1.02, true, &r));
        assert!(!c.due(1.03, false, &r));
    }

    #[test]
    fn a_burst_of_samples_in_one_poll_plays_smoothly() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        // 5 m/s, a sample every 33 ms, the first on time, then five that arrive together after a hitch
        t.push(10.0, at(1, 0.0, 0.0), &r);
        for i in 0..5u32 {
            t.push(10.3, at(i + 2, (i + 1) as f32 * 5.0 / 30.0, 0.0), &r);
        }
        let times: Vec<f64> = t.buf.iter().map(|b| b.0).collect();
        for w in times[1..].windows(2) {
            assert!((w[1] - w[0] - r.send_interval()).abs() < 1e-6, "spaced by the send interval: {times:?}");
        }
        assert!(t.delay(&r) <= 0.25 && t.jitter < 0.05, "a burst does not inflate the jitter: {}", t.jitter);
        // playback moves at a steady speed, not a leap
        let d = t.delay(&r) as f64;
        let mut last = t.shown(10.3 + d - 0.1, &r).unwrap().x;
        let mut now = 10.3 + d - 0.1;
        while now < 10.3 + d + 0.1 {
            now += 0.005;
            let x = t.shown(now, &r).unwrap().x;
            assert!(x >= last && x - last < 0.05, "step of {} at {now}", x - last);
            last = x;
        }
    }

    #[test]
    fn the_lead_a_burst_leaves_drains_at_the_regular_rate() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        t.push(10.0, at(1, 0.0, 0.0), &r);
        for i in 0..5u32 {
            t.push(10.3, at(i + 2, 0.0, 0.0), &r);
        }
        assert!(t.buf.back().unwrap().0 - 10.3 > 0.1, "the burst runs ahead of the clock");
        let mut arrival = 10.3;
        for i in 0..60u32 {
            arrival += r.send_interval();
            t.push(arrival, at(i + 7, 0.0, 0.0), &r);
        }
        let lead = t.buf.back().unwrap().0 - arrival;
        assert!(lead < 0.05, "the lead is gone: {lead}");
    }

    #[test]
    fn the_picture_never_runs_backwards_and_early_sends_do_not_teach_the_interval() {
        let r = Rules::default();
        let mut t = Track::new(&r);
        for i in 0..10u32 {
            t.push(5.0 + i as f64 * 0.03, at(i + 1, i as f32, 0.0), &r);
        }
        let x0 = t.shown(5.3, &r).unwrap().x;
        // a late jittery arrival raises the delay; an earlier render time must not come back
        t.push(5.9, at(11, 10.0, 0.0), &r);
        t.push(6.2, at(12, 11.0, 0.0), &r);
        assert!(t.shown(5.3, &r).unwrap().x >= x0);
        let before = t.interval;
        t.push(6.2 + 0.005, at(13, 12.0, 0.0), &r);
        assert_eq!(t.interval, before, "an early send is not an interval");
    }

    #[test]
    fn a_hostile_velocity_is_bounded() {
        let r = Rules::default();
        let (vx, vz) = clamp_velocity(1e38, 1e38, r.max_speed_mps);
        assert!(vx.hypot(vz) <= r.max_speed_mps + 1e-3 && vx > 0.0, "scaled down: {vx} {vz}");
        assert_eq!(clamp_velocity(3.0, 4.0, r.max_speed_mps), (3.0, 4.0), "an ordinary speed is untouched");
        assert_eq!(clamp_velocity(f32::NAN, 1.0, r.max_speed_mps), (0.0, 0.0));
        let mut t = Track::new(&r);
        let mut a = at(1, 0.0, 0.0);
        a.vx = 1e38;
        t.push(1.0, a, &r);
        let mut b = at(2, 0.1, 0.0);
        b.vx = 1e38;
        t.push(1.05, b, &r);
        let far = t.shown(1.05 + t.delay(&r) as f64 + 5.0, &r).unwrap();
        assert!(far.x < 0.1 + r.max_speed_mps * r.max_extrapolation_s + 1e-3, "extrapolation is bounded: {}", far.x);
    }
}
