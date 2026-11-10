//! Night Watch hunters: the bbmodel rigs with the Night Watch pose clips ("nw/<pose>") and the actors the modes
//! place. Section 3.8 of docs/NIGHT_WATCH_3D_PORT.md.
//!
//! * `bbmodel`    -- copy of lr_story's Blockbench reader (`Model::build_posed`).
//! * `anim`       -- copy of lr_story's rig / clip / player (no runner).
//! * `poses`      -- night_watch_poses.json compiled into clips on a rig (pure).
//! * `pose_state` -- `PoseState`, state -> pose table, the vent crawl state machine (pure).
//! * `place`      -- yaw helpers, hunter scale, stand slots (pure).
//! * `rig`        -- `HunterLib` (lazy loading) and `Actor` (CPU skinning, lit draw).
//! * `nav`        -- per-map navigation graph, deterministic A*, `NavWalker` (pure; API notes at the top of the file).
//! * `behaviour`  -- idle behaviour (stare / task / wait), `Crowd`, `ActorDriver` (pure but for `ActorDriver`).
#![allow(dead_code)]

pub mod anim;
pub mod bbmodel;
pub mod behaviour;
pub mod nav;
pub mod place;
pub mod pose_state;
pub mod poses;
pub mod rig;

pub use anim::MODEL_SCALE;
pub use behaviour::{ActorDriver, Agent, BehaviourData, BehaviourEvent, Crowd};
pub use nav::{NavKind, NavMap, NavNode, NavWalker, Pace};
pub use place::{face, hunter_scale, slot_offset, stand_spot, yaw_of_dir, yaw_toward};
pub use pose_state::{pose_for_key, pose_for_state, PoseState, VentPose, VentStep};
pub use poses::{compile_nw_poses, compile_nw_poses_for};
pub use rig::{bone_matrices, Actor, HunterLib};
