//! Everything drawn flat on the screen: the shared style, the in-run HUD and maps, the menus,
//! the settings and pause overlays, the death moment and run card, and the admin tools.

pub mod admin;
pub mod boot;
pub mod chat_box;
pub mod clipboard;
pub mod emote_wheel;
pub mod hud;
pub mod hud_bars;
pub mod map;
pub mod markers;
pub mod radar_ping;
pub mod room_panel;
pub mod teammate_markers;
pub mod inventory;
pub mod menus;
pub mod pause;
#[cfg(not(target_arch = "wasm32"))]
pub mod profile_screen;
pub mod report;
pub mod skin_preview;
pub mod mods_tab;
pub mod skin_tab;
pub mod settings_panel;
pub mod style;
pub mod text_field;
pub mod update_screen;
pub mod account_panel;
pub mod update_badge;
