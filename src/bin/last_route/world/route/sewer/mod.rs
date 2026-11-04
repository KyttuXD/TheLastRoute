//! The sewer levels -1 and -2 of the Infinite route (`SewerRouteGenerator`, sewer_route_generator.gd).
//!
//! Stage 3d-i: the layout and grid. `core` holds the port; the seams (fixtures, pickups, hunter spawns and
//! breakables, which need the seal planner and the stage 2/3c streams) are functions there that return
//! empty lists. The Inferno stage (3e) uses `ramps_at` and `ramp_cells` with the same contract as Godot's
//! `sewer().ramps_at(x, z, level)` and `sewer().ramp_cells(x, z, level)`.

pub mod breach;
pub mod core;
pub mod fixtures;
pub mod hunters;
pub mod level2;
pub mod pickups;
pub mod rooms;
pub mod seals;
pub mod water;

#[allow(unused_imports)] // re-exported API
pub use core::{
    chunk, chunk_id, falls_between, is_chunk_allowed, ramp_between, ramp_cells, ramps_at, trench_cells_below,
    Falls, Gate, Inputs, LadderLink, Ramp, RampCell, RampLink, SewerChunk, SewerProp, SewerRoom, SewerWell,
};
