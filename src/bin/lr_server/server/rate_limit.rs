//! The per-address request limiter: a fixed window per key, and the sign-in / sign-up bucket with its own tunable limit.
//!
//! Limits are data: `LASTROUTE_API_RATE_LIMIT`, `LASTROUTE_MOD_RATE_LIMIT`, `LASTROUTE_ACCOUNT_RATE_LIMIT`,
//! `LASTROUTE_AUTH_RATE_LIMIT` (sign-in, sign-up, password change, account deletion) and
//! `LASTROUTE_RATE_LIMIT_WINDOW_SECONDS` (see `options_from_environment`). The limiter itself only counts.

use std::collections::HashMap;

/// The routes that take a password: they share one small bucket per address, so guessing is slow.
pub const AUTH_RATE_LIMIT_PATHS: [&str; 4] =
    ["/api/account/register", "/api/account/login", "/api/account/password", "/api/account/delete"];

/// True for a route that counts against the sign-in / sign-up bucket.
pub fn is_auth_path(path: &str) -> bool {
    AUTH_RATE_LIMIT_PATHS.contains(&path)
}

/// The text a 429 carries (player-facing, upper case). `auth` picks the sign-in wording.
pub fn limited_message(auth: bool, retry_after_seconds: i64) -> String {
    let seconds = retry_after_seconds.max(1);
    let unit = if seconds == 1 { "SECOND" } else { "SECONDS" };
    if auth {
        format!("TOO MANY SIGN-IN ATTEMPTS. TRY AGAIN IN {seconds} {unit}.")
    } else {
        format!("TOO MANY REQUESTS. TRY AGAIN IN {seconds} {unit}.")
    }
}

/// Fixed-window counters keyed by text (`bucket|address`).
#[derive(Default)]
pub struct RateLimiter {
    /// key -> (hits in the window, the time the window ends)
    buckets: HashMap<String, (u32, i64)>,
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter::default()
    }

    /// Counts one hit for `key`. Returns 0 when it is allowed, else the whole seconds until the window resets (at least 1).
    pub fn hit(&mut self, key: &str, maximum: u32, window_ms: i64, now: i64) -> i64 {
        let maximum = maximum.max(1);
        let window = window_ms.max(1000);
        let rate = self.buckets.entry(key.to_string()).or_insert((0, now + window));
        if now >= rate.1 {
            *rate = (0, now + window);
        }
        rate.0 += 1;
        if rate.0 <= maximum {
            0
        } else {
            (((rate.1 - now) as f64) / 1000.0).ceil().max(1.0) as i64
        }
    }

    /// Forgets `key` (a bucket that should start clean).
    #[allow(dead_code)]
    pub fn reset(&mut self, key: &str) {
        self.buckets.remove(key);
    }

    /// Drops the buckets whose window ended.
    pub fn prune(&mut self, now: i64) {
        self.buckets.retain(|_, (_, reset)| now < *reset);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}
