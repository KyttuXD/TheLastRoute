//! Copy of lr_story's body animation (clip + player; the runner module is not needed here). The rig text comes from
//! `crate::assets::rig_text` instead of lr_story's assets folder.
//!
//! * `clip`   -- a Blockbench rig + clips, baked and sampled like Godot's glTF import.
//! * `player` -- AnimationPlayer / AnimationMixer: play with cross-fade, speed scale, loop / one-shot.
#![allow(dead_code)]

pub mod clip;
pub mod player;

/// HunterRoster.MODEL_SCALE: the humanoid hunters are 1.5x (about 2.58 m).
pub const MODEL_SCALE: f32 = 1.5;
