//! The world a run takes place in: the seeded random generator, the room catalogue, Godot's
//! sort order and the floor generator itself.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod catalog;
pub mod door_clearance;
pub mod facility;
pub mod fixtures;
pub mod godot_sort;
pub mod inferno;
pub mod inferno_caves;
pub mod inferno_terrain;
pub mod infinite;
pub mod sewer_emergency;
pub mod sewer_props;
pub mod level_links;
pub mod rng;
pub mod route;
pub mod vertical;
pub mod flight;
pub mod flight_links;
#[cfg(test)]
mod stack_contract_tests;

pub use facility::{Cell, Facility};

/// Texture set used for a room region (the finite route's rooms carry catalogue regions;
/// the floor/wall pictures are the original's region swatches). Halls use the maze set.
pub fn region_texture(region: &str) -> &'static str {
    match region {
        "arcade" | "circus" => "carnival",
        "maintenance" => "prison",
        "office" => "lab",
        "mansion" => "mansion",
        "nether" => "inferno",
        "beach" => "beach",
        "garden" => "greenhouse",
        "hotel" => "hotel",
        "apartment" => "apartment",
        "gothic" => "gothic",
        "hospital" => "hospital",
        "mall" => "mall",
        _ => "maze",
    }
}

pub const TEXTURE_SETS: [&str; 13] = [
    "maze",
    "mansion",
    "hotel",
    "apartment",
    "prison",
    "gothic",
    "beach",
    "carnival",
    "greenhouse",
    "inferno",
    "lab",
    "hospital",
    "mall",
];
