//! Seeded random streams: Mulberry32 (the 2D game's stream, bit for bit), the Stream wrapper with stub and neutral
//! answers, and the Pacer that decides when a hunter moves. Pure logic.
#![allow(dead_code)]

use super::data::move_window;

pub const VARIETY_SALT: u32 = 0x7a3c19e5;

#[derive(Clone, Debug)]
pub struct Mulberry32 {
    a: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Mulberry32 { a: seed }
    }
    pub fn state(&self) -> u32 {
        self.a
    }
    pub fn set_state(&mut self, s: u32) {
        self.a = s;
    }
    /// One draw in [0, 1).
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b79f5);
        let mut t = self.a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4294967296.0
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next() * (hi - lo)
    }
    /// Index in 0..n (always one draw). 0 when n is 0.
    pub fn pick(&mut self, n: usize) -> usize {
        let u = self.next();
        if n == 0 {
            return 0;
        }
        ((u * n as f64).floor() as usize).min(n - 1)
    }
    /// Fisher-Yates from the end.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        let mut i = v.len();
        while i > 1 {
            i -= 1;
            let j = (self.next() * (i as f64 + 1.0)).floor() as usize;
            v.swap(i, j.min(i));
        }
    }
}

/// base +/- spread (a fraction), one draw (core/jitter.js).
pub fn jitter(base: f64, spread: f64, r: &mut Mulberry32) -> f64 {
    let k = spread.clamp(0.0, 0.9);
    base * (1.0 - k + 2.0 * k * r.next())
}

/// ms +/- spread, evenly (core/time.js spreadMs). Callers pass 0.2 by default.
pub fn spread_ms(ms: f64, r: &mut Mulberry32, spread: f64) -> f64 {
    ms * (1.0 - spread + 2.0 * spread * r.next())
}

/// NightWatchStream: a Mulberry32 that can be a "stub" (fixed answer) or neutral (0.5).
#[derive(Clone, Debug)]
pub struct Stream {
    r: Mulberry32,
    stub: Option<f64>,
    pub neutral: bool,
}

impl Stream {
    pub fn seeded(seed: u32) -> Stream {
        Stream { r: Mulberry32::new(seed), stub: None, neutral: false }
    }
    /// A fixed answer, with no state.
    pub fn fixed(v: f64) -> Stream {
        Stream { r: Mulberry32::new(0), stub: Some(v), neutral: false }
    }
    /// Answers 0.5: every jitter is its base, every chance fails, every pick is the first.
    pub fn neutral() -> Stream {
        Stream { r: Mulberry32::new(0), stub: Some(0.5), neutral: true }
    }
    pub fn next(&mut self) -> f64 {
        match self.stub {
            Some(v) => v,
            None => self.r.next(),
        }
    }
    pub fn has_state(&self) -> bool {
        self.stub.is_none()
    }
    pub fn state(&self) -> u32 {
        if self.has_state() {
            self.r.state()
        } else {
            0
        }
    }
    pub fn set_state(&mut self, s: u32) {
        self.r.set_state(s);
    }
    /// `given` wins, else a stream derived from rng's state (before the night's first draw), else neutral.
    pub fn variety_of(rng: &Stream, given: Option<Stream>) -> Stream {
        if let Some(g) = given {
            return g;
        }
        if rng.has_state() {
            return Stream::seeded(rng.state() ^ VARIETY_SALT);
        }
        Stream::neutral()
    }
    /// A pacer multiplier built so the WAIT is what is even.
    pub fn tempo(&mut self, spread: f64) -> f64 {
        let k = spread.clamp(0.0, 0.9);
        1.0 / (1.0 - k + 2.0 * k * self.next())
    }
    /// Always draws; true only when not neutral and the draw is under p.
    pub fn chance(&mut self, p: f64) -> bool {
        let u = self.next();
        !self.neutral && u < p
    }
    /// Always draws; 0 when neutral.
    pub fn pick_index(&mut self, n: usize) -> usize {
        let u = self.next();
        if self.neutral || n == 0 {
            0
        } else {
            ((u * n as f64).floor() as usize).min(n - 1)
        }
    }
}

/// How often a hunter moves: after each move she waits a random time from a three-second window that slides with her level.
#[derive(Clone, Debug)]
pub struct Pacer {
    pub t: f64,
    pub next: f64,
}

impl Pacer {
    pub fn new() -> Pacer {
        Pacer { t: 0.0, next: -1.0 }
    }
    fn draw_wait(level: i32, draw: &mut dyn FnMut() -> f64) -> f64 {
        let w = move_window(level);
        w.0 + draw() * (w.1 - w.0)
    }
    /// Advance by dt_ms at this level; true when she is due to move.
    pub fn step(&mut self, dt_ms: f64, level: i32, draw: &mut dyn FnMut() -> f64) -> bool {
        if level <= 0 {
            self.t = 0.0;
            self.next = -1.0;
            return false;
        }
        if self.next < 0.0 {
            self.next = Pacer::draw_wait(level, draw);
        }
        self.t += dt_ms;
        if self.t < self.next {
            return false;
        }
        self.t = 0.0;
        self.next = Pacer::draw_wait(level, draw);
        true
    }
    pub fn reset(&mut self) {
        self.t = 0.0;
        self.next = -1.0;
    }
    /// A nudge: she is that much nearer her move.
    pub fn skip(&mut self, ms: f64) {
        self.t += ms;
    }
}

impl Default for Pacer {
    fn default() -> Self {
        Pacer::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(seed: u32, n: usize) -> Vec<u32> {
        let mut r = Mulberry32::new(seed);
        (0..n).map(|_| (r.next() * 4294967296.0) as u32).collect()
    }

    #[test]
    fn mulberry_vectors_match_the_2d_stream() {
        assert_eq!(raw(0, 3), vec![1144304738, 1416247, 958946056]);
        assert_eq!(raw(1, 3), vec![2693262067, 11749833, 2265367787]);
        assert_eq!(raw(12345, 3), vec![4207900869, 1317490944, 2079646450]);
    }

    #[test]
    fn mulberry_stays_in_range_and_is_repeatable() {
        let mut a = Mulberry32::new(99);
        let mut b = Mulberry32::new(99);
        for _ in 0..1000 {
            let x = a.next();
            assert!((0.0..1.0).contains(&x));
            assert_eq!(x, b.next());
        }
        let mut c = Mulberry32::new(5);
        for _ in 0..200 {
            let p = c.pick(4);
            assert!(p < 4);
            let r = c.range(2.0, 3.0);
            assert!((2.0..3.0).contains(&r));
        }
    }

    #[test]
    fn shuffle_keeps_the_items() {
        let mut r = Mulberry32::new(7);
        let mut v: Vec<i32> = (0..10).collect();
        r.shuffle(&mut v);
        let mut s = v.clone();
        s.sort();
        assert_eq!(s, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn jitter_stays_inside_its_spread() {
        let mut r = Mulberry32::new(3);
        for _ in 0..200 {
            let j = jitter(100.0, 0.2, &mut r);
            assert!((80.0..=120.0).contains(&j));
            let s = spread_ms(1000.0, &mut r, 0.2);
            assert!((800.0..=1200.0).contains(&s));
        }
    }

    #[test]
    fn neutral_stream_is_flat() {
        let mut n = Stream::neutral();
        assert!(!n.has_state());
        assert_eq!(n.state(), 0);
        assert_eq!(n.tempo(0.3), 1.0);
        assert!(!n.chance(1.0));
        assert_eq!(n.pick_index(5), 0);
        let mut f = Stream::fixed(0.999);
        assert!(f.chance(1.0));
        assert_eq!(f.pick_index(4), 3);
    }

    #[test]
    fn variety_is_derived_from_the_state() {
        let a = Stream::seeded(1234);
        let v = Stream::variety_of(&a, None);
        assert_eq!(v.state(), 1234 ^ VARIETY_SALT);
        let n = Stream::variety_of(&Stream::fixed(0.3), None);
        assert!(n.neutral);
        let g = Stream::variety_of(&a, Some(Stream::fixed(0.1)));
        assert!(!g.has_state());
    }

    #[test]
    fn chance_always_draws() {
        let mut a = Stream::seeded(8);
        let mut b = Stream::seeded(8);
        let _ = a.chance(0.0);
        b.next();
        assert_eq!(a.state(), b.state());
    }

    #[test]
    fn pacer_windows_follow_the_level() {
        for (level, lo, hi) in [(1, 12500.0, 15500.0), (20, 3000.0, 6000.0)] {
            // draw 0 gives the low edge, draw ~1 the high edge
            let mut p = Pacer::new();
            let mut zero = || 0.0;
            assert!(!p.step(lo - 1.0, level, &mut zero));
            assert!(p.step(1.0, level, &mut zero));
            let mut p = Pacer::new();
            let mut one = || 0.999999;
            assert!(!p.step(hi - 10.0, level, &mut one));
            assert!(p.step(10.0, level, &mut one));
        }
    }

    #[test]
    fn pacer_level_zero_never_moves_and_draws_nothing() {
        let mut p = Pacer::new();
        let mut draws = 0;
        let mut d = || {
            draws += 1;
            0.5
        };
        for _ in 0..100 {
            assert!(!p.step(5000.0, 0, &mut d));
        }
        assert_eq!(draws, 0);
        assert_eq!(p.next, -1.0);
    }

    #[test]
    fn pacer_skip_brings_the_move_closer() {
        let mut p = Pacer::new();
        let mut zero = || 0.0;
        assert!(!p.step(0.0, 1, &mut zero));
        p.skip(12000.0);
        assert!(p.step(600.0, 1, &mut zero));
    }
}
