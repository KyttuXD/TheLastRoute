//! The browser build's key/value storage: the page's localStorage, reached through `lr_storage_get` and
//! `lr_storage_set`, which dist/web/index.html registers as miniquad plugin imports. Settings are the one thing kept
//! between page loads; saves are not (`paths::WRITES_TO_DISK`). Storage can be missing or blocked (a private window,
//! cleared site data): then nothing is stored and the game runs on its defaults.
//!
//! Native builds have no page and no use for this module: `app/settings.rs` keeps its settings file there.

/// The localStorage key of the settings JSON (the same text the native settings file holds).
pub const SETTINGS_KEY: &str = "last_route.settings";

extern "C" {
    /// Copies the value stored under `key` into `dst` (at most `cap` bytes) and returns the value's byte length.
    /// -1 when nothing is stored or storage is blocked. A length above `cap` means nothing was copied.
    fn lr_storage_get(key: *const u8, key_len: u32, dst: *mut u8, cap: u32) -> i32;
    /// Stores `value` under `key`. Returns 1 when stored, 0 when the browser refused.
    fn lr_storage_set(key: *const u8, key_len: u32, value: *const u8, value_len: u32) -> i32;
}

/// The first read buffer; a larger value is read again at its own size.
const FIRST_CAP: usize = 64 * 1024;

/// How many times a read is retried when the value keeps growing between reads.
const MAX_READS: usize = 4;

/// Reads through `read(buf) -> length` (negative: nothing; above the buffer: nothing copied), growing the buffer to the
/// reported length and trying again, up to `MAX_READS` times.
fn read_growing(mut read: impl FnMut(&mut [u8]) -> i32) -> Option<Vec<u8>> {
    let mut cap = FIRST_CAP;
    for _ in 0..MAX_READS {
        let mut buf = vec![0u8; cap];
        let n = read(&mut buf);
        if n < 0 {
            return None;
        }
        let n = n as usize;
        if n <= cap {
            buf.truncate(n);
            return Some(buf);
        }
        // the value is longer than the buffer: read it again at its size
        cap = n;
    }
    None
}

/// The stored text under `key`, or None when nothing is stored or the browser will not say.
pub fn get(key: &str) -> Option<String> {
    // SAFETY: the buffer is `buf.len()` bytes long and the key is a live slice for the whole call; the page copies at most that many bytes into it.
    let bytes = read_growing(|buf| unsafe { lr_storage_get(key.as_ptr(), key.len() as u32, buf.as_mut_ptr(), buf.len() as u32) })?;
    String::from_utf8(bytes).ok()
}

/// Stores `value` under `key`. False when the browser refused it (full, blocked or missing storage).
pub fn set(key: &str, value: &str) -> bool {
    // SAFETY: both slices are live for the whole call and their lengths are passed with them.
    unsafe { lr_storage_set(key.as_ptr(), key.len() as u32, value.as_ptr(), value.len() as u32) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_that_grows_between_reads_is_still_read() {
        let mut calls: usize = 0;
        let got = read_growing(|buf| {
            calls += 1;
            let len = FIRST_CAP + calls.min(2) * 10;
            if len <= buf.len() {
                buf[..len].fill(b'a');
            }
            len as i32
        });
        assert_eq!(got.map(|v| v.len()), Some(FIRST_CAP + 20));
    }

    #[test]
    fn a_value_that_never_settles_gives_up_and_a_missing_one_is_none() {
        let mut calls = 0;
        assert!(read_growing(|_| {
            calls += 1;
            (FIRST_CAP * (calls + 1)) as i32
        })
        .is_none());
        assert_eq!(calls, MAX_READS);
        assert!(read_growing(|_| -1).is_none());
    }
}
