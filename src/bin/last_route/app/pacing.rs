//! Frame pacing for the VSYNC and FPS LIMIT rows.
//!
//! VSYNC: miniquad (0.4.11) creates the GL context with swap interval 1 unless the window config
//! sets `platform.swap_interval`, and it never changes it afterwards. `main.rs` sets the creation
//! value from the saved setting, and `apply_vsync` changes the live context when the row is
//! toggled (on Windows, through the WGL_EXT_swap_control entry point the context was made with; on
//! macOS, through NSOpenGLContext's swap interval parameter). On macOS miniquad forces the creation
//! value to 0 whatever the config says, so the first frame's `apply_vsync` is what turns vsync on.
//!
//! FPS LIMIT: `Pacer::wait` sleeps the loop to a fixed frame grid before the next frame. With no
//! cap it returns at once, so the loop runs as fast as vsync (if on) allows.

use std::time::{Duration, Instant};

/// The GL swap interval for the VSYNC row: 1 waits for the display's refresh, 0 does not.
pub fn swap_interval_for(on: bool) -> i32 {
    if on {
        1
    } else {
        0
    }
}

/// Applies the VSYNC row to the live GL context. Returns whether the change took effect. Windows
/// and macOS change the swap interval at run time; elsewhere the creation-time value stays until
/// the next launch, and the FPS LIMIT still paces the loop.
pub fn apply_vsync(on: bool) -> bool {
    let interval = swap_interval_for(on);
    let state = if on { "on" } else { "off" };
    let applied = set_swap_interval(interval);
    if applied {
        crate::lr_info!("vsync {state} (swap interval {interval})");
    } else {
        crate::lr_warn!("vsync {state}: the live swap interval could not be changed");
    }
    applied
}

#[cfg(windows)]
fn set_swap_interval(interval: i32) -> bool {
    use core::ffi::c_void;
    type SwapIntervalFn = unsafe extern "system" fn(i32) -> i32;
    #[link(name = "opengl32")]
    extern "system" {
        // Needs the context of this thread to be current, which it is: miniquad runs the window
        // loop on the thread that created the context.
        fn wglGetProcAddress(name: *const u8) -> *const c_void;
    }
    // wglGetProcAddress reports a missing extension as NULL or one of 1, 2, 3, -1
    let proc = unsafe { wglGetProcAddress(b"wglSwapIntervalEXT\0".as_ptr()) };
    if (proc as isize) <= 3 {
        return false;
    }
    let swap_interval: SwapIntervalFn = unsafe { std::mem::transmute(proc) };
    unsafe { swap_interval(interval) != 0 }
}

/// NSOpenGLCPSwapInterval (NSOpenGLContextParameter 222): the swap interval of a macOS GL context.
#[cfg(target_os = "macos")]
const NS_OPENGL_CP_SWAP_INTERVAL: std::ffi::c_long = 222;

/// Sets the swap interval of the current NSOpenGLContext through the Objective-C runtime:
/// `[NSOpenGLContext currentContext] setValues:&interval forParameter:NSOpenGLCPSwapInterval`.
/// miniquad's window loop runs on the thread that made the context, so it is the current one.
/// `objc_msgSend` is called through a pointer typed to each call's signature, as the runtime
/// requires on arm64 and x86_64 alike. libobjc is always present on macOS.
#[cfg(target_os = "macos")]
fn set_swap_interval(interval: i32) -> bool {
    use core::ffi::{c_char, c_int, c_void};
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }
    type ClassMsg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type SetValuesMsg = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_int, std::ffi::c_long);
    unsafe {
        let class = objc_getClass(b"NSOpenGLContext\0".as_ptr() as *const c_char);
        if class.is_null() {
            return false;
        }
        let msg = objc_msgSend as *const ();
        let current: ClassMsg = std::mem::transmute(msg);
        let context = current(
            class,
            sel_registerName(b"currentContext\0".as_ptr() as *const c_char),
        );
        if context.is_null() {
            return false;
        }
        let set_values: SetValuesMsg = std::mem::transmute(msg);
        let value: c_int = interval;
        set_values(
            context,
            sel_registerName(b"setValues:forParameter:\0".as_ptr() as *const c_char),
            &value,
            NS_OPENGL_CP_SWAP_INTERVAL,
        );
    }
    true
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_swap_interval(_interval: i32) -> bool {
    false
}

/// The instant the next frame may start, given the previous frame's grid point. The grid is kept
/// while the loop is at most one period behind it; a bigger hitch restarts it at `now` so the
/// loop does not burst to catch up.
pub fn grid_due(prev: Option<Instant>, now: Instant, period: Duration) -> Instant {
    match prev {
        Some(t) if now.saturating_duration_since(t) < period => t,
        _ => now,
    }
}

/// Holds the loop to the FPS LIMIT. Call `wait` once per frame, before the frame is presented.
#[derive(Default)]
pub struct Pacer {
    grid: Option<Instant>,
}

impl Pacer {
    pub fn wait(&mut self, cap: Option<u32>) {
        let Some(fps) = cap else {
            self.grid = None;
            return;
        };
        let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        let due = grid_due(self.grid, Instant::now(), period);
        sleep_until(due);
        self.grid = Some(due + period);
    }
}

/// Sleeps to `due`. OS sleeps overshoot by up to a timer tick (about 15 ms by default on
/// Windows), so the loop sleeps to 2 ms short and spins the rest.
fn sleep_until(due: Instant) {
    const SLACK: Duration = Duration::from_millis(2);
    loop {
        let now = Instant::now();
        if now >= due {
            return;
        }
        let left = due.duration_since(now);
        if left > SLACK {
            std::thread::sleep(left - SLACK);
        } else {
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsync_row_maps_to_swap_interval_one_or_zero() {
        assert_eq!(swap_interval_for(true), 1);
        assert_eq!(swap_interval_for(false), 0);
    }

    #[test]
    fn grid_starts_now_on_the_first_frame_and_after_a_big_hitch() {
        let now = Instant::now();
        let period = Duration::from_millis(10);
        assert_eq!(grid_due(None, now, period), now);
        // 25 ms behind a 10 ms grid: restart at now instead of bursting
        let stale = now - Duration::from_millis(25);
        assert_eq!(grid_due(Some(stale), now, period), now);
    }

    #[test]
    fn grid_is_kept_while_on_schedule_or_slightly_late() {
        let now = Instant::now();
        let period = Duration::from_millis(10);
        // the next grid point is still ahead
        let ahead = now + Duration::from_millis(4);
        assert_eq!(grid_due(Some(ahead), now, period), ahead);
        // a few ms late but within one period: keep the grid
        let late = now - Duration::from_millis(3);
        assert_eq!(grid_due(Some(late), now, period), late);
    }

    #[test]
    fn uncapped_pacer_does_not_sleep() {
        let mut p = Pacer::default();
        let t0 = Instant::now();
        for _ in 0..1000 {
            p.wait(None);
        }
        assert!(t0.elapsed() < Duration::from_millis(100));
        assert!(p.grid.is_none());
    }

    #[test]
    fn capped_pacer_holds_the_frame_rate() {
        let mut p = Pacer::default();
        let t0 = Instant::now();
        // 30 frames at a 60 fps cap take about half a second (29 gaps of 16.7 ms)
        for _ in 0..30 {
            p.wait(Some(60));
        }
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(480), "took {took:?}");
        assert!(took < Duration::from_millis(900), "took {took:?}");
    }
}
