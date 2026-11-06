//! Background work and waiting, for the game on native and in the browser.
//!
//! The browser build (wasm32-unknown-unknown) has one thread and no way to block it: `std::thread::spawn` and
//! `std::thread::sleep` panic there. [`detach`] runs the work at once on the page's thread, and [`pause`] does not
//! wait. Native builds keep the real thread and the real sleep.

use std::time::Duration;

/// Runs `work` on its own thread (native). In the browser it runs at once, on the calling thread, so it must
/// finish quickly (a request that the browser build refuses does).
#[cfg(not(target_arch = "wasm32"))]
pub fn detach<F: FnOnce() + Send + 'static>(work: F) {
    drop(std::thread::spawn(work));
}

#[cfg(target_arch = "wasm32")]
pub fn detach<F: FnOnce() + Send + 'static>(work: F) {
    work();
}

/// Sleeps for `d` (native). The browser cannot block the page, so it returns at once; a loop that waits on
/// another thread must not be used there (networking is off in the browser build, see net/http.rs).
#[cfg(not(target_arch = "wasm32"))]
pub fn pause(d: Duration) {
    std::thread::sleep(d);
}

#[cfg(target_arch = "wasm32")]
pub fn pause(_d: Duration) {}

/// True in the browser build (wasm32-unknown-unknown): no threads, no sockets, no filesystem, no blocking.
pub const IN_BROWSER: bool = cfg!(target_arch = "wasm32");

/// This process's id (native). The browser has no process ids: 0, which only names temporary files and crash markers
/// (both are off in the browser, see app/gfx.rs).
#[cfg(not(target_arch = "wasm32"))]
pub fn process_id() -> u32 {
    std::process::id()
}

#[cfg(target_arch = "wasm32")]
pub fn process_id() -> u32 {
    0
}
