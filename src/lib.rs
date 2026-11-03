//! The shared core of the Last Route games: the run rules, the world and its generators, the hunters,
//! rendering, audio and content loading. KYTTU: The Last Route (`src/bin/last_route`) and the Night Watch
//! executable (`src/bin/night_watch`) build on it; each game's own front end lives in its bin folder.

pub mod audio;
pub mod config;
pub mod content;
pub mod game;
pub mod hunters;
pub mod render;
pub mod shared;
pub mod world;
