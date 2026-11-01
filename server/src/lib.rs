//! Rust port of the Last Route Node.js backend (`KyttusLabApp/backend/*.mjs`).
//!
//! One module per original `.mjs` file; `server` is the HTTP router (`server.mjs`) and `main.rs` the `index.js`
//! entry point. Not ported: skins.mjs, city_maps.mjs, ws_relay.mjs, github_*.mjs, deploy_mod_pack.mjs,
//! region_agent.mjs, zip_archive.mjs (see `server/mod.rs` and `main.rs`).

pub mod util;

pub mod account_service;
pub mod game_saves;
pub mod guest_names;

pub mod hunter_id_aliases;
pub mod public_route;
pub mod room_service;

pub mod crash_reports;
pub mod moderation;
pub mod reserved_rooms;

pub mod env_file;
pub mod mod_packs;
pub mod pack_kinds;
pub mod regions;

pub mod enet_relay;

pub mod server;
