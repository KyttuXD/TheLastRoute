//! The original's animation system, ported (see each file's header for the Godot source it follows):
//!
//! * `clip`   -- the hunters' Blockbench rig + clips (extracted from the original's GLBs by `extract.py` into
//!              assets/hunters/anim/<id>.json), baked and sampled exactly like Godot's glTF import + Animation tracks.
//! * `player` -- AnimationPlayer / AnimationMixer: play with cross-fade, speed scale, loop / one-shot, deterministic blend.
//! * `profile` -- the per-hunter animation profile (data/render/hunter_anims.json): gait speed, stride, lean, crouch, mechanic states.
//! * `hunter` -- HunterView3D's clip choice, HunterStride, HunterFacing (body yaw, head look), HunterChaseStyle, the
//!              heavy-stun reel, BlockyFace expressions and the stun stars.
//! * `grab`   -- HunterGrab / GrabStyles: the hunter's hold on a caught runner (style pick, body points, two-bone arm IK).
//! * `held`   -- the first-person held item: HeldItemPoses plus the player's tool tweens (swing, use, taser kick, torch click).
//! * `hide`   -- the hiding spot's door cycle and the runner's crouch pose, driven by the hiding state.
//!
//! The views keep state between frames (smoothed speed, current clip, blends...), which the immutable `Game` does not
//! carry, so they live here, keyed by the hunter's slot, and advance on the game clock.

pub mod clip;
#[cfg(test)]
mod creature_tests;
pub mod grab;
pub mod held;
pub mod hide;
pub mod hunter;
pub mod player;
pub mod profile;
pub mod runner;

use hunter::HunterView;
use std::cell::RefCell;
use std::collections::HashMap;

pub struct Views {
    /// slot -> (hunter id the slot was made for, its view)
    pub by_slot: HashMap<usize, (String, HunterView)>,
    pub last_time: f32,
}

thread_local! {
    pub static VIEWS: RefCell<Views> = RefCell::new(Views { by_slot: HashMap::new(), last_time: f32::NAN });
}

/// The game-clock step since the last drawn frame (0 on the first, clamped like a process delta), and the view table.
/// A clock that went backwards (a new run) drops every view.
pub fn with_views<R>(time: f32, f: impl FnOnce(&mut Views, f32) -> R) -> R {
    VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        let delta = if v.last_time.is_nan() {
            0.0
        } else {
            time - v.last_time
        };
        if delta < 0.0 {
            v.by_slot.clear();
        }
        v.last_time = time;
        f(&mut v, delta.clamp(0.0, 0.1))
    })
}
