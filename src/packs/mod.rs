//! Mod pack types that need nothing from the game. `lr_server` builds against this module alone: it depends
//! only on `serde_json` and the standard library, never on `game`, `world`, `render`, `hunters`, `audio`,
//! `config` or `shared`. The game's `content::packs` loads packs into the content registry and re-exports
//! these types, so the game's paths do not change.

pub mod regions;

use std::path::PathBuf;

/// One loaded mod pack (`pack.json`, `lr.pack/1`).
#[derive(Clone, Debug)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub dir: PathBuf,
    /// Content ids this pack added or replaced.
    pub provides: Vec<String>,
}
