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

/// Most keys the limiter remembers at once. A flood from many addresses stops growing the map here: a new key is then
/// refused until old windows end (the keys already counted keep working).
pub const MAX_RATE_BUCKETS: usize = 200_000;

/// The text an address is counted under. An IPv4 address counts as itself. An IPv6 address counts as its /64 network,
/// because one subscriber normally holds a whole /64 and could otherwise use a new address for every request.
/// Anything that is not an IP address is returned unchanged.
pub fn rate_address(address: &str) -> String {
    match address.trim().parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.to_string(),
        Ok(std::net::IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        Err(_) => address.to_string(),
    }
}

/// Fixed-window counters keyed by text (`bucket|address`).
pub struct RateLimiter {
    /// key -> (hits in the window, the time the window ends)
    buckets: HashMap<String, (u32, i64)>,
    /// Most keys kept (`MAX_RATE_BUCKETS`).
    capacity: usize,
    /// When a full map was last swept for ended windows (at most one sweep a second).
    swept_at: i64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter { buckets: HashMap::new(), capacity: MAX_RATE_BUCKETS, swept_at: i64::MIN }
    }
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter::default()
    }

    /// A limiter that keeps at most `capacity` keys (tests).
    #[cfg(test)]
    pub fn with_capacity(capacity: usize) -> RateLimiter {
        RateLimiter { capacity: capacity.max(1), ..RateLimiter::default() }
    }

    /// Counts one hit for `key`. Returns 0 when it is allowed, else the whole seconds until the window resets (at least 1).
    /// A new key that does not fit in a full map is refused for one second.
    pub fn hit(&mut self, key: &str, maximum: u32, window_ms: i64, now: i64) -> i64 {
        let maximum = maximum.max(1);
        let window = window_ms.max(1000);
        if self.buckets.len() >= self.capacity && !self.buckets.contains_key(key) {
            if now.saturating_sub(self.swept_at) >= 1000 {
                self.swept_at = now;
                self.prune(now);
            }
            if self.buckets.len() >= self.capacity {
                return 1;
            }
        }
        let rate = self.buckets.entry(key.to_string()).or_insert((0, now + window));
        if now >= rate.1 {
            *rate = (0, now + window);
        }
        rate.0 = rate.0.saturating_add(1);
        if rate.0 <= maximum {
            0
        } else {
            (((rate.1 - now) as f64) / 1000.0).ceil().max(1.0) as i64
        }
    }

    /// Whether `key` already used up `maximum` hits in its current window, without counting anything: 0 when it did
    /// not, else the whole seconds until the window resets (at least 1).
    pub fn blocked(&self, key: &str, maximum: u32, now: i64) -> i64 {
        match self.buckets.get(key) {
            Some((hits, reset)) if now < *reset && *hits >= maximum.max(1) => (((*reset - now) as f64) / 1000.0).ceil().max(1.0) as i64,
            _ => 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipv6_address_is_counted_by_its_64_bit_network_and_an_ipv4_address_by_itself() {
        assert_eq!(rate_address("203.0.113.9"), "203.0.113.9");
        assert_eq!(rate_address("::ffff:203.0.113.9"), "203.0.113.9");
        assert_eq!(rate_address("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), "2001:db8:1:2::/64");
        assert_eq!(rate_address("2001:db8:1:2::1"), rate_address("2001:db8:1:2:ffff:ffff:ffff:ffff"));
        assert_ne!(rate_address("2001:db8:1:2::1"), rate_address("2001:db8:1:3::1"));
        assert_eq!(rate_address("unknown"), "unknown");
    }

    #[test]
    fn a_full_limiter_refuses_new_keys_keeps_counting_known_ones_and_recovers_when_windows_end() {
        let mut limiter = RateLimiter::with_capacity(3);
        for key in ["a", "b", "c"] {
            assert_eq!(limiter.hit(key, 5, 60_000, 1_000), 0);
        }
        assert_eq!(limiter.hit("d", 5, 60_000, 1_000), 1, "a fourth key does not fit");
        assert_eq!(limiter.len(), 3, "the map did not grow");
        assert_eq!(limiter.hit("a", 5, 60_000, 1_000), 0, "a known key is still counted");
        // the windows end: the sweep makes room again
        assert_eq!(limiter.hit("d", 5, 60_000, 62_000), 0);
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn blocked_reports_a_used_up_window_without_counting() {
        let mut limiter = RateLimiter::new();
        assert_eq!(limiter.blocked("k", 2, 1_000), 0, "an unknown key is not blocked");
        limiter.hit("k", 2, 60_000, 1_000);
        assert_eq!(limiter.blocked("k", 2, 1_000), 0);
        limiter.hit("k", 2, 60_000, 1_000);
        let wait = limiter.blocked("k", 2, 1_000);
        assert!((1..=60).contains(&wait), "{wait}");
        assert_eq!(limiter.blocked("k", 2, 1_000), wait, "asking does not count");
        assert_eq!(limiter.blocked("k", 2, 61_000), 0, "the window ended");
    }
}
