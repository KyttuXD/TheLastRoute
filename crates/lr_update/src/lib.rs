//! Native build delivery for The Last Route (docs/LAUNCHER.md). The launcher (`crates/lr_launcher`) and the in-game
//! updater both use it, so they cannot disagree about what a valid build is.
//!
//! * `manifest`: `latest.json` and `manifest.json` (schema `lr.native/1`) and every check made on them.
//! * `verify`: sha256 and ed25519 (the public keys are compiled in from `keys/trusted.txt`).
//! * `http`, `fetch`: reading the website (https through rustls, streaming, progress).
//! * `install`: download, verify and unpack a build into a fresh folder.
//! * `state`: `state.json`, rollback bookkeeping, garbage collection, the install lock.
//! * `paths`, `osver`, `browser`: the per-user folder, the OS version, opening the default browser.
//!
//! No macroquad and no game code here: building this never compiles the game.

pub mod browser;
pub mod fetch;
pub mod http;
pub mod install;
pub mod manifest;
pub mod osver;
pub mod paths;
pub mod state;
pub mod verify;

/// A tiny local website for tests (loopback, signed trees, injectable faults). Not part of a normal build.
#[cfg(any(test, feature = "testsite"))]
pub mod testsite;

#[cfg(test)]
mod e2e_tests;
