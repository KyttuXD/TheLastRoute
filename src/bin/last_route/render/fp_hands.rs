//! The runner's own forearm and hand in the first-person view (player_controller_3d.gd `_build_skinned_hand`, 2549-2615,
//! and the climb hands, 1448-1485). The mesh is the mannequin's R-Forearm group (its forearm, hand and sleeve cubes; a slim
//! group when the saved ARMS resolves slim), placed so the hand's prop point sits on the held item's grip (the view
//! origin) and tilted so the forearm comes up from below-behind. On a ladder two mirrored hands alternate rung by rung.
//! Every offset, scale and tilt is data (data/render/fp_hands.json). The pose rows of the lockpick and the revive kit
//! (held_item_poses.gd FIRST_PERSON / GRIP / REACH) live there too.

use super::bbmodel::{Group, Model};
use super::scene::{camera, lighting, view};
use super::shader;
use crate::game::Game;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../data/render/fp_hands.json");

/// A held-item pose row (HeldItemPoses FIRST_PERSON / GRIP / REACH).
#[derive(Clone, Debug, PartialEq)]
pub struct PoseRow {
    pub view: Vec3,
    pub scale: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub grip: f32,
    pub reach: f32,
}

#[derive(Clone, Debug)]
pub struct HandData {
    pub scale: f32,
    pub tilt_deg: f32,
    pub bare_view: Vec3,
    pub climb_scale: f32,
    pub climb_tilt_deg: f32,
    pub climb_offset: Vec3,
    pub rung_spacing: f32,
    pub reach: f32,
    pub pull: f32,
    pub classic_group: String,
    pub slim_group: String,
    pub prop_point: Vec3,
    pub slim_width: f32,
    pub poses: Vec<(String, PoseRow)>,
}

fn num(v: &serde_json::Value, path: &str) -> Result<f32, String> {
    v.as_f64().map(|x| x as f32).ok_or_else(|| format!("fp_hands.json: missing number {path}"))
}

fn vec3_of(v: &serde_json::Value, path: &str) -> Result<Vec3, String> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("fp_hands.json: missing [x, y, z] {path}"))?;
    Ok(vec3(num(&a[0], path)?, num(&a[1], path)?, num(&a[2], path)?))
}

fn text(v: &serde_json::Value, path: &str) -> Result<String, String> {
    v.as_str().map(str::to_string).ok_or_else(|| format!("fp_hands.json: missing text {path}"))
}

impl HandData {
    /// The data from its JSON text. A missing or mistyped key is reported by its path.
    pub fn parse(src: &str) -> Result<HandData, String> {
        let j: serde_json::Value = serde_json::from_str(src).map_err(|e| format!("fp_hands.json: {e}"))?;
        let mut poses = vec![];
        let table = j["poses"].as_object().ok_or("fp_hands.json: missing object poses")?;
        for (name, r) in table {
            let p = |k: &str| format!("poses.{name}.{k}");
            poses.push((
                name.clone(),
                PoseRow {
                    view: vec3_of(&r["view"], &p("view"))?,
                    scale: num(&r["scale"], &p("scale"))?,
                    yaw: num(&r["yaw"], &p("yaw"))?,
                    pitch: num(&r["pitch"], &p("pitch"))?,
                    grip: num(&r["grip"], &p("grip"))?,
                    reach: num(&r["reach"], &p("reach"))?,
                },
            ));
        }
        Ok(HandData {
            scale: num(&j["hand"]["scale"], "hand.scale")?,
            tilt_deg: num(&j["hand"]["tilt_deg"], "hand.tilt_deg")?,
            bare_view: vec3_of(&j["hand"]["bare_view"], "hand.bare_view")?,
            climb_scale: num(&j["climb"]["scale"], "climb.scale")?,
            climb_tilt_deg: num(&j["climb"]["tilt_deg"], "climb.tilt_deg")?,
            climb_offset: vec3_of(&j["climb"]["offset"], "climb.offset")?,
            rung_spacing: num(&j["climb"]["rung_spacing"], "climb.rung_spacing")?.max(0.01),
            reach: num(&j["climb"]["reach"], "climb.reach")?,
            pull: num(&j["climb"]["pull"], "climb.pull")?,
            classic_group: text(&j["model"]["classic_group"], "model.classic_group")?,
            slim_group: text(&j["model"]["slim_group"], "model.slim_group")?,
            prop_point: vec3_of(&j["model"]["prop_point"], "model.prop_point")?,
            slim_width: num(&j["model"]["slim_width"], "model.slim_width")?,
            poses,
        })
    }

    /// The pose row of a held item by its Godot name ("lockpick", "revive_kit").
    pub fn pose(&self, name: &str) -> Option<&PoseRow> {
        self.poses.iter().find(|(n, _)| n == name).map(|(_, p)| p)
    }
}

/// The shipped data (a broken file fails the tests; at run time it falls back to no hands).
pub fn data() -> Option<&'static HandData> {
    static D: OnceLock<Option<HandData>> = OnceLock::new();
    D.get_or_init(|| HandData::parse(TEXT).ok()).as_ref()
}

/// The pose row of a held item that has no Item variant yet (lockpick, revive kit).
#[allow(dead_code)]
pub fn item_pose(name: &str) -> Option<PoseRow> {
    data().and_then(|d| d.pose(name)).cloned()
}

/// The skin and arm style the local runner wears. Set by the app whenever the worn skin changes: `skin` is the wire id
/// (the canonical hash the texture cache knows, or DEFAULT), `slim` is the arm style already resolved from the ARMS
/// setting and the skin's layout (see `skins::worn_skin`).
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub skin: String,
    pub slim: bool,
}

thread_local! {
    static LOOK: RefCell<Look> = RefCell::new(Look { skin: "DEFAULT".into(), slim: false });
}

/// Sets the look of the local hand: the worn skin's hash and whether it wears slim arms.
pub fn set_look(skin: &str, slim: bool) {
    LOOK.with(|l| *l.borrow_mut() = Look { skin: skin.into(), slim });
}

/// The look the hand draws with now.
pub fn look() -> Look {
    LOOK.with(|l| l.borrow().clone())
}

/// The outliner group of the forearm to draw: the slim group when slim is wanted and the model has one, else the classic.
pub fn hand_group(groups: &[Group], slim: bool, d: &HandData) -> Option<usize> {
    let find = |n: &str| groups.iter().position(|g| g.name == n);
    if slim {
        if let Some(i) = find(&d.slim_group) {
            return Some(i);
        }
    }
    find(&d.classic_group)
}

/// Whether the model has its own slim group (otherwise a slim hand is the classic one narrowed by `slim_width`).
pub fn has_slim_group(groups: &[Group], d: &HandData) -> bool {
    groups.iter().any(|g| g.name == d.slim_group)
}

fn under(groups: &[Group], mut k: usize, root: usize) -> bool {
    loop {
        if k == root {
            return true;
        }
        match groups[k].parent {
            Some(p) => k = p,
            None => return false,
        }
    }
}

/// The hand node: at `pos` in the item's view space, tilted about X and scaled (mirrored in X for a left hand).
pub fn hand_matrix(pos: Vec3, tilt_deg: f32, scale: f32, mirror: bool) -> Mat4 {
    let sx = if mirror { -scale } else { scale };
    Mat4::from_translation(pos) * Mat4::from_rotation_x(tilt_deg.to_radians()) * Mat4::from_scale(vec3(sx, scale, scale))
}

/// The item-less hand (Godot's FP_HAND_SCALE / FP_ARM_TILT at the bare view).
pub fn bare_hand_matrix(d: &HandData) -> Mat4 {
    hand_matrix(d.bare_view, d.tilt_deg, d.scale, false)
}

/// One climbing hand at `height` metres up the ladder: (y offset, pull toward the face, gripping). The hands alternate
/// every `rung_spacing` metres; `side` is -1 for the left hand, +1 for the right.
pub fn climb_hand_pose(d: &HandData, height: f32, side: f32) -> (f32, f32, bool) {
    let cycle = height / (d.rung_spacing * 2.0) + if side < 0.0 { 0.0 } else { 0.5 };
    let t = cycle - cycle.floor();
    if t < 0.5 {
        (d.reach - 2.0 * d.reach * (t * 2.0), (t * 2.0 * std::f32::consts::PI).sin() * d.pull, true)
    } else {
        let k = (t - 0.5) * 2.0;
        let e = k * k * (3.0 - 2.0 * k);
        (-d.reach + 2.0 * d.reach * e, 0.0, false)
    }
}

/// The forearm-and-hand triangles of `model`, with the prop point at the origin of `xf`'s space. `xf` maps metres (the
/// mannequin's scale is applied here) to the layer. A mirrored hand reverses its winding so it is not culled.
pub fn hand_mesh(model: &Model, d: &HandData, slim: bool, xf: Mat4, mirror: bool) -> (Vec<Vertex>, Vec<u16>) {
    let mut out = vec![];
    let mut idx = vec![];
    let Some(root) = hand_group(&model.groups, slim, d) else {
        return (out, idx);
    };
    let narrow = slim && !has_slim_group(&model.groups, d);
    let m = xf
        * Mat4::from_scale(vec3(if narrow { d.slim_width } else { 1.0 }, 1.0, 1.0))
        * Mat4::from_scale(Vec3::splat(model.scale))
        * Mat4::from_translation(-d.prop_point);
    let nm = Mat3::from_mat4(m);
    let col: [u8; 4] = WHITE.into();
    for (g, f) in model.faces() {
        let Some(g) = g else { continue };
        if !under(&model.groups, g, root) {
            continue;
        }
        let n = (nm * f.normal).normalize_or_zero();
        let base = out.len() as u16;
        for (c, uv) in f.corners.iter().zip(&f.uv) {
            out.push(Vertex { position: m.transform_point3(*c), uv: *uv, color: col, normal: vec4(n.x, n.y, n.z, 1.0) });
        }
        let last = f.corners.len() as u16;
        for k in 1..last.saturating_sub(1) {
            if mirror {
                idx.extend_from_slice(&[base, base + k + 1, base + k]);
            } else {
                idx.extend_from_slice(&[base, base + k, base + k + 1]);
            }
        }
    }
    (out, idx)
}

fn draw_hand(a: &Assets, d: &HandData, look: &Look, xf: Mat4, mirror: bool) {
    let Some(body) = a.npc_body.as_ref() else { return };
    let (vertices, indices) = hand_mesh(body, d, look.slim, xf, mirror);
    if indices.is_empty() {
        return;
    }
    let texture = a.skin_texture(&look.skin).unwrap_or_else(|| body.texture.clone());
    draw_mesh(&Mesh { vertices, indices, texture: Some(texture) });
}

/// The hand under a held item: `base` is camera * items * the item's view node (the grip is its origin), so the hand rides
/// the item's tween and the wall push. Called by draw_viewmodel with the shader already set.
pub fn draw_item_hand(a: &Assets, base: Mat4) {
    let Some(d) = data() else { return };
    let look = LOOK.with(|l| l.borrow().clone());
    draw_hand(a, d, &look, base * hand_matrix(Vec3::ZERO, d.tilt_deg, d.scale, false), false);
}

/// The two hands on a ladder, driven by the height climbed. `mat` is the viewmodel material; draws into the layer
/// draw_viewmodel just cleared (it leaves it empty while a climb runs), so call it right after draw_viewmodel.
pub fn draw_ladder_hands(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget) {
    let Some(c) = g.ladder.as_ref() else { return };
    if g.p.hidden.is_some() || g.report.is_some() || g.escape.is_some() {
        return;
    }
    let Some(d) = data() else { return };
    let look = LOOK.with(|l| l.borrow().clone());
    set_camera(&camera(g, rt));
    gl_use_material(mat);
    shader::apply(mat, &lighting(g), &g.world.lamps);
    let (eye, fwd, right, up) = view(g);
    let cam = Mat4::from_cols(right.extend(0.0), up.extend(0.0), (-fwd).extend(0.0), eye.extend(1.0));
    for side in [-1.0f32, 1.0] {
        let (y, pull, _grip) = climb_hand_pose(d, c.y, side);
        let pos = vec3(d.climb_offset.x * side, d.climb_offset.y + y, d.climb_offset.z + pull);
        draw_hand(a, d, &look, cam * hand_matrix(pos, d.climb_tilt_deg, d.climb_scale, side < 0.0), side < 0.0);
    }
    gl_use_default_material();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> HandData {
        HandData::parse(TEXT).expect("shipped fp_hands.json parses")
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn item_less_hand_matches_the_data() {
        let d = d();
        // Standard values are Godot's: FP_HAND_SCALE .52, FP_ARM_TILT 118
        assert!(near(d.scale, 0.52) && near(d.tilt_deg, 118.0));
        let m = bare_hand_matrix(&d);
        assert!((m.transform_point3(Vec3::ZERO) - d.bare_view).length() < 1e-5, "the hand origin is the bare view");
        assert!(near(m.transform_vector3(Vec3::X).length(), d.scale), "scaled by the data scale");
        let up = m.transform_vector3(Vec3::Y).normalize();
        let want = Mat3::from_rotation_x(d.tilt_deg.to_radians()) * Vec3::Y;
        assert!((up - want).length() < 1e-4, "tilted by the data angle about X");
    }

    #[test]
    fn a_changed_data_value_moves_the_hand() {
        let mut e = d();
        e.bare_view = vec3(0.0, 0.0, -1.0);
        e.scale = 1.0;
        assert!((bare_hand_matrix(&e).transform_point3(Vec3::ZERO) - e.bare_view).length() < 1e-5);
        assert!(near(bare_hand_matrix(&e).transform_vector3(Vec3::Z).length(), 1.0));
    }

    #[test]
    fn ladder_hands_alternate_per_rung() {
        let d = d();
        assert!(near(d.rung_spacing, 0.3));
        for h in [0.0f32, 0.1, 0.5, 1.7] {
            let (_, _, l0) = climb_hand_pose(&d, h, -1.0);
            let (_, _, r0) = climb_hand_pose(&d, h, 1.0);
            assert_ne!(l0, r0, "one hand grips while the other reaches");
            let (_, _, l1) = climb_hand_pose(&d, h + d.rung_spacing, -1.0);
            let (_, _, r1) = climb_hand_pose(&d, h + d.rung_spacing, 1.0);
            assert_eq!((l1, r1), (r0, l0), "one rung further up the hands swap");
        }
        let (y, _, _) = climb_hand_pose(&d, 0.0, -1.0);
        assert!(y.abs() <= d.reach + 1e-5, "the hand stays within the reach");
    }

    #[test]
    fn lockpick_and_revive_kit_rows_exist() {
        let d = d();
        let l = d.pose("lockpick").expect("lockpick row");
        assert_eq!((l.view, l.scale), (vec3(0.25, -0.26, -0.52), 0.56));
        let r = d.pose("revive_kit").expect("revive_kit row");
        assert_eq!((r.view, r.scale), (vec3(0.24, -0.40, -0.56), 0.55));
        assert!(item_pose("lockpick").is_some() && item_pose("nope").is_none());
    }

    #[test]
    fn a_missing_key_is_reported() {
        let broken = TEXT.replace("\"tilt_deg\": 118.0", "\"tilt\": 118.0");
        let e = HandData::parse(&broken).unwrap_err();
        assert!(e.contains("hand.tilt_deg"), "{e}");
        let e = HandData::parse("{").unwrap_err();
        assert!(e.contains("fp_hands.json"));
        let e = HandData::parse(&TEXT.replace("\"prop_point\"", "\"prop\"")).unwrap_err();
        assert!(e.contains("model.prop_point"), "{e}");
    }

    fn group(name: &str, parent: Option<usize>) -> Group {
        Group { name: name.into(), parent, origin: Vec3::ZERO }
    }

    #[test]
    fn slim_arm_picks_the_slim_group() {
        let d = d();
        let groups = vec![group("R-Arm", None), group(&d.classic_group, Some(0)), group(&d.slim_group, Some(0))];
        assert_eq!(hand_group(&groups, true, &d), Some(2));
        assert_eq!(hand_group(&groups, false, &d), Some(1));
        assert!(has_slim_group(&groups, &d));
        // a model without a slim group wears the classic one (narrowed when drawn)
        assert_eq!(hand_group(&groups[..2], true, &d), Some(1));
        assert!(!has_slim_group(&groups[..2], &d));
    }

    #[test]
    fn the_hand_wears_the_resolved_skin_and_arm_style() {
        // the app resolves hash and slim once (skins::worn_skin); the hand takes both as given, so an Alex skin under
        // ARMS: AUTO gives a slim hand, which the old "slim only when forced" rule never did
        set_look("abc", true);
        assert_eq!(look(), Look { skin: "abc".into(), slim: true });
        set_look("DEFAULT", false);
        assert_eq!(look(), Look { skin: "DEFAULT".into(), slim: false });
        let slim_auto = crate::shared::skins::ArmsSetting::Auto.resolve(true);
        let slim_forced_off = crate::shared::skins::ArmsSetting::Classic.resolve(true);
        assert!(slim_auto && !slim_forced_off);
    }

    #[test]
    fn the_shipped_mannequin_has_the_classic_hand_group() {
        let d = d();
        let model = include_str!("../assets/player/player_mannequin.bbmodel");
        assert!(model.contains(&format!("\"{}\"", d.classic_group)));
    }
}
