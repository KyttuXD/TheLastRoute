#![allow(dead_code)]
//! Rendering core and classic world: lit shader, textures, cubes and baked worlds, first-person camera,
//! feed render target, shared room helpers, and (set C3) the classic office, building and palette.

pub mod art;
pub mod building;
pub mod camera;
pub mod mesh;
pub mod office;
pub mod palette;
pub mod prims;
pub mod shader;

#[allow(unused_imports)] // some of these are used by the tests only
pub use art::{is_emissive_name, kind_for, Textures, EMISSIVE_NAMES, RAMPS};
#[allow(unused_imports)] // some of these are used by the tests only
pub use building::Building;
#[allow(unused_imports)] // some of these are used by the tests only
pub use camera::{FeedTarget, FpCamera, Gfx, FEED_H, FEED_W, PITCH_LIMIT};
#[allow(unused_imports)] // some of these are used by the tests only
pub use mesh::{
    build_batches, cube_aabb, cube_geometry, resolve_slide, Aabb, BatchData, Cube, Group, World, WorldBuilder,
};
pub use office::{Office, OfficeConfig, OfficeVariant, StationDef};
#[allow(unused_imports)] // some of these are used by the tests only
pub use prims::{blocky_prop, room_shell, stairs, wall_with_gaps};
#[allow(unused_imports)] // some of these are used by the tests only
pub use shader::{
    pack_lights, LightSet, LitShader, Omni, PackedLights, Spot, EMISSIVE_MODE, LIT_MODE, MAX_OMNI, MAX_SPOT,
};
