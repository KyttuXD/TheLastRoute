//! Shared items used by the game only: the paths every executable shares (re-exported from the lib),
//! the asset loader, the checkpoint, the admin switches and the safe room.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub use last_route_core::shared::paths;
pub use last_route_core::shared::skin_validate;

pub mod admin_flags;
pub mod assets;
pub mod checkpoint;
pub mod clock;
pub mod file_pick;
pub mod safe_room;
pub mod skin_png;
pub mod skins;
pub mod task;
