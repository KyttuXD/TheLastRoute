//! The figure a runner or an NPC teammate wears in the 3D view. The body is the blocky mannequin (a regular and a slim
//! mesh, `Assets::npc_body` for the regular), textured with the mannequin atlas by default or with the runner's skin
//! PNG (see `Assets::skin_texture`). A skin that is DEFAULT, or not known yet, draws the atlas.
//!
//! The pose comes from the player rig's clips through the one driver, `render/anim/runner.rs`: each figure owns a
//! `RunnerView` keyed by peer id (runners) or guest name (bots). Its `RunnerState` (idle, walk, sprint, chase,
//! crouch, jump, fall, climb, swim, stun, down) picks the clip, paces it to the real ground speed, fades the body while
//! it crouches and leans it while it swims. A remote runner's state comes from its net sample (`Sample::clip`, or the
//! moving, sprinting and crouching flags of an older client), a bot's from its movement, and the sender builds its own
//! with [`local_state`]. Everything tunable is data: `data/player/anim.json` (clips, pace, fade, wire names) and
//! `data/player/models.json` (body, atlas, height, bots).
//!
//! The local runner has no body here. The original hides it ("The body is hidden because you are inside it",
//! player_controller_3d.gd) and draws no shadow or mirror of it, so first person keeps its hands only (render/fp_hands.rs).

use super::held_wire;
use super::hide_sync::{self, HideState};
use super::world_sync::{HideSpot, RemoteView, SKIN_DEFAULT};
use crate::content::embedded;
use crate::game::{Game, Item};
use crate::render::anim::clip::Rig;
use crate::render::anim::hide;
use crate::render::anim::hunter::model_space_stretched;
use crate::render::anim::player::{AnimPlayer, Local};
use crate::render::anim::runner::{self as driver, Pose, RunnerState, RunnerView};
use crate::render::bbmodel::Model;
use crate::render::locker_occupant::{self, BodyState};
use crate::render::scene;
use crate::render::shader;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

/// The data file (under src/bin/last_route/data) that holds the runner body and its height.
const MODELS_FILE: &str = "player/models.json";

/// Which texture a runner's body draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The mannequin atlas (the default skin, or a skin not known yet).
    Atlas,
    /// The cached skin with this id.
    Skin(String),
}

/// How NPC teammates (bots) wear the runner body, from the `bots` section of models.json.
#[derive(Clone, Debug, PartialEq)]
pub struct BotSpec {
    /// The skin ids a bot may wear. A bot picks one by a hash of its guest name; an empty list draws the atlas.
    pub skins: Vec<String>,
    /// A bot moving at least this fast (m/s) walks.
    pub walk_min_mps: f32,
    /// A bot moving at least this fast (m/s) sprints.
    pub sprint_min_mps: f32,
}

/// A body mesh: its Blockbench model and atlas, as asset paths.
#[derive(Clone, Debug, PartialEq)]
pub struct BodySpec {
    pub model: String,
    pub atlas: String,
}

/// The runner figure as models.json states it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunnerSpec {
    pub body: BodySpec,
    pub slim_body: BodySpec,
    /// Model units from the feet to the top of the head, and metres per unit (audit P13).
    pub head_top_units: f32,
    pub metres_per_unit: f32,
    pub bots: BotSpec,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v[key].as_f64().map(|n| n as f32).ok_or_else(|| format!("{key} is missing or not a number"))
}

fn string<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    v[key].as_str().ok_or_else(|| format!("{key} is missing or not a string"))
}

fn body_spec(v: &serde_json::Value, id: &str) -> Result<BodySpec, String> {
    let b = &v["bodies"][id];
    Ok(BodySpec {
        model: string(b, "model").map_err(|e| format!("bodies.{id}: {e}"))?.to_string(),
        atlas: string(b, "atlas").map_err(|e| format!("bodies.{id}: {e}"))?.to_string(),
    })
}

impl RunnerSpec {
    /// Reads models.json. Every value is required; a missing or bad one is an error, never a silent default.
    pub fn parse(src: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        let r = &v["runner"];
        let body = body_spec(&v, string(r, "body").map_err(|e| format!("runner.{e}"))?)?;
        let slim_body = body_spec(&v, string(r, "slim_body").map_err(|e| format!("runner.{e}"))?)?;
        let b = &v["bots"];
        let skins = b["skins"]
            .as_array()
            .ok_or("bots.skins is missing or not a list")?
            .iter()
            .map(|s| s.as_str().map(str::to_string).ok_or("bots.skins holds a value that is not a string"))
            .collect::<Result<Vec<_>, _>>()?;
        let t = &b["state_for_speed"];
        Ok(RunnerSpec {
            body,
            slim_body,
            head_top_units: num(r, "head_top_units").map_err(|e| format!("runner.{e}"))?,
            metres_per_unit: num(r, "metres_per_unit").map_err(|e| format!("runner.{e}"))?,
            bots: BotSpec {
                skins,
                walk_min_mps: num(t, "walk_min_mps").map_err(|e| format!("bots.state_for_speed.{e}"))?,
                sprint_min_mps: num(t, "sprint_min_mps").map_err(|e| format!("bots.state_for_speed.{e}"))?,
            },
        })
    }

    /// The figure's height in metres: the head top in model units times the metres per unit.
    pub fn height_m(&self) -> f32 {
        self.head_top_units * self.metres_per_unit
    }

    /// The body to draw: the slim mesh when `slim`, the regular otherwise.
    pub fn body_for(&self, slim: bool) -> &BodySpec {
        if slim {
            &self.slim_body
        } else {
            &self.body
        }
    }
}

/// The state of a bot that moved at `speed` metres per second, on foot, on a ladder (`climbing`) or in a drop (`falling`).
pub fn bot_state(bots: &BotSpec, speed: f32, climbing: bool, falling: bool) -> RunnerState {
    RunnerState {
        moving: speed >= bots.walk_min_mps,
        sprinting: speed >= bots.sprint_min_mps,
        climbing,
        airborne_down: falling,
        speed,
        ..RunnerState::default()
    }
}

/// What a remote runner's sample says it is doing: the pose it names, or an older client's three flags.
pub fn remote_state(r: &RemoteView) -> RunnerState {
    let mut state = RunnerState::from_sample(r.clip, r.moving, r.sprinting, r.crouching, r.vy, r.speed);
    // an airborne runner's jump clip is placed by its wire height and vertical speed, so it spans the real airtime
    if state.airborne_up || state.airborne_down {
        state.air_phase = Some(driver::air_phase(r.jump, r.vy));
    }
    state
}

/// The state of this runner's own body, for the sample it sends (`Sample::with_body`) and for anything that draws it.
/// Down once its route has ended or while it lies downed (game/revive.rs), stunned while locked, on a ladder (`game/ladder.rs`),
/// airborne with the sign of its vertical speed, swimming in water as deep as the swim rules say, chased while a
/// hunter is in the chase.
pub fn local_state(g: &Game) -> RunnerState {
    let p = &g.p;
    let airborne = !p.grounded;
    RunnerState {
        moving: p.moving,
        sprinting: p.sprinting,
        crouching: p.crouching,
        airborne_up: airborne && p.vy() >= 0.0,
        airborne_down: airborne && p.vy() < 0.0,
        climbing: g.ladder.is_some(),
        chased: g.hunters.iter().any(|h| h.active && h.state == crate::hunters::brain::State::Chase),
        swimming: crate::game::swim_effects::SwimState::is_swimming(&g.swim_cfg, g.swim.depth),
        stunned: p.stun > 0.0,
        // a downed runner lies down for the others too (game/revive.rs); a dead one's route has ended
        down: g.report.is_some() || g.runner_downed(),
        speed: p.move_vel.0.hypot(p.move_vel.1),
        // the sample's pose is the walk clip either way; a viewer reads a backwards step from the sample's position
        backward: false,
        // the host plays its own jump in time
        air_phase: None,
    }
}

/// The locker this runner is hidden in, in absolute cells, for its sample.
pub fn local_hide(g: &Game) -> Option<HideSpot> {
    let l = g.fac.hiding.get(g.p.hidden?)?;
    let o = g.window_origin();
    Some(HideSpot { cell: (l.cell.x + o.0, l.cell.y + o.1), facing: l.facing })
}

/// The spec from data/player/models.json, read once. None when the file is missing or invalid: the figures then stand
/// in their rest pose (the error is printed once).
fn runner_spec() -> Option<&'static RunnerSpec> {
    static SPEC: OnceLock<Option<RunnerSpec>> = OnceLock::new();
    SPEC.get_or_init(|| {
        let (path, src) = embedded::data_under(MODELS_FILE).next()?;
        match RunnerSpec::parse(src) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("{path}: {e}");
                None
            }
        }
    })
    .as_ref()
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

/// The scene y a remote runner is drawn at: its absolute feet height less the active floor's absolute y (the scene
/// frame puts the active floor at 0). A runner on a flight is drawn at its height whatever its label says.
pub fn runner_draw_y(h: f32, active_floor_y: f32) -> f32 {
    h - active_floor_y
}

/// The scene y a remote runner's figure is drawn at: [`runner_draw_y`] plus the runner's jump height from the wire, so a
/// jumping runner leaves the floor in every view that draws it.
pub fn remote_draw_y(r: &RemoteView, active_floor_y: f32) -> f32 {
    runner_draw_y(r.h, active_floor_y) + r.jump
}

/// The body alpha a fade amount draws with: Godot's `transparency = fade`, so 0 is opaque and 0.55 is 45% opaque.
pub fn body_alpha(fade: f32) -> f32 {
    (1.0 - fade).clamp(0.0, 1.0)
}

/// The shader mode a body draws in: opaque and lit, or lit and blended once it is faded (a crouch).
pub fn body_mode(alpha: f32) -> f32 {
    if alpha < 1.0 {
        shader::GHOST_MODE
    } else {
        shader::LIT_MODE
    }
}

/// Whether a remote runner that is hidden has finished sliding into its locker: from then on its body is gone
/// (Godot remote_players.gd 542-563 keeps it visible for the length of the local hide move).
pub fn hidden_for_good(r: &RemoteView) -> bool {
    r.hide.is_some() && r.hide_since.map_or(true, |t| t.as_secs_f32() >= crate::render::hiding_cam::glide_seconds(true))
}

/// Who a running clip belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FigureKey {
    Peer(u32),
    /// An NPC teammate, by its guest name (unique per NPC within a run).
    Bot(String),
}

struct Figure {
    /// The driver state; None when the player rig asset is missing (the body then stands in its rest pose).
    view: Option<RunnerView>,
    slim: bool,
    /// A bot's last floor position, for its speed.
    last_at: Option<(f32, f32)>,
    /// The body's last facing, for the head's lag behind a turn.
    last_yaw: Option<f32>,
    /// The emote it plays: (id, the sample's start sequence number, seconds since it began).
    emote: Option<(String, u32, f32)>,
    seen: u64,
}

impl Figure {
    fn new(epoch: u64, slim: bool) -> Figure {
        Figure { view: RunnerView::load(slim), slim, last_at: None, last_yaw: None, emote: None, seen: epoch }
    }
}

/// What a remote runner's sample says it is emoting: the emote id and its start sequence number. None when it names
/// none (an older client, or the emote ended). The id may be one this build does not know; posing ignores it.
pub fn remote_emote(r: &RemoteView) -> Option<(&str, u32)> {
    r.emote.as_ref().map(|(id, seq)| (id.as_str(), *seq))
}

/// Moves a figure's emote clock by `dt`. A new id or a new sequence number restarts it; a sample without one clears it.
fn advance_emote(slot: &mut Option<(String, u32, f32)>, wanted: Option<(&str, u32)>, dt: f32) {
    match wanted {
        None => *slot = None,
        Some((id, seq)) => match slot {
            Some((cur, cur_seq, t)) if cur == id && *cur_seq == seq => *t += dt.max(0.0),
            _ => *slot = Some((id.to_string(), seq, 0.0)),
        },
    }
}

/// Lays emote `id` at `t` seconds over a body's bone pose. False (pose untouched) for an unknown id or a finished emote.
fn pose_emote(rig: &Rig, locals: &mut [Local], id: &str, t: f32) -> bool {
    crate::render::anim::emote_pose::set().is_some_and(|set| crate::render::anim::emote_pose::apply(locals, rig, set, id, t))
}

thread_local! {
    /// (the draw pass counter, every figure with a running clip)
    static FIGURES: RefCell<(u64, HashMap<FigureKey, Figure>)> = RefCell::new((0, HashMap::new()));
    static SLIM: RefCell<Option<Option<Rc<Model>>>> = const { RefCell::new(None) };
    /// Where each bot's head looks, by guest name: (yaw offset from the body, + left; pitch, + up), in radians. The host
    /// sets it from its bot's life state and a guest from the wire (`set_bot_look`); a bot with none looks straight.
    static BOT_LOOK: RefCell<HashMap<String, (f32, f32)>> = RefCell::new(HashMap::new());
    /// The vertex and index buffers every figure is built into, so a frame allocates none per figure.
    static SCRATCH: RefCell<(Vec<Vertex>, Vec<u16>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// The still pose of `pose` (its clip at `phase` of its length), for a figure with no view of its own.
fn still_pose(rig: &Rig, pose: Pose, phase: f32) -> Vec<Local> {
    let clip = driver::config().clips.get(pose.key()).map(String::as_str).unwrap_or(pose.key());
    let mut p = AnimPlayer::default();
    p.play(rig, clip, 0.0);
    p.seek(phase.clamp(0.0, 1.0) * rig.length(clip));
    p.advance(rig, 0.0);
    p.pose(rig)
}

/// The slim body model, loaded once on the regular body's atlas. None when the slim model is not embedded (the regular
/// body is drawn then).
fn slim_body(a: &Assets, spec: Option<&RunnerSpec>) -> Option<Rc<Model>> {
    SLIM.with(|s| {
        let mut s = s.borrow_mut();
        s.get_or_insert_with(|| {
            let spec = spec?;
            let regular = a.npc_body.as_ref()?;
            let src = embedded::asset(&spec.slim_body.model).and_then(|b| std::str::from_utf8(b).ok())?;
            let find = |name: &str| embedded::asset(&format!("player/{name}")).map(|b| b.to_vec());
            Model::parse(src, Some(regular.texture.clone()), &find, None).map(Rc::new)
        })
        .clone()
    })
}

/// How one figure is placed and shown: where it stands, which way it faces, how far it leans forward (a swimmer) and how
/// transparent it is (a crouch).
#[derive(Clone, Copy, Debug)]
struct Look {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    alpha: f32,
}

/// The root transform of a figure: feet at `pos`, turned by `yaw`, leaning forward by `pitch` about the hips.
fn body_root(look: &Look, unit: f32) -> Mat4 {
    let pivot = Vec3::Y * driver::config().swim_pivot;
    let lean = if look.pitch.abs() > 1e-4 {
        Mat4::from_translation(pivot) * Mat4::from_rotation_x(look.pitch) * Mat4::from_translation(-pivot)
    } else {
        Mat4::IDENTITY
    };
    Mat4::from_translation(look.pos) * Mat4::from_rotation_y(look.yaw) * lean * Mat4::from_scale(Vec3::splat(unit))
}

/// Builds and draws one figure. `pose` is the rig and its local bone pose; None leaves the body in its rest pose.
/// Returns the world point of a held flashlight's lamp (where its beam starts), None when no flashlight is drawn.
fn draw_body(a: &Assets, look: Look, pick: &Pick, slim: bool, pose: Option<(&Rig, &[Local])>, held: Option<Item>) -> Option<Vec3> {
    let Some(regular) = a.npc_body.as_ref() else {
        return None;
    };
    let spec = runner_spec();
    let slim_model = if slim { slim_body(a, spec) } else { None };
    let body: &Model = slim_model.as_deref().unwrap_or(regular);
    let texture = match pick {
        Pick::Skin(id) => a.skin_texture(id).unwrap_or_else(|| body.texture.clone()),
        Pick::Atlas => body.texture.clone(),
    };
    let unit = spec.map(|s| s.metres_per_unit).unwrap_or(body.scale);
    let root = body_root(&look, unit);
    let mats = pose.map(|(rig, locals)| (rig, model_space_stretched(rig, locals, &[])));
    let bone_of = |k: usize| {
        let (rig, m) = mats.as_ref()?;
        rig.bone(&body.groups[k].name).map(|b| m[b])
    };
    let (mut verts, mut idx) = SCRATCH.with(|s| std::mem::take(&mut *s.borrow_mut()));
    verts.clear();
    idx.clear();
    let tint = Color::new(1.0, 1.0, 1.0, look.alpha);
    body.build_posed_mode(&mut verts, &mut idx, root, &bone_of, Vec2::ZERO, tint, body_mode(look.alpha));
    let mesh = Mesh { vertices: verts, indices: idx, texture: Some(texture) };
    crate::render::draw_stats::draw_counted(&mesh);
    SCRATCH.with(|s| *s.borrow_mut() = (mesh.vertices, mesh.indices));
    // the held item rides the hand bone (no rig, no hand, no item)
    if let (Some(item), Some((rig, m)), Some(d)) = (held, mats.as_ref(), held_wire::data()) {
        if let (Some(b), Some(model)) = (rig.bone(&d.bone), scene::item_model(a, scene::item_file(item))) {
            let xf = held_wire::item_transform(d, item, root, m[b], unit);
            model.draw_faded(xf, tint);
            // a flashlight's beam starts at its lamp, out at the end of the model, not at the feet
            if item == Item::Flashlight {
                return Some(xf.transform_point3(held_wire::lamp_point(model.min, model.max)));
            }
        }
    }
    None
}

/// Draws one still figure: the same body a runner wears, at `pos` (feet) and `yaw`, in `pose` at `phase` (0..1) of its
/// clip. The caller sets the camera and material, as for [`draw_runners`]. The locker occupant uses this.
pub fn draw_figure(a: &Assets, pos: Vec3, yaw: f32, pick: Pick, slim: bool, pose: Pose, phase: f32) {
    draw_figure_emoting(a, pos, yaw, pick, slim, pose, phase, None);
}

/// [`draw_figure`] with an emote laid over the pose: `emote` is (id, seconds since it began). The local player's own
/// body in the third-person views (spectate, locker) uses this with `EmoteState::local_pose`; first person draws none.
#[allow(clippy::too_many_arguments)]
pub fn draw_figure_emoting(a: &Assets, pos: Vec3, yaw: f32, pick: Pick, slim: bool, pose: Pose, phase: f32, emote: Option<(&str, f32)>) {
    let rig = driver::player_rig(slim);
    let mut locals = rig.as_ref().map(|r| still_pose(r, pose, phase));
    if let (Some(r), Some(l), Some((id, t))) = (rig.as_ref(), locals.as_mut(), emote) {
        pose_emote(r, l, id, t);
    }
    let posed = rig.as_ref().zip(locals.as_ref()).map(|(r, l)| (&**r, l.as_slice()));
    let _ = draw_body(a, Look { pos, yaw, pitch: 0.0, alpha: 1.0 }, &pick, slim, posed, None);
}

/// Above this ground speed a step is a teleport (a respawn or a ladder snap), not a walk, so it never reads as backwards.
const TELEPORT_MPS: f32 = 15.0;

/// Whether a figure with ground velocity `vel` (x, z in metres per second) that faces `yaw` is walking backwards: its
/// motion points more than 120 degrees away from the way it faces (`forward2` is (-sin yaw, -cos yaw), as the game
/// uses it). Sideways and forward motion, and a still figure, are not backwards.
pub fn moving_backward(vel: (f32, f32), yaw: f32) -> bool {
    let speed = vel.0.hypot(vel.1);
    if speed <= 1e-4 || speed > TELEPORT_MPS {
        return false;
    }
    let (fx, fz) = (-yaw.sin(), -yaw.cos());
    (vel.0 * fx + vel.1 * fz) / speed < -0.5
}

/// Sets where bot `name` looks this frame: the head's yaw offset from the body (+ left) and its pitch (+ up), in radians.
/// Call before [`draw_bots`] (the host from the bot's life state, a guest from the wire).
pub fn set_bot_look(name: &str, yaw: f32, pitch: f32) {
    if !yaw.is_finite() || !pitch.is_finite() {
        return;
    }
    BOT_LOOK.with(|m| {
        let mut m = m.borrow_mut();
        // the name is copied only the first time a bot looks; after that the entry is updated in place
        match m.get_mut(name) {
            Some(look) => *look = (yaw, pitch),
            None => {
                m.insert(name.to_string(), (yaw, pitch));
            }
        }
    });
}

/// The head's yaw once a bot's look offset is added to what the driver eased it to (the lag behind a turn): capped so the
/// head never turns right round.
pub fn looked_head(base: f32, look: f32) -> f32 {
    (base + look).clamp(-LOOK_YAW_CAP, LOOK_YAW_CAP)
}

/// The most a head may turn from the body with a bot's look added (radians): a safety cap above the data's limit.
const LOOK_YAW_CAP: f32 = 1.5;

/// Whether a body in this state holds its head straight: down, stunned, or on a ladder.
fn holds_head(state: &RunnerState) -> bool {
    matches!(state.pose(), Pose::Down | Pose::Stun | Pose::Climb)
}

/// Aims one driven figure's head and body at its facing `yaw`: the head turns first, and the body follows once the head is
/// past its free range (`RunnerView::aim_head`). `first` seeds the body to the facing, for a new figure or a rebuilt one.
/// A body that holds its head aims straight ahead of itself, so its head stays level and its body keeps its facing.
fn aim(v: &mut RunnerView, first: bool, yaw: f32, look_pitch: f32, turn: f32, state: &RunnerState, dt: f32) {
    if first {
        v.seed_yaw(yaw);
    }
    if holds_head(state) {
        let body = v.body_yaw;
        v.aim_head(0.0, body, 0.0, false, dt);
    } else {
        v.aim_head(look_pitch, yaw, turn, state.moving, dt);
    }
}

/// Draws one driven figure and returns its held flashlight's lamp point (see [`draw_body`]). `look_pitch` is the runner's
/// aim (+ up) that tilts its head; a bot passes where it looks. `look_yaw` turns the head from the body (+ left); a
/// runner or an older peer passes 0. The figure's own ground step since its last frame says whether a moving state is
/// walking backwards (a remote runner's or a bot's facing is its yaw).
#[allow(clippy::too_many_arguments)]
fn draw_driven(a: &Assets, key: FigureKey, pos: Vec3, yaw: f32, look_yaw: f32, look_pitch: f32, pick: &Pick, slim: bool, state: &RunnerState, held: Option<Item>, emote: Option<(&str, u32)>, dt: f32, epoch: u64) -> Option<Vec3> {
    let driven = FIGURES.with(|f| {
        let mut f = f.borrow_mut();
        let fig = f.1.entry(key).or_insert_with(|| Figure::new(epoch, slim));
        if fig.slim != slim {
            *fig = Figure { last_at: fig.last_at, ..Figure::new(epoch, slim) };
        }
        fig.seen = epoch;
        // a new figure, or one rebuilt for a slim switch, has no earlier facing: its body starts where it looks
        let first = fig.last_yaw.is_none();
        // the body's turn rate (radians a second, + left), for the head's lag; 0 on the first frame
        let turn = match (fig.last_yaw, dt > 1e-5) {
            (Some(last), true) => (driver::wrap_angle(yaw - last)) / dt,
            _ => 0.0,
        };
        fig.last_yaw = Some(yaw);
        advance_emote(&mut fig.emote, emote, dt);
        let emoting = fig.emote.clone();
        fig.view.as_mut().map(|v| {
            let here = (pos.x, pos.z);
            let mut state = *state;
            if let (Some(last), true) = (v.last_pos, dt > 1e-5) {
                let vel = ((here.0 - last.0) / dt, (here.1 - last.1) / dt);
                state.backward = state.moving && moving_backward(vel, yaw);
            }
            v.last_pos = Some(here);
            v.update(&state, dt);
            aim(v, first, yaw, look_pitch, turn, &state, dt);
            // a bot's look turns the head from the body; the driver's own easing is kept for the next frame
            let eased = v.head_yaw;
            if !holds_head(&state) {
                v.head_yaw = looked_head(eased, look_yaw);
            }
            let mut locals = v.locals();
            v.head_yaw = eased;
            if let Some((id, _, t)) = &emoting {
                pose_emote(&v.rig, &mut locals, id, *t);
            }
            (v.rig.clone(), locals, v.fade, v.pitch, v.body_yaw)
        })
    });
    match driven {
        Some((rig, locals, fade, pitch, body_yaw)) => {
            let alpha = body_alpha(fade);
            let held = held_wire::shown(held, false, state.pose(), alpha);
            // the body is drawn at its own facing, which trails the aim (the head turns first); the aim is for the head
            draw_body(a, Look { pos, yaw: body_yaw, pitch, alpha }, pick, slim, Some((&rig, &locals)), held)
        }
        None => draw_body(a, Look { pos, yaw, pitch: 0.0, alpha: 1.0 }, pick, slim, None, None),
    }
}

/// Starts a draw pass and returns its number.
fn next_epoch() -> u64 {
    FIGURES.with(|f| {
        let mut f = f.borrow_mut();
        f.0 += 1;
        f.0
    })
}

/// Drops the figures of one kind (bots or runners) that pass `epoch` did not draw: a runner who left, a bot removed.
fn prune(epoch: u64, bots_pass: bool) {
    FIGURES.with(|f| {
        f.borrow_mut().1.retain(|k, fig| fig.seen == epoch || matches!(k, FigureKey::Bot(_)) != bots_pass);
    });
}

/// Tells the locker doors which lockers the remote runners are in, so a door swings when one goes in or comes out
/// (`render/anim/hide.rs`). Call once per drawn frame, with an empty list when no runner is in the room.
pub fn update_remote_doors(g: &Game, remotes: &[RemoteView]) {
    let cells: Vec<(i32, i32)> = remotes.iter().filter_map(|r| r.hide.map(|h| h.cell)).collect();
    hide::tick_remote(&cells, g.time);
}

/// Draws the runners that are inside a locker whose door is swinging: the door opens and the body is seen for that
/// moment ([`locker_occupant`] places it, [`hide_sync::occupancy`] gives each hider its slot).
fn draw_occupants(g: &Game, a: &Assets, remotes: &[RemoteView], active_y: f32) {
    let hidden: Vec<&RemoteView> = remotes.iter().filter(|r| hidden_for_good(r)).collect();
    let mut cells: Vec<(i32, i32)> = vec![];
    for r in &hidden {
        let c = r.hide.map(|h| h.cell).unwrap_or_default();
        if !cells.contains(&c) {
            cells.push(c);
        }
    }
    if cells.is_empty() {
        return;
    }
    let style = &hide_sync::config().locker_style;
    let capacity = hide_sync::config().max_capacity;
    let states: Vec<HideState> = hidden
        .iter()
        .map(|r| {
            let cell = r.hide.map(|h| h.cell).unwrap_or_default();
            HideState::hiding(cells.iter().position(|c| *c == cell).unwrap_or(0) as i32, style, 0)
        })
        .collect();
    let spots = hide_sync::occupancy(&vec![capacity; cells.len()], &states);
    let data = locker_occupant::data();
    for (ci, spot) in spots.iter().enumerate() {
        if spot.occupants.is_empty() || hide::remote_door_for(cells[ci], g.time) <= 0.0 {
            continue;
        }
        let who: Vec<&RemoteView> = spot.occupants.iter().map(|&(i, _)| hidden[i]).collect();
        let facing = who[0].hide.map(|h| h.facing).unwrap_or((0, 1));
        let locker = crate::world::facility::Hiding {
            cell: crate::world::facility::Cell::new(cells[ci].0, cells[ci].1),
            facing,
            room: None,
            kind: String::new(),
        };
        let centre = g.fac.locker_center(&locker);
        locker_occupant::draw(data, style, who.len(), capacity as usize, centre, facing, &mut |slot, p| {
            let r = who[slot];
            let tex = pick(&r.skin, &|id| a.skin_texture(id).is_some());
            let (pose, phase) = match p.state {
                BodyState::Standing => (Pose::Idle, p.idle_phase),
                BodyState::Lying => (Pose::Down, 0.0),
            };
            let pos = vec3(p.pos[0], runner_draw_y(r.h, active_y) + p.pos[1], p.pos[2]);
            draw_figure(a, pos, p.yaw, tex, r.slim, pose, phase);
        });
    }
}

/// Draws the remote runners in the 3D view, lit like the scene. Each one is the mannequin at its position and yaw,
/// wearing its skin's texture, posed by its state. A runner that has slid into a locker is gone from the open (its body
/// shows only while the locker door swings). Draws nothing for an empty list, so a single-player frame is unchanged.
/// The world lights of the other runners' lit flashlights on this floor, for the world pass to add to the room's lamps.
/// A hidden runner has no beam, so it has no light either. The lamp sits at the usual lamp height (the held lamp's tip
/// is only known once the body is drawn, a few centimetres away).
pub fn flashlight_lamps(g: &Game, remotes: &[RemoteView]) -> Vec<crate::render::shader::Lamp> {
    let active_y = crate::game::level_stack::floor_y_of(g.fac.level);
    remotes
        .iter()
        .filter(|r| !hidden_for_good(r) && r.hide.is_none())
        .filter(|r| r.flash_pitch.is_some())
        .map(|r| {
            let pos = vec3(r.x, remote_draw_y(r, active_y), r.y);
            crate::render::beam::remote_lamp(crate::render::beam::flashlight_apex(pos, r.yaw, r.crouching))
        })
        .collect()
}

pub fn draw_runners(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    let epoch = next_epoch();
    if remotes.is_empty() {
        prune(epoch, false);
        return;
    }
    set_camera(&scene::camera(g, rt));
    let active_y = crate::game::level_stack::floor_y_of(g.fac.level);
    // released on every way out, the early return below included: a leaked world material blacks out the rest of the frame
    let _world = crate::render::pipeline::world_material(mat);
    shader::apply(mat, &scene::lighting(g), &scene::flickered(g, &g.world.lamps));
    if a.npc_body.is_none() {
        // The mannequin failed to load: a plain grey post keeps the runner visible in the 3D view.
        let mut b = crate::render::builder::Builder::new(Some(a.white.clone()));
        for r in remotes {
            b.cube(vec3(r.x, remote_draw_y(r, active_y) + 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, GRAY, 2.0);
        }
        b.draw_and_clear();
        return;
    }
    let dt = get_frame_time().clamp(0.0, 0.1);
    for r in remotes {
        if hidden_for_good(r) {
            continue;
        }
        let tex = pick(&r.skin, &|id| a.skin_texture(id).is_some());
        let pos = vec3(r.x, remote_draw_y(r, active_y), r.y);
        let held = if r.hide.is_some() { None } else { r.held };
        let lamp = draw_driven(a, FigureKey::Peer(r.peer), pos, r.yaw, 0.0, r.look_pitch, &tex, r.slim, &remote_state(r), held, remote_emote(r), dt, epoch);
        // a lit flashlight shows its beam to everyone who sees the runner (render/beam.rs); a hidden runner has none.
        // The beam leaves the lamp in the held flashlight's hand; without one drawn it leaves the usual lamp height.
        if let (Some(pitch), None) = (r.flash_pitch, r.hide) {
            let apex = lamp.unwrap_or_else(|| crate::render::beam::flashlight_apex(pos, r.yaw, r.crouching));
            let mut beam = crate::render::beam::flashlight_cone(apex, r.yaw, pitch);
            beam.texture = Some(a.white.clone());
            crate::render::draw_stats::draw_counted(&beam);
        }
    }
    prune(epoch, false);
    draw_occupants(g, a, remotes, active_y);
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

/// The ground speed of a bot that moved from `last` to `now` in `dt` seconds. 0 for the first frame or a zero step.
pub fn bot_speed(last: Option<(f32, f32)>, now: (f32, f32), dt: f32) -> f32 {
    match last {
        Some(p) if dt > 0.0 => (now.0 - p.0).hypot(now.1 - p.1) / dt,
        _ => 0.0,
    }
}

/// One NPC teammate as the 3D view needs it: its guest name (the skin seed and figure key), its floor position and yaw.
/// `stride` and `walking` stay for the callers that fill them; the figure derives its state from its movement and from
/// the edge it walks (`climbing` on a ladder, `falling` down a drop).
pub struct BotView<'a> {
    pub name: &'a str,
    pub at: (f32, f32),
    pub facing: f32,
    pub stride: f32,
    pub walking: bool,
    /// Draw offset of the feet from the drawn floor (render::stack_actors::actor_dy); 0 on the drawn floor.
    pub y: f32,
    pub climbing: bool,
    pub falling: bool,
    /// Crouching, and the emote playing (id, play counter); None for no emote.
    pub crouch: bool,
    pub emote: Option<(&'a str, u32)>,
}

/// Draws NPC teammates with the runner mannequin and the same driver, each wearing its skin (see [`bot_skin`]). A bot's
/// state comes from how far it moved since the last frame ([`bot_state`]). The caller sets the camera and
/// material, as for [`draw_runners`]. Draws nothing for an empty list.
pub fn draw_bots(a: &Assets, bots: &[BotView]) {
    let dt = get_frame_time().clamp(0.0, 0.1);
    let epoch = next_epoch();
    if bots.is_empty() || a.npc_body.is_none() {
        prune(epoch, true);
        return;
    }
    let spec = runner_spec();
    let skins: &[String] = spec.map(|p| p.bots.skins.as_slice()).unwrap_or(&[]);
    for b in bots {
        let key = FigureKey::Bot(b.name.to_string());
        let speed = FIGURES.with(|f| {
            let mut f = f.borrow_mut();
            let fig = f.1.entry(key.clone()).or_insert_with(|| Figure::new(epoch, false));
            let speed = bot_speed(fig.last_at, b.at, dt);
            fig.last_at = Some(b.at);
            speed
        });
        let mut state = spec.map(|s| bot_state(&s.bots, speed, b.climbing, b.falling)).unwrap_or_default();
        state.crouching = b.crouch;
        let tex = pick(&bot_skin(b.name, skins), &|id| a.skin_texture(id).is_some());
        let (look_yaw, look_pitch) = BOT_LOOK.with(|m| m.borrow().get(b.name).copied()).unwrap_or((0.0, 0.0));
        draw_driven(a, key, vec3(b.at.0, b.y, b.at.1), b.facing, look_yaw, look_pitch, &tex, false, &state, None, b.emote, dt, epoch);
    }
    // a bot that was not drawn this pass loses its look
    BOT_LOOK.with(|m| m.borrow_mut().retain(|name, _| bots.iter().any(|b| b.name == name)));
    prune(epoch, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::anim::player::AnimPlayer;
    use crate::render::anim::runner::player_rig;

    /// The models.json text the game embeds.
    fn shipped_text() -> &'static str {
        embedded::data_under(MODELS_FILE).next().expect("player/models.json is embedded").1
    }

    fn shipped() -> RunnerSpec {
        RunnerSpec::parse(shipped_text()).expect("models.json parses")
    }

    fn remote(clip: Option<Pose>, moving: bool, sprinting: bool, crouching: bool, vy: f32) -> RemoteView {
        RemoteView {
            peer: 3,
            name: String::new(),
            x: 0.0,
            y: 0.0,
            yaw: 0.0,
            h: 0.0,
            level: 0,
            skin: String::new(),
            moving,
            sprinting,
            crouching,
            slim: false,
            speed: 0.0,
            vy,
            jump: 0.0,
            clip,
            held: None,
            flash_pitch: None,
            look_pitch: 0.0,
            emote: None,
            hide: None,
            hide_since: None,
        }
    }

    #[test]
    fn remote_on_a_flight_is_drawn_at_its_height() {
        assert_eq!(runner_draw_y(-2.5, 0.0), -2.5);
        assert_eq!(runner_draw_y(-2.5, -5.0), 2.5);
    }

    #[test]
    fn a_remote_jump_is_drawn_off_the_floor_by_its_wire_height() {
        let mut r = remote(None, false, false, false, 0.0);
        r.h = 2.0;
        r.jump = 0.5;
        assert_eq!(remote_draw_y(&r, 0.0), 2.5, "the height plus the jump");
        assert_eq!(remote_draw_y(&r, -1.0), 3.5, "the active floor is taken off both");
        r.jump = 0.0;
        assert_eq!(remote_draw_y(&r, 0.0), runner_draw_y(r.h, 0.0), "on the ground the draw is unchanged");
    }

    #[test]
    fn a_remote_jump_places_its_air_clip_by_height_and_vertical_speed() {
        let peak = driver::config().jump_peak;
        let mut up = remote(Some(Pose::Jump), false, false, false, 1.0);
        up.jump = peak * 0.5;
        let s = remote_state(&up);
        assert_eq!(s.pose(), Pose::Jump);
        let phase = |s: &RunnerState| s.air_phase.expect("an airborne runner has a phase");
        assert!((phase(&s) - 0.25).abs() < 1e-5, "rising at half the peak is a quarter into the clip");
        let mut down = remote(Some(Pose::Fall), false, false, false, -1.0);
        down.jump = peak * 0.5;
        assert!((phase(&remote_state(&down)) - 0.75).abs() < 1e-5, "falling at half the peak is three quarters in");
        let ground = remote(None, true, false, false, 0.0);
        assert_eq!(remote_state(&ground).air_phase, None, "a runner on the floor has no air phase");
    }

    #[test]
    fn a_rebuilt_figure_starts_its_body_at_its_aim() {
        let mut v = RunnerView::load(false).expect("the player rig is embedded");
        let still = RunnerState::default();
        // a new figure: its body is where it looks, and its head is level
        aim(&mut v, true, 1.2, 0.0, 0.0, &still, 1.0 / 60.0);
        assert!((v.body_yaw - 1.2).abs() < 1e-6, "the first frame seeds the body: {}", v.body_yaw);
        assert!(v.head_yaw.abs() < 1e-3, "the head is level on the first frame");
        // turn the aim far past the free range: the body starts to follow
        for _ in 0..120 {
            aim(&mut v, false, 3.0, 0.0, 0.0, &still, 1.0 / 60.0);
        }
        assert!((v.body_yaw - 1.2).abs() > 0.1, "the body follows a turn past the free range: {}", v.body_yaw);
        // a slim switch rebuilds the figure: the new body starts where the aim is, not where the old body was
        aim(&mut v, true, -0.7, 0.0, 0.0, &still, 1.0 / 60.0);
        assert!((v.body_yaw + 0.7).abs() < 1e-6, "a rebuilt figure reseeds: {}", v.body_yaw);
    }

    #[test]
    fn a_figure_that_holds_its_head_keeps_its_body_facing() {
        let mut v = RunnerView::load(false).expect("rig");
        v.seed_yaw(0.4);
        let down = RunnerState { down: true, ..RunnerState::default() };
        for _ in 0..120 {
            aim(&mut v, false, 2.5, 0.0, 0.0, &down, 1.0 / 60.0);
        }
        assert_eq!(v.body_yaw, 0.4, "a downed body does not turn to its aim");
        assert!(v.head_yaw.abs() < 1e-3, "and its head stays straight");
    }

    #[test]
    fn a_default_runner_uses_the_atlas_and_an_unknown_skin_falls_back() {
        let none = |_: &str| false;
        assert_eq!(pick(SKIN_DEFAULT, &|_| true), Pick::Atlas, "DEFAULT is always the atlas");
        assert_eq!(pick("NO_SUCH_SKIN", &none), Pick::Atlas, "an unknown id draws the atlas");
        let known = "ABC";
        assert_eq!(pick(known, &|id| id == known), Pick::Skin(known.into()));
        assert_eq!(pick(known, &none), Pick::Atlas, "a skin not loaded yet draws the atlas");
    }

    #[test]
    fn an_older_clients_flags_pick_the_pose() {
        let pose = |m, s, c| remote_state(&remote(None, m, s, c, 0.0)).pose();
        assert_eq!(pose(true, false, false), Pose::Walk);
        assert_eq!(pose(true, true, false), Pose::Sprint);
        assert_eq!(pose(false, false, false), Pose::Idle);
        assert_eq!(pose(false, true, false), Pose::Idle, "sprint without moving is idle");
        assert_eq!(pose(true, true, true), Pose::Crouch, "crouching wins");
    }

    #[test]
    fn a_named_pose_is_the_pose_shown() {
        for p in Pose::ALL {
            let shown = remote_state(&remote(Some(p), false, false, false, if p == Pose::Fall { -3.0 } else { 1.0 })).pose();
            assert_eq!(shown, p, "{p:?} survives the sample");
        }
        // an airborne sample is a jump while rising and a fall while sinking, whichever of the two it names
        assert_eq!(remote_state(&remote(Some(Pose::Jump), true, false, false, -2.0)).pose(), Pose::Fall);
        assert_eq!(remote_state(&remote(Some(Pose::Fall), true, false, false, 2.0)).pose(), Pose::Jump);
    }

    #[test]
    fn a_bot_state_follows_its_speed_and_its_edge() {
        let spec = shipped();
        let dt = 1.0 / 60.0;
        let still = bot_speed(Some((3.0, 4.0)), (3.0, 4.0), dt);
        assert_eq!(bot_state(&spec.bots, still, false, false).pose(), Pose::Idle);
        let first = bot_speed(None, (3.0, 4.0), dt);
        assert_eq!(bot_state(&spec.bots, first, false, false).pose(), Pose::Idle, "no earlier frame means no speed");
        let moved = bot_speed(Some((3.0, 4.0)), (3.05, 4.0), dt);
        assert!((moved - 3.0).abs() < 1e-3, "0.05 m in a 60 Hz frame is 3 m/s, got {moved}");
        assert_eq!(bot_state(&spec.bots, moved, false, false).pose(), Pose::Walk);
        let fast = bot_speed(Some((0.0, 0.0)), (0.1, 0.0), dt);
        assert_eq!(bot_state(&spec.bots, fast, false, false).pose(), Pose::Sprint, "6 m/s sprints");
        assert_eq!(bot_state(&spec.bots, 0.0, true, false).pose(), Pose::Climb, "a ladder climbs");
        assert_eq!(bot_state(&spec.bots, 0.0, false, true).pose(), Pose::Fall, "a drop falls");
    }

    #[test]
    fn a_backwards_step_is_backwards_and_a_forward_one_is_not() {
        // facing yaw 0 looks along -z (forward2 = (0, -1)); facing yaw pi/2 looks along -x
        assert!(moving_backward((0.0, 2.0), 0.0), "a step back at yaw 0 is backwards");
        assert!(!moving_backward((0.0, -2.0), 0.0), "a step forward at yaw 0 is not");
        assert!(moving_backward((2.0, 0.0), std::f32::consts::FRAC_PI_2), "a step back at yaw pi/2 is backwards");
        assert!(!moving_backward((-2.0, 0.0), std::f32::consts::FRAC_PI_2), "a step forward at yaw pi/2 is not");
        assert!(!moving_backward((2.0, 0.0), 0.0), "a sideways step is not backwards");
        assert!(!moving_backward((0.0, 0.0), 0.0), "standing still is not backwards");
        assert!(!moving_backward((0.0, 40.0), 0.0), "a teleport-sized step is not a walk");
    }

    #[test]
    fn slim_flag_chooses_the_slim_body() {
        let spec = shipped();
        assert_eq!(spec.body_for(false), &spec.body);
        assert_eq!(spec.body_for(true), &spec.slim_body);
        assert_ne!(spec.body.model, spec.slim_body.model, "the slim body is a different mesh");
    }

    #[test]
    fn the_figure_is_one_point_nine_metres_tall() {
        let spec = shipped();
        assert_eq!(spec.head_top_units, 113.92, "audit P13");
        assert!((spec.metres_per_unit - 1.067 / 64.0).abs() < 1e-5);
        assert!((spec.height_m() - 1.90).abs() < 0.01, "got {}", spec.height_m());
    }

    #[test]
    fn models_json_parses_without_the_old_state_table() {
        let src = shipped_text();
        assert!(RunnerSpec::parse(src).is_ok());
        assert!(!src.contains("\"bones\""), "the swing amplitude table is gone");
        assert!(!src.contains("\"amp\""), "no swing amplitude is left");
        assert!(!src.contains("\"states\""), "the clip per state is data/player/anim.json now");
        let spec = shipped();
        assert!(embedded::asset(&spec.body.model).is_some(), "the runner model {} is embedded", spec.body.model);
        assert!(embedded::asset(&spec.body.atlas).is_some(), "the runner atlas {} is embedded", spec.body.atlas);
    }

    #[test]
    fn changing_the_data_changes_the_figure() {
        let text = shipped_text();
        let taller = RunnerSpec::parse(&text.replacen("113.92", "120.0", 1)).unwrap();
        assert!(taller.height_m() > shipped().height_m());
        let lazier = RunnerSpec::parse(&text.replacen("\"walk_min_mps\": 0.3", "\"walk_min_mps\": 3.5", 1)).unwrap();
        assert_eq!(bot_state(&lazier.bots, 3.0, false, false).pose(), Pose::Idle, "a higher walk threshold keeps a slow bot idle");
        assert_eq!(bot_state(&shipped().bots, 3.0, false, false).pose(), Pose::Walk);
    }

    #[test]
    fn bad_data_is_an_error_not_a_silent_default() {
        assert!(RunnerSpec::parse("not json").is_err());
        assert!(RunnerSpec::parse("{}").is_err());
        let no_body = shipped_text().replacen("\"slim_body\": \"blocky_slim\"", "\"slim_bdy\": \"blocky_slim\"", 1);
        assert!(RunnerSpec::parse(&no_body).is_err(), "a missing body is reported");
        let no_bots = shipped_text().replacen("\"bots\": {", "\"bots_gone\": {", 1);
        assert!(RunnerSpec::parse(&no_bots).is_err(), "a missing bots section is reported");
    }

    #[test]
    fn shipped_bots_wear_the_default_skin() {
        assert_eq!(shipped().bots.skins, vec![SKIN_DEFAULT.to_string()], "Standard bots draw the atlas, as before");
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
    fn changing_the_bot_skin_list_changes_the_bots() {
        let swapped = shipped_text().replacen("\"skins\": [\"DEFAULT\"]", "\"skins\": [\"SKIN_A\", \"SKIN_B\"]", 1);
        let spec = RunnerSpec::parse(&swapped).unwrap();
        assert_eq!(spec.bots.skins, vec!["SKIN_A".to_string(), "SKIN_B".to_string()]);
    }

    #[test]
    fn the_crouch_fade_is_the_transparency_godot_sets() {
        assert_eq!(body_alpha(0.0), 1.0, "a standing body is opaque");
        assert!((body_alpha(driver::config().crouch_alpha) - 0.45).abs() < 1e-6, "a crouch fades to 0.55 transparency");
        let mut view = RunnerView::load(false).expect("rig");
        let crouch = RunnerState { crouching: true, ..RunnerState::default() };
        for _ in 0..60 {
            view.update(&crouch, 0.1);
        }
        assert!(body_alpha(view.fade) < 1.0, "the drawn alpha follows the view's fade");
    }

    #[test]
    fn a_faded_body_is_drawn_in_the_translucent_lit_mode() {
        use crate::render::shader::{GHOST_MODE, LIT_MODE};
        assert_eq!(body_mode(1.0), LIT_MODE, "a standing body stays opaque");
        assert_eq!(body_mode(body_alpha(driver::config().crouch_alpha)), GHOST_MODE, "a crouching body blends");
        assert!(GHOST_MODE > 3.5 && LIT_MODE < 1.5, "the shader reads these as ghost and lit");
    }

    #[test]
    fn a_runner_vanishes_once_it_has_slid_into_its_locker() {
        let mut r = remote(None, false, false, false, 0.0);
        assert!(!hidden_for_good(&r), "in the open it is drawn");
        r.hide = Some(HideSpot { cell: (4, 5), facing: (0, 1) });
        r.hide_since = Some(std::time::Duration::ZERO);
        assert!(!hidden_for_good(&r), "it stays visible while it slides in");
        r.hide_since = Some(std::time::Duration::from_secs_f32(crate::render::hiding_cam::glide_seconds(true) + 0.1));
        assert!(hidden_for_good(&r), "then it is inside the locker");
    }

    // ---- the rig: bind pose, bends, clips, skin UVs

    /// (group name, origin in model units) of every outliner group of a bbmodel, parents first.
    fn bb_groups(path: &str) -> (Vec<(String, Vec3)>, Vec<[f32; 4]>) {
        let src = std::str::from_utf8(embedded::asset(path).expect("embedded model")).unwrap();
        let j: serde_json::Value = serde_json::from_str(src).unwrap();
        fn walk(n: &serde_json::Value, out: &mut Vec<(String, Vec3)>) {
            if let Some(name) = n["name"].as_str() {
                if n.get("origin").is_some() && n.get("children").is_some() {
                    let o = &n["origin"];
                    let c = |i: usize| o[i].as_f64().unwrap_or(0.0) as f32;
                    out.push((name.to_string(), vec3(c(0), c(1), c(2))));
                }
            }
            for c in n["children"].as_array().into_iter().flatten() {
                walk(c, out);
            }
        }
        let mut groups = vec![];
        for n in j["outliner"].as_array().unwrap() {
            walk(n, &mut groups);
        }
        let mut uvs = vec![];
        for e in j["elements"].as_array().unwrap() {
            for f in e["faces"].as_object().into_iter().flat_map(|f| f.values()) {
                if let Some(a) = f["uv"].as_array() {
                    let u = |i: usize| a[i].as_f64().unwrap_or(f64::NAN) as f32;
                    uvs.push([u(0), u(1), u(2), u(3)]);
                }
            }
        }
        (groups, uvs)
    }

    fn clip_pose(rig: &Rig, clip: &str, t: f32) -> Vec<Local> {
        let mut p = AnimPlayer::default();
        p.play(rig, clip, 0.0);
        p.seek(t);
        p.advance(rig, 0.0);
        p.pose(rig)
    }

    fn bone_angle(rig: &Rig, pose: &[Local], name: &str) -> f32 {
        let b = rig.bone(name).unwrap_or_else(|| panic!("no bone {name}"));
        (pose[b].r.angle_between(rig.bones[b].rest_r)).to_degrees()
    }

    fn peak(rig: &Rig, clip: &str, bones: &[&str]) -> f32 {
        let len = rig.length(clip);
        (0..=20)
            .map(|i| {
                let pose = clip_pose(rig, clip, len * i as f32 / 20.0);
                bones.iter().map(|b| bone_angle(rig, &pose, b)).fold(0.0f32, f32::max)
            })
            .fold(0.0f32, f32::max)
    }

    /// How far (model units) the rig's forearm origin sits from the model's forearm group at rest; the hand's is twice it.
    const ARM_CHAIN_UNITS: f32 = 1.25;

    #[test]
    fn rest_pose_is_the_bind_pose() {
        for (slim, path) in [(false, "player/player_mannequin.bbmodel"), (true, "player/player_mannequin_slim.bbmodel")] {
            let rig = player_rig(slim).expect("rig");
            let rest: Vec<Local> = AnimPlayer::default().pose(&rig);
            let mats = model_space_stretched(&rig, &rest, &[]);
            let (groups, _) = bb_groups(path);
            assert_eq!(groups.len(), 16, "{path}: 16 outliner groups (pelvis, belly, chest, head, two arms and two legs of three)");
            let mut off = vec![];
            for (name, origin) in groups {
                let b = rig.bone(&name).unwrap_or_else(|| panic!("{path}: the rig has no bone {name}"));
                off.push((name, (mats[b].transform_point3(Vec3::ZERO) - origin).length()));
            }
            // The torso, head and legs rest exactly on their group origins. The arm chain is authored in the glb with its
            // forearm and hand origins ARM_CHAIN_UNITS further along the limb per segment than the model's groups (the
            // same data Godot imports); the joint plugs close that gap, so it is the one allowed deviation.
            for (name, d) in &off {
                let limit = if name.ends_with("Forearm") {
                    ARM_CHAIN_UNITS
                } else if name.ends_with("Hand") {
                    2.0 * ARM_CHAIN_UNITS
                } else {
                    1e-3
                };
                assert!(*d <= limit + 1e-3, "{path}: {name} rests {d} units off its group origin (allowed {limit}): {off:?}");
            }
        }
    }

    #[test]
    fn clips_bend_the_joints() {
        let rig = player_rig(false).expect("rig");
        let knees = ["L-Calf", "R-Calf"];
        let elbows = ["L-Forearm", "R-Forearm"];
        assert!(peak(&rig, "walk", &knees) >= 40.0, "a walking knee bends");
        assert!(peak(&rig, "walk", &elbows) >= 15.0, "a walking elbow bends");
        assert!(peak(&rig, "sprint", &knees) >= 60.0, "a sprinting knee bends");
        assert!(peak(&rig, "sprint", &elbows) >= 70.0, "a sprinting elbow bends");
        assert!(peak(&rig, "crouch", &knees) >= 100.0, "a crouching knee folds");
        assert!(peak(&rig, "climb", &["L-Arm", "R-Arm"]) >= 150.0, "a climbing arm reaches up");
        let all: Vec<String> = rig.bones.iter().map(|b| b.name.clone()).collect();
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        assert!(peak(&rig, "idle", &all) <= 10.0, "the idle clip barely moves any joint");
    }

    #[test]
    fn every_runner_pose_has_a_clip_in_both_rigs() {
        for slim in [false, true] {
            let rig = player_rig(slim).expect("rig");
            for pose in Pose::ALL {
                let clip = driver::config().clips.get(pose.key()).unwrap_or_else(|| panic!("no clip for {pose:?}"));
                assert!(rig.has_clip(clip), "slim={slim}: {pose:?} plays {clip}, which the rig lacks");
                assert_eq!(Pose::from_wire(pose.wire()), Some(pose), "{pose:?} has its own wire name");
            }
        }
    }

    #[test]
    fn skin_uvs_stay_valid_and_both_bodies_share_their_bones() {
        let (regular, regular_uv) = bb_groups("player/player_mannequin.bbmodel");
        let (slim, slim_uv) = bb_groups("player/player_mannequin_slim.bbmodel");
        let names = |g: &[(String, Vec3)]| g.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        assert_eq!(names(&regular), names(&slim), "bone lookup by group name needs the same groups");
        for (what, uvs) in [("regular", regular_uv), ("slim", slim_uv)] {
            assert!(!uvs.is_empty());
            for uv in uvs {
                assert!(uv.iter().all(|c| (0.0..=64.0).contains(c)), "{what}: a face samples outside the 64x64 atlas: {uv:?}");
            }
        }
    }

    #[test]
    fn a_held_item_sits_in_the_real_rigs_hand_and_points_forward() {
        let d = held_wire::data().expect("the pose table loads");
        let unit = runner_spec().map(|s| s.metres_per_unit).expect("runner spec");
        for slim in [false, true] {
            let rig = player_rig(slim).expect("rig");
            let rest: Vec<Local> = AnimPlayer::default().pose(&rig);
            let mats = model_space_stretched(&rig, &rest, &[]);
            let b = rig.bone(&d.bone).expect("the hand bone");
            let root = body_root(&Look { pos: Vec3::ZERO, yaw: 0.0, pitch: 0.0, alpha: 1.0 }, unit);
            // the hand matrix is the bone's world matrix in model space (not a skinning matrix): its origin is the pivot
            let pivot = root.transform_point3(mats[b].transform_point3(Vec3::ZERO));
            let grip_point = root.transform_point3(mats[b].transform_point3(d.anchor.offset_units));
            assert!(pivot.y > 0.6 && pivot.y < 1.3, "slim={slim}: the hand pivot is at wrist height, got {pivot:?}");
            assert!(pivot.x.abs() > 0.15, "slim={slim}: the right hand is out to the side, got {pivot:?}");
            for item in [Item::Flashlight, Item::Taser, Item::Crowbar, Item::Axe, Item::Battery, Item::Stim, Item::MusicBox] {
                let xf = held_wire::item_transform(d, item, root, mats[b], unit);
                let origin = xf.transform_point3(Vec3::ZERO);
                let reach = d.pose(item).grip * d.pose(item).scale * d.anchor.rig_scale + 0.15;
                assert!((origin - grip_point).length() < reach, "slim={slim} {item:?}: the item origin {origin:?} is far from the hand {grip_point:?}");
                let tip = xf.transform_point3(Vec3::Y * 0.5);
                assert!(tip.z < origin.z - 0.05, "slim={slim} {item:?}: the item points forward (-Z), origin {origin:?} tip {tip:?}");
            }
        }
    }

    #[test]
    fn the_swim_lean_pivots_at_the_hips_and_a_standing_body_does_not_lean() {
        let look = |pitch| Look { pos: Vec3::ZERO, yaw: 0.0, pitch, alpha: 1.0 };
        let hips = Vec3::Y * driver::config().swim_pivot;
        let upright = body_root(&look(0.0), 1.0);
        let leaning = body_root(&look(0.6), 1.0);
        assert!((upright.transform_point3(hips) - hips).length() < 1e-5);
        assert!((leaning.transform_point3(hips) - hips).length() < 1e-5, "the hips stay put");
        assert!((leaning.transform_point3(Vec3::Y * 1.9) - Vec3::Y * 1.9).length() > 0.3, "the head swings");
    }

    #[test]
    fn a_new_sequence_restarts_the_emote_clock_and_no_emote_clears_it() {
        let mut slot = None;
        advance_emote(&mut slot, Some(("wave", 1)), 0.1);
        advance_emote(&mut slot, Some(("wave", 1)), 0.5);
        assert_eq!(slot.as_ref().map(|s| (s.1, s.2)), Some((1, 0.5)));
        advance_emote(&mut slot, Some(("wave", 2)), 0.5);
        assert_eq!(slot.as_ref().map(|s| (s.1, s.2)), Some((2, 0.0)), "the same emote played again restarts");
        advance_emote(&mut slot, None, 0.5);
        assert!(slot.is_none());
    }

    #[test]
    fn an_emote_poses_the_body_and_an_unknown_id_is_ignored() {
        let Some(rig) = player_rig(false) else {
            return;
        };
        let rest = still_pose(&rig, Pose::Idle, 0.0);
        let mut posed = rest.clone();
        assert!(pose_emote(&rig, &mut posed, "wave", 0.9));
        let arm = rig.bone("R-Arm").unwrap();
        assert!(posed[arm].r.dot(rest[arm].r).abs() < 0.99, "the waving arm moved off its idle pose");
        let mut other = rest.clone();
        assert!(!pose_emote(&rig, &mut other, "no-such-emote", 0.9));
        assert!(other.iter().zip(&rest).all(|(a, b)| a.r == b.r && a.t == b.t));
    }

    #[test]
    fn a_sample_without_an_emote_names_none() {
        assert_eq!(remote_emote(&remote(None, false, false, false, 0.0)), None);
    }

    #[test]
    fn a_remote_sample_with_an_emote_names_it_and_poses_the_arm() {
        let mut r = remote(None, false, false, false, 0.0);
        r.emote = Some(("wave".into(), 1));
        assert_eq!(remote_emote(&r), Some(("wave", 1)));
        let Some(rig) = player_rig(false) else {
            return;
        };
        let rest = still_pose(&rig, Pose::Idle, 0.0);
        let mut posed = rest.clone();
        let (id, _) = remote_emote(&r).unwrap();
        assert!(pose_emote(&rig, &mut posed, id, 0.9));
        let arm = rig.bone("R-Arm").unwrap();
        assert!(posed[arm].r.dot(rest[arm].r).abs() < 0.99, "R-Arm moved");
    }

    #[test]
    fn a_bots_look_turns_the_head_from_the_body_and_is_capped() {
        assert_eq!(looked_head(0.0, 0.0), 0.0);
        assert!((looked_head(0.1, 0.5) - 0.6).abs() < 1e-6, "the look adds to the driver's own easing");
        assert!(looked_head(0.1, -0.5) < 0.0, "a look to the right turns the head right");
        assert_eq!(looked_head(1.0, 5.0), LOOK_YAW_CAP);
        assert_eq!(looked_head(-1.0, -5.0), -LOOK_YAW_CAP);
    }

    #[test]
    fn set_bot_look_keeps_a_finite_look_and_ignores_the_rest() {
        set_bot_look("R-00001", 0.4, -0.1);
        assert_eq!(BOT_LOOK.with(|m| m.borrow().get("R-00001").copied()), Some((0.4, -0.1)));
        set_bot_look("R-00001", f32::NAN, 0.0);
        assert_eq!(BOT_LOOK.with(|m| m.borrow().get("R-00001").copied()), Some((0.4, -0.1)), "a bad sample leaves the look");
    }

    #[test]
    fn a_remote_runners_lockpick_set_and_revive_kit_are_carried_by_their_first_person_grip() {
        let d = crate::net::held_wire::data().expect("remote_held.json parses");
        for (item, name) in [(Item::Lockpick, "lockpick"), (Item::ReviveKit, "revive_kit")] {
            let p = d.pose(item);
            assert!(!p.upright && p.tilt_deg == 0.0, "{name} is carried, not stood on its handle like a crowbar");
            let fp = crate::render::fp_hands::item_pose(name).expect("first-person row");
            assert_eq!(p.grip, fp.grip, "{name} hangs by the same grip as in first person");
            assert!(p.scale > 0.0);
        }
    }

    #[test]
    fn set_bot_look_updates_a_known_bot_in_place() {
        set_bot_look("R-LOOK-2", 0.1, 0.2);
        let n = BOT_LOOK.with(|m| m.borrow().len());
        set_bot_look("R-LOOK-2", -0.3, 0.05);
        assert_eq!(BOT_LOOK.with(|m| m.borrow().get("R-LOOK-2").copied()), Some((-0.3, 0.05)), "the newest look wins");
        assert_eq!(BOT_LOOK.with(|m| m.borrow().len()), n, "a second look adds no entry");
        set_bot_look("R-LOOK-3", 0.0, 0.0);
        assert_eq!(BOT_LOOK.with(|m| m.borrow().len()), n + 1, "a new bot is added");
    }
}
