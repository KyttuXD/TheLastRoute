//! The system clipboard for the menus' text fields, native and browser.
//!
//! * Native (Windows, macOS, Linux): miniquad's clipboard, read when the player presses the paste shortcut and written when
//!   they copy or cut.
//! * Web (wasm32): the stock miniquad page script cannot do this alone. Its `paste` listener keeps the last pasted text for
//!   ever (so a Ctrl+V the browser never answers pastes old text again), its `copy` listener answers the browser's copy
//!   event with whatever the game stored a frame earlier (the event fires before the game's next frame has run), and
//!   `clipboard_set` from a button click never reaches the real clipboard at all. `packaging/web/index.html` therefore
//!   registers the `lr_clip` plugin:
//!   - `lr_paste_take` hands over the text of the last `paste` event once (and only while it is fresh), also when the page
//!     had to ask `navigator.clipboard.readText()` because no paste event came;
//!   - `lr_clip_offer` tells the page what Ctrl/Cmd+C or +X would copy *right now* (the focused field's text, or nothing for
//!     a password), so the browser's own copy/cut event can be answered inside the event, which every browser allows
//!     without a permission prompt;
//!   - `lr_clip_write` puts text on the clipboard from the game (the COPY CODE button, a copy shortcut), with a fallback
//!     for browsers that only allow it inside a user gesture.
//!
//! Nothing here throws or panics into the game: a clipboard that cannot be reached answers `None` / does nothing.

/// Puts `text` on the system clipboard.
pub fn copy(text: &str) {
    imp::copy(text);
}

/// The text the player pasted, once. `key_pressed` is whether the paste shortcut was pressed this frame: native builds read
/// the clipboard only then; the web build ignores it and takes whatever the page's paste event delivered since the last
/// call (the browser answers the shortcut a moment after the key, and sometimes a frame later).
pub fn poll_paste(key_pressed: bool) -> Option<String> {
    imp::poll_paste(key_pressed)
}

/// Tells the web page what a copy or cut shortcut would copy this frame (empty: copy nothing, as a password field must).
/// Call it every frame a text field is focused; the page forgets the offer after a moment, so an unfocused field offers
/// nothing. Native builds read the field themselves and ignore it.
pub fn offer(text: &str) {
    imp::offer(text);
}

#[cfg(target_arch = "wasm32")]
mod imp {
    extern "C" {
        /// Copies the pending pasted text into `dst` (at most `cap` bytes) and returns its byte length, -1 when nothing is
        /// pending. A length above `cap` means nothing was copied and the text is still pending.
        fn lr_paste_take(dst: *mut u8, cap: u32) -> i32;
        fn lr_clip_offer(ptr: *const u8, len: u32);
        fn lr_clip_write(ptr: *const u8, len: u32);
    }

    /// The first read buffer; a longer paste is read again at its own size.
    const FIRST_CAP: usize = 4096;

    pub fn copy(text: &str) {
        // SAFETY: the slice is live for the whole call and its length is passed with it.
        unsafe { lr_clip_write(text.as_ptr(), text.len() as u32) }
    }

    pub fn poll_paste(_key_pressed: bool) -> Option<String> {
        let mut cap = FIRST_CAP;
        for _ in 0..2 {
            let mut buf = vec![0u8; cap];
            // SAFETY: the buffer is `cap` bytes long; the page copies at most `cap` bytes into it.
            let n = unsafe { lr_paste_take(buf.as_mut_ptr(), cap as u32) };
            if n < 0 {
                return None;
            }
            let n = n as usize;
            if n <= cap {
                buf.truncate(n);
                return String::from_utf8(buf).ok();
            }
            cap = n;
        }
        None
    }

    pub fn offer(text: &str) {
        // SAFETY: the slice is live for the whole call and its length is passed with it.
        unsafe { lr_clip_offer(text.as_ptr(), text.len() as u32) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub fn copy(text: &str) {
        macroquad::miniquad::window::clipboard_set(text);
    }

    pub fn poll_paste(key_pressed: bool) -> Option<String> {
        if key_pressed {
            macroquad::miniquad::window::clipboard_get()
        } else {
            None
        }
    }

    pub fn offer(_text: &str) {}
}
