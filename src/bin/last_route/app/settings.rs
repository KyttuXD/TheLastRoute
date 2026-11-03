//! Player settings (game/autoload/settings.gd defaults): volumes, field of view, mouse
//! sensitivity, fullscreen, and the rest of the original's settings panel. Saved to a small
//! JSON file in the per-user folder.
//!
//! MENU / UI volume scales the menu blip. Some rows are kept for the panel only (spoken-chat volume,
//! voices, subtitles, graphics quality, ads, pause-on-focus-loss, teammate waypoints): this build
//! has no voice chat or quality presets, so they are stored but change nothing. VSYNC and the FPS
//! LIMIT are live: `app::pacing` applies them (see `swap_interval` and `frame_cap_fps` below).

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
    /// 0 = unlimited, else 30..240 in steps of 30.
    pub fps_limit: f32,
    /// AUTO / LOW / MEDIUM / HIGH.
    pub quality: String,
    /// off / gentle / annoying.
    pub ads: String,
    pub pause_on_focus_loss: bool,
    pub teammate_waypoints: bool,
    /// The local profile name the title's account button shows (the admin's is KYTTU).
    pub username: String,
    /// The menus' colour scheme (palette.gd SCHEMES): "last_route" or "city". The original
    /// switches it with the game that is open; here LR_SCHEME or this saved value picks it.
    pub scheme: String,
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
            username: "KYTTU".into(),
            scheme: "last_route".into(),
        }
    }
}

fn path() -> Option<PathBuf> {
    super::paths::user_file("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        if let Some(v) = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        {
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
            s.fps_limit = f("fps_limit", 0.0).clamp(0.0, 240.0);
            s.pause_on_focus_loss = b("pause_on_focus_loss", true);
            s.teammate_waypoints = b("teammate_waypoints", true);
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
            if let Some(u) = v["username"].as_str() {
                s.username = u
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(24)
                    .collect::<String>()
                    .to_uppercase();
            }
        }
        s
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let v = serde_json::json!({
                "master_volume": self.master_volume, "gameplay_volume": self.gameplay_volume,
                "ui_volume": self.ui_volume, "voice_volume": self.voice_volume, "voices": self.voices,
                "subtitles": self.subtitles, "fov": self.fov, "sensitivity": self.sensitivity,
                "fullscreen": self.fullscreen, "vsync": self.vsync, "fps_limit": self.fps_limit,
                "quality": self.quality, "ads": self.ads, "pause_on_focus_loss": self.pause_on_focus_loss,
                "teammate_waypoints": self.teammate_waypoints, "username": self.username, "scheme": self.scheme,
            });
            let _ = std::fs::write(p, v.to_string());
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
    /// kept in 30..240.
    pub fn frame_cap_fps(&self) -> Option<u32> {
        if self.fps_limit < 15.0 {
            None
        } else {
            Some(self.fps_limit.round().clamp(30.0, 240.0) as u32)
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
        // the row snaps to 30..240 in steps of 30
        assert_eq!(with_limit(15.0).frame_cap_fps(), Some(30));
        assert_eq!(with_limit(30.0).frame_cap_fps(), Some(30));
        assert_eq!(with_limit(60.0).frame_cap_fps(), Some(60));
        assert_eq!(with_limit(144.0).frame_cap_fps(), Some(144));
        assert_eq!(with_limit(240.0).frame_cap_fps(), Some(240));
        // out-of-range values from a hand-edited file stay inside the clamp
        assert_eq!(with_limit(1000.0).frame_cap_fps(), Some(240));
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
}
