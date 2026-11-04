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
//! 5. otherwise a crash-guard marker left by an earlier Metal start forces OpenGL;
//! 6. AUTO is Metal on Apple Silicon and OpenGL on Intel (Metal there is not measured yet).
//!
//! The crash guard: a Metal start writes the marker file before the window opens and `boot_guard_tick` (called
//! once per drawn frame) deletes it after the data file's `good_frames`. A panic inside miniquad's Metal
//! backend aborts the process, so the marker survives and the next AUTO launch uses OpenGL. To try Metal again
//! after fixing the cause, start once with `--gfx metal`: a good run deletes the marker.

use std::sync::atomic::{AtomicU32, Ordering};
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
    pub marker: bool,
    pub readback_mode: bool,
}

/// The decision and the reason, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub gfx: Gfx,
    pub why: &'static str,
}

fn gl(why: &'static str) -> Decision {
    Decision { gfx: Gfx::Gl, why }
}

fn metal(why: &'static str) -> Decision {
    Decision { gfx: Gfx::Metal, why }
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
        return gl("the last Metal start did not finish (delete the marker or run --gfx metal to retry)");
    }
    match choice {
        Choice::Metal => metal("Metal chosen in settings"),
        _ if i.apple_silicon => metal("AUTO on Apple Silicon"),
        _ => gl("AUTO on Intel: OpenGL until Metal is measured there"),
    }
}

/// The marker's file name and the frame count that clears it (data/app/gfx.json).
#[derive(Clone, Debug, PartialEq)]
pub struct GfxRules {
    pub boot_marker: String,
    pub good_frames: u32,
}

impl Default for GfxRules {
    fn default() -> GfxRules {
        GfxRules { boot_marker: "gfx_boot.marker".into(), good_frames: 120 }
    }
}

impl GfxRules {
    /// Parses `{ schema, boot_marker, good_frames }`: the marker a plain file name, the frame count at least 1.
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
        Ok(GfxRules { boot_marker: marker.into(), good_frames: frames as u32 })
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
        marker: marker_path().map_or(false, |p| p.exists()),
        readback_mode: has("--screenshot") || has("--self-test") || has("--agent-window"),
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

/// Writes the crash-guard marker when this launch uses Metal. Call once, before the window opens.
pub fn boot_guard_start() {
    if chosen().gfx != Gfx::Metal {
        return;
    }
    if let Some(p) = marker_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, "Metal start in progress; deleted after a good run.\n");
    }
}

static TICKS: AtomicU32 = AtomicU32::new(0);

/// Call once per drawn frame. After `good_frames` of them the marker is deleted: the Metal start worked.
pub fn boot_guard_tick() {
    if tick(&TICKS, gfx_rules().good_frames) {
        if let Some(p) = marker_path() {
            let _ = std::fs::remove_file(p);
        }
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
            ..Inputs::default()
        }
    }

    fn pick(i: &Inputs) -> Gfx {
        decide(i).gfx
    }

    #[test]
    fn auto_is_metal_on_apple_silicon_and_opengl_on_intel() {
        assert_eq!(pick(&mac()), Gfx::Metal);
        assert_eq!(pick(&Inputs { apple_silicon: false, ..mac() }), Gfx::Gl);
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
        assert_eq!(pick(&Inputs { setting: "METAL".into(), ..m.clone() }), Gfx::Gl);
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
            GfxRules::parse(&format!(r#"{{ "schema": "lr.gfx/1", "boot_marker": {marker}, "good_frames": {frames} }}"#))
        };
        assert!(with(r#""m.marker""#, "5").is_ok());
        assert!(with(r#""../m""#, "5").is_err());
        assert!(with(r#""""#, "5").is_err());
        assert!(with(r#""m""#, "0").is_err());
        assert!(GfxRules::parse(r#"{ "boot_marker": "m", "good_frames": 5 }"#).is_err());
    }

    #[test]
    fn the_guard_clears_exactly_once_on_the_good_frame() {
        let c = AtomicU32::new(0);
        let fired: Vec<bool> = (0..6).map(|_| tick(&c, 3)).collect();
        assert_eq!(fired, [false, false, true, false, false, false]);
    }
}
