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

/// The stored text under `key`, or None when nothing is stored or the browser will not say.
pub fn get(key: &str) -> Option<String> {
    let mut cap = FIRST_CAP;
    for _ in 0..2 {
        let mut buf = vec![0u8; cap];
        // SAFETY: the buffer is `cap` bytes long and the key is a live slice for the whole call; the page copies at most `cap` bytes into it.
        let n = unsafe { lr_storage_get(key.as_ptr(), key.len() as u32, buf.as_mut_ptr(), cap as u32) };
        if n < 0 {
            return None;
        }
        let n = n as usize;
        if n <= cap {
            buf.truncate(n);
            return String::from_utf8(buf).ok();
        }
        // the value is longer than the buffer: read it again at its size
        cap = n;
    }
    None
}

/// Stores `value` under `key`. False when the browser refused it (full, blocked or missing storage).
pub fn set(key: &str, value: &str) -> bool {
    // SAFETY: both slices are live for the whole call and their lengths are passed with them.
    unsafe { lr_storage_set(key.as_ptr(), key.len() as u32, value.as_ptr(), value.len() as u32) == 1 }
}
