//! The figure a remote runner wears in the 3D view. The body is the blocky mannequin the game loads for the runners and
//! the NPC teammates (`Assets::npc_body`, parsed at the runner's height), textured with the mannequin atlas by default.
//! A runner whose skin is on disk (see `Assets::skin_texture`) wears that skin's PNG on the same model. A skin that is
//! DEFAULT, or not known yet, draws the atlas.
//!
//! The pose is data: `data/player/models.json` names the body, its standing swing and each limb bone's swing amplitude,
//! side and joint limit (Godot: game/config/player_models.gd, the mannequin rig in assets/hunters_blocky/player_mannequin).
//! The limbs bend about their bone origins, like the hunters' rig, and every angle is clamped to that bone's limit.

use super::world_sync::{RemoteView, SKIN_DEFAULT};
use crate::content::embedded;
use crate::game::Game;
use crate::render::bbmodel::Group;
use crate::render::scene;
use crate::render::shader;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use std::sync::OnceLock;

/// The data file (under src/bin/last_route/data) that holds the runner body and its pose.
const MODELS_FILE: &str = "player/models.json";

/// Which texture a runner's body draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The mannequin atlas (the default skin, or a skin not known yet).
    Atlas,
    /// The cached skin with this id.
    Skin(String),
}

/// One limb bone of the runner body, from models.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    /// The outliner group name, e.g. "R-Arm".
    pub name: String,
    /// Which way the limb swings with the stride: +1 or -1.
    pub side: f32,
    /// Swing amplitude in radians at full stride.
    pub amp: f32,
    /// Joint limit: the largest bend either way, in radians.
    pub max_rad: f32,
}

/// How NPC teammates (bots) wear the runner body, from the `bots` section of models.json.
#[derive(Clone, Debug, PartialEq)]
pub struct BotSpec {
    /// The skin ids a bot may wear. A bot picks one by a hash of its guest name; an empty list draws the atlas.
    pub skins: Vec<String>,
    /// Swing amplitude of a walking bot, the same units as a runner's `amp`.
    pub walk_amp: f32,
    /// Radians of swing per metre walked (a bot's stride is in metres).
    pub stride_swing: f32,
}

/// The runner body and its pose, as models.json states them.
#[derive(Clone, Debug, PartialEq)]
pub struct RunnerPose {
    /// The body id under `bodies`.
    pub body: String,
    /// The body's Blockbench model and atlas, as asset paths.
    pub model: String,
    pub atlas: String,
    /// The swing and amplitude a runner uses when no stride is known (it stands still).
    pub standing_swing: f32,
    pub standing_amp: f32,
    pub bones: Vec<Bone>,
    /// How NPC teammates wear and swing the same body.
    pub bots: BotSpec,
}

impl RunnerPose {
    /// Reads models.json. Every number is required; a missing or bad one is an error, never a silent default.
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let body = v["runner"]["body"].as_str().ok_or("runner.body is missing")?.to_string();
        let standing = &v["runner"]["standing"];
        let standing_swing = num(standing, "swing")?;
        let standing_amp = num(standing, "amp")?;
        let spec = &v["bodies"][body.as_str()];
        let model = spec["model"].as_str().ok_or("bodies.<body>.model is missing")?.to_string();
        let atlas = spec["atlas"].as_str().ok_or("bodies.<body>.atlas is missing")?.to_string();
        let map = spec["bones"].as_object().ok_or("bodies.<body>.bones is missing")?;
        let mut bones = Vec::with_capacity(map.len());
        for (name, b) in map {
            bones.push(Bone {
                name: name.clone(),
                side: num(b, "side")?,
                amp: num(b, "amp")?,
                max_rad: num(b, "max_deg")?.to_radians(),
            });
        }
        bones.sort_by(|a, b| a.name.cmp(&b.name));
        let bots_v = &v["bots"];
        let skins = bots_v["skins"]
            .as_array()
            .ok_or("bots.skins is missing or not a list")?
            .iter()
            .map(|s| s.as_str().map(str::to_string).ok_or("bots.skins holds a value that is not a string"))
            .collect::<Result<Vec<_>, _>>()?;
        let bots = BotSpec {
            skins,
            walk_amp: num(bots_v, "walk_amp")?,
            stride_swing: num(bots_v, "stride_swing")?,
        };
        Ok(RunnerPose {
            body,
            model,
            atlas,
            standing_swing,
            standing_amp,
            bones,
            bots,
        })
    }

    /// The bend of one bone at `swing` (a stride phase in radians) and `amp`, clamped to its joint limit. None when the
    /// bone is not a posed limb.
    pub fn angle(&self, bone: &str, swing: f32, amp: f32) -> Option<f32> {
        let b = self.bones.iter().find(|b| b.name == bone)?;
        Some((swing.sin() * amp * b.amp * b.side).clamp(-b.max_rad, b.max_rad))
    }
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v[key].as_f64().map(|n| n as f32).ok_or_else(|| format!("{key} is missing or not a number"))
}

/// The pose from data/player/models.json, read once. None when the file is missing or invalid: the runners then stand
/// in their rest pose (the error is printed once).
fn runner_pose() -> Option<&'static RunnerPose> {
    static POSE: OnceLock<Option<RunnerPose>> = OnceLock::new();
    POSE.get_or_init(|| {
        let (path, text) = embedded::data_under(MODELS_FILE).next()?;
        match RunnerPose::parse(text) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("{path}: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// The transform of every outliner group for a runner at `swing`/`amp`: a limb bone turns about its origin by its
/// clamped angle; every other group is None (the rig leaves it at rest, riding its parent).
fn bone_transforms(pose: Option<&RunnerPose>, groups: &[Group], swing: f32, amp: f32) -> Vec<Option<Mat4>> {
    groups
        .iter()
        .map(|g| {
            let angle = pose?.angle(&g.name, swing, amp)?;
            Some(Mat4::from_translation(g.origin) * Mat4::from_rotation_x(angle))
        })
        .collect()
}

/// The texture choice for a runner: DEFAULT is the atlas; a skin id is used only when `has_skin` says its texture is
/// ready; any other id falls back to the atlas.
pub fn pick(skin: &str, has_skin: &dyn Fn(&str) -> bool) -> Pick {
    if skin != SKIN_DEFAULT && has_skin(skin) {
        Pick::Skin(skin.to_string())
    } else {
        Pick::Atlas
    }
}

/// Draws the remote runners in the 3D view, lit like the scene. Each one is the mannequin at its position and yaw,
/// wearing its skin's texture. Draws nothing when `remotes` is empty, so a single-player frame is unchanged.
pub fn draw_runners(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    if remotes.is_empty() {
        return;
    }
    set_camera(&scene::camera(g, rt));
    gl_use_material(mat);
    shader::apply(mat, &scene::lighting(g), &g.world.lamps);
    match a.npc_body.as_ref() {
        Some(body) => {
            let pose = runner_pose();
            let (swing, amp) = pose.map(|p| (p.standing_swing, p.standing_amp)).unwrap_or((0.0, 0.0));
            let bones = bone_transforms(pose, &body.groups, swing, amp);
            for r in remotes {
                let texture = match pick(&r.skin, &|id| a.skin_texture(id).is_some()) {
                    Pick::Skin(id) => a.skin_texture(&id).unwrap_or_else(|| body.texture.clone()),
                    Pick::Atlas => body.texture.clone(),
                };
                let root = Mat4::from_translation(vec3(r.x, 0.0, r.y))
                    * Mat4::from_rotation_y(r.yaw)
                    * Mat4::from_scale(Vec3::splat(body.scale));
                let mut verts = vec![];
                let mut idx = vec![];
                body.build_posed(&mut verts, &mut idx, root, &|k: usize| bones[k], Vec2::ZERO, WHITE);
                draw_mesh(&Mesh {
                    vertices: verts,
                    indices: idx,
                    texture: Some(texture),
                });
            }
        }
        None => {
            // The mannequin failed to load: a plain grey post keeps the runner visible in the 3D view.
            let mut b = crate::render::builder::Builder::new(Some(a.white.clone()));
            for r in remotes {
                b.cube(vec3(r.x, 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, GRAY, 2.0);
            }
            b.draw_and_clear();
        }
    }
}

/// The skin a bot wears: one of `skins`, picked by an FNV-1a hash of its guest name, so the same name always gets the
/// same skin. An empty list draws DEFAULT.
pub fn bot_skin(name: &str, skins: &[String]) -> String {
    if skins.is_empty() {
        return SKIN_DEFAULT.to_string();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    skins[(h % skins.len() as u64) as usize].clone()
}

/// The swing and amplitude of a bot at its stride. A bot that is not walking stands still (amplitude 0).
pub fn bot_swing(bots: &BotSpec, stride: f32, walking: bool) -> (f32, f32) {
    (stride * bots.stride_swing, if walking { bots.walk_amp } else { 0.0 })
}

/// One NPC teammate as the 3D view needs it: its guest name (the skin seed), its floor position and yaw, and its
/// stride and walk state.
pub struct BotView<'a> {
    pub name: &'a str,
    pub at: (f32, f32),
    pub facing: f32,
    pub stride: f32,
    pub walking: bool,
}

/// Draws NPC teammates with the runner mannequin, bent the same way a runner's limbs bend, each wearing its skin (see
/// [`bot_skin`]). The caller sets the camera and material, as for [`draw_runners`]. Draws nothing for an empty list.
pub fn draw_bots(a: &Assets, bots: &[BotView]) {
    if bots.is_empty() {
        return;
    }
    let Some(body) = a.npc_body.as_ref() else {
        return;
    };
    let pose = runner_pose();
    let skins: &[String] = pose.map(|p| p.bots.skins.as_slice()).unwrap_or(&[]);
    for b in bots {
        let (swing, amp) = pose.map(|p| bot_swing(&p.bots, b.stride, b.walking)).unwrap_or((0.0, 0.0));
        let bones = bone_transforms(pose, &body.groups, swing, amp);
        let skin = bot_skin(b.name, skins);
        let texture = match pick(&skin, &|id| a.skin_texture(id).is_some()) {
            Pick::Skin(id) => a.skin_texture(&id).unwrap_or_else(|| body.texture.clone()),
            Pick::Atlas => body.texture.clone(),
        };
        let root = Mat4::from_translation(vec3(b.at.0, 0.0, b.at.1))
            * Mat4::from_rotation_y(b.facing)
            * Mat4::from_scale(Vec3::splat(body.scale));
        let mut verts = vec![];
        let mut idx = vec![];
        body.build_posed(&mut verts, &mut idx, root, &|k: usize| bones[k], Vec2::ZERO, WHITE);
        draw_mesh(&Mesh {
            vertices: verts,
            indices: idx,
            texture: Some(texture),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The models.json text the game embeds.
    fn shipped_text() -> &'static str {
        embedded::data_under(MODELS_FILE).next().expect("player/models.json is embedded").1
    }

    /// Every outliner group name of the shipped mannequin model.
    fn mannequin_group_names() -> Vec<String> {
        fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(list) = v.as_array() {
                for c in list {
                    if let Some(name) = c["name"].as_str() {
                        out.push(name.to_string());
                    }
                    walk(&c["children"], out);
                }
            }
        }
        let model: serde_json::Value =
            serde_json::from_str(include_str!("../assets/player/player_mannequin.bbmodel")).unwrap();
        let mut out = vec![];
        walk(&model["outliner"], &mut out);
        out
    }

    #[test]
    fn default_runner_uses_the_atlas_and_an_unknown_skin_falls_back() {
        let none = |_: &str| false;
        assert_eq!(pick(SKIN_DEFAULT, &|_| true), Pick::Atlas, "DEFAULT is always the atlas");
        assert_eq!(pick("NO_SUCH_SKIN", &none), Pick::Atlas, "an unknown id draws the atlas");
        let known = "ABC";
        assert_eq!(pick(known, &|id| id == known), Pick::Skin(known.into()));
        assert_eq!(pick(known, &none), Pick::Atlas, "a skin not loaded yet draws the atlas");
    }

    #[test]
    fn shipped_pose_matches_the_standard_swing_of_the_current_game() {
        // Standard values: the arm swing 0.9 and leg swing 0.75 that render/bbmodel.rs used before the data existed.
        let pose = RunnerPose::parse(shipped_text()).expect("models.json parses");
        assert_eq!(pose.standing_swing, 0.0);
        assert_eq!(pose.standing_amp, 0.0, "a runner with no stride stands still, as before");
        let quarter = std::f32::consts::FRAC_PI_2;
        assert_eq!(pose.angle("R-Arm", quarter, 1.0), Some(0.9));
        assert_eq!(pose.angle("L-Arm", quarter, 1.0), Some(-0.9), "the left arm swings the other way");
        assert_eq!(pose.angle("L-Thigh", quarter, 1.0), Some(0.75));
        assert_eq!(pose.angle("R-Thigh", quarter, 1.0), Some(-0.75));
        assert_eq!(pose.angle("Head", quarter, 1.0), None, "the head is not a posed limb");
        // The standard swing is inside every joint limit, so no bend is clamped at full stride.
        for b in &pose.bones {
            assert!(b.amp <= b.max_rad + 1e-6, "{} clamps its standard swing", b.name);
        }
    }

    #[test]
    fn changing_the_data_changes_the_pose() {
        let text = shipped_text();
        let quarter = std::f32::consts::FRAC_PI_2;
        let base = RunnerPose::parse(text).unwrap();
        // The first "amp": 0.9 in the file is the R-Arm's.
        let softer = RunnerPose::parse(&text.replacen("\"amp\": 0.9", "\"amp\": 0.5", 1)).unwrap();
        assert_eq!(base.angle("R-Arm", quarter, 1.0), Some(0.9));
        assert_eq!(softer.angle("R-Arm", quarter, 1.0), Some(0.5), "a new amplitude moves the arm");
        assert_eq!(softer.angle("L-Arm", quarter, 1.0), Some(-0.9), "the other arm keeps its own value");
        // A joint limit clamps the bend: 10 degrees caps the thigh at its limit.
        let stiff = RunnerPose::parse(&text.replacen("\"max_deg\": 50.0", "\"max_deg\": 10.0", 2)).unwrap();
        let capped = stiff.angle("L-Thigh", quarter, 1.0).unwrap();
        assert!((capped - 10f32.to_radians()).abs() < 1e-6, "the limit caps the bend, got {capped}");
    }

    #[test]
    fn every_posed_limb_is_a_real_bone_of_the_mannequin() {
        let pose = RunnerPose::parse(shipped_text()).unwrap();
        let names = mannequin_group_names();
        for b in &pose.bones {
            assert!(names.contains(&b.name), "{} is not an outliner group of the mannequin", b.name);
        }
        assert!(embedded::asset(&pose.model).is_some(), "the runner model {} is embedded", pose.model);
        assert!(embedded::asset(&pose.atlas).is_some(), "the runner atlas {} is embedded", pose.atlas);
    }

    #[test]
    fn bad_data_is_an_error_not_a_silent_default() {
        assert!(RunnerPose::parse("not json").is_err());
        assert!(RunnerPose::parse("{}").is_err());
        let text = shipped_text().replacen("\"side\": 1.0", "\"sde\": 1.0", 1);
        assert!(RunnerPose::parse(&text).is_err(), "a missing bone field is reported");
        let no_bots = shipped_text().replacen("\"bots\": {", "\"bots_gone\": {", 1);
        assert!(RunnerPose::parse(&no_bots).is_err(), "a missing bots section is reported");
    }

    #[test]
    fn shipped_bots_wear_the_default_skin_and_swing_as_the_current_game() {
        let pose = RunnerPose::parse(shipped_text()).unwrap();
        assert_eq!(pose.bots.skins, vec![SKIN_DEFAULT.to_string()], "Standard bots draw the atlas, as before");
        assert_eq!(pose.bots.walk_amp, 0.6, "the walking NPC swing scene.rs used");
        assert_eq!(pose.bots.stride_swing, 2.4, "the stride scale scene.rs used");
        // A standing bot does not swing; a walking bot bends its arms and legs like a runner does.
        assert_eq!(bot_swing(&pose.bots, 1.0, false), (2.4, 0.0));
        let (swing, amp) = bot_swing(&pose.bots, std::f32::consts::FRAC_PI_2 / 2.4, true);
        let bent = pose.angle("R-Arm", swing, amp).unwrap();
        assert!(bent.abs() > 0.1, "a walking bot's arm bends, got {bent}");
        assert!((bent - 0.9 * 0.6).abs() < 1e-5, "the arm bends by its amp times the walk amp, got {bent}");
    }

    #[test]
    fn a_bot_skin_is_stable_and_comes_from_the_data_list() {
        let none: Vec<String> = vec![];
        assert_eq!(bot_skin("K-04817", &none), SKIN_DEFAULT, "an empty list draws DEFAULT");
        let two = vec!["SKIN_A".to_string(), "SKIN_B".to_string()];
        let names = ["K-04817", "R-04821", "B-00001", "Q-99999", "M-12345", "T-54321", "D-20000", "F-30303"];
        let picked: Vec<String> = names.iter().map(|n| bot_skin(n, &two)).collect();
        for p in &picked {
            assert!(two.contains(p), "{p} is not one of the data skins");
        }
        for (n, p) in names.iter().zip(&picked) {
            assert_eq!(&bot_skin(n, &two), p, "the same guest name keeps its skin");
        }
        assert!(picked.iter().any(|p| p == "SKIN_A") && picked.iter().any(|p| p == "SKIN_B"), "the names spread over the list");
        let one = vec!["ONLY".to_string()];
        assert_eq!(bot_skin("K-04817", &one), "ONLY");
    }

    #[test]
    fn changing_the_data_changes_the_bots() {
        let text = shipped_text();
        let swapped = text.replacen("\"skins\": [\"DEFAULT\"]", "\"skins\": [\"SKIN_A\", \"SKIN_B\"]", 1);
        let pose = RunnerPose::parse(&swapped).unwrap();
        assert_eq!(pose.bots.skins, vec!["SKIN_A".to_string(), "SKIN_B".to_string()], "the skin list comes from data");
        let slower = text.replacen("\"walk_amp\": 0.6", "\"walk_amp\": 0.3", 1);
        let pose = RunnerPose::parse(&slower).unwrap();
        let (swing, amp) = bot_swing(&pose.bots, 1.0, true);
        assert_eq!(amp, 0.3, "a new walk amplitude changes how far a bot bends");
        assert_eq!(swing, 2.4);
        let quicker = text.replacen("\"stride_swing\": 2.4", "\"stride_swing\": 4.0", 1);
        let pose = RunnerPose::parse(&quicker).unwrap();
        assert_eq!(bot_swing(&pose.bots, 1.0, true).0, 4.0, "a new stride scale changes the swing rate");
    }
}
