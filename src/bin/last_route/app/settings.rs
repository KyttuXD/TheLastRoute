//! Player settings (game/autoload/settings.gd defaults): volumes, field of view, mouse
//! sensitivity, fullscreen, and the rest of the original's settings panel. Saved to a small
//! JSON file in the per-user folder.
//!
//! MENU / UI volume scales the menu blip. Graphics quality drives the 3D render-target scale
//! via `app::targets` (LOW/MEDIUM/HIGH/AUTO). Some rows are kept for the panel only (spoken-chat
//! volume, voices, speak nearby chat, subtitles, ads, pause-on-focus-loss, teammate waypoints): the chat speaker
//! (audio/tts.rs) is not fed by chat yet, so the spoken-chat rows are stored but change nothing until it is. When it
//! is, a line's level is Master x SPOKEN CHAT (`tts::speech_volume`). VSYNC and the FPS LIMIT are live:
//! `app::pacing` applies them (see `swap_interval` and `frame_cap_fps` below).

use crate::net::world_sync::{clean_skin, SKIN_DEFAULT};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[derive(Clone)]
pub struct Settings {
    pub master_volume: f32,
    pub gameplay_volume: f32,
    /// HUNTERS, AMBIENCE and MUSIC sit under GAMEPLAY (audio/cues/rules.rs `bus_gain`). 0..1, default 1.
    pub hunters_volume: f32,
    pub ambience_volume: f32,
    pub music_volume: f32,
    pub ui_volume: f32,
    /// 0..100, steps of 5 (SPOKEN CHAT). Scales spoken chat with Master once the speaker is connected (audio/tts.rs).
    pub voice_volume: f32,
    pub voices: bool,
    pub subtitles: bool,
    pub fov: f32,
    /// Radians per pixel; the original's slider shows it as 0..100.
    pub sensitivity: f32,
    pub fullscreen: bool,
    pub vsync: bool,
    /// 0 = unlimited, else 30 or more in steps of 30. No upper cap: the row goes as high as the screen can show.
    pub fps_limit: f32,
    /// AUTO / LOW / MEDIUM / HIGH.
    pub quality: String,
    /// ADAPTIVE QUALITY: while QUALITY is AUTO, the frame-time governor (app/quality.rs) gives up lamps, view distance and
    /// render scale when frames run slow and takes them back when they run clean. On by default in the browser build, off
    /// on a native build (whose AUTO stays at the HIGH look).
    pub adaptive_quality: bool,
    /// off / gentle / annoying.
    pub ads: String,
    /// AUTO / METAL / OPENGL (macOS only; read at launch, so a change needs a restart). See `app::gfx`.
    pub graphics_api: String,
    pub pause_on_focus_loss: bool,
    pub teammate_waypoints: bool,
    /// Prefer calm HUD motion (Porcelle's radar ping stays a steady dim dot).
    pub reduced_motion: bool,
    /// The local profile name the title's account button shows (the admin's is KYTTU).
    pub username: String,
    /// The menus' colour scheme (palette.gd SCHEMES): "last_route" or "city". The original
    /// switches it with the game that is open; here LR_SCHEME or this saved value picks it.
    pub scheme: String,
    /// The runner's skin id, sent to the room in the wire profile (`world_sync::clean_skin`).
    pub skin: String,
    /// Whether the applied skin has the slim (Alex) arm layout, read when it was applied. The saved skin is the canonical
    /// PNG with its base layer made opaque, so the layout cannot be read from it again (Godot `Settings.skin_slim`).
    pub skin_slim: bool,
    /// OTHERS MAY COPY: other runners may copy this skin (sent in the wire profile; Godot `Settings.skin_copyable`).
    pub skin_copy: bool,
    /// Arm style for the runner body and the first-person hands: auto, classic or slim.
    pub arms: String,
    /// Speak nearby chat lines aloud (audio/tts.rs).
    pub speech_enabled: bool,
    /// Peers whose chat is never spoken. Session only; not saved.
    pub muted_peers: std::collections::HashSet<i64>,
    /// Rebound controls (only actions changed from data/controls.json; see app/bindings.rs).
    pub bindings: super::bindings::Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master_volume: 1.0,
            gameplay_volume: 1.0,
            hunters_volume: 1.0,
            ambience_volume: 1.0,
            music_volume: 1.0,
            ui_volume: 1.0,
            voice_volume: 80.0,
            voices: true,
            subtitles: true,
            fov: crate::config::FIELD_OF_VIEW_DEG,
            sensitivity: crate::config::MOUSE_SENSITIVITY,
            fullscreen: false,
            vsync: true,
            fps_limit: 0.0,
            quality: "AUTO".into(),
            adaptive_quality: cfg!(target_arch = "wasm32"),
            ads: "off".into(),
            graphics_api: "AUTO".into(),
            pause_on_focus_loss: true,
            teammate_waypoints: true,
            reduced_motion: false,
            // empty: a new game starts signed out (guest). Only signing in as KYTTU enables the admin tools.
            username: String::new(),
            scheme: "last_route".into(),
            skin: SKIN_DEFAULT.into(),
            skin_slim: false,
            skin_copy: true,
            arms: "auto".into(),
            speech_enabled: true,
            muted_peers: Default::default(),
            bindings: Default::default(),
        }
    }
}

/// The ARMS setting as the skin panel uses it.
pub struct ArmsSetting;

impl ArmsSetting {
    /// True when slim arms are worn: the saved setting `arms`, and for auto the skin's layout (`skin_slim`, read when the
    /// skin was validated; the canonical PNG cannot be asked again).
    pub fn resolve(arms: &str, skin_slim: bool) -> bool {
        use crate::shared::skins;
        skins::ArmsSetting::parse(arms).unwrap_or(skins::ArmsSetting::Auto).resolve(skin_slim)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> Option<PathBuf> {
    super::paths::user_file("settings.json")
}

/// The saved settings text: the page's localStorage in the browser build, the settings file everywhere else.
#[cfg(target_arch = "wasm32")]
fn read_text() -> Option<String> {
    super::web_storage::get(super::web_storage::SETTINGS_KEY)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_text() -> Option<String> {
    path().and_then(|p| std::fs::read_to_string(p).ok())
}

impl Settings {
    pub fn load() -> Settings {
        read_text()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|v| Settings::from_json(&v))
            .unwrap_or_default()
    }

    /// Reads the saved keys from a parsed settings file. A missing or invalid key keeps its default.
    pub fn from_json(v: &serde_json::Value) -> Settings {
        let mut s = Settings::default();
        let f = |k: &str, d: f32| v[k].as_f64().map(|x| x as f32).unwrap_or(d);
        let b = |k: &str, d: bool| v[k].as_bool().unwrap_or(d);
        s.master_volume = f("master_volume", 1.0).clamp(0.0, 1.0);
        s.gameplay_volume = f("gameplay_volume", 1.0).clamp(0.0, 1.0);
        s.hunters_volume = f("hunters_volume", 1.0).clamp(0.0, 1.0);
        s.ambience_volume = f("ambience_volume", 1.0).clamp(0.0, 1.0);
        s.music_volume = f("music_volume", 1.0).clamp(0.0, 1.0);
        s.ui_volume = f("ui_volume", 1.0).clamp(0.0, 1.0);
        s.voice_volume = f("voice_volume", 80.0).clamp(0.0, 100.0);
        s.voices = b("voices", true);
        s.subtitles = b("subtitles", true);
        s.fov = f("fov", s.fov).clamp(crate::config::FOV_MIN_DEG, crate::config::FOV_MAX_DEG);
        s.sensitivity = f("sensitivity", s.sensitivity).clamp(
            crate::config::SENSITIVITY_MIN,
            crate::config::SENSITIVITY_MAX,
        );
        s.fullscreen = b("fullscreen", false);
        s.vsync = b("vsync", true);
        s.fps_limit = f("fps_limit", 0.0).max(0.0);
        s.pause_on_focus_loss = b("pause_on_focus_loss", true);
        s.teammate_waypoints = b("teammate_waypoints", true);
        s.reduced_motion = b("reduced_motion", false);
        s.adaptive_quality = b("adaptive_quality", s.adaptive_quality);
        if let Some(q) = v["quality"]
            .as_str()
            .filter(|q| ["AUTO", "LOW", "MEDIUM", "HIGH"].contains(q))
        {
            s.quality = q.into();
        }
        if let Some(q) = v["ads"]
            .as_str()
            .filter(|q| ["off", "gentle", "annoying"].contains(q))
        {
            s.ads = q.into();
        }
        if let Some(c) = v["graphics_api"].as_str().and_then(super::gfx::Choice::parse) {
            s.graphics_api = c.id().into();
        }
        if let Some(q) = v["scheme"].as_str() {
            s.scheme = if q == "city" { "city" } else { "last_route" }.into();
        }
        if let Some(k) = v["skin"].as_str() {
            s.skin = clean_skin(k);
        }
        s.skin_slim = b("skin_slim", false);
        s.skin_copy = b("skin_copy", true);
        if let Some(a) = v["arms"].as_str().and_then(crate::shared::skins::ArmsSetting::parse) {
            s.arms = a.id().into();
        }
        s.speech_enabled = b("speech_enabled", true);
        s.bindings = super::bindings::Bindings::from_json(&v["bindings"]);
        if let Some(u) = v["username"].as_str() {
            s.username = u
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(24)
                .collect::<String>()
                .to_uppercase();
        }
        s
    }

    /// The saved keys, as `save` writes them.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "master_volume": self.master_volume, "gameplay_volume": self.gameplay_volume,
            "hunters_volume": self.hunters_volume, "ambience_volume": self.ambience_volume, "music_volume": self.music_volume,
            "ui_volume": self.ui_volume, "voice_volume": self.voice_volume, "voices": self.voices,
            "subtitles": self.subtitles, "fov": self.fov, "sensitivity": self.sensitivity,
            "fullscreen": self.fullscreen, "vsync": self.vsync, "fps_limit": self.fps_limit,
            "quality": self.quality, "adaptive_quality": self.adaptive_quality, "ads": self.ads, "graphics_api": self.graphics_api, "pause_on_focus_loss": self.pause_on_focus_loss,
            "teammate_waypoints": self.teammate_waypoints, "reduced_motion": self.reduced_motion, "username": self.username, "scheme": self.scheme,
            "skin": self.skin, "skin_slim": self.skin_slim, "skin_copy": self.skin_copy,
            "arms": self.arms, "speech_enabled": self.speech_enabled, "bindings": self.bindings.to_json(),
        })
    }

    pub fn save(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = super::web_storage::set(super::web_storage::SETTINGS_KEY, &self.to_json().to_string());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(p, self.to_json().to_string());
        }
    }

    /// The native settings loader's slider mapping: 0..100 over the clamped range.
    pub fn sensitivity_percent(&self) -> f32 {
        (self.sensitivity - crate::config::SENSITIVITY_MIN)
            / (crate::config::SENSITIVITY_MAX - crate::config::SENSITIVITY_MIN)
            * 100.0
    }

    /// The GL swap interval the VSYNC row asks for: 1 = wait for the display's refresh, 0 = present
    /// as soon as a frame is done. Without this, the window opens with swap interval 1 and the
    /// game stays locked to the refresh rate (60 on most panels) whatever the row says.
    pub fn swap_interval(&self) -> i32 {
        if self.vsync {
            1
        } else {
            0
        }
    }

    /// The frame cap the FPS LIMIT row asks for: `None` is UNLIMITED. Mirrors Godot's
    /// `Settings.set_fps_limit`: a value under 15 means unlimited, anything else is rounded and
    /// kept at the data floor (`frame.min_cap_fps` in data/controls.json) or more.
    pub fn frame_cap_fps(&self) -> Option<u32> {
        if self.fps_limit < 15.0 {
            None
        } else {
            let floor = super::bindings::rules().frame.min_cap_fps as f32;
            Some(self.fps_limit.round().max(floor) as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    fn with_limit(fps_limit: f32) -> Settings {
        Settings {
            fps_limit,
            ..Settings::default()
        }
    }

    #[test]
    fn fps_limit_maps_to_frame_cap() {
        // the row's 0 and anything under 15 are UNLIMITED (settings.gd set_fps_limit)
        assert_eq!(with_limit(0.0).frame_cap_fps(), None);
        assert_eq!(with_limit(14.9).frame_cap_fps(), None);
        // the row snaps to 30 and up in steps of 30; nothing above 240 is clamped away
        assert_eq!(with_limit(15.0).frame_cap_fps(), Some(30));
        assert_eq!(with_limit(30.0).frame_cap_fps(), Some(30));
        assert_eq!(with_limit(60.0).frame_cap_fps(), Some(60));
        assert_eq!(with_limit(144.0).frame_cap_fps(), Some(144));
        assert_eq!(with_limit(240.0).frame_cap_fps(), Some(240));
        assert_eq!(with_limit(1000.0).frame_cap_fps(), Some(1000));
    }

    #[test]
    fn frame_cap_floor_comes_from_the_data_knob() {
        // the clamp floor is the data knob `frame.min_cap_fps`, not a hardcoded 30
        let floor = crate::app::bindings::rules().frame.min_cap_fps;
        assert_eq!(with_limit(15.0).frame_cap_fps(), Some(floor));
        assert_eq!(with_limit(floor as f32 - 1.0).frame_cap_fps(), Some(floor));
        // a request above the floor is kept as asked
        assert_eq!(with_limit(floor as f32 + 30.0).frame_cap_fps(), Some(floor + 30));
    }

    #[test]
    fn vsync_maps_to_swap_interval() {
        let on = Settings {
            vsync: true,
            ..Settings::default()
        };
        let off = Settings {
            vsync: false,
            ..Settings::default()
        };
        assert_eq!(on.swap_interval(), 1);
        assert_eq!(off.swap_interval(), 0);
    }

    #[test]
    fn saved_key_names_are_unchanged() {
        let mut keys: Vec<String> = Settings::default()
            .to_json()
            .as_object()
            .expect("settings are a JSON object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        let mut expected: Vec<String> = [
            "master_volume",
            "gameplay_volume",
            "hunters_volume",
            "ambience_volume",
            "music_volume",
            "ui_volume",
            "voice_volume",
            "voices",
            "subtitles",
            "fov",
            "sensitivity",
            "fullscreen",
            "vsync",
            "fps_limit",
            "quality",
            "adaptive_quality",
            "ads",
            "graphics_api",
            "pause_on_focus_loss",
            "teammate_waypoints",
            "reduced_motion",
            "username",
            "scheme",
            "skin",
            "skin_slim",
            "skin_copy",
            "arms",
            "speech_enabled",
            "bindings",
        ]
        .iter()
        .map(|k| k.to_string())
        .collect();
        expected.sort();
        assert_eq!(keys, expected);
    }

    #[test]
    fn saved_keys_round_trip() {
        let saved = Settings {
            master_volume: 0.4,
            gameplay_volume: 0.6,
            hunters_volume: 0.7,
            ambience_volume: 0.5,
            music_volume: 0.3,
            ui_volume: 0.2,
            voice_volume: 45.0,
            voices: false,
            subtitles: false,
            fov: (crate::config::FOV_MIN_DEG + crate::config::FOV_MAX_DEG) * 0.5,
            sensitivity: (crate::config::SENSITIVITY_MIN + crate::config::SENSITIVITY_MAX) * 0.5,
            fullscreen: true,
            vsync: false,
            fps_limit: 90.0,
            quality: "MEDIUM".into(),
            adaptive_quality: !cfg!(target_arch = "wasm32"),
            ads: "gentle".into(),
            graphics_api: "METAL".into(),
            pause_on_focus_loss: false,
            teammate_waypoints: false,
            reduced_motion: true,
            username: "ALICE".into(),
            scheme: "city".into(),
            skin: "NIGHT".into(),
            skin_slim: true,
            skin_copy: false,
            arms: "slim".into(),
            speech_enabled: false,
            muted_peers: Default::default(),
            bindings: { let mut b = super::super::bindings::Bindings::default(); b.set("flashlight", "H").unwrap(); b },
        };
        let json = saved.to_json();
        let back = Settings::from_json(&json);
        assert_eq!(back.to_json(), json);
        assert_eq!(back.quality, "MEDIUM");
        assert_eq!(back.adaptive_quality, !cfg!(target_arch = "wasm32"), "the saved choice, not the default, comes back");
        assert_eq!(back.ads, "gentle");
        assert_eq!(back.graphics_api, "METAL");
        assert_eq!(back.username, "ALICE");
        assert_eq!(back.scheme, "city");
        assert_eq!(back.skin, "NIGHT");
        assert_eq!(Settings::from_json(&serde_json::json!({"skin": "no way!!"})).skin, "NOWAY");
        assert_eq!(Settings::from_json(&serde_json::json!({"skin": ""})).skin, "DEFAULT");
        assert!(!back.vsync && back.fullscreen);
        assert!(back.reduced_motion);
        assert_eq!(back.bindings.keys("flashlight"), ["H"], "a rebind is saved and comes back");
        assert_eq!(Settings::from_json(&serde_json::json!({"bindings": "junk"})).bindings.keys("flashlight"), ["F"]);
    }

    #[test]
    fn adaptive_quality_defaults_on_for_the_browser_and_off_native_and_old_files_load() {
        let default_on = cfg!(target_arch = "wasm32");
        assert_eq!(Settings::default().adaptive_quality, default_on);
        // a settings file from before the row existed takes the platform's default; a saved choice wins either way
        assert_eq!(Settings::from_json(&serde_json::json!({ "quality": "AUTO" })).adaptive_quality, default_on);
        assert!(Settings::from_json(&serde_json::json!({ "adaptive_quality": true })).adaptive_quality);
        assert!(!Settings::from_json(&serde_json::json!({ "adaptive_quality": false })).adaptive_quality);
        assert_eq!(Settings::from_json(&serde_json::json!({ "adaptive_quality": "yes" })).adaptive_quality, default_on);
    }

    #[test]
    fn graphics_api_defaults_to_auto_and_old_files_load() {
        // a settings file from before the row existed
        let old = serde_json::json!({ "vsync": false, "quality": "HIGH" });
        assert_eq!(Settings::from_json(&old).graphics_api, "AUTO");
        assert_eq!(Settings::default().graphics_api, "AUTO");
        // the saved text is normalised; junk is ignored
        assert_eq!(Settings::from_json(&serde_json::json!({ "graphics_api": "gl" })).graphics_api, "OPENGL");
        assert_eq!(Settings::from_json(&serde_json::json!({ "graphics_api": "metal" })).graphics_api, "METAL");
        assert_eq!(Settings::from_json(&serde_json::json!({ "graphics_api": "vulkan" })).graphics_api, "AUTO");
    }

    #[test]
    fn a_file_from_before_the_category_sliders_loads_them_at_full() {
        let old = serde_json::json!({ "master_volume": 0.5, "gameplay_volume": 0.4, "ui_volume": 0.3 });
        let s = Settings::from_json(&old);
        assert_eq!((s.hunters_volume, s.ambience_volume, s.music_volume), (1.0, 1.0, 1.0));
        assert_eq!((s.master_volume, s.gameplay_volume, s.ui_volume), (0.5, 0.4, 0.3));
        let wild = Settings::from_json(&serde_json::json!({ "hunters_volume": 7.0, "music_volume": -1.0 }));
        assert_eq!((wild.hunters_volume, wild.music_volume), (1.0, 0.0), "clamped to 0..1");
    }

    #[test]
    fn empty_file_keeps_defaults() {
        let from_empty = Settings::from_json(&serde_json::json!({}));
        assert_eq!(from_empty.to_json(), Settings::default().to_json());
    }
}
