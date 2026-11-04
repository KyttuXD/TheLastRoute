//! The figure a runner or an NPC teammate wears in the 3D view. The body is the blocky mannequin (a regular and a slim
//! mesh, `Assets::npc_body` for the regular), textured with the mannequin atlas by default or with the runner's skin
//! PNG (see `Assets::skin_texture`). A skin that is DEFAULT, or not known yet, draws the atlas.
//!
//! The pose comes from the player rig's clips (render/anim/clip.rs, rig id in models.json): each figure owns an
//! `AnimPlayer` keyed by peer id (runners) or guest name (bots), and its state (idle, walk, sprint, crouch) picks the
//! clip. Everything tunable is data: `data/player/models.json` (clip per state, speeds, figure height, bot thresholds).

use super::world_sync::{RemoteView, SKIN_DEFAULT};
use crate::content::embedded;
use crate::game::Game;
use crate::render::anim::clip::{self, Rig};
use crate::render::anim::hunter::model_space_stretched;
use crate::render::anim::player::{AnimPlayer, Local};
use crate::render::bbmodel::Model;
use crate::render::scene;
use crate::render::shader;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

/// The data file (under src/bin/last_route/data) that holds the runner body, its clips and its height.
const MODELS_FILE: &str = "player/models.json";

/// Which texture a runner's body draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The mannequin atlas (the default skin, or a skin not known yet).
    Atlas,
    /// The cached skin with this id.
    Skin(String),
}

/// What a figure is doing, which picks its clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunnerState {
    Idle,
    Walk,
    Sprint,
    Crouch,
}

impl RunnerState {
    fn index(self) -> usize {
        self as usize
    }

    /// The state a remote runner's flags describe: crouching wins, then sprinting while moving, then walking.
    pub fn from_flags(moving: bool, sprinting: bool, crouching: bool) -> RunnerState {
        if crouching {
            RunnerState::Crouch
        } else if moving && sprinting {
            RunnerState::Sprint
        } else if moving {
            RunnerState::Walk
        } else {
            RunnerState::Idle
        }
    }
}

/// The per-frame facts about a remote runner the figure needs. `RemoteView` carries these after the integrator's hook
/// (world_sync.rs); until then [`RemoteFigure`] answers with the default (idle, regular arms).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FigureFlags {
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
    /// The slim arm mesh (ArmsSetting resolved for the runner's skin).
    pub slim: bool,
    /// Ground speed in metres per second (0 when unknown).
    pub speed: f32,
    /// Vertical speed in metres per second.
    pub vy: f32,
}

impl FigureFlags {
    /// The constructor the integrator calls when building a `RemoteView` from a `Sample`.
    pub fn new(moving: bool, sprinting: bool, crouching: bool, slim: bool, speed: f32, vy: f32) -> Self {
        FigureFlags { moving, sprinting, crouching, slim, speed, vy }
    }

    pub fn state(&self) -> RunnerState {
        RunnerState::from_flags(self.moving, self.sprinting, self.crouching)
    }
}

/// How the figure code reads a remote runner. HOOK: once `RemoteView` has the fields, return them here.
pub trait RemoteFigure {
    fn figure_flags(&self) -> FigureFlags;
}

impl RemoteFigure for RemoteView {
    fn figure_flags(&self) -> FigureFlags {
        FigureFlags::new(self.moving, self.sprinting, self.crouching, self.slim, self.speed, self.vy)
    }
}

/// The clip and playback speed of one state, from models.json.
#[derive(Clone, Debug, PartialEq)]
pub struct StateSpec {
    pub clip: String,
    /// Playback speed scale of the clip.
    pub speed: f32,
    /// The ground speed the clip's stride was authored for; 0 plays at `speed` whatever the runner's speed.
    pub ref_mps: f32,
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
    /// The rig id (render/anim/clip.rs `rig`).
    pub rig: String,
    /// Cross-fade between clips, seconds.
    pub blend_s: f32,
    /// Indexed by [`RunnerState`].
    pub states: [StateSpec; 4],
    pub scale_min: f32,
    pub scale_max: f32,
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
        let mut states = Vec::with_capacity(4);
        for name in ["idle", "walk", "sprint", "crouch"] {
            let s = &r["states"][name];
            states.push(StateSpec {
                clip: string(s, "clip").map_err(|e| format!("runner.states.{name}: {e}"))?.to_string(),
                speed: num(s, "speed").map_err(|e| format!("runner.states.{name}: {e}"))?,
                ref_mps: s["ref_mps"].as_f64().unwrap_or(0.0) as f32,
            });
        }
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
            rig: string(r, "rig").map_err(|e| format!("runner.{e}"))?.to_string(),
            blend_s: num(r, "blend_s").map_err(|e| format!("runner.{e}"))?,
            states: states.try_into().map_err(|_| "runner.states is incomplete".to_string())?,
            scale_min: num(r, "speed_scale_min").map_err(|e| format!("runner.{e}"))?,
            scale_max: num(r, "speed_scale_max").map_err(|e| format!("runner.{e}"))?,
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

    /// The clip a state plays and its playback speed for a figure moving at `speed` m/s.
    pub fn clip_for(&self, state: RunnerState, speed: f32) -> (&str, f32) {
        let s = &self.states[state.index()];
        let scale = if s.ref_mps > 0.0 && speed > 0.0 {
            (speed / s.ref_mps).clamp(self.scale_min, self.scale_max) * s.speed
        } else {
            s.speed
        };
        (&s.clip, scale)
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

/// The state of a bot that moved at `speed` metres per second.
pub fn state_for_speed(bots: &BotSpec, speed: f32) -> RunnerState {
    if speed >= bots.sprint_min_mps {
        RunnerState::Sprint
    } else if speed >= bots.walk_min_mps {
        RunnerState::Walk
    } else {
        RunnerState::Idle
    }
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

/// Who a running clip belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FigureKey {
    Peer(u32),
    /// An NPC teammate, by its guest name (unique per NPC within a run).
    Bot(String),
}

struct Figure {
    player: AnimPlayer,
    state: Option<RunnerState>,
    /// A bot's last floor position, for its speed.
    last_at: Option<(f32, f32)>,
    seen: u64,
}

impl Figure {
    fn new(epoch: u64) -> Figure {
        Figure { player: AnimPlayer::default(), state: None, last_at: None, seen: epoch }
    }
}

thread_local! {
    /// (the draw pass counter, every figure with a running clip)
    static FIGURES: RefCell<(u64, HashMap<FigureKey, Figure>)> = RefCell::new((0, HashMap::new()));
    static SLIM: RefCell<Option<Option<Rc<Model>>>> = const { RefCell::new(None) };
}

/// Advances a figure's clip player by `dt` at `state` and returns its local pose.
fn step_figure(fig: &mut Figure, spec: &RunnerSpec, rig: &Rig, state: RunnerState, speed: f32, dt: f32) -> Vec<Local> {
    let (clip_name, scale) = spec.clip_for(state, speed);
    // The clip restarts only when the state changes; a speed change just retimes it.
    if fig.state != Some(state) {
        fig.player.play(rig, clip_name, spec.blend_s);
        fig.state = Some(state);
    }
    fig.player.speed_scale = scale;
    fig.player.advance(rig, dt);
    fig.player.pose(rig)
}

/// The still pose of `state` (the first frame of its clip), for a figure with no clip player of its own.
fn still_pose(spec: &RunnerSpec, rig: &Rig, state: RunnerState) -> Vec<Local> {
    let mut p = AnimPlayer::default();
    p.play(rig, spec.clip_for(state, 0.0).0, 0.0);
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

/// Builds and draws one figure. `pose` is the rig and its local bone pose; None leaves the body in its rest pose.
fn draw_body(a: &Assets, pos: Vec3, yaw: f32, pick: &Pick, slim: bool, pose: Option<(&Rig, &[Local])>) {
    let Some(regular) = a.npc_body.as_ref() else {
        return;
    };
    let spec = runner_spec();
    let slim_model = if slim { slim_body(a, spec) } else { None };
    let body: &Model = slim_model.as_deref().unwrap_or(regular);
    let texture = match pick {
        Pick::Skin(id) => a.skin_texture(id).unwrap_or_else(|| body.texture.clone()),
        Pick::Atlas => body.texture.clone(),
    };
    let unit = spec.map(|s| s.metres_per_unit).unwrap_or(body.scale);
    let root = Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw) * Mat4::from_scale(Vec3::splat(unit));
    let mats = pose.map(|(rig, locals)| (rig, model_space_stretched(rig, locals, &[])));
    let bone_of = |k: usize| {
        let (rig, m) = mats.as_ref()?;
        rig.bone(&body.groups[k].name).map(|b| m[b])
    };
    let mut verts = vec![];
    let mut idx = vec![];
    body.build_posed(&mut verts, &mut idx, root, &bone_of, Vec2::ZERO, WHITE);
    draw_mesh(&Mesh { vertices: verts, indices: idx, texture: Some(texture) });
}

/// Draws one still figure: the same body a runner wears, at `pos` (feet) and `yaw`, in the pose of `state`. The caller
/// sets the camera and material, as for [`draw_runners`]. The locker occupant uses this.
pub fn draw_figure(a: &Assets, pos: Vec3, yaw: f32, pick: Pick, slim: bool, state: RunnerState) {
    let spec = runner_spec();
    let rig = spec.and_then(|s| clip::rig(&s.rig));
    let locals = match (spec, rig.as_ref()) {
        (Some(s), Some(r)) => Some(still_pose(s, r, state)),
        _ => None,
    };
    let pose = rig.as_ref().zip(locals.as_ref()).map(|(r, l)| (&**r, l.as_slice()));
    draw_body(a, pos, yaw, &pick, slim, pose);
}

/// Runs the clip player of `key` for this frame and draws the figure.
#[allow(clippy::too_many_arguments)]
fn draw_driven(
    a: &Assets,
    key: FigureKey,
    pos: Vec3,
    yaw: f32,
    pick: &Pick,
    slim: bool,
    state: RunnerState,
    speed: f32,
    dt: f32,
    epoch: u64,
) {
    let spec = runner_spec();
    let rig = spec.and_then(|s| clip::rig(&s.rig));
    let locals = match (spec, rig.as_ref()) {
        (Some(s), Some(r)) => Some(FIGURES.with(|f| {
            let mut f = f.borrow_mut();
            let fig = f.1.entry(key).or_insert_with(|| Figure::new(epoch));
            fig.seen = epoch;
            step_figure(fig, s, r, state, speed, dt)
        })),
        _ => None,
    };
    let pose = rig.as_ref().zip(locals.as_ref()).map(|(r, l)| (&**r, l.as_slice()));
    draw_body(a, pos, yaw, pick, slim, pose);
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

/// Draws the remote runners in the 3D view, lit like the scene. Each one is the mannequin at its position and yaw,
/// wearing its skin's texture, posed by its state. Draws nothing for an empty list, so a single-player frame is unchanged.
pub fn draw_runners(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    let epoch = next_epoch();
    if remotes.is_empty() {
        prune(epoch, false);
        return;
    }
    set_camera(&scene::camera(g, rt));
    let active_y = crate::game::level_stack::floor_y_of(g.fac.level);
    gl_use_material(mat);
    shader::apply(mat, &scene::lighting(g), &g.world.lamps);
    if a.npc_body.is_none() {
        // The mannequin failed to load: a plain grey post keeps the runner visible in the 3D view.
        let mut b = crate::render::builder::Builder::new(Some(a.white.clone()));
        for r in remotes {
            b.cube(vec3(r.x, runner_draw_y(r.h, active_y) + 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, GRAY, 2.0);
        }
        b.draw_and_clear();
        return;
    }
    let dt = get_frame_time().clamp(0.0, 0.1);
    for r in remotes {
        let f = r.figure_flags();
        let tex = pick(&r.skin, &|id| a.skin_texture(id).is_some());
        let pos = vec3(r.x, runner_draw_y(r.h, active_y), r.y);
        draw_driven(a, FigureKey::Peer(r.peer), pos, r.yaw, &tex, f.slim, f.state(), f.speed, dt, epoch);
    }
    prune(epoch, false);
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
/// `stride` and `walking` stay for the callers that fill them; the figure derives its state from its movement.
pub struct BotView<'a> {
    pub name: &'a str,
    pub at: (f32, f32),
    pub facing: f32,
    pub stride: f32,
    pub walking: bool,
    /// Draw offset of the feet from the drawn floor (render::stack_actors::actor_dy); 0 on the drawn floor.
    pub y: f32,
}

/// Draws NPC teammates with the runner mannequin and the same clips, each wearing its skin (see [`bot_skin`]). A bot's
/// state comes from how far it moved since the last frame ([`state_for_speed`]). The caller sets the camera and
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
            let fig = f.1.entry(key.clone()).or_insert_with(|| Figure::new(epoch));
            let speed = bot_speed(fig.last_at, b.at, dt);
            fig.last_at = Some(b.at);
            speed
        });
        let state = spec.map(|s| state_for_speed(&s.bots, speed)).unwrap_or(RunnerState::Idle);
        let tex = pick(&bot_skin(b.name, skins), &|id| a.skin_texture(id).is_some());
        draw_driven(a, key, vec3(b.at.0, b.y, b.at.1), b.facing, &tex, false, state, speed, dt, epoch);
    }
    prune(epoch, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The models.json text the game embeds.
    fn shipped_text() -> &'static str {
        embedded::data_under(MODELS_FILE).next().expect("player/models.json is embedded").1
    }

    fn shipped() -> RunnerSpec {
        RunnerSpec::parse(shipped_text()).expect("models.json parses")
    }

    #[test]
    fn remote_on_a_flight_is_drawn_at_its_height() {
        assert_eq!(runner_draw_y(-2.5, 0.0), -2.5);
        assert_eq!(runner_draw_y(-2.5, -5.0), 2.5);
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
    fn flags_map_to_states() {
        assert_eq!(RunnerState::from_flags(true, false, false), RunnerState::Walk);
        assert_eq!(RunnerState::from_flags(true, true, false), RunnerState::Sprint);
        assert_eq!(RunnerState::from_flags(false, false, false), RunnerState::Idle);
        assert_eq!(RunnerState::from_flags(false, true, false), RunnerState::Idle, "sprint without moving is idle");
        assert_eq!(RunnerState::from_flags(true, true, true), RunnerState::Crouch, "crouching wins");
        assert_eq!(FigureFlags::new(true, true, false, false, 5.0, 0.0).state(), RunnerState::Sprint);
        assert_eq!(FigureFlags::default().state(), RunnerState::Idle);
    }

    #[test]
    fn states_pick_their_clips() {
        let spec = shipped();
        assert_eq!(spec.clip_for(RunnerState::Walk, 0.0).0, "walk");
        assert_eq!(spec.clip_for(RunnerState::Sprint, 0.0).0, "sprint");
        assert_eq!(spec.clip_for(RunnerState::Idle, 0.0).0, "idle");
        let (clip, speed) = spec.clip_for(RunnerState::Crouch, 0.0);
        assert_eq!(clip, "idle", "a crouch plays the idle clip");
        assert_eq!(speed, 0.55, "slowed to 0.55");
    }

    #[test]
    fn a_walk_clip_follows_the_runners_speed_within_the_clamp() {
        let spec = shipped();
        let at_ref = spec.clip_for(RunnerState::Walk, spec.states[1].ref_mps).1;
        assert!((at_ref - spec.states[1].speed).abs() < 1e-5, "the authored speed plays at its authored rate");
        let fast = spec.clip_for(RunnerState::Walk, 100.0).1;
        let slow = spec.clip_for(RunnerState::Walk, 0.01).1;
        assert!((fast - spec.scale_max * spec.states[1].speed).abs() < 1e-5, "the top of the range is clamped");
        assert!((slow - spec.scale_min * spec.states[1].speed).abs() < 1e-5, "the bottom of the range is clamped");
    }

    #[test]
    fn a_stationary_bot_idles_and_a_moving_bot_walks() {
        let spec = shipped();
        let dt = 1.0 / 60.0;
        let still = bot_speed(Some((3.0, 4.0)), (3.0, 4.0), dt);
        assert_eq!(state_for_speed(&spec.bots, still), RunnerState::Idle);
        let first = bot_speed(None, (3.0, 4.0), dt);
        assert_eq!(state_for_speed(&spec.bots, first), RunnerState::Idle, "no earlier frame means no speed");
        let moved = bot_speed(Some((3.0, 4.0)), (3.05, 4.0), dt);
        assert!((moved - 3.0).abs() < 1e-3, "0.05 m in a 60 Hz frame is 3 m/s, got {moved}");
        assert_eq!(state_for_speed(&spec.bots, moved), RunnerState::Walk);
        let fast = bot_speed(Some((0.0, 0.0)), (0.1, 0.0), dt);
        assert_eq!(state_for_speed(&spec.bots, fast), RunnerState::Sprint, "6 m/s sprints");
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
    fn models_json_parses_without_the_old_swing_table() {
        let src = shipped_text();
        assert!(RunnerSpec::parse(src).is_ok());
        assert!(!src.contains("\"bones\""), "the swing amplitude table is gone");
        assert!(!src.contains("\"amp\""), "no swing amplitude is left");
        let spec = shipped();
        assert!(embedded::asset(&spec.body.model).is_some(), "the runner model {} is embedded", spec.body.model);
        assert!(embedded::asset(&spec.body.atlas).is_some(), "the runner atlas {} is embedded", spec.body.atlas);
    }

    #[test]
    fn changing_the_data_changes_the_figure() {
        let text = shipped_text();
        let slower = RunnerSpec::parse(&text.replacen("\"speed\": 0.55", "\"speed\": 0.25", 1)).unwrap();
        assert_eq!(slower.clip_for(RunnerState::Crouch, 0.0).1, 0.25, "a new crouch speed retimes the clip");
        let taller = RunnerSpec::parse(&text.replacen("113.92", "120.0", 1)).unwrap();
        assert!(taller.height_m() > shipped().height_m());
        let lazier = RunnerSpec::parse(&text.replacen("\"walk_min_mps\": 0.3", "\"walk_min_mps\": 3.5", 1)).unwrap();
        assert_eq!(state_for_speed(&lazier.bots, 3.0), RunnerState::Idle, "a higher walk threshold keeps a slow bot idle");
        assert_eq!(state_for_speed(&shipped().bots, 3.0), RunnerState::Walk);
    }

    #[test]
    fn bad_data_is_an_error_not_a_silent_default() {
        assert!(RunnerSpec::parse("not json").is_err());
        assert!(RunnerSpec::parse("{}").is_err());
        let no_clip = shipped_text().replacen("\"clip\": \"walk\"", "\"clp\": \"walk\"", 1);
        assert!(RunnerSpec::parse(&no_clip).is_err(), "a state without a clip is reported");
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
}
