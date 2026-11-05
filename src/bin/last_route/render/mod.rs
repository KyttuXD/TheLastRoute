//! Drawing the 3D view: the lit material, mesh building, Blockbench models, the original's
//! painted surfaces, the floor's static geometry, the per-frame scene and the held item.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod ambient;
pub mod anim;
pub mod bbmodel;
pub mod builder;
pub mod catwalk;
pub mod door;
pub mod fp_hands;
pub mod hiding_cam;
pub mod hunter_cache;
pub mod inferno_deco;
pub mod lamp_flicker;
pub mod locker_occupant;
pub mod locker_shell;
pub mod prop_model;
pub mod prop_scale;
pub mod scene;
pub mod seams;
pub mod sewer_rails;
pub mod shader;
pub mod stairs;
pub mod stack_actors;
pub mod surfaces;
pub mod textures;
pub mod water;
pub mod viewmodel;
pub mod waterfall;
pub mod world_mesh;
pub mod world_stack;
