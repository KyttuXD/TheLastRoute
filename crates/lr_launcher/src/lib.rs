//! The Last Route launcher (docs/LAUNCHER.md): a tiny, almost-never-updated native program that installs the newest
//! signed native build from the website into a per-user folder, starts it, keeps the previous build for a rollback,
//! and opens the web version when no native build can run. The program logic is here so tests can run it in-process
//! with a fake website; `main.rs` only reads the command line.

pub mod args;
pub mod config;
pub mod flow;
pub mod log;
pub mod report;

pub use config::{Config, Mode};
pub use flow::{run, Kind, Outcome};
