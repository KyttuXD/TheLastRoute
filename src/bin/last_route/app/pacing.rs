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

/// Where the pacing rules live under data/ (the embedded file's name).
pub const PACING_RULES_PATH: &str = "app/pacing.json";

/// The pacing rules (data/app/pacing.json).
#[derive(Clone, Debug, PartialEq)]
pub struct PacingRules {
    /// OS sleeps overshoot by up to a timer tick (about 15 ms by default on Windows), so the loop sleeps to this
    /// many milliseconds short of the due time and spins the rest.
    pub spin_slack_ms: f64,
}

impl Default for PacingRules {
    fn default() -> PacingRules {
        PacingRules { spin_slack_ms: 2.0 }
    }
}

impl PacingRules {
    /// Parses `{ "spin_slack_ms" }`; the slack must be a number of at least 0.
    pub fn parse(text: &str) -> Result<PacingRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let slack = v
            .get("spin_slack_ms")
            .and_then(|x| x.as_f64())
            .filter(|x| *x >= 0.0 && x.is_finite())
            .ok_or_else(|| "spin_slack_ms: must be a number of at least 0".to_string())?;
        Ok(PacingRules { spin_slack_ms: slack })
    }

    fn slack(&self) -> Duration {
        Duration::from_secs_f64(self.spin_slack_ms / 1000.0)
    }
}

/// The rules compiled into the executable; a table that fails to parse is reported on stderr and Standard is used.
pub fn pacing_rules() -> &'static PacingRules {
    static RULES: std::sync::OnceLock<PacingRules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(PACING_RULES_PATH)
            .next()
            .map(|(_, t)| t)
            .unwrap_or("{}");
        PacingRules::parse(text).unwrap_or_else(|e| {
            eprintln!("{PACING_RULES_PATH}: {e}");
            PacingRules::default()
        })
    })
}

/// Holds the loop to the FPS LIMIT. Call `wait` once per frame, before the frame is presented.
#[derive(Default)]
pub struct Pacer {
    grid: Option<Instant>,
}

impl Pacer {
    pub fn wait(&mut self, cap: Option<u32>) {
        self.wait_with(cap, pacing_rules(), &mut Instant::now, &mut nap);
    }

    /// `wait` with the time source and the sleep injected: `now` reads the clock, `nap(d)` waits about `d`
    /// (a zero `d` only yields). The tests drive a fake clock through it.
    pub fn wait_with(
        &mut self,
        cap: Option<u32>,
        rules: &PacingRules,
        now: &mut dyn FnMut() -> Instant,
        nap: &mut dyn FnMut(Duration),
    ) {
        let Some(fps) = cap else {
            self.grid = None;
            return;
        };
        let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        let due = grid_due(self.grid, now(), period);
        sleep_until(due, rules.slack(), now, nap);
        self.grid = Some(due + period);
    }
}

/// The real sleep: a zero wait yields the thread, anything else sleeps it.
fn nap(d: Duration) {
    if d.is_zero() {
        std::thread::yield_now();
    } else {
        std::thread::sleep(d);
    }
}

/// Waits to `due`: sleeps while more than `slack` is left (to `slack` short of it), spins the rest.
fn sleep_until(due: Instant, slack: Duration, now: &mut dyn FnMut() -> Instant, nap: &mut dyn FnMut(Duration)) {
    loop {
        let t = now();
        if t >= due {
            return;
        }
        let left = due.duration_since(t);
        if left > slack {
            nap(left - slack);
        } else {
            nap(Duration::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A fake clock: reading it returns the base plus the elapsed fake time; a nap advances it (a yield costs 50 us).
    struct Fake {
        base: Instant,
        elapsed: Cell<Duration>,
        naps: RefCell<Vec<Duration>>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { base: Instant::now(), elapsed: Cell::new(Duration::ZERO), naps: RefCell::new(Vec::new()) }
        }
        fn now(&self) -> Instant {
            self.base + self.elapsed.get()
        }
        fn nap(&self, d: Duration) {
            self.naps.borrow_mut().push(d);
            self.elapsed.set(self.elapsed.get() + if d.is_zero() { Duration::from_micros(50) } else { d });
        }
        fn wait(&self, p: &mut Pacer, cap: Option<u32>, rules: &PacingRules) {
            p.wait_with(cap, rules, &mut || self.now(), &mut |d| self.nap(d));
        }
    }

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
    fn pacing_data_matches_standard() {
        let r = PacingRules::parse(include_str!("../data/app/pacing.json")).expect("pacing.json parses");
        assert_eq!(r, PacingRules::default());
        assert_eq!(pacing_rules(), &r);
        assert!(PacingRules::parse("{}").is_err());
        assert!(PacingRules::parse(r#"{ "spin_slack_ms": -1 }"#).is_err());
    }

    #[test]
    fn uncapped_pacer_does_not_sleep() {
        let clock = Fake::new();
        let mut p = Pacer::default();
        for _ in 0..1000 {
            clock.wait(&mut p, None, &PacingRules::default());
        }
        assert!(clock.naps.borrow().is_empty());
        assert_eq!(clock.elapsed.get(), Duration::ZERO);
        assert!(p.grid.is_none());
    }

    #[test]
    fn capped_pacer_holds_the_frame_rate() {
        let clock = Fake::new();
        let mut p = Pacer::default();
        // 30 frames at a 60 fps cap take 29 gaps of 16.7 ms on the grid, give or take a spin step
        for _ in 0..30 {
            clock.wait(&mut p, Some(60), &PacingRules::default());
        }
        let took = clock.elapsed.get();
        let want = Duration::from_secs_f64(29.0 / 60.0);
        assert!(took >= want && took < want + Duration::from_millis(5), "took {took:?}");
    }

    #[test]
    fn spin_slack_in_data_changes_how_far_short_the_sleep_stops() {
        let first_sleep = |slack_ms: f64| {
            let clock = Fake::new();
            let mut p = Pacer::default();
            let rules = PacingRules { spin_slack_ms: slack_ms };
            clock.wait(&mut p, Some(100), &rules); // first frame: due now
            clock.wait(&mut p, Some(100), &rules); // second: a 10 ms gap
            let naps = clock.naps.borrow();
            *naps.iter().find(|d| !d.is_zero()).expect("a real sleep")
        };
        let tight = first_sleep(2.0);
        let loose = first_sleep(5.0);
        assert!((tight.as_secs_f64() - 0.008).abs() < 1e-3, "{tight:?}");
        assert!((loose.as_secs_f64() - 0.005).abs() < 1e-3, "{loose:?}");
        assert!(loose < tight);
    }
}
