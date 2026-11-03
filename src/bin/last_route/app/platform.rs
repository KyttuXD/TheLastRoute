//! Small per-OS helpers: the console for the command-line modes (the game itself is a window
//! program on Windows, so no empty terminal opens), and the screen size the window is fitted to.

/// Command-line modes print to the terminal they were started from. The Windows build has no
/// console of its own; attach to the parent's when stdout isn't already a pipe or file.
pub fn attach_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(pid: u32) -> i32;
            fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        }
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        unsafe {
            if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}

/// The primary screen's size in the window system's (logical) pixels, when known.
pub fn screen_size() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetSystemMetrics(index: i32) -> i32;
        }
        // SM_CXFULLSCREEN / SM_CYFULLSCREEN: the work area a maximised window gets
        let (w, h) = unsafe { (GetSystemMetrics(16), GetSystemMetrics(17)) };
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    None
}

/// The starting window size: 16:9, at most 1280x720 and at most 80% of the screen, so it
/// never opens larger than the display (also on scaled 125-150% displays). The screenshot
/// and self-test modes always use 1280x720.
pub fn window_size(fixed: bool) -> (i32, i32) {
    const W: i32 = 1280;
    const H: i32 = 720;
    if fixed {
        return (W, H);
    }
    match screen_size() {
        Some((sw, sh)) => {
            let fit = (sw as f32 * 0.8 / W as f32)
                .min(sh as f32 * 0.8 / H as f32)
                .min(1.0);
            (
                (W as f32 * fit).round().max(640.0) as i32,
                (H as f32 * fit).round().max(360.0) as i32,
            )
        }
        None => (W, H),
    }
}
