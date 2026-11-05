//! Which graphics API the window opens with: Metal or OpenGL (macOS only; every other system is OpenGL).
//!
//! miniquad reads the choice once, when the window is created (`platform.apple_gfx_api`), so it is made in
//! `main.rs` `window_conf` from `decide`. Precedence, first match wins:
//!
//! 1. not macOS, or no Metal device (`MTLCreateSystemDefaultDevice` is null): OpenGL;
//! 2. `--gfx auto|metal|gl`, then env `LR_GFX`, then the saved GRAPHICS API setting (AUTO | METAL | OPENGL);
//! 3. an explicit `--gfx metal` / `LR_GFX=metal` is always honoured (it is the way to retry after a crash);
//! 4. otherwise the screenshot modes (`--screenshot`, `--self-test`, `--agent-window`) use OpenGL, because
//!    miniquad's Metal backend and macroquad cannot read the screen back (`get_screen_data`);
//! 5. otherwise a crash marker (see below) forces OpenGL;
//! 6. AUTO is OpenGL everywhere, because Metal has not been measured faster than OpenGL on any Mac yet
//!    (owner decision; `auto_metal_on_apple_silicon` in data/app/gfx.json is the switch to flip when it is).
//!    METAL in the setting, `--gfx metal` and `LR_GFX=metal` still give Metal.
//!
//! The crash guard: a Metal start writes a marker file holding the process id before the window opens. A clean
//! end removes it: `boot_guard_tick` after the data file's `good_frames` drawn frames, or an `atexit` hook when
//! the process exits normally earlier (Cmd-Q on the boot screen, a double launch, `--self-test`), unless it
//! panicked (a panic hook records that). A panic that aborts skips `atexit` altogether, so the marker survives. At the
//! next launch a marker counts as a crash only when its process is gone: a second instance started next to a
//! running one ignores the first's marker. A crash drops AUTO and METAL-in-settings to OpenGL and the
//! settings row says so; `--gfx metal` retries, and a good run clears the marker.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

/// Where the rules live under data/ (the embedded file's name).
pub const GFX_RULES_PATH: &str = "app/gfx.json";

/// The API the window draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gfx {
    Gl,
    Metal,
}

/// What the player (or a flag) asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Metal,
    OpenGl,
}

impl Choice {
    /// Reads a setting value, a `--gfx` value or an `LR_GFX` value, in any case: auto, metal, gl or opengl.
    pub fn parse(text: &str) -> Option<Choice> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Choice::Auto),
            "metal" => Some(Choice::Metal),
            "gl" | "opengl" => Some(Choice::OpenGl),
            _ => None,
        }
    }

    /// The saved setting text.
    pub fn id(self) -> &'static str {
        match self {
            Choice::Auto => "AUTO",
            Choice::Metal => "METAL",
            Choice::OpenGl => "OPENGL",
        }
    }

    /// The next value of the GRAPHICS API row.
    pub fn next(self) -> Choice {
        match self {
            Choice::Auto => Choice::Metal,
            Choice::Metal => Choice::OpenGl,
            Choice::OpenGl => Choice::Auto,
        }
    }
}

/// Everything `decide` looks at, so the tests can set each one.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub flag: Option<String>,
    pub env: Option<String>,
    pub setting: String,
    pub macos: bool,
    pub apple_silicon: bool,
    pub metal_device: bool,
    /// A marker from a Metal start whose process died without a clean end (see `marker_is_crash`).
    pub marker: bool,
    pub readback_mode: bool,
    /// AUTO picks Metal on Apple Silicon (data/app/gfx.json `auto_metal_on_apple_silicon`).
    pub auto_metal: bool,
}

/// The decision and the reason, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub gfx: Gfx,
    pub why: &'static str,
    /// True when a crash marker forced OpenGL (the settings row says so).
    pub marker_forced: bool,
}

fn gl(why: &'static str) -> Decision {
    Decision { gfx: Gfx::Gl, why, marker_forced: false }
}

fn metal(why: &'static str) -> Decision {
    Decision { gfx: Gfx::Metal, why, marker_forced: false }
}

/// The pure choice. See the module docs for the order.
pub fn decide(i: &Inputs) -> Decision {
    if !i.macos {
        return gl("not macOS");
    }
    if !i.metal_device {
        return gl("no Metal device");
    }
    let explicit = i
        .flag
        .as_deref()
        .and_then(Choice::parse)
        .or_else(|| i.env.as_deref().and_then(Choice::parse));
    let choice = explicit.or_else(|| Choice::parse(&i.setting)).unwrap_or(Choice::Auto);
    match (explicit, choice) {
        (_, Choice::OpenGl) => return gl("OpenGL chosen"),
        (Some(Choice::Metal), _) => return metal("Metal chosen by --gfx or LR_GFX"),
        _ => {}
    }
    if i.readback_mode {
        return gl("this mode reads the screen back, which Metal cannot");
    }
    if i.marker {
        return Decision {
            gfx: Gfx::Gl,
            why: "the last Metal start did not finish (run --gfx metal to retry)",
            marker_forced: true,
        };
    }
    match choice {
        Choice::Metal => metal("Metal chosen in settings"),
        _ if i.apple_silicon && i.auto_metal => metal("AUTO on Apple Silicon"),
        _ => gl("AUTO: OpenGL until Metal is measured faster"),
    }
}

/// The marker's file name, the frame count that clears it and the AUTO rule (data/app/gfx.json).
#[derive(Clone, Debug, PartialEq)]
pub struct GfxRules {
    pub boot_marker: String,
    pub good_frames: u32,
    /// AUTO gives Metal on Apple Silicon. False until Metal is measured faster than OpenGL there.
    pub auto_metal_on_apple_silicon: bool,
}

impl Default for GfxRules {
    fn default() -> GfxRules {
        GfxRules { boot_marker: "gfx_boot.marker".into(), good_frames: 120, auto_metal_on_apple_silicon: false }
    }
}

impl GfxRules {
    /// Parses `{ schema, boot_marker, good_frames, auto_metal_on_apple_silicon }`: the marker a plain file name,
    /// the frame count at least 1, the AUTO switch a boolean.
    pub fn parse(text: &str) -> Result<GfxRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v["schema"].as_str() != Some("lr.gfx/1") {
            return Err("schema: must be lr.gfx/1".into());
        }
        let marker = v["boot_marker"]
            .as_str()
            .filter(|m| !m.is_empty() && !m.contains(['/', '\\']) && *m != "." && *m != "..")
            .ok_or_else(|| "boot_marker: must be a plain file name".to_string())?;
        let frames = v["good_frames"]
            .as_u64()
            .filter(|n| (1..=u32::MAX as u64).contains(n))
            .ok_or_else(|| "good_frames: must be a whole number of at least 1".to_string())?;
        let auto_metal = v["auto_metal_on_apple_silicon"]
            .as_bool()
            .ok_or_else(|| "auto_metal_on_apple_silicon: must be true or false".to_string())?;
        Ok(GfxRules { boot_marker: marker.into(), good_frames: frames as u32, auto_metal_on_apple_silicon: auto_metal })
    }
}

/// The rules compiled into the executable; a table that fails to parse is reported on stderr and the defaults are used.
pub fn gfx_rules() -> &'static GfxRules {
    static RULES: OnceLock<GfxRules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(GFX_RULES_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        GfxRules::parse(text).unwrap_or_else(|e| {
            eprintln!("{GFX_RULES_PATH}: {e}");
            GfxRules::default()
        })
    })
}

/// True when Metal can open a device here. Always false off macOS.
pub fn metal_device_present() -> bool {
    #[cfg(target_os = "macos")]
    {
        use core::ffi::c_void;
        #[link(name = "Metal", kind = "framework")]
        extern "C" {
            fn MTLCreateSystemDefaultDevice() -> *mut c_void;
        }
        #[link(name = "objc")]
        extern "C" {
            fn objc_release(obj: *mut c_void);
        }
        let device = unsafe { MTLCreateSystemDefaultDevice() };
        if device.is_null() {
            return false;
        }
        unsafe { objc_release(device) };
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn marker_path() -> Option<std::path::PathBuf> {
    super::paths::user_file(&gfx_rules().boot_marker)
}

/// The text of a marker: the process id of the Metal start that wrote it.
fn marker_text(pid: u32) -> String {
    format!("pid={pid}\nMetal start in progress; removed by a clean end.\n")
}

/// The process id a marker holds. None for a marker in any other shape (an old one or a damaged file).
fn marker_pid(text: &str) -> Option<u32> {
    text.lines().find_map(|l| l.strip_prefix("pid=")).and_then(|n| n.trim().parse().ok())
}

/// Whether a marker means the Metal start crashed: its process is gone (`alive` says whether a pid still runs).
/// A marker whose process still runs belongs to another running instance; a marker without a readable pid is
/// from before pids were written, and counts as a crash.
fn marker_is_crash(text: &str, alive: &dyn Fn(u32) -> bool) -> bool {
    match marker_pid(text) {
        Some(pid) => !alive(pid),
        None => true,
    }
}

/// Whether a process with this id exists. `kill(pid, 0)` sends nothing; a permission error still means it exists.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    if unsafe { kill(pid as i32, 0) } == 0 {
        return true;
    }
    // EPERM (1): it exists but is not ours. ESRCH (3): gone.
    std::io::Error::last_os_error().raw_os_error() == Some(1)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

/// Writes the marker for `pid` (the folder is created when missing).
fn write_marker(path: &std::path::Path, pid: u32) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, marker_text(pid));
}

/// Removes the marker when it is `pid`'s own (another instance's marker stays).
fn clear_marker(path: &std::path::Path, pid: u32) {
    let own = std::fs::read_to_string(path).ok().and_then(|t| marker_pid(&t)) == Some(pid);
    if own {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the file at `path` is a marker left by a crashed Metal start.
fn crash_marker_at(path: &std::path::Path, alive: &dyn Fn(u32) -> bool) -> bool {
    std::fs::read_to_string(path).map_or(false, |t| marker_is_crash(&t, alive))
}

/// The command-line value after `name`, if any.
fn flag_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// The real inputs of this launch.
fn real_inputs() -> Inputs {
    let has = |f: &str| std::env::args().any(|a| a == f);
    Inputs {
        flag: flag_value("--gfx"),
        env: std::env::var("LR_GFX").ok(),
        setting: super::settings::Settings::load().graphics_api,
        macos: cfg!(target_os = "macos"),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        metal_device: metal_device_present(),
        marker: marker_path().map_or(false, |p| crash_marker_at(&p, &process_alive)),
        readback_mode: has("--screenshot") || has("--self-test") || has("--agent-window"),
        auto_metal: gfx_rules().auto_metal_on_apple_silicon,
    }
}

/// This launch's choice, made once (window_conf runs before the window exists). Logs the decision.
pub fn chosen() -> Decision {
    static DECISION: OnceLock<Decision> = OnceLock::new();
    *DECISION.get_or_init(|| {
        let d = decide(&real_inputs());
        crate::lr_info!("graphics api: {:?} ({})", d.gfx, d.why);
        d
    })
}

/// Writes the crash-guard marker when this launch uses Metal, and arranges for a normal exit to remove it.
/// Call once, before the window opens.
pub fn boot_guard_start() {
    if chosen().gfx != Gfx::Metal {
        return;
    }
    if let Some(p) = marker_path() {
        write_marker(&p, std::process::id());
    }
    // a panic that unwinds still ends the process through exit(), so the exit hook must know one happened: the
    // hook below records it (chained onto the log's) and the exit hook then leaves the marker
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICKED.store(true, Ordering::Relaxed);
        prev(info);
    }));
    #[cfg(unix)]
    {
        extern "C" fn clean_exit() {
            if exit_is_clean(PANICKED.load(Ordering::Relaxed)) {
                boot_guard_clear();
            }
        }
        extern "C" {
            fn atexit(cb: extern "C" fn()) -> i32;
        }
        unsafe { atexit(clean_exit) };
    }
}

/// Set by the panic hook `boot_guard_start` installs: this run panicked, so its end is not a clean one.
static PANICKED: AtomicBool = AtomicBool::new(false);

/// Whether a process ending now ended cleanly: it did not panic. (A hard abort or a kill never reaches the exit
/// hook at all, which is what leaves the marker behind.)
fn exit_is_clean(panicked: bool) -> bool {
    !panicked
}

/// Removes this process's marker: the Metal start worked, or the process is ending normally.
pub fn boot_guard_clear() {
    if let Some(p) = marker_path() {
        clear_marker(&p, std::process::id());
    }
}

static TICKS: AtomicU32 = AtomicU32::new(0);

/// Call once per drawn frame. After `good_frames` of them the marker is deleted: the Metal start worked.
pub fn boot_guard_tick() {
    if tick(&TICKS, gfx_rules().good_frames) {
        boot_guard_clear();
    }
}

/// Counts a frame; true exactly once, on the `good`th.
fn tick(counter: &AtomicU32, good: u32) -> bool {
    counter.fetch_add(1, Ordering::Relaxed) + 1 == good
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac() -> Inputs {
        Inputs {
            setting: "AUTO".into(),
            macos: true,
            apple_silicon: true,
            metal_device: true,
            auto_metal: false,
            ..Inputs::default()
        }
    }

    fn pick(i: &Inputs) -> Gfx {
        decide(i).gfx
    }

    #[test]
    fn auto_is_opengl_everywhere_until_metal_is_measured_faster() {
        assert_eq!(pick(&mac()), Gfx::Gl);
        assert_eq!(pick(&Inputs { apple_silicon: false, ..mac() }), Gfx::Gl);
        assert!(!GfxRules::default().auto_metal_on_apple_silicon);
    }

    #[test]
    fn the_data_switch_gives_auto_metal_on_apple_silicon_only() {
        let on = Inputs { auto_metal: true, ..mac() };
        assert_eq!(pick(&on), Gfx::Metal);
        assert_eq!(decide(&on).why, "AUTO on Apple Silicon");
        assert_eq!(pick(&Inputs { apple_silicon: false, ..on.clone() }), Gfx::Gl);
        // OPENGL in the setting still wins
        assert_eq!(pick(&Inputs { setting: "OPENGL".into(), ..on }), Gfx::Gl);
    }

    #[test]
    fn metal_in_the_setting_or_a_flag_is_honoured_without_the_auto_switch() {
        assert_eq!(pick(&Inputs { setting: "METAL".into(), ..mac() }), Gfx::Metal);
        assert_eq!(pick(&Inputs { flag: Some("metal".into()), ..mac() }), Gfx::Metal);
        assert_eq!(pick(&Inputs { env: Some("metal".into()), ..mac() }), Gfx::Metal);
    }

    #[test]
    fn intel_still_gets_metal_when_asked_for() {
        let intel = Inputs { apple_silicon: false, ..mac() };
        assert_eq!(pick(&Inputs { setting: "METAL".into(), ..intel.clone() }), Gfx::Metal);
        assert_eq!(pick(&Inputs { flag: Some("metal".into()), ..intel }), Gfx::Metal);
    }

    #[test]
    fn flag_beats_env_beats_setting() {
        let i = Inputs { setting: "OPENGL".into(), ..mac() };
        assert_eq!(pick(&i), Gfx::Gl);
        assert_eq!(pick(&Inputs { env: Some("metal".into()), ..i.clone() }), Gfx::Metal);
        assert_eq!(pick(&Inputs { env: Some("metal".into()), flag: Some("gl".into()), ..i.clone() }), Gfx::Gl);
        let j = Inputs { setting: "METAL".into(), env: Some("gl".into()), ..mac() };
        assert_eq!(pick(&j), Gfx::Gl);
        assert_eq!(pick(&Inputs { flag: Some("metal".into()), ..j }), Gfx::Metal);
        // an unreadable flag is ignored, so the next source decides
        let k = Inputs { flag: Some("vulkan".into()), env: Some("gl".into()), ..mac() };
        assert_eq!(pick(&k), Gfx::Gl);
    }

    #[test]
    fn a_leftover_marker_forces_opengl_unless_metal_is_asked_for_by_flag() {
        let m = Inputs { marker: true, ..mac() };
        assert_eq!(pick(&m), Gfx::Gl);
        let forced = decide(&Inputs { setting: "METAL".into(), ..m.clone() });
        assert_eq!(forced.gfx, Gfx::Gl);
        assert!(forced.marker_forced);
        assert!(!decide(&mac()).marker_forced);
        assert_eq!(pick(&Inputs { flag: Some("metal".into()), ..m.clone() }), Gfx::Metal);
        assert_eq!(pick(&Inputs { env: Some("metal".into()), ..m }), Gfx::Metal);
    }

    #[test]
    fn screenshot_modes_use_opengl_unless_metal_is_forced() {
        let s = Inputs { readback_mode: true, ..mac() };
        assert_eq!(pick(&s), Gfx::Gl);
        assert_eq!(pick(&Inputs { setting: "METAL".into(), ..s.clone() }), Gfx::Gl);
        assert_eq!(pick(&Inputs { env: Some("metal".into()), ..s }), Gfx::Metal);
    }

    #[test]
    fn other_systems_and_missing_devices_are_always_opengl() {
        let forced = Inputs { flag: Some("metal".into()), setting: "METAL".into(), ..mac() };
        assert_eq!(pick(&Inputs { macos: false, ..forced.clone() }), Gfx::Gl);
        assert_eq!(pick(&Inputs { metal_device: false, ..forced }), Gfx::Gl);
    }

    #[test]
    fn choice_text_round_trips_and_cycles() {
        for c in [Choice::Auto, Choice::Metal, Choice::OpenGl] {
            assert_eq!(Choice::parse(c.id()), Some(c));
        }
        assert_eq!(Choice::parse(" GL "), Some(Choice::OpenGl));
        assert_eq!(Choice::parse("vulkan"), None);
        assert_eq!(Choice::Auto.next().next().next(), Choice::Auto);
    }

    #[test]
    fn gfx_data_matches_defaults_and_refuses_bad_values() {
        let r = GfxRules::parse(include_str!("../data/app/gfx.json")).expect("gfx.json parses");
        assert_eq!(r, GfxRules::default());
        assert_eq!(gfx_rules(), &r);
        let with = |marker: &str, frames: &str| {
            GfxRules::parse(&format!(
                r#"{{ "schema": "lr.gfx/1", "boot_marker": {marker}, "good_frames": {frames}, "auto_metal_on_apple_silicon": false }}"#
            ))
        };
        assert!(with(r#""m.marker""#, "5").is_ok());
        assert!(with(r#""../m""#, "5").is_err());
        assert!(with(r#""""#, "5").is_err());
        assert!(with(r#""m""#, "0").is_err());
        assert!(GfxRules::parse(r#"{ "boot_marker": "m", "good_frames": 5 }"#).is_err());
        assert!(GfxRules::parse(r#"{ "schema": "lr.gfx/1", "boot_marker": "m", "good_frames": 5 }"#).is_err());
        assert!(GfxRules::parse(
            r#"{ "schema": "lr.gfx/1", "boot_marker": "m", "good_frames": 5, "auto_metal_on_apple_silicon": "yes" }"#
        )
        .is_err());
    }

    #[test]
    fn the_guard_clears_exactly_once_on_the_good_frame() {
        let c = AtomicU32::new(0);
        let fired: Vec<bool> = (0..6).map(|_| tick(&c, 3)).collect();
        assert_eq!(fired, [false, false, true, false, false, false]);
    }

    /// A scratch folder for the marker tests, removed at the end of each.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_gfx_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn marker_text_round_trips_the_pid() {
        assert_eq!(marker_pid(&marker_text(4242)), Some(4242));
        assert_eq!(marker_pid("Metal start in progress; deleted after a good run.\n"), None);
        assert_eq!(marker_pid("pid=notanumber"), None);
    }

    #[test]
    fn a_marker_is_a_crash_only_when_its_process_is_gone() {
        let gone = |_: u32| false;
        let running = |_: u32| true;
        assert!(marker_is_crash(&marker_text(7), &gone));
        assert!(!marker_is_crash(&marker_text(7), &running));
        // an old marker with no pid cannot be tied to a process: it counts
        assert!(marker_is_crash("Metal start in progress\n", &running));
    }

    #[test]
    fn marker_lifecycle_start_writes_clean_end_clears_and_a_second_instance_is_ignored() {
        let dir = scratch("lifecycle");
        let path = dir.join("gfx_boot.marker");
        let me = std::process::id();
        let nobody = |_: u32| false;
        let only_me = move |p: u32| p == me;
        assert!(!crash_marker_at(&path, &nobody), "no file, no crash");
        write_marker(&path, me);
        assert!(path.exists(), "a Metal start writes the marker");
        // a second instance, while the first still runs: not a crash
        assert!(!crash_marker_at(&path, &only_me));
        // the first dies without a clean end: a crash
        assert!(crash_marker_at(&path, &nobody));
        // another process's clean end does not remove it
        clear_marker(&path, me + 1);
        assert!(path.exists());
        // the owner's clean end (after the good frames, or at exit) does
        clear_marker(&path, me);
        assert!(!path.exists());
        assert!(!crash_marker_at(&path, &nobody));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_is_not_a_clean_end() {
        assert!(exit_is_clean(false));
        assert!(!exit_is_clean(true));
    }

    #[test]
    fn this_process_is_alive_and_pid_zero_is_not() {
        #[cfg(unix)]
        {
            assert!(process_alive(std::process::id()));
            assert!(!process_alive(0));
        }
    }
}
