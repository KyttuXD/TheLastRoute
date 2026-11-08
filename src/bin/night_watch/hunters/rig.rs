//! Hunter actors: a rig + model + pose player, skinned on the CPU every draw and lit by the Night Watch shader.
//!
//! `HunterLib` loads rigs (with the "nw/<pose>" clips compiled in) and models lazily on the first `actor()` call,
//! after the window exists. Everything that does not touch the GPU (`Actor::from_parts` with no model, state
//! changes, `animate`, the fallback figure, bone matrices) works without a GL context and is unit tested.
#![allow(dead_code)]

use super::anim::clip::Rig;
use super::anim::player::{AnimPlayer, Local};
use super::bbmodel::{Model, LIT_MODE};
use super::pose_state::{pose_for_key, pose_for_state, PoseState, VentPose};
use super::poses::{clip_name, compile_nw_poses_for, pose_blends, DEFAULT_BLEND};
use crate::gfx::{Gfx, LightSet};
use macroquad::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;

/// Model-space matrices of the rig's bones for a local pose, in rig bone order (a bone's matrix is its parent's matrix
/// times its own local transform).
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

// ------------------------------------------------------------------------------------------- fallback figure

fn push_box(out: &mut Vec<Vertex>, idx: &mut Vec<u16>, root: Mat4, min: Vec3, max: Vec3, col: [u8; 4]) {
    let c = (min + max) * 0.5;
    let h = (max - min) * 0.5;
    let nm = Mat3::from_mat4(root);
    let axes = [(Vec3::X, Vec3::Y, Vec3::Z), (Vec3::Y, Vec3::Z, Vec3::X), (Vec3::Z, Vec3::X, Vec3::Y)];
    for (n, u, v) in axes {
        for s in [-1.0f32, 1.0] {
            let nn = n * s;
            let nw = (nm * nn).normalize_or_zero();
            let base = out.len() as u16;
            for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = c + nn * h.dot(n) + u * (su * h.dot(u)) + v * (sv * h.dot(v));
                out.push(Vertex {
                    position: root.transform_point3(p),
                    uv: vec2(0.0, 0.0),
                    color: col,
                    normal: vec4(nw.x, nw.y, nw.z, LIT_MODE),
                });
            }
            if s > 0.0 {
                idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            } else {
                idx.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }
}

fn shade(c: Color, k: f32) -> [u8; 4] {
    [
        ((c.r * k).clamp(0.0, 1.0) * 255.0) as u8,
        ((c.g * k).clamp(0.0, 1.0) * 255.0) as u8,
        ((c.b * k).clamp(0.0, 1.0) * 255.0) as u8,
        (c.a.clamp(0.0, 1.0) * 255.0) as u8,
    ]
}

/// The box figure drawn when a model does not parse (about 2.58 m, feet at the origin, facing -z). Seven boxes.
pub fn fallback_figure(out: &mut Vec<Vertex>, idx: &mut Vec<u16>, root: Mat4, tint: Color) {
    let base = Color::new(0.42 * tint.r, 0.38 * tint.g, 0.46 * tint.b, tint.a);
    let dark = shade(base, 0.8);
    let mid = shade(base, 1.0);
    let light = shade(base, 1.3);
    push_box(out, idx, root, vec3(-0.30, 0.0, -0.12), vec3(-0.06, 1.18, 0.12), dark);
    push_box(out, idx, root, vec3(0.06, 0.0, -0.12), vec3(0.30, 1.18, 0.12), dark);
    push_box(out, idx, root, vec3(-0.36, 1.18, -0.18), vec3(0.36, 2.04, 0.18), mid);
    push_box(out, idx, root, vec3(-0.58, 1.10, -0.12), vec3(-0.36, 2.02, 0.12), mid);
    push_box(out, idx, root, vec3(0.36, 1.10, -0.12), vec3(0.58, 2.02, 0.12), mid);
    push_box(out, idx, root, vec3(-0.26, 2.04, -0.26), vec3(0.26, 2.58, 0.26), light);
    push_box(out, idx, root, vec3(-0.18, 2.20, -0.28), vec3(0.18, 2.34, -0.26), shade(Color::new(0.9, 0.2, 0.2, tint.a), 1.0));
}

// ------------------------------------------------------------------------------------------- Actor

pub struct Actor {
    pub id: String,
    pub pos: Vec3,
    pub yaw: f32,
    pub visible: bool,
    /// Crawling inside a duct (the nav walker's `hidden`): `draw` skips it, `draw_in` shows it only to a duct camera.
    pub hidden: bool,
    /// Multiplies the model colour (and alpha) when drawn.
    pub tint: Color,
    rig: Rc<Rig>,
    model: Option<Rc<Model>>,
    blends: Rc<HashMap<String, f32>>,
    player: AnimPlayer,
    state: PoseState,
    moving: bool,
    running: bool,
    crouch_amt: f32,
    crouched: bool,
    vent: VentPose,
    scale: f32,
}

impl Actor {
    /// Re-scales this actor for a mode (data/hunter_scale.json per_mode). Call once after the actor is built in a mode;
    /// without it the actor uses the base world scale.
    pub fn set_scale_mode(&mut self, mode: &str) {
        self.scale = super::place::world_scale(&self.rig, &self.id, mode);
    }

    /// The metres per bbmodel unit this actor is drawn at (`place::world_scale` for its mode).
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The height in metres this actor stands, measured against the Godot reference humanoid (its model is
    /// `HUNTER_HEIGHT_M` tall at the Godot draw scale). Creature rigs report their reference height unscaled.
    pub fn world_height_m(&self) -> f32 {
        self.scale * super::place::HUNTER_HEIGHT_M / self.rig.draw_scale()
    }

    /// An actor from loaded parts (no GL needed when `model` is None: it then draws the box figure).
    pub fn from_parts(id: &str, rig: Rc<Rig>, model: Option<Rc<Model>>, blends: Rc<HashMap<String, f32>>) -> Actor {
        let scale = super::place::world_scale(&rig, id, "");
        let mut a = Actor {
            id: id.to_string(),
            pos: Vec3::ZERO,
            yaw: 0.0,
            visible: true,
            hidden: false,
            tint: WHITE,
            rig,
            model,
            blends,
            player: AnimPlayer::default(),
            state: PoseState::Rig,
            moving: false,
            running: false,
            crouch_amt: 0.0,
            crouched: false,
            vent: VentPose::new(),
            scale,
        };
        a.play_loco();
        a
    }

    /// The clip the player is on ("nw/door_lurk", "idle", ...).
    pub fn current_clip(&self) -> &str {
        &self.player.assigned
    }

    pub fn state(&self) -> PoseState {
        self.state
    }

    pub fn is_crouched(&self) -> bool {
        self.crouched
    }

    pub fn has_model(&self) -> bool {
        self.model.is_some()
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    fn play_loco(&mut self) {
        let want = if self.moving && self.running {
            "sprint"
        } else if self.moving {
            "walk"
        } else {
            "idle"
        };
        let name = if self.rig.has_clip(want) {
            want
        } else if self.rig.has_clip("idle") {
            "idle"
        } else {
            return;
        };
        self.player.speed_scale = 1.0;
        self.player.play(&self.rig, name, 0.3);
    }

    fn play_nw(&mut self, pose: &str, speed: f32) -> bool {
        let clip = clip_name(pose);
        if !self.rig.has_clip(&clip) {
            return false;
        }
        let blend = self.blends.get(pose).copied().unwrap_or(DEFAULT_BLEND);
        self.player.speed_scale = speed;
        self.player.play(&self.rig, &clip, blend);
        true
    }

    fn apply(&mut self) {
        if self.crouched {
            let speed = if self.moving { 1.0 } else { 0.35 };
            if let Some(p) = pose_for_key("vent", &self.id) {
                if self.play_nw(p, speed) {
                    return;
                }
            }
        }
        match pose_for_state(self.state, &self.id) {
            Some(p) => {
                if !self.play_nw(p, 1.0) {
                    self.play_loco();
                }
            }
            None => self.play_loco(),
        }
    }

    /// Plays the pose for a Night Watch state (the "nw/<pose>" clip, blended by the pose's own blend time), or the
    /// rig's own idle / walk / sprint for `PoseState::Rig`. The same state again changes nothing.
    pub fn set_pose_state(&mut self, s: PoseState) {
        if s == self.state {
            return;
        }
        self.state = s;
        if self.vent.active() {
            return;
        }
        self.apply();
    }

    /// Walk / sprint selection while the state is `PoseState::Rig`.
    pub fn set_locomotion(&mut self, moving: bool, running: bool) {
        if moving == self.moving && running == self.running {
            return;
        }
        self.moving = moving;
        self.running = running;
        if self.vent.active() {
            return;
        }
        if self.state == PoseState::Rig || self.crouched {
            self.apply();
        }
    }

    /// Plays one pose by id at a speed scale, whatever the state (a one-off; the next `set_pose_state` change
    /// replaces it). False when the rig has no such clip.
    pub fn play_pose(&mut self, pose: &str, speed: f32) -> bool {
        self.play_nw(pose, speed)
    }

    /// Back to the rig's own clips.
    pub fn stop_pose(&mut self) {
        self.state = PoseState::Rig;
        self.crouched = false;
        self.crouch_amt = 0.0;
        self.apply();
    }

    /// 0..1: from 0.5 up the actor takes the vent crawl pose ("vent_crawl", Verrine's own for her) and back below.
    pub fn crouch(&mut self, amount: f32) {
        self.crouch_amt = amount.clamp(0.0, 1.0);
        let now = self.crouch_amt >= 0.5;
        if now == self.crouched {
            return;
        }
        self.crouched = now;
        if self.vent.active() {
            return;
        }
        self.apply();
    }

    /// NightWatchVentPose.update for this actor: while it returns true a vent pose owns the model.
    pub fn update_vent(&mut self, in_shaft: bool, moving: bool, dt_s: f32) -> bool {
        let id = self.id.clone();
        let rig = &self.rig;
        let step = self.vent.update(in_shaft, moving, dt_s, &|k| match pose_for_key(k, &id) {
            Some(p) => rig.has_clip(&clip_name(p)),
            None => false,
        });
        if let Some((key, speed)) = step.play {
            if let Some(p) = pose_for_key(key, &id) {
                self.play_nw(p, speed);
            }
        }
        if step.stop {
            self.apply();
        }
        step.owns
    }

    pub fn animate(&mut self, dt_s: f32) {
        self.player.advance(&self.rig, dt_s);
        // a finished one-shot (flinch) hands back to the rig's own clips
        if !self.player.playing && !self.vent.active() {
            let a = self.player.assigned.clone();
            if a.starts_with("nw/") && self.rig.clips.get(&a).map(|c| !c.looping).unwrap_or(false) {
                self.play_loco();
            }
        }
    }

    /// The current local pose of every rig bone.
    pub fn pose(&self) -> Vec<Local> {
        self.player.pose(&self.rig)
    }

    /// CPU-skins the model for the current pose into `out`/`idx`, root = T(pos) * Ry(yaw) * S(rig.draw_scale()).
    /// Returns the texture to draw with (None for the box figure).
    pub fn build_mesh(&self, out: &mut Vec<Vertex>, idx: &mut Vec<u16>) -> Option<Texture2D> {
        let place = Mat4::from_translation(self.pos) * Mat4::from_rotation_y(self.yaw);
        match &self.model {
            Some(model) => {
                let root = place * Mat4::from_scale(Vec3::splat(self.scale));
                let pose = self.player.pose(&self.rig);
                let bones = bone_matrices(&self.rig, &pose);
                let rig = &self.rig;
                let bone_of = |k: usize| model.groups.get(k).and_then(|g| rig.bone(&g.name)).map(|b| bones[b]);
                model.build_posed(out, idx, root, &bone_of, model.face_cell_uv(0), self.tint);
                Some(model.texture.clone())
            }
            None => {
                fallback_figure(out, idx, place, self.tint);
                None
            }
        }
    }

    /// True when `name` is a vent or duct camera room (a camera that may see a crawling hunter).
    pub fn is_duct_room(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n.contains("vent") || n.contains("duct")
    }

    /// Whether this actor is drawn: visible, and not hidden in a duct unless the camera is in the duct.
    pub fn drawn_for(&self, cam_in_duct: bool) -> bool {
        self.visible && (!self.hidden || cam_in_duct)
    }

    /// Whether this actor is drawn in the feed of camera room `room` (the one rule every feed path and the shots tour use).
    pub fn drawn_in_room(&self, room: &str) -> bool {
        self.drawn_for(Self::is_duct_room(room))
    }

    /// Whether this actor is submitted to a feed's draw: drawn for the camera (see `drawn_for`) AND inside its view. The one rule
    /// `draw_feed` and every mode's `feed_report` share, so "drawn" in the tour means "really in the picture".
    /// `shown` is whether the mode shows her in this feed at all (her `visible` flag, or the rules' "on the floor tonight").
    pub fn feed_drawn(&self, shown: bool, cam_in_duct: bool, view: &FeedView) -> bool {
        shown && (!self.hidden || cam_in_duct) && view.sees_body(self.pos)
    }

    /// Draw into the feed that is begun (see `feed_drawn`). Returns whether anything was submitted.
    pub fn draw_feed(&self, gfx: &Gfx, lights: &LightSet, view: &FeedView, shown: bool, cam_in_duct: bool) -> bool {
        if !self.feed_drawn(shown, cam_in_duct, view) {
            return false;
        }
        self.draw_mesh(gfx, lights, view.pos);
        true
    }

    /// Draw for a camera feed; `cam_in_duct` lets a vent cam show a crawling hunter.
    pub fn draw_in(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3, cam_in_duct: bool) {
        if !self.drawn_for(cam_in_duct) {
            return;
        }
        self.draw_mesh(gfx, lights, cam_pos);
    }

    /// Player view / office draw: hidden (in-duct) hunters are skipped.
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        self.draw_in(gfx, lights, cam_pos, false);
    }

    fn draw_mesh(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        let (mut verts, mut idx) = (Vec::new(), Vec::new());
        let texture = self.build_mesh(&mut verts, &mut idx);
        if verts.is_empty() {
            return;
        }
        gfx.shader.begin(lights, cam_pos);
        crate::gfx::mesh::draw_mesh_safe(&Mesh { vertices: verts, indices: idx, texture });
        gfx.shader.end();
    }
}

// ------------------------------------------------------------------------------------------- FeedView

/// A camera feed's view: where the camera sits, what it looks at, the vertical field of view in degrees and the aspect.
#[derive(Clone, Copy, Debug)]
pub struct FeedView {
    pub pos: Vec3,
    pub target: Vec3,
    pub fov_deg: f32,
    pub aspect: f32,
}

impl FeedView {
    /// A 16:9 feed.
    pub fn new(pos: Vec3, target: Vec3, fov_deg: f32) -> FeedView {
        FeedView { pos, target, fov_deg, aspect: 16.0 / 9.0 }
    }

    /// Is the point inside the view frustum (in front of the camera, within the field of view, inside the far plane)?
    pub fn contains(&self, p: Vec3) -> bool {
        let fwd = (self.target - self.pos).normalize_or_zero();
        if fwd == Vec3::ZERO {
            return false;
        }
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        if right == Vec3::ZERO {
            return false;
        }
        let up = right.cross(fwd);
        let d = p - self.pos;
        let depth = d.dot(fwd);
        if depth < 0.05 || depth > 80.0 {
            return false;
        }
        let half_v = (self.fov_deg.to_radians() * 0.5).tan();
        d.dot(up).abs() <= depth * half_v && d.dot(right).abs() <= depth * half_v * self.aspect
    }

    /// Is any part of a standing body (feet, chest or head, `pos` at the feet) inside the view?
    pub fn sees_body(&self, pos: Vec3) -> bool {
        [0.1f32, 0.9, 1.7].iter().any(|h| self.contains(pos + Vec3::Y * *h))
    }
}

// ------------------------------------------------------------------------------------------- HunterLib

/// Loads hunters lazily (after the window exists): one rig and one model per id, shared by every actor.
pub struct HunterLib {
    rigs: HashMap<String, Option<Rc<Rig>>>,
    models: HashMap<String, Option<Rc<Model>>>,
    blends: Option<Rc<HashMap<String, f32>>>,
}

impl Default for HunterLib {
    fn default() -> Self {
        HunterLib::new()
    }
}

fn load_rig(id: &str) -> Option<Rc<Rig>> {
    let text = crate::assets::rig_text(id)?;
    let mut rig = Rig::parse(id, text)?;
    compile_nw_poses_for(&mut rig, crate::assets::POSES_JSON, id);
    Some(Rc::new(rig))
}

fn load_model(id: &str) -> Option<Rc<Model>> {
    let text = crate::assets::bbmodel_text(id)?;
    Model::parse(text, None, &|_| None, None).map(Rc::new)
}

impl HunterLib {
    pub fn new() -> HunterLib {
        HunterLib { rigs: HashMap::new(), models: HashMap::new(), blends: None }
    }

    fn blends(&mut self) -> Rc<HashMap<String, f32>> {
        self.blends
            .get_or_insert_with(|| Rc::new(pose_blends(crate::assets::POSES_JSON)))
            .clone()
    }

    /// The hunter's rig with the "nw/<pose>" clips compiled in (no GL needed).
    pub fn rig(&mut self, id: &str) -> Option<Rc<Rig>> {
        if !self.rigs.contains_key(id) {
            let r = load_rig(id);
            self.rigs.insert(id.to_string(), r);
        }
        self.rigs.get(id).cloned().flatten()
    }

    /// The hunter's model (needs the GL context for its atlas); None when it does not parse.
    pub fn model(&mut self, id: &str) -> Option<Rc<Model>> {
        if !self.models.contains_key(id) {
            let m = load_model(id);
            self.models.insert(id.to_string(), m);
        }
        self.models.get(id).cloned().flatten()
    }

    /// A new actor for the hunter in a mode: `actor` plus the mode's scale (data/hunter_scale.json per_mode). Every mode
    /// builds its actors with this, so every actor is drawn at the one `place::world_scale`.
    pub fn actor_in(&mut self, id: &str, mode: &str) -> Option<Actor> {
        let mut a = self.actor(id)?;
        a.set_scale_mode(mode);
        Some(a)
    }

    /// A new actor for the hunter. None only if the id has no embedded rig. A model that does not parse gives the
    /// box figure instead.
    pub fn actor(&mut self, id: &str) -> Option<Actor> {
        let rig = self.rig(id)?;
        let model = self.model(id);
        let blends = self.blends();
        Some(Actor::from_parts(id, rig, model, blends))
    }
}

/// One of the 26 shared Last Route characters (docs LAST_ROUTE_MATCH.md): her id, display name and the role she
/// plays in Night Watch. 25 have a rig and a bbmodel; Zellia is the dispatcher, a voice on the headset with no body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RosterEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub role: &'static str,
    pub has_body: bool,
}

const fn r(id: &'static str, name: &'static str, role: &'static str) -> RosterEntry {
    RosterEntry { id, name, role, has_body: true }
}

/// The shared roster, in the order of the match table (by Last Route letter, Zellia last).
pub const ROSTER: [RosterEntry; 26] = [
    r("aki", "Aki", "Classic: only moves while the cameras are down"),
    r("briselle", "Briselle", "Dead Air: a phantom that breaks one system until you reboot it"),
    r("ceryth", "Ceryth", "Salvage Run: waits at the right door"),
    r("dovelle", "Dovelle", "Classic: the left-door stalker, the reference"),
    r("elise", "Elise", "The Descent: climbs the shaft behind the window cage"),
    r("ferryn", "Ferryn", "Dead Air: a phantom that takes the cameras"),
    r("glyra", "Glyra", "Masked Hours: the music box"),
    r("haelith", "Haelith", "Bedtime: waits where the breathing is"),
    r("ilyra", "Ilyra", "Classic: freezes while you watch her room"),
    r("javelle", "Javelle", "Bedtime: the hall door"),
    r("kathy", "Kathy", "Every game: the observer from night 2"),
    r("loxelle", "Loxelle", "Bedtime: the closet door"),
    r("mavrine", "Mavrine", "Dead Air: breaks the heater and the fan with the vents"),
    r("nyxie", "Nyxie", "Classic and Masked Hours: comes only with the dark"),
    r("opalie", "Opalie", "The Facility: hears noise, warns Porcelle"),
    r("porcelle", "Porcelle", "The Facility: moves in the dark, only the beacon shows her"),
    r("quenelle", "Quenelle", "The Facility: the lift on its schedule"),
    r("ravelle", "Ravelle", "Classic (right door) and Salvage Run (left door)"),
    r("solyth", "Solyth", "Dead Air: the heat of the temperature pair"),
    r("thessine", "Thessine", "Masked Hours: a hall walker"),
    r("umbriel", "Umbriel", "Masked Hours: a hall walker"),
    r("verrine", "Verrine", "Masked Hours: the vents; Open Air: the rooftop"),
    r("wynelle", "Wynelle", "Dead Air: the cold of the temperature pair; Open Air: the bus stop"),
    r("xivvy", "Xivvy", "Dead Air: the hunter in the vents"),
    r("ysolde", "Ysolde", "Salvage Run: the window; Open Air: the campfire"),
    RosterEntry { id: "zellia", name: "Zellia", role: "The dispatcher: the voice on the headset in every game", has_body: false },
];

/// The roster entry for an id (lowercase).
pub fn roster_entry(id: &str) -> Option<&'static RosterEntry> {
    ROSTER.iter().find(|e| e.id == id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn every_modes_humanoid_actors_stand_the_height_of_hunter_scale_json() {
        let cfg = super::super::place::HunterScale::get();
        let mut lib = HunterLib::new();
        let mut checked = 0;
        for mode in cfg.modes().into_iter().chain([String::new()]) {
            for e in ROSTER.iter().filter(|e| e.has_body) {
                let Some(rig) = lib.rig(e.id) else { continue };
                if rig.is_creature() {
                    continue;
                }
                let blends = Rc::new(pose_blends(crate::assets::POSES_JSON));
                let mut a = Actor::from_parts(e.id, rig, None, blends);
                a.set_scale_mode(&mode);
                let want = cfg.height_m(e.id, &mode);
                let got = a.world_height_m();
                assert!((got - want).abs() <= want * 0.05, "{} in '{mode}' stands {got} m, hunter_scale.json says {want} m", e.id);
                checked += 1;
            }
        }
        assert!(checked >= 9 * 20, "only {checked} actors checked");
    }

    #[test]
    fn a_creature_keeps_its_own_scale_in_every_mode() {
        let mut j: serde_json::Value = serde_json::from_str(DOVELLE).unwrap();
        j["metres"] = serde_json::json!(0.0525);
        let rig = Rig::parse("spider", &j.to_string()).expect("rig");
        assert!(rig.is_creature());
        for mode in ["", "classic", "dayshift", "story"] {
            assert_eq!(super::super::place::world_scale(&rig, "spider", mode), 0.0525);
        }
    }

    #[test]
    fn the_classic_dayshift_and_story_modes_build_their_actors_with_their_mode_scale() {
        for (src, mode) in [
            (include_str!("../modes/classic/game.rs"), "classic"),
            (include_str!("../modes/dayshift/game.rs"), "dayshift"),
            (include_str!("../modes/story/game.rs"), "story"),
        ] {
            assert!(src.contains("actor_in(") && src.contains(&format!("\"{mode}\"")), "{mode} builds actors without actor_in");
            assert!(!src.contains("hunters.actor("), "{mode} still calls HunterLib::actor directly");
        }
    }

    #[test]
    fn roster_has_26_and_matches_the_embedded_rigs() {
        assert_eq!(ROSTER.len(), 26);
        for e in ROSTER.iter() {
            assert!(!e.role.is_empty() && !e.name.is_empty());
            assert_eq!(crate::assets::HUNTER_IDS.contains(&e.id), e.has_body, "{}", e.id);
        }
        for id in crate::assets::HUNTER_IDS {
            assert!(roster_entry(id).is_some(), "{id} missing from the roster");
        }
        assert!(!roster_entry("zellia").unwrap().has_body);
        assert!(roster_entry("nobody").is_none());
    }

    #[test]
    fn every_actor_is_built_through_from_parts_only() {
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, hits);
                } else if p.extension().map_or(false, |x| x == "rs") {
                    let t = std::fs::read_to_string(&p).unwrap_or_default();
                    if t.contains("Actor::from_parts(") {
                        hits.push(p.file_name().unwrap().to_string_lossy().to_string());
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/night_watch"), &mut hits);
        hits.sort();
        // rig.rs (HunterLib::actor and its test helper) is the only caller; every mode asks HunterLib::actor.
        assert_eq!(hits, vec!["rig.rs".to_string()], "Actor::from_parts outside rig.rs");
    }
    use crate::hunters::poses::compile_nw_poses;

    const DOVELLE: &str = include_str!("../../last_route/assets/hunters/anim/dovelle.json");
    const VERRINE: &str = include_str!("../../last_route/assets/hunters/anim/verrine.json");

    /// A GL-free dovelle actor for the modes' feed tests (Actor::from_parts stays in this file).
    pub fn test_actor() -> Actor {
        actor("dovelle", DOVELLE)
    }

    #[test]
    fn a_feed_view_sees_what_is_in_front_and_not_what_is_behind() {
        let v = FeedView::new(vec3(0.0, 2.0, 5.0), vec3(0.0, 1.0, 0.0), 62.0);
        assert!(v.sees_body(vec3(0.0, 0.0, 0.0)));
        assert!(!v.sees_body(vec3(0.0, 0.0, 9.0)), "behind the camera");
        assert!(!v.sees_body(vec3(40.0, 0.0, 0.0)), "far to the side");
        let mut a = test_actor();
        a.pos = vec3(0.0, 0.0, 0.0);
        assert!(a.feed_drawn(true, false, &v));
        assert!(!a.feed_drawn(false, false, &v), "not shown");
        a.hidden = true;
        assert!(!a.feed_drawn(true, false, &v));
        assert!(a.feed_drawn(true, true, &v));
        a.pos = vec3(40.0, 0.0, 0.0);
        assert!(!a.feed_drawn(true, true, &v));
    }

    fn actor(id: &str, text: &str) -> Actor {
        let mut rig = Rig::parse(id, text).expect("rig");
        compile_nw_poses(&mut rig, crate::assets::POSES_JSON);
        let blends = Rc::new(pose_blends(crate::assets::POSES_JSON));
        Actor::from_parts(id, Rc::new(rig), None, blends)
    }

    #[test]
    fn hidden_actor_is_skipped_except_for_a_duct_camera() {
        let mut a = actor("dovelle", DOVELLE);
        assert!(a.drawn_for(false));
        a.hidden = true;
        assert!(!a.drawn_for(false));
        assert!(a.drawn_for(true));
        a.visible = false;
        assert!(!a.drawn_for(true));
        assert!(Actor::is_duct_room("vent_a"));
        assert!(Actor::is_duct_room("Duct2"));
        assert!(!Actor::is_duct_room("hall"));
    }

    #[test]
    fn a_new_actor_idles_and_states_pick_their_clips() {
        let mut a = actor("dovelle", DOVELLE);
        assert_eq!(a.current_clip(), "idle");
        a.set_pose_state(PoseState::Attack);
        assert_eq!(a.current_clip(), "nw/sprint_lunge");
        a.set_pose_state(PoseState::AtDoor);
        assert_eq!(a.current_clip(), "nw/door_pound");
        a.set_pose_state(PoseState::Rig);
        assert_eq!(a.current_clip(), "idle");
        a.set_locomotion(true, false);
        assert_eq!(a.current_clip(), "walk");
        a.set_locomotion(true, true);
        assert_eq!(a.current_clip(), "sprint");
        a.set_locomotion(false, false);
        assert_eq!(a.current_clip(), "idle");
    }

    #[test]
    fn locomotion_does_not_interrupt_a_night_watch_pose() {
        let mut a = actor("dovelle", DOVELLE);
        a.set_pose_state(PoseState::Idle);
        a.set_locomotion(true, true);
        assert_eq!(a.current_clip(), "nw/door_lurk");
    }

    #[test]
    fn a_flinch_plays_once_then_hands_back_to_the_rig() {
        let mut a = actor("dovelle", DOVELLE);
        a.set_pose_state(PoseState::LightFlinch);
        assert_eq!(a.current_clip(), "nw/flinch");
        for _ in 0..30 {
            a.animate(0.1);
        }
        assert_eq!(a.current_clip(), "idle");
    }

    #[test]
    fn crouch_takes_the_vent_crawl_and_verrine_her_own() {
        let mut a = actor("dovelle", DOVELLE);
        a.crouch(0.4);
        assert_eq!(a.current_clip(), "idle");
        a.crouch(1.0);
        assert_eq!(a.current_clip(), "nw/vent_crawl");
        a.crouch(0.0);
        assert_eq!(a.current_clip(), "idle");
        let mut v = actor("verrine", VERRINE);
        v.crouch(1.0);
        assert_eq!(v.current_clip(), "nw/vent_crawl_verrine");
    }

    #[test]
    fn verrine_drops_into_the_vent_crawls_and_rises() {
        let mut v = actor("verrine", VERRINE);
        assert!(v.update_vent(true, true, 0.1));
        assert_eq!(v.current_clip(), "nw/vent_drop_verrine");
        for _ in 0..12 {
            assert!(v.update_vent(true, true, 0.1));
        }
        assert_eq!(v.current_clip(), "nw/vent_crawl_verrine");
        assert!(v.update_vent(false, true, 0.1));
        assert_eq!(v.current_clip(), "nw/vent_rise_verrine");
        let mut owned = true;
        for _ in 0..12 {
            owned = v.update_vent(false, true, 0.1);
        }
        assert!(!owned);
        assert_eq!(v.current_clip(), "idle");
    }

    #[test]
    fn other_hunters_crawl_at_once_and_stop_on_exit() {
        let mut a = actor("dovelle", DOVELLE);
        assert!(a.update_vent(true, true, 0.1));
        assert_eq!(a.current_clip(), "nw/vent_crawl");
        assert!(!a.update_vent(false, true, 0.1));
        assert_eq!(a.current_clip(), "idle");
    }

    #[test]
    fn the_rest_pose_stacks_the_bones_to_the_head() {
        let mut rig = Rig::parse("dovelle", DOVELLE).unwrap();
        compile_nw_poses(&mut rig, crate::assets::POSES_JSON);
        let player = AnimPlayer::default();
        let pose = player.pose(&rig);
        let m = bone_matrices(&rig, &pose);
        let head = rig.bone("Head").unwrap();
        let p = m[head].transform_point3(Vec3::ZERO);
        assert!((p.y - (42.72 + 21.36 + 21.36)).abs() < 1e-2, "{p:?}");
        assert!(m.iter().all(|x| x.to_cols_array().iter().all(|v| v.is_finite())));
    }

    #[test]
    fn the_box_figure_is_seven_lit_boxes_about_2_58_m_tall() {
        let (mut v, mut i) = (vec![], vec![]);
        fallback_figure(&mut v, &mut i, Mat4::IDENTITY, WHITE);
        assert_eq!(v.len(), 7 * 24);
        assert_eq!(i.len(), 7 * 36);
        assert!(v.iter().all(|x| x.normal.w == LIT_MODE));
        let top = v.iter().map(|x| x.position.y).fold(f32::MIN, f32::max);
        let bottom = v.iter().map(|x| x.position.y).fold(f32::MAX, f32::min);
        assert!((top - 2.58).abs() < 1e-4);
        assert!(bottom.abs() < 1e-4);
        assert!(i.iter().all(|k| (*k as usize) < v.len()));
    }

    #[test]
    fn a_modelless_actor_builds_the_figure_at_its_position_and_yaw() {
        let mut a = actor("dovelle", DOVELLE);
        a.pos = vec3(3.0, 0.0, -2.0);
        let (mut v, mut i) = (vec![], vec![]);
        assert!(a.build_mesh(&mut v, &mut i).is_none());
        let cx = v.iter().map(|x| x.position.x).sum::<f32>() / v.len() as f32;
        assert!((cx - 3.0).abs() < 0.1);
    }

    #[test]
    fn the_humanoid_rig_draws_at_one_and_a_half_over_its_unit() {
        let a = actor("dovelle", DOVELLE);
        assert!((a.rig.draw_scale() - 1.5 / 65.0).abs() < 1e-6);
        let f = super::super::place::HunterScale::get().factor("dovelle", "");
        assert!((a.scale - 1.5 / 65.0 * f).abs() < 1e-6);
    }
}
