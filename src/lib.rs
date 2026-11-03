//! The shared core of the Last Route executables: the mod pack types (`packs`, which `lr_server` builds
//! against alone) and the paths every binary shares (`shared::paths`). The game (`src/bin/last_route`)
//! owns the run rules, the world, the hunters, rendering, audio and content; the Night Watch executable
//! (`src/bin/night_watch`) and the account server (`src/bin/lr_server`) each keep their own code in
//! their bin folder.

pub mod packs;
pub mod shared;
