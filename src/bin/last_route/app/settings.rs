//! Player settings (game/autoload/settings.gd defaults): volumes, field of view, mouse
//! sensitivity, fullscreen, and the rest of the original's settings panel. Saved to a small
//! JSON file in the per-user folder.
//!
//! MENU / UI volume scales the menu blip. Graphics quality drives the 3D render-target scale
//! via `app::targets` (LOW/MEDIUM/HIGH/AUTO). Some rows are kept for the panel only (spoken-chat
//! volume, voices, subtitles, ads, pause-on-focus-loss, teammate waypoints): this build has no
//! voice chat, so they are stored but change nothing. VSYNC and the FPS LIMIT are live:
//! `app::pacing` applies them (see `swap_interval` and `frame_cap_fps` below).

use crate::net::world_sync::{clean_skin, SKIN_DEFAULT};
use std::path::PathBuf;

#[derive(Clone)]
pub struct Settings {
    pub master_volume: f32,
    pub gameplay_volume: f32,
    pub ui_volume: f32,
    /// 0..100, steps of 5 (SPOKEN CHAT).
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
    /// off / gentle / annoying.
    pub ads: String,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master_volume: 1.0,
            gameplay_volume: 1.0,
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
            ads: "off".into(),
            pause_on_focus_loss: true,
            teammate_waypoints: true,
            reduced_motion: false,
            // empty: a new game starts signed out (guest). Only signing in as KYTTU enables the admin tools.
            username: String::new(),
            scheme: "last_route".into(),
            skin: SKIN_DEFAULT.into(),
        }
    }
}

fn path() -> Option<PathBuf> {
    super::paths::user_file("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
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
        if let Some(q) = v["scheme"].as_str() {
            s.scheme = if q == "city" { "city" } else { "last_route" }.into();
        }
        if let Some(k) = v["skin"].as_str() {
            s.skin = clean_skin(k);
        }
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
            "ui_volume": self.ui_volume, "voice_volume": self.voice_volume, "voices": self.voices,
            "subtitles": self.subtitles, "fov": self.fov, "sensitivity": self.sensitivity,
            "fullscreen": self.fullscreen, "vsync": self.vsync, "fps_limit": self.fps_limit,
            "quality": self.quality, "ads": self.ads, "pause_on_focus_loss": self.pause_on_focus_loss,
            "teammate_waypoints": self.teammate_waypoints, "reduced_motion": self.reduced_motion, "username": self.username, "scheme": self.scheme,
            "skin": self.skin,
        })
    }

    pub fn save(&self) {
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
    /// kept at 30 or more.
    pub fn frame_cap_fps(&self) -> Option<u32> {
        if self.fps_limit < 15.0 {
            None
        } else {
            Some(self.fps_limit.round().max(30.0) as u32)
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
            "ads",
            "pause_on_focus_loss",
            "teammate_waypoints",
            "reduced_motion",
            "username",
            "scheme",
            "skin",
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
            ads: "gentle".into(),
            pause_on_focus_loss: false,
            teammate_waypoints: false,
            reduced_motion: true,
            username: "ALICE".into(),
            scheme: "city".into(),
            skin: "NIGHT".into(),
        };
        let json = saved.to_json();
        let back = Settings::from_json(&json);
        assert_eq!(back.to_json(), json);
        assert_eq!(back.quality, "MEDIUM");
        assert_eq!(back.ads, "gentle");
        assert_eq!(back.username, "ALICE");
        assert_eq!(back.scheme, "city");
        assert_eq!(back.skin, "NIGHT");
        assert_eq!(Settings::from_json(&serde_json::json!({"skin": "no way!!"})).skin, "NOWAY");
        assert_eq!(Settings::from_json(&serde_json::json!({"skin": ""})).skin, "DEFAULT");
        assert!(!back.vsync && back.fullscreen);
        assert!(back.reduced_motion);
    }

    #[test]
    fn empty_file_keeps_defaults() {
        let from_empty = Settings::from_json(&serde_json::json!({}));
        assert_eq!(from_empty.to_json(), Settings::default().to_json());
    }
}
