//! The Last Route runner's body animation, copied into lr_story (not moved: the originals stay in last_route's
//! render/anim, which this module does not depend on).
//!
//! * `clip`   -- a Blockbench rig + clips (assets/player/anim/*.json), baked and sampled like Godot's glTF import.
//! * `player` -- AnimationPlayer / AnimationMixer: play with cross-fade, speed scale, loop / one-shot.
//! * `runner` -- the runner's clip choice, pace and pose (data/player/anim.json), and the player rig loader.
//!
//! Data is read at run time from lr_story's assets folder (`assets_dir`), not embedded in the binary.

// Kept whole from last_route's runner so its clips and rigs load as they do there; lr_story uses part of it so far.
#![allow(dead_code)]

pub mod clip;
pub mod player;
pub mod runner;

use std::path::{Path, PathBuf};

/// HunterRoster.MODEL_SCALE, copied from last_route's render/anim/hunter.rs (hunter.rs is not part of lr_story).
pub const MODEL_SCALE: f32 = 1.5;

/// lr_story's assets folder (the same root props and textures use).
pub fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets")
}

/// The bytes of one file under the assets folder, by its path there (e.g. "player/anim/player_mannequin.json").
pub fn asset(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(assets_dir().join(rel)).ok()
}

#[cfg(test)]
mod tests {
    use super::clip::Rig;
    use super::player::AnimPlayer;
    use super::runner;

    /// Samples the walk clip at 30 fps from 0 to its end: every bone gets a finite translation and rotation at every
    /// frame, and the clip really moves the body (some rotation leaves its rest value).
    #[test]
    fn walk_clip_sampled_at_30fps_is_finite_for_every_bone() {
        let rig: std::rc::Rc<Rig> = runner::player_rig(false).expect("player rig asset is in assets");
        let walk = runner::config().clips.get("walk").expect("anim.json maps walk to a clip").clone();
        let length = rig.length(&walk);
        assert!(length > 0.0, "walk clip {walk} has a length");
        let frames = (length * 30.0).ceil() as usize;
        let mut p = AnimPlayer::default();
        p.play(&rig, &walk, 0.0);
        let mut moved = false;
        for f in 0..=frames {
            p.seek(f as f32 / 30.0);
            let pose = p.pose(&rig);
            assert_eq!(pose.len(), rig.bones.len());
            for (bone, l) in rig.bones.iter().zip(&pose) {
                assert!(l.t.to_array().iter().all(|v| v.is_finite()), "{} translation at frame {f}", bone.name);
                assert!(l.r.to_array().iter().all(|v| v.is_finite()), "{} rotation at frame {f}", bone.name);
                if l.r.angle_between(bone.rest_r) > 1e-3 {
                    moved = true;
                }
            }
        }
        assert!(moved, "the walk clip moves at least one bone");
    }

    /// The five staff skins (Felix, Renata, Owen, Priya, Dia) are placeholder atlases copied from the mannequin's. Each
    /// file exists under player/staff/, starts with the PNG signature, and decodes at the mannequin atlas's size. The
    /// renderer does not use them yet: NPCs still draw the mannequin.
    #[test]
    fn five_staff_atlases_exist_and_decode_at_the_mannequin_size() {
        use macroquad::prelude::{Image, ImageFormat};
        let decode = |rel: &str| -> Image {
            let bytes = super::asset(rel).unwrap_or_else(|| panic!("{rel} is missing from the assets folder"));
            assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a], "{rel} is not a PNG");
            Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).ok().unwrap_or_else(|| panic!("{rel} does not decode"))
        };
        let mannequin = decode("player/player_mannequin_atlas.png");
        let size = (mannequin.width(), mannequin.height());
        for name in ["felix", "renata", "owen", "priya", "dia"] {
            let rel = format!("player/staff/{name}_atlas.png");
            let img = decode(&rel);
            assert_eq!((img.width(), img.height()), size, "{rel} is not the mannequin atlas's size");
        }
    }
}
