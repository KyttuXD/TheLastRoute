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

/// True when this run draws through Metal (macOS with the Metal window; see `app::gfx`). Reads what miniquad
/// opened, once (the API cannot change after the window exists), so it is right once the window exists and costs
/// an atomic load afterwards. Call it only after the window is open: miniquad's accessor locks the native display.
pub fn is_metal() -> bool {
    #[cfg(target_os = "macos")]
    {
        static METAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *METAL.get_or_init(|| {
            macroquad::miniquad::window::apple_gfx_api() == macroquad::miniquad::conf::AppleGfxApi::Metal
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The design size of the game window: 1280x720.
const DESIGN_SIZE: (i32, i32) = (1280, 720);

/// The work area of the primary screen (the screen less the taskbar and other docked bars) as
/// (left, top, width, height).
#[cfg(windows)]
fn work_area() -> Option<(i32, i32, i32, i32)> {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut core::ffi::c_void, win_ini: u32) -> i32;
    }
    // SPI_GETWORKAREA
    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut area = Rect { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut Rect as *mut _, 0) };
    (ok != 0).then(|| (area.left, area.top, area.right - area.left, area.bottom - area.top))
}

/// The non-client size of a window: both frame borders and the caption, as (width, height).
#[cfg(windows)]
fn frame_size() -> (i32, i32) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    // SM_CXSIZEFRAME 32, SM_CYSIZEFRAME 33, SM_CYCAPTION 4, SM_CXPADDEDBORDER 92
    unsafe {
        (
            2 * (GetSystemMetrics(32) + GetSystemMetrics(92)),
            2 * (GetSystemMetrics(33) + GetSystemMetrics(92)) + GetSystemMetrics(4),
        )
    }
}

/// The client area a window can take on the primary screen: the work area less the frame and caption, so a
/// window of this client size stays on the screen.
pub fn client_area() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        let (_, _, w, h) = work_area()?;
        let (fw, fh) = frame_size();
        Some((w - fw, h - fh))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Moves the window to the middle of the work area. miniquad creates the window at the system's default
/// position, which can leave its bottom edge under the taskbar. A no-op where the work area is unknown.
pub fn centre_on_work_area() {
    #[cfg(windows)]
    {
        let Some((left, top, w, h)) = work_area() else {
            return;
        };
        let (fw, fh) = frame_size();
        let (cw, ch) = macroquad::miniquad::window::screen_size();
        let (total_w, total_h) = (cw as i32 + fw, ch as i32 + fh);
        let x = left + ((w - total_w) / 2).max(0);
        let y = top + ((h - total_h) / 2).max(0);
        macroquad::miniquad::window::set_window_position(x as u32, y as u32);
    }
}

/// The starting window size: the design size, scaled down to fit the client area when the screen is smaller
/// (the taskbar is not covered). The screenshot and self-test modes always use the design size.
pub fn window_size(fixed: bool) -> (i32, i32) {
    let (dw, dh) = DESIGN_SIZE;
    if fixed {
        return DESIGN_SIZE;
    }
    match client_area() {
        Some((w, h)) if w > 0 && h > 0 => {
            let scale = (w as f32 / dw as f32).min(h as f32 / dh as f32).min(1.0);
            ((dw as f32 * scale) as i32, (dh as f32 * scale) as i32)
        }
        _ => DESIGN_SIZE,
    }
}
