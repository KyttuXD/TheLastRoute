//! The mannequin body that NPCs are drawn with, and the player's own body in the third-person view. The rig and clips
//! come from anim (the runner's pose: `runner::RunnerView`), the mesh from props/bbmodel.rs (`Model::build_posed`, the
//! bone-posed mesh builder), and the body's size from data/player/models.json (`runner.metres_per_unit`).
//!
//! Loading and drawing need a GL context. The pure parts (bone matrices, facing, the third-person camera offset, the
//! metres per unit) are tested without a window. A body that fails to load is reported once on stderr, and the caller
//! draws its placeholder cube instead.

use crate::anim::asset;
use crate::anim::clip::Rig;
use crate::anim::player::Local;
use crate::anim::runner::{self, RunnerState, RunnerView};
use crate::body::Body;
use crate::model::BodyChoice;
use crate::npc_ai::NpcAction;
use crate::npc_anim::{self, PoseBlend};
use crate::props::bbmodel::{try_tex, Model};
use macroquad::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;

/// The atlas a body is painted on unless its NPC names a skin from a pack's skins folder (see `Figures::draw_npc`).
const ATLAS: &str = "player/player_mannequin_atlas.png";

/// What a loaded body is keyed by: its body and the name of its skin (None: the default atlas).
type BodyKey = (BodyChoice, Option<String>);

/// The third-person camera sits this far behind the body's facing (metres)...
pub const TP_BACK_M: f32 = 3.0;
/// ...and this far above the body's feet (metres).
pub const TP_UP_M: f32 = 0.5;
/// The height the third-person camera looks at on the body (metres above the feet, about the chest).
pub const TP_LOOK_M: f32 = 1.2;

/// Where the third-person camera sits relative to the body's feet, for a facing given as unit (x, z) on the ground.
pub fn third_person_offset(facing: (f32, f32)) -> Vec3 {
    vec3(-facing.0 * TP_BACK_M, TP_UP_M, -facing.1 * TP_BACK_M)
}

/// The yaw that turns a body standing at `from` to face `to`, on the ground plane. The convention is body.rs's: yaw 0
/// faces -z, and `Body::forward2` is (-sin yaw, -cos yaw).
pub fn yaw_toward(from: (f32, f32), to: (f32, f32)) -> f32 {
    (from.0 - to.0).atan2(from.1 - to.1)
}

/// Model-space matrices of the rig's bones for a local pose, in rig bone order. A bone's matrix is its parent's matrix
/// times its own local transform, so the rest pose puts every bone where the rig authored it. The rig lists parents
/// before their children (a test checks it); a bone whose parent is not yet in the list takes its local transform alone.
pub fn bone_matrices(rig: &Rig, pose: &[Local]) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(rig.bones.len());
    for (k, bone) in rig.bones.iter().enumerate() {
        let local = match pose.get(k) {
            Some(l) => Mat4::from_rotation_translation(l.r, l.t),
            None => Mat4::from_rotation_translation(bone.rest_r, bone.rest_t),
        };
        let m = match bone.parent {
            Some(p) if p < out.len() => out[p] * local,
            _ => local,
        };
        out.push(m);
    }
    out
}

/// How opaque the body is drawn for a crouch fade. `RunnerView::fade` is 0 standing and eases to the config's
/// `crouch_alpha` while crouched; it is the share of the body that fades away, so the opacity is one minus it.
pub fn body_opacity(fade: f32) -> f32 {
    (1.0 - fade).clamp(0.0, 1.0)
}

/// The animation state of the player's own body: moving, sprinting and crouching as the body has them, and in the air
/// either rising (the jump clip) or falling (the fall clip).
pub fn player_state(body: &Body, speed: f32) -> RunnerState {
    RunnerState {
        moving: body.moving,
        sprinting: body.sprinting,
        crouching: body.crouching,
        airborne_up: !body.grounded && body.rising(),
        airborne_down: !body.grounded && !body.rising(),
        speed,
        ..RunnerState::default()
    }
}

/// Metres per bbmodel unit of the body (data/player/models.json, `runner.metres_per_unit`: the head top at 113.92 units
/// is 1.9 m). None when the file is missing or holds no such number.
pub fn metres_per_unit() -> Option<f32> {
    let bytes = asset("data/player/models.json")?;
    let j: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    j["runner"]["metres_per_unit"].as_f64().map(|m| m as f32).filter(|m| *m > 0.0)
}

/// One loaded body: its posed model (mesh, outliner groups and atlas), its rig, and the metres per unit it is drawn at.
pub struct Mannequin {
    pub model: Model,
    pub rig: Rc<Rig>,
    pub metres_per_unit: f32,
}

impl Mannequin {
    /// Reads the body's model, rig, scale and atlas (`skin`, or the default atlas when None). None when any of them is
    /// missing or does not parse.
    pub fn load(choice: BodyChoice, skin: Option<&Texture2D>) -> Option<Mannequin> {
        let rig = runner::player_rig(choice.slim())?;
        let metres_per_unit = metres_per_unit()?;
        let file = match choice {
            BodyChoice::Blocky => "player/player_mannequin.bbmodel",
            BodyChoice::BlockySlim => "player/player_mannequin_slim.bbmodel",
        };
        let text = String::from_utf8(asset(file)?).ok()?;
        let atlas = match skin {
            Some(tex) => tex.clone(),
            None => try_tex(&asset(ATLAS)?)?,
        };
        let find = |name: &str| asset(&format!("player/{name}"));
        let model = Model::parse(&text, Some(atlas), &find, None)?;
        Some(Mannequin { model, rig, metres_per_unit })
    }

    /// Draws the body standing with its feet at `feet`, turned to `yaw`, posed by `pose` (local transforms in rig bone
    /// order) and drawn at `opacity` (1 solid; a crouching body is fainter, see `body_opacity`). Its vertices go
    /// through the root (bbmodel units to metres, then yaw, then the feet position).
    pub fn draw_at(&self, pose: &[Local], feet: Vec3, yaw: f32, opacity: f32, tint: f32) {
        let root = Mat4::from_translation(feet)
            * Mat4::from_rotation_y(yaw)
            * Mat4::from_scale(Vec3::splat(self.metres_per_unit));
        let bones = bone_matrices(&self.rig, pose);
        let bone_of = |k: usize| {
            self.model
                .groups
                .get(k)
                .and_then(|g| self.rig.bone(&g.name))
                .map(|b| bones[b])
        };
        let (mut verts, mut idx) = (Vec::new(), Vec::new());
        self.model.build_posed(&mut verts, &mut idx, root, &bone_of, Vec2::ZERO, Color::new(1.0, 1.0, 1.0, opacity));
        if !verts.is_empty() {
            draw_mesh(&Mesh {
                vertices: verts,
                indices: idx,
                texture: Some(self.model.texture.clone()),
            });
            // A white tint (0 none, 1 strong): the same posed shape again, untextured and white.
            let tint = tint.clamp(0.0, 1.0);
            if tint > 0.0 {
                let white = Color::new(1.0, 1.0, 1.0, 0.6 * tint * opacity);
                let (mut v2, mut i2) = (Vec::new(), Vec::new());
                self.model.build_posed(&mut v2, &mut i2, root, &bone_of, Vec2::ZERO, white);
                draw_mesh(&Mesh { vertices: v2, indices: i2, texture: None });
            }
        }
    }
}

/// Loads a body (with its skin's atlas, or the default) on first use. A body that fails is reported once; the cache keeps
/// the failure so it is not retried.
fn load_once<'a>(bodies: &'a mut HashMap<BodyKey, Option<Mannequin>>, key: BodyKey, skin: Option<&Texture2D>) -> &'a Option<Mannequin> {
    bodies.entry(key.clone()).or_insert_with(|| {
        let loaded = Mannequin::load(key.0, skin);
        if loaded.is_none() {
            let which = match &key.1 {
                Some(name) => format!("the {} body with skin '{name}'", key.0.name()),
                None => format!("the {} body", key.0.name()),
            };
            eprintln!("lr_story: {which} did not load; its figures are drawn as cubes");
        }
        loaded
    })
}

/// The figures a run draws: each body loaded once per skin, the animation state of every NPC on screen, and the player's
/// own third-person body. The window loop keeps it (`Game` holds game state only).
#[derive(Default)]
pub struct Figures {
    bodies: HashMap<BodyKey, Option<Mannequin>>,
    /// Keyed by the map id and the NPC's tile.
    npcs: HashMap<(String, i32, i32), RunnerView>,
    /// Keyed by the NPC agent's key ("map@x,y"): its animation state and its action pose blend.
    agents: HashMap<String, (RunnerView, PoseBlend)>,
    player: Option<RunnerView>,
}

/// The animation state of an NPC agent: walking at the NPCs' pace while it moves, standing in the idle clip otherwise.
pub fn agent_state(moving: bool) -> RunnerState {
    RunnerState { moving, speed: if moving { AGENT_WALK_MPS } else { 0.0 }, ..RunnerState::default() }
}

/// The ground speed an NPC agent walks at (m/s); npc_ai.rs moves them at the same pace.
pub const AGENT_WALK_MPS: f32 = 1.4;

impl Figures {
    /// Draws the NPC on tile `tile` of `map` as its mannequin, standing still in the idle clip, turned to face `toward`
    /// (the player's feet, metres). `centre` is the tile centre in metres. `skin` is the NPC's skin: its name and its
    /// texture (None: the default atlas). `tint` whitens it for a hit flash (0 none, 1 strong). False when the body does not load, so the caller draws the cube.
    pub fn draw_npc(
        &mut self,
        map: &str,
        tile: (i32, i32),
        centre: (f32, f32),
        choice: BodyChoice,
        skin: Option<(&str, &Texture2D)>,
        toward: (f32, f32),
        dt: f32,
        tint: f32,
    ) -> bool {
        let key: BodyKey = (choice, skin.map(|(name, _)| name.to_string()));
        let Some(m) = load_once(&mut self.bodies, key, skin.map(|(_, tex)| tex)).as_ref() else {
            return false;
        };
        let view = self
            .npcs
            .entry((map.to_string(), tile.0, tile.1))
            .or_insert_with(|| RunnerView::new(m.rig.clone()));
        view.update(&RunnerState::default(), dt);
        let pose = view.locals();
        m.draw_at(&pose, vec3(centre.0, 0.0, centre.1), yaw_toward(centre, toward), body_opacity(view.fade), tint);
        true
    }

    /// Draws an NPC agent (npc_ai.rs) at a free position `pos` (metres) and `yaw`, in the pose of its `action`: the walk
    /// clip while it `moving`, else the idle clip with the action's pose layered on (npc_anim.rs). `key` is the agent's
    /// key in `Game::npcs`. False when the body does not load, so the caller draws the cube.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_agent(
        &mut self,
        key: &str,
        pos: (f32, f32),
        yaw: f32,
        action: NpcAction,
        moving: bool,
        choice: BodyChoice,
        skin: Option<(&str, &Texture2D)>,
        dt: f32,
        tint: f32,
    ) -> bool {
        let bkey: BodyKey = (choice, skin.map(|(name, _)| name.to_string()));
        let Some(m) = load_once(&mut self.bodies, bkey, skin.map(|(_, tex)| tex)).as_ref() else {
            return false;
        };
        let (view, blend) = self
            .agents
            .entry(key.to_string())
            .or_insert_with(|| (RunnerView::new(m.rig.clone()), PoseBlend::default()));
        view.update(&agent_state(moving && npc_anim::uses_walk_clip(action, true)), dt);
        let mut pose = view.locals();
        let delta = blend.step(action, moving, dt);
        npc_anim::apply(&m.rig, &mut pose, &delta);
        m.draw_at(&pose, vec3(pos.0, 0.0, pos.1), yaw, body_opacity(view.fade), tint);
        true
    }

    /// Draws the player's own body at the body's feet, facing its way. It walks while the body moves, and is in the air
    /// while it is off the ground. False when the body does not load, so the caller draws a cube.
    pub fn draw_player(&mut self, body: &Body, choice: BodyChoice, skin: Option<(&str, &Texture2D)>, dt: f32) -> bool {
        let key: BodyKey = (choice, skin.map(|(name, _)| name.to_string()));
        let Some(m) = load_once(&mut self.bodies, key, skin.map(|(_, tex)| tex)).as_ref() else {
            return false;
        };
        let view = self.player.get_or_insert_with(|| RunnerView::new(m.rig.clone()));
        let speed = view.speed_from(body.pos, dt);
        view.update(&player_state(body, speed), dt);
        let pose = view.locals();
        m.draw_at(&pose, vec3(body.pos.0, body.feet_y, body.pos.1), body.yaw, body_opacity(view.fade), 0.0);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finite(m: &Mat4) -> bool {
        m.to_cols_array().iter().all(|v| v.is_finite())
    }

    #[test]
    fn npc_idle_pose_gives_finite_transforms_for_both_bodies() {
        for choice in [BodyChoice::Blocky, BodyChoice::BlockySlim] {
            let mut view = RunnerView::load(choice.slim()).expect("player rig asset is in assets");
            view.update(&RunnerState::default(), 1.0 / 30.0);
            let pose = view.locals();
            let bones = bone_matrices(&view.rig, &pose);
            assert_eq!(bones.len(), view.rig.bones.len(), "{}", choice.name());
            for (bone, m) in view.rig.bones.iter().zip(&bones) {
                assert!(finite(m), "{} {} is not finite", choice.name(), bone.name);
            }
            let scale = metres_per_unit().expect("models.json gives the metres per unit");
            let root = Mat4::from_translation(vec3(2.0, 0.0, 3.0))
                * Mat4::from_rotation_y(0.7)
                * Mat4::from_scale(Vec3::splat(scale));
            let head = view.rig.bone("Head").expect("the rig has a Head bone");
            let world = root * bones[head] * Vec4::new(0.0, 0.0, 0.0, 1.0);
            assert!(world.to_array().iter().all(|v| v.is_finite()), "the head joint is finite");
            // A standing body's head joint is well above the feet and well below two metres.
            let height = (bones[head] * Vec4::new(0.0, 0.0, 0.0, 1.0)).y * scale;
            assert!((1.0..2.0).contains(&height), "{} head joint at {height} m", choice.name());
        }
    }

    #[test]
    fn the_player_body_chooses_jump_fall_sprint_and_crouch_clips() {
        use crate::anim::runner::{clip_for, config};
        let mut b = Body::new((1.0, 1.0), 0.0);
        let clip = |b: &Body| clip_for(&player_state(b, 3.0)).clip;
        b.moving = true;
        assert_eq!(clip(&b), config().clips["walk"]);
        b.sprinting = true;
        assert_eq!(clip(&b), config().clips["sprint"]);
        b.crouching = true;
        b.sprinting = false;
        assert_eq!(clip(&b), config().clips["idle"], "crouching plays the idle clip");
        b.crouching = false;
        // Rising: the jump clip. Falling (after the top of the jump): the fall clip.
        b.update(0.016, &crate::body::Input { jump: true, ..Default::default() }, &|_, _| false);
        assert!(!b.grounded && b.rising());
        assert_eq!(clip(&b), config().clips["jump"]);
        for _ in 0..20 {
            b.update(0.016, &crate::body::Input::default(), &|_, _| false);
        }
        assert!(!b.grounded && !b.rising(), "falling now");
        assert_eq!(clip(&b), config().clips["fall"]);
    }

    #[test]
    fn a_crouching_body_is_drawn_fainter() {
        assert_eq!(body_opacity(0.0), 1.0);
        let crouched = body_opacity(crate::anim::runner::config().crouch_alpha);
        assert!(crouched < 1.0 && crouched > 0.0, "{crouched}");
        assert_eq!(body_opacity(2.0), 0.0);
        assert_eq!(body_opacity(-1.0), 1.0);
    }

    /// Build order 9: the walk clip played at 30 fps for four seconds gives finite transforms on every bone of both
    /// bodies, and the bones actually move (the pose is not frozen at rest).
    #[test]
    fn the_walk_clip_at_thirty_fps_gives_finite_transforms() {
        for choice in [BodyChoice::Blocky, BodyChoice::BlockySlim] {
            let mut view = RunnerView::load(choice.slim()).expect("player rig asset is in assets");
            let walk = RunnerState { moving: true, speed: 4.0, ..RunnerState::default() };
            let mut first: Option<Vec<Mat4>> = None;
            let mut moved = false;
            for frame in 0..120 {
                view.update(&walk, 1.0 / 30.0);
                let bones = bone_matrices(&view.rig, &view.locals());
                assert_eq!(bones.len(), view.rig.bones.len());
                for (bone, m) in view.rig.bones.iter().zip(&bones) {
                    assert!(finite(m), "{} frame {frame}: {} is not finite", choice.name(), bone.name);
                }
                match &first {
                    None => first = Some(bones),
                    Some(f) => moved |= f.iter().zip(&bones).any(|(a, b)| a.abs_diff_eq(*b, 1e-4) == false),
                }
            }
            assert!(moved, "{}: the walk clip never moved a bone", choice.name());
        }
    }

    #[test]
    fn rigs_list_every_parent_before_its_children() {
        for choice in [BodyChoice::Blocky, BodyChoice::BlockySlim] {
            let rig = runner::player_rig(choice.slim()).expect("player rig asset is in assets");
            for (k, bone) in rig.bones.iter().enumerate() {
                if let Some(p) = bone.parent {
                    assert!(p < k, "{}: {} comes before its parent", choice.name(), bone.name);
                }
            }
        }
    }

    #[test]
    fn an_agent_walks_only_while_it_moves_and_standing_actions_stay_in_idle() {
        use crate::anim::runner::{clip_for, config};
        assert_eq!(clip_for(&agent_state(true)).clip, config().clips["walk"]);
        assert_eq!(clip_for(&agent_state(false)).clip, config().clips["idle"]);
        assert_eq!(agent_state(false).speed, 0.0);
        assert!(npc_anim::uses_walk_clip(NpcAction::Walk, true));
        assert!(!npc_anim::uses_walk_clip(NpcAction::Sit, true));
    }

    #[test]
    fn metres_per_unit_is_the_runner_scale() {
        let m = metres_per_unit().expect("models.json gives the metres per unit");
        assert!((m - 1.067 / 64.0).abs() < 1e-6, "{m}");
    }

    #[test]
    fn yaw_toward_turns_a_body_to_face_its_target() {
        for (to, want) in [((0.0, 5.0), (0.0, 1.0)), ((3.0, 4.0), (0.6, 0.8)), ((-2.0, 0.0), (-1.0, 0.0))] {
            let yaw = yaw_toward((0.0, 0.0), to);
            let facing = Body::new((0.0, 0.0), yaw).forward2();
            assert!((facing.0 - want.0).abs() < 1e-5 && (facing.1 - want.1).abs() < 1e-5, "{to:?}: {facing:?}");
        }
    }

    #[test]
    fn third_person_camera_sits_behind_the_facing() {
        for yaw in [0.0f32, 0.9, std::f32::consts::PI, -2.2, 5.0] {
            let body = Body::new((10.0, 20.0), yaw);
            let facing = body.forward2();
            let off = third_person_offset(facing);
            assert!((off.y - TP_UP_M).abs() < 1e-6, "0.5 m up");
            let behind = off.x * facing.0 + off.z * facing.1;
            assert!(behind < 0.0, "yaw {yaw}: the camera is in front ({behind})");
            assert!((behind + TP_BACK_M).abs() < 1e-5, "yaw {yaw}: the camera is 3 m back ({behind})");
            let horizontal = (off.x * off.x + off.z * off.z).sqrt();
            assert!((horizontal - TP_BACK_M).abs() < 1e-5, "yaw {yaw}: {horizontal} m away");
        }
    }
}
