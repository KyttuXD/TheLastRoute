//! `Instant` and `SystemTime` for the game. Native builds re-export the standard library types unchanged.
//!
//! The browser build (wasm32-unknown-unknown) has no clock in `std`: `Instant::now()` and `SystemTime::now()`
//! panic there. The shim below reads the same source macroquad uses (`miniquad::date::now`, seconds since the Unix
//! epoch, from the page's JavaScript clock) and offers the subset of the std API the game calls. The browser's clock
//! is not monotonic in theory (a system clock change can move it), so differences are clamped at zero.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A `SystemTime` that the standard library gave (a file's modified time): the same value here.
#[cfg(not(target_arch = "wasm32"))]
pub fn from_std(t: std::time::SystemTime) -> SystemTime {
    t
}

#[cfg(target_arch = "wasm32")]
pub use web::{from_std, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cmp::Ordering;
    use std::hash::{Hash, Hasher};
    use std::ops::{Add, AddAssign, Sub, SubAssign};
    use std::time::Duration;

    /// Seconds since the Unix epoch, from the browser.
    fn now_secs() -> f64 {
        macroquad::miniquad::date::now()
    }

    fn secs(d: Duration) -> f64 {
        d.as_secs_f64()
    }

    /// A point in time (the browser clock, in seconds).
    #[derive(Clone, Copy, Debug)]
    pub struct Instant(f64);

    impl Instant {
        pub fn now() -> Instant {
            Instant(now_secs())
        }
        pub fn elapsed(&self) -> Duration {
            Instant::now().duration_since(*self)
        }
        /// Time from `earlier` to `self`, zero when `earlier` is later (std panics then; the game never relies on that).
        pub fn duration_since(&self, earlier: Instant) -> Duration {
            Duration::from_secs_f64((self.0 - earlier.0).max(0.0))
        }
        pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
            self.duration_since(earlier)
        }
        pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
            (self.0 >= earlier.0).then(|| self.duration_since(earlier))
        }
        pub fn checked_add(&self, d: Duration) -> Option<Instant> {
            Some(Instant(self.0 + secs(d)))
        }
        pub fn checked_sub(&self, d: Duration) -> Option<Instant> {
            Some(Instant(self.0 - secs(d)))
        }
    }

    impl PartialEq for Instant {
        fn eq(&self, other: &Instant) -> bool {
            self.0 == other.0
        }
    }
    impl Eq for Instant {}
    impl PartialOrd for Instant {
        fn partial_cmp(&self, other: &Instant) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Instant {
        fn cmp(&self, other: &Instant) -> Ordering {
            self.0.total_cmp(&other.0)
        }
    }
    impl Hash for Instant {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.to_bits().hash(state);
        }
    }
    impl Add<Duration> for Instant {
        type Output = Instant;
        fn add(self, d: Duration) -> Instant {
            Instant(self.0 + secs(d))
        }
    }
    impl AddAssign<Duration> for Instant {
        fn add_assign(&mut self, d: Duration) {
            self.0 += secs(d);
        }
    }
    impl Sub<Duration> for Instant {
        type Output = Instant;
        fn sub(self, d: Duration) -> Instant {
            Instant(self.0 - secs(d))
        }
    }
    impl SubAssign<Duration> for Instant {
        fn sub_assign(&mut self, d: Duration) {
            self.0 -= secs(d);
        }
    }
    impl Sub<Instant> for Instant {
        type Output = Duration;
        fn sub(self, other: Instant) -> Duration {
            self.duration_since(other)
        }
    }

    /// A wall-clock time (seconds since the Unix epoch, from the browser).
    #[derive(Clone, Copy, Debug)]
    pub struct SystemTime(f64);

    pub const UNIX_EPOCH: SystemTime = SystemTime(0.0);

    /// A standard-library time (a file's modified time) as a browser-clock time.
    pub fn from_std(t: std::time::SystemTime) -> SystemTime {
        SystemTime(t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0))
    }

    /// The error of [`SystemTime::duration_since`] when the time is before the earlier one (never on the browser clock).
    #[derive(Clone, Debug)]
    pub struct SystemTimeError;

    impl std::fmt::Display for SystemTimeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("second time provided was later than self")
        }
    }

    impl std::error::Error for SystemTimeError {}

    impl SystemTime {
        pub fn now() -> SystemTime {
            SystemTime(now_secs())
        }
        pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
            if self.0 >= earlier.0 {
                Ok(Duration::from_secs_f64(self.0 - earlier.0))
            } else {
                Err(SystemTimeError)
            }
        }
        pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
            SystemTime::now().duration_since(*self)
        }
    }

    impl PartialEq for SystemTime {
        fn eq(&self, other: &SystemTime) -> bool {
            self.0 == other.0
        }
    }
    impl Eq for SystemTime {}
    impl PartialOrd for SystemTime {
        fn partial_cmp(&self, other: &SystemTime) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for SystemTime {
        fn cmp(&self, other: &SystemTime) -> Ordering {
            self.0.total_cmp(&other.0)
        }
    }
    impl Hash for SystemTime {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.to_bits().hash(state);
        }
    }
    impl Add<Duration> for SystemTime {
        type Output = SystemTime;
        fn add(self, d: Duration) -> SystemTime {
            SystemTime(self.0 + secs(d))
        }
    }
    impl Sub<Duration> for SystemTime {
        type Output = SystemTime;
        fn sub(self, d: Duration) -> SystemTime {
            SystemTime(self.0 - secs(d))
        }
    }
}
