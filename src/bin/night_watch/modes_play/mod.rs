//! Playable drivers, one file per night type. Each owns its screen (menu, picture, controls, result) and calls the mode's
//! rules in `modes/`. The shell starts them from the hub (see `main.rs`).

pub mod bedtime;
pub mod dead_air;
pub mod descent;
pub mod facility;
pub mod masked_hours;
pub mod mixed_shift;
pub mod open_air;
pub mod salvage_run;
