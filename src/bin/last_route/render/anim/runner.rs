//! The runner's body clips (audit P1, P2, P10): the player mannequin rig (assets/player/anim/player_mannequin.json, and
//! the slim variant) driven by an `AnimPlayer` the way the original drives every runner, yours and each teammate's copy.
//!
//! * clip choice: player_controller_3d.gd 1923-1935 and remote_players.gd 736-763 -- idle, walk, sprint (the chase clip
//!   while chased), jump / fall in the air, climb on a ladder; crouching plays IDLE (the imported crouch pose folds the
//!   knees backward) while the body fades to 0.55 alpha (model_slot.gd 444, `_update_crouch_fade`).
//! * pace: player_stride.gd + hunter_stride.gd `choose` -- walk and sprint play at real ground speed / the clip's
//!   measured stride speed, so the planted foot stays on the floor; below `still_mps` the pose holds (scale 0).
//!
//! Every number lives in data/player/anim.json (`Config`), parsed with errors that name the key.

use super::clip::Rig;
use super::player::{AnimPlayer, Local};
use crate::content::embedded;
use macroquad::prelude::Quat;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

const CONFIG_TEXT: &str = include_str!("../../data/player/anim.json");

// ------------------------------------------------------------------------------------------------ data

/// data/player/anim.json
#[derive(Debug, Clone)]
pub struct Config {
    pub rig: String,
    pub rig_slim: String,
    pub expected_clips: Vec<String>,
    /// state key ("idle", "walk", "sprint", "chased", "crouch", "jump", "fall", "climb") -> clip name
    pub clips: HashMap<String, String>,
    pub crouch_alpha: f32,
    pub fade_rate: f32,
    pub blend_travel: f32,
    pub blend_idle: f32,
    pub blend_air: f32,
    pub blend_climb: f32,
    pub blend_default: f32,
    pub still_mps: f32,
    pub speed_smoothing: f32,
    pub scale_smoothing: f32,
    pub stride_table: HashMap<String, f32>,
    pub fallback_ref: f32,
    pub model_scale: f32,
    pub min_ratio: f32,
    pub max_ratio: f32,
    pub hard_min: f32,
    pub hard_max: f32,
    pub paced_clips: Vec<String>,
    pub swap_clips: Vec<String>,
    /// state key -> the pose name a net sample carries
    pub wire: HashMap<String, String>,
    /// forward lean of a swimming body, radians, and how fast it eases (per second)
    pub swim_pitch: f32,
    pub swim_pitch_rate: f32,
    /// height above the feet the swim lean pivots about, metres
    pub swim_pivot: f32,
    /// the head's largest tilt up or down, and its largest turn left or right (radians)
    pub head_pitch_limit: f32,
    pub head_yaw_limit: f32,
    /// how far the head trails a turn of the body, in seconds of turn rate
    pub head_yaw_lag: f32,
    /// how fast the head eases toward its aim, per second
    pub head_smoothing: f32,
    /// the head's largest turn from the body toward the aim yaw, either way (radians)
    pub head_aim_yaw_limit: f32,
    /// how far the aim may lead the body before the body turns: standing, and walking or sprinting (radians)
    pub body_still_free: f32,
    pub body_moving_free: f32,
    /// seconds the aim must stay past the free range before the body turns, and how fast it turns (per second)
    pub body_follow_delay: f32,
    pub body_follow_smoothing: f32,
    /// the jump height at its peak, metres: a remote jump's wire height over this maps onto the jump clip
    pub jump_peak: f32,
}

fn at<'a>(j: &'a serde_json::Value, path: &str) -> Result<&'a serde_json::Value, String> {
    let mut v = j;
    for key in path.split('.') {
        v = v
            .get(key)
            .ok_or_else(|| format!("anim.json: missing key \"{path}\""))?;
    }
    Ok(v)
}

fn num(j: &serde_json::Value, path: &str) -> Result<f32, String> {
    at(j, path)?
        .as_f64()
        .map(|x| x as f32)
        .ok_or_else(|| format!("anim.json: key \"{path}\" must be a number"))
}

fn text(j: &serde_json::Value, path: &str) -> Result<String, String> {
    at(j, path)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("anim.json: key \"{path}\" must be a string"))
}

fn strings(j: &serde_json::Value, path: &str) -> Result<Vec<String>, String> {
    at(j, path)?
        .as_array()
        .ok_or_else(|| format!("anim.json: key \"{path}\" must be a list of strings"))?
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("anim.json: key \"{path}\" must be a list of strings"))
        })
        .collect()
}

fn map_of<T>(
    j: &serde_json::Value,
    path: &str,
    f: impl Fn(&serde_json::Value) -> Option<T>,
    what: &str,
) -> Result<HashMap<String, T>, String> {
    let obj = at(j, path)?
        .as_object()
        .ok_or_else(|| format!("anim.json: key \"{path}\" must be an object"))?;
    let mut out = HashMap::new();
    for (k, v) in obj {
        if k.starts_with('_') {
            continue;
        }
        out.insert(
            k.clone(),
            f(v).ok_or_else(|| format!("anim.json: key \"{path}.{k}\" must be {what}"))?,
        );
    }
    Ok(out)
}

impl Config {
    pub fn parse(src: &str) -> Result<Config, String> {
        let j: serde_json::Value =
            serde_json::from_str(src).map_err(|e| format!("anim.json: not valid JSON ({e})"))?;
        let cfg = Config {
            rig: text(&j, "rig")?,
            rig_slim: text(&j, "rig_slim")?,
            expected_clips: strings(&j, "expected_clips")?,
            clips: map_of(&j, "clips", |v| v.as_str().map(str::to_string), "a string")?,
            crouch_alpha: num(&j, "crouch_alpha")?,
            fade_rate: num(&j, "fade_rate")?,
            blend_travel: num(&j, "blend.travel")?,
            blend_idle: num(&j, "blend.idle")?,
            blend_air: num(&j, "blend.air")?,
            blend_climb: num(&j, "blend.climb")?,
            blend_default: num(&j, "blend.default")?,
            still_mps: num(&j, "speed.still_mps")?,
            speed_smoothing: num(&j, "speed.smoothing")?,
            scale_smoothing: num(&j, "speed.scale_smoothing")?,
            stride_table: map_of(
                &j,
                "stride.table",
                |v| v.as_f64().map(|x| x as f32),
                "a number",
            )?,
            fallback_ref: num(&j, "stride.fallback_ref")?,
            model_scale: num(&j, "stride.model_scale")?,
            min_ratio: num(&j, "stride.min_ratio")?,
            max_ratio: num(&j, "stride.max_ratio")?,
            hard_min: num(&j, "stride.hard_min")?,
            hard_max: num(&j, "stride.hard_max")?,
            paced_clips: strings(&j, "stride.paced_clips")?,
            swap_clips: strings(&j, "stride.swap_clips")?,
            wire: map_of(&j, "wire", |v| v.as_str().map(str::to_string), "a string")?,
            swim_pitch: num(&j, "swim.pitch_deg")?.to_radians(),
            swim_pitch_rate: num(&j, "swim.pitch_rate")?,
            swim_pivot: num(&j, "swim.pivot_m")?,
            head_pitch_limit: num(&j, "head.pitch_limit_deg")?.to_radians(),
            head_yaw_limit: num(&j, "head.yaw_limit_deg")?.to_radians(),
            head_yaw_lag: num(&j, "head.yaw_lag_s")?,
            head_smoothing: num(&j, "head.smoothing")?,
            head_aim_yaw_limit: num(&j, "head.aim_yaw_limit_deg")?.to_radians(),
            body_still_free: num(&j, "body.still_free_deg")?.to_radians(),
            body_moving_free: num(&j, "body.moving_free_deg")?.to_radians(),
            body_follow_delay: num(&j, "body.follow_delay_s")?,
            body_follow_smoothing: num(&j, "body.follow_smoothing")?,
            jump_peak: num(&j, "jump.peak_m")?,
        };
        if cfg.jump_peak <= 0.0 {
            return Err("anim.json: key \"jump.peak_m\" must be above 0".to_string());
        }
        for pose in Pose::ALL {
            let key = pose.key();
            if !cfg.clips.contains_key(key) {
                return Err(format!("anim.json: missing key \"clips.{key}\""));
            }
            if !cfg.wire.contains_key(key) {
                return Err(format!("anim.json: missing key \"wire.{key}\""));
            }
        }
        Ok(cfg)
    }

    fn clip(&self, key: &str) -> String {
        self.clips.get(key).cloned().unwrap_or_else(|| key.to_string())
    }
}

/// The embedded anim.json; a malformed one is a build-content bug the tests catch, so it stops loudly with the key.
pub fn config() -> &'static Config {
    static CFG: OnceLock<Config> = OnceLock::new();
    CFG.get_or_init(|| Config::parse(CONFIG_TEXT).unwrap_or_else(|e| panic!("{e}")))
}

// ------------------------------------------------------------------------------------------------ clip choice

/// The poses a runner shows. One per state key of anim.json `clips`; also the pose name a net sample carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pose {
    Idle,
    Walk,
    Sprint,
    /// The sprint while a hunter chases the runner (the chase clip).
    Chased,
    Crouch,
    Jump,
    Fall,
    Climb,
    /// Deep water (no clip of its own in the Godot rig: the walk clip, leaning forward).
    Swim,
    /// Taser or pounce stun.
    Stun,
    /// Caught and down (the lay clip).
    Down,
}

impl Pose {
    pub const ALL: [Pose; 11] = [
        Pose::Idle,
        Pose::Walk,
        Pose::Sprint,
        Pose::Chased,
        Pose::Crouch,
        Pose::Jump,
        Pose::Fall,
        Pose::Climb,
        Pose::Swim,
        Pose::Stun,
        Pose::Down,
    ];

    /// The state key in anim.json (`clips` and `wire` are keyed by it).
    pub fn key(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Walk => "walk",
            Pose::Sprint => "sprint",
            Pose::Chased => "chased",
            Pose::Crouch => "crouch",
            Pose::Jump => "jump",
            Pose::Fall => "fall",
            Pose::Climb => "climb",
            Pose::Swim => "swim",
            Pose::Stun => "stun",
            Pose::Down => "down",
        }
    }

    /// The name this pose travels under in a net sample (anim.json `wire`).
    pub fn wire(self) -> &'static str {
        config().wire.get(self.key()).map(String::as_str).unwrap_or(self.key())
    }

    /// The pose a sample's name stands for; None for a name this build does not know (a newer client).
    pub fn from_wire(name: &str) -> Option<Pose> {
        Pose::ALL.into_iter().find(|p| p.wire() == name)
    }
}

/// What a runner is doing, as the clip choice reads it (local controller state, or a remote's replicated flags).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RunnerState {
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
    pub airborne_up: bool,
    pub airborne_down: bool,
    pub climbing: bool,
    /// a hunter is chasing this runner: the sprint becomes the chase clip
    pub chased: bool,
    /// in water deep enough to swim (wading is the walk clip at the slowed pace)
    pub swimming: bool,
    /// locked by a taser or a pounce
    pub stunned: bool,
    /// caught: the runner lies down
    pub down: bool,
    /// real ground speed, metres per second
    pub speed: f32,
    /// moving opposite to the way the runner faces: a paced clip plays time-reversed (see `ClipChoice::backward`)
    pub backward: bool,
    /// a remote jump's place in its jump or fall clip, 0 (takeoff) .. 1 (landing), from the wire (see [`air_phase`]);
    /// None plays the air clip in time
    pub air_phase: Option<f32>,
}

/// Where a remote runner is in its jump clip: `jump_m` is its height above the floor, `vy` its vertical speed. The rise
/// (vy >= 0) plays the first half of the clip, going up to the peak, and the fall the second half, back down to 0 at landing.
pub fn air_phase(jump_m: f32, vy: f32) -> f32 {
    let f = (jump_m / config().jump_peak).clamp(0.0, 1.0);
    if vy >= 0.0 {
        0.5 * f
    } else {
        1.0 - 0.5 * f
    }
}

impl RunnerState {
    /// The pose these flags show. Down beats stunned beats a ladder beats the air beats swimming beats a crouch.
    pub fn pose(&self) -> Pose {
        if self.down {
            Pose::Down
        } else if self.stunned {
            Pose::Stun
        } else if self.climbing {
            Pose::Climb
        } else if self.airborne_down {
            Pose::Fall
        } else if self.airborne_up {
            Pose::Jump
        } else if self.swimming && self.moving {
            Pose::Swim
        } else if self.crouching {
            Pose::Crouch
        } else if self.moving && self.sprinting {
            if self.chased {
                Pose::Chased
            } else {
                Pose::Sprint
            }
        } else if self.moving {
            Pose::Walk
        } else {
            Pose::Idle
        }
    }

    /// The state a net sample describes. A sample that names its pose gets exactly that pose (`vy` picks jump or fall
    /// for an airborne one); an older client's sample has no name, so its three flags decide. `speed` is the ground speed.
    pub fn from_sample(pose: Option<Pose>, moving: bool, sprinting: bool, crouching: bool, vy: f32, speed: f32) -> RunnerState {
        let base = RunnerState { moving, sprinting, crouching, speed, ..RunnerState::default() };
        let Some(pose) = pose else {
            return base;
        };
        let airborne = matches!(pose, Pose::Jump | Pose::Fall);
        RunnerState {
            moving: moving || matches!(pose, Pose::Walk | Pose::Sprint | Pose::Chased | Pose::Swim),
            sprinting: sprinting || matches!(pose, Pose::Sprint | Pose::Chased),
            crouching: crouching || pose == Pose::Crouch,
            airborne_up: airborne && vy >= 0.0,
            airborne_down: airborne && vy < 0.0,
            climbing: pose == Pose::Climb,
            chased: pose == Pose::Chased,
            swimming: pose == Pose::Swim,
            stunned: pose == Pose::Stun,
            down: pose == Pose::Down,
            ..base
        }
    }
}

/// The clip to play and how the body looks while it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipChoice {
    pub clip: String,
    /// body fade target (0 opaque, 0.55 crouched)
    pub alpha: f32,
    /// cross-fade seconds when switching to this clip
    pub blend: f32,
    /// walk / sprint / chase: played at ground-speed pace
    pub paced: bool,
    /// forward lean target of the body, radians (a swimmer)
    pub pitch: f32,
    /// a paced clip moving backwards: the same clip plays with a negative speed scale, so the gait runs in reverse
    pub backward: bool,
}

/// player_controller_3d.gd 1923-1935 / remote_players.gd 736-763.
pub fn clip_for(state: &RunnerState) -> ClipChoice {
    let c = config();
    let pose = state.pose();
    let (alpha, blend, pitch) = match pose {
        Pose::Climb => (0.0, c.blend_climb, 0.0),
        Pose::Fall | Pose::Jump => (0.0, c.blend_air, 0.0),
        Pose::Crouch => (c.crouch_alpha, c.blend_idle, 0.0),
        Pose::Swim => (0.0, c.blend_travel, c.swim_pitch),
        Pose::Sprint | Pose::Chased | Pose::Walk => (0.0, c.blend_travel, 0.0),
        Pose::Idle => (0.0, c.blend_idle, 0.0),
        Pose::Stun | Pose::Down => (0.0, c.blend_default, 0.0),
    };
    let clip = c.clip(pose.key());
    let paced = c.paced_clips.contains(&clip);
    ClipChoice {
        paced,
        // Time reversal, not a walk_back clip: a walk cycle reversed in time keeps the left/right phase (each leg and
        // arm runs the same curve backwards), so it needs no leg/arm swap and no extra rig data.
        backward: paced && state.backward && state.moving,
        clip,
        alpha,
        blend,
        pitch,
    }
}

// ------------------------------------------------------------------------------------------------ pace

fn ref_speed(rig: Option<&Rig>, clip: &str) -> (f32, bool) {
    let c = config();
    let (r, rev) = match rig.and_then(|r| r.stride.get(clip)) {
        Some(&(r, rev)) => (r, rev),
        None => (c.stride_table.get(clip).copied().unwrap_or(c.fallback_ref), false),
    };
    (r.max(0.3) * c.model_scale, rev)
}

/// PlayerStride.pick / HunterStride.choose: the clip and speed_scale for a state clip at a real ground speed.
/// A still body (below `still_mps`) keeps its clip at scale 0; a ratio outside min..max swaps walk <-> sprint.
pub fn pick(rig: Option<&Rig>, state_clip: &str, ground_speed: f32) -> (String, f32) {
    let c = config();
    if ground_speed < c.still_mps {
        return (state_clip.to_string(), 0.0);
    }
    let mut clip = state_clip.to_string();
    let ratio = ground_speed / ref_speed(rig, &clip).0;
    if (ratio < c.min_ratio || ratio > c.max_ratio) && c.swap_clips.iter().any(|s| s == state_clip) {
        let mut best = f32::INFINITY;
        for cand in &c.swap_clips {
            let gap = (ground_speed.max(0.05) / ref_speed(rig, cand).0).ln().abs();
            if gap < best {
                best = gap;
                clip = cand.clone();
            }
        }
    }
    let (r, rev) = ref_speed(rig, &clip);
    let mut scale = (ground_speed / r).clamp(c.hard_min, c.hard_max);
    if rev {
        scale = -scale;
    }
    (clip, scale)
}

// ------------------------------------------------------------------------------------------------ rigs

thread_local! {
    static RIGS: RefCell<HashMap<bool, Option<Rc<Rig>>>> = RefCell::new(HashMap::new());
}

/// The player rig (`slim` = the 3 px arm variant), loaded once through the same embedded-asset path as the hunters'.
pub fn player_rig(slim: bool) -> Option<Rc<Rig>> {
    RIGS.with(|r| {
        r.borrow_mut()
            .entry(slim)
            .or_insert_with(|| {
                let c = config();
                let path = if slim { &c.rig_slim } else { &c.rig };
                let bytes = embedded::asset(path)?;
                Rig::parse("player_mannequin", std::str::from_utf8(bytes).ok()?).map(Rc::new)
            })
            .clone()
    })
}

// ------------------------------------------------------------------------------------------------ view

/// `a` turned into -PI..PI (radians), so a difference of two headings is the short way round.
pub fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// One runner's animation state, kept between frames (the immutable `Game` does not carry it).
pub struct RunnerView {
    pub rig: Rc<Rig>,
    pub player: AnimPlayer,
    pub last_pos: Option<(f32, f32)>,
    /// smoothed real ground speed, m/s
    pub speed: f32,
    /// smoothed pace scale of the current walk / sprint clip (None while not paced)
    pace: Option<f32>,
    /// body fade, 0 opaque .. crouch_alpha
    pub fade: f32,
    /// forward lean of the body now, radians (eases toward the swim pitch)
    pub pitch: f32,
    /// the head's tilt up (+) or down (-) and turn left (+) or right (-) now, radians, relative to the body
    pub head_pitch: f32,
    pub head_yaw: f32,
    /// the body's facing now, radians (+ left like the head): it turns only once the head has turned as far as its free
    /// range (see [`RunnerView::aim_head`]), so it is drawn with this yaw, not the aim
    pub body_yaw: f32,
    /// seconds the aim has been past the body's free range; the body turns once this passes `body_follow_delay`
    follow_timer: f32,
    primed: bool,
}

impl RunnerView {
    pub fn new(rig: Rc<Rig>) -> RunnerView {
        RunnerView {
            rig,
            player: AnimPlayer::default(),
            last_pos: None,
            speed: 0.0,
            pace: None,
            fade: 0.0,
            pitch: 0.0,
            head_pitch: 0.0,
            head_yaw: 0.0,
            body_yaw: 0.0,
            follow_timer: 0.0,
            primed: false,
        }
    }

    /// Sets the body's facing to `yaw` and clears the follow timer: the first frame of a figure, or a rebuilt one, starts
    /// with its body where its aim is.
    pub fn seed_yaw(&mut self, yaw: f32) {
        self.body_yaw = yaw;
        self.follow_timer = 0.0;
    }

    /// Eases the head toward its aim: `pitch` is the runner's aim (+ up), `aim_yaw` its aim's yaw (the facing, + left) in
    /// the same frame as `body_yaw`, `turn_rate` the body's yaw rate in radians a second (+ turning left), and `moving`
    /// whether the body is walking or sprinting (the free range is smaller then).
    ///
    /// Head first: the head turns toward the aim, up to `head_aim_yaw_limit`, and the body stays. When the aim is past the
    /// free range (`body_still_free` or `body_moving_free`), the overshoot runs a timer; after `body_follow_delay` the body
    /// eases (`body_follow_smoothing`) until the aim is back at the free range's edge. In range, the timer resets.
    /// The head trails a turn, so its yaw goes against the turn and back to the front.
    pub fn aim_head(&mut self, pitch: f32, aim_yaw: f32, turn_rate: f32, moving: bool, dt: f32) {
        let c = config();
        let dt = dt.max(0.0);
        let free = if moving { c.body_moving_free } else { c.body_still_free };
        let off = wrap_angle(aim_yaw - self.body_yaw);
        if off.abs() > free {
            self.follow_timer += dt;
            if self.follow_timer >= c.body_follow_delay {
                let target = aim_yaw - off.signum() * free;
                let k = 1.0 - (-c.body_follow_smoothing * dt).exp();
                self.body_yaw = wrap_angle(self.body_yaw + wrap_angle(target - self.body_yaw) * k);
            }
        } else {
            self.follow_timer = 0.0;
        }
        // the head's share of the aim, after the body has moved
        let off = wrap_angle(aim_yaw - self.body_yaw).clamp(-c.head_aim_yaw_limit, c.head_aim_yaw_limit);
        let goal_pitch = pitch.clamp(-c.head_pitch_limit, c.head_pitch_limit);
        let lag = (-turn_rate * c.head_yaw_lag).clamp(-c.head_yaw_limit, c.head_yaw_limit);
        let goal_yaw = (off + lag).clamp(-c.head_aim_yaw_limit, c.head_aim_yaw_limit);
        let k = 1.0 - (-c.head_smoothing * dt).exp();
        self.head_pitch += (goal_pitch - self.head_pitch) * k;
        self.head_yaw += (goal_yaw - self.head_yaw) * k;
    }

    /// A view for the player rig; None when the rig asset is missing.
    pub fn load(slim: bool) -> Option<RunnerView> {
        player_rig(slim).map(RunnerView::new)
    }

    /// Ground speed (m/s) from the last seen position; remembers `pos`. 0 on the first call or a zero step.
    pub fn speed_from(&mut self, pos: (f32, f32), dt: f32) -> f32 {
        let v = match self.last_pos {
            Some(p) if dt > 1e-5 => ((pos.0 - p.0).powi(2) + (pos.1 - p.1).powi(2)).sqrt() / dt,
            _ => 0.0,
        };
        self.last_pos = Some(pos);
        v
    }

    /// Advances the view by `dt` for `state`: picks the clip, paces it, fades the body and steps the player.
    pub fn update(&mut self, state: &RunnerState, dt: f32) {
        let c = config();
        let dt = dt.max(0.0);
        if !self.primed {
            self.speed = state.speed;
            self.primed = true;
        } else {
            self.speed += (state.speed - self.speed) * (1.0 - (-c.speed_smoothing * dt).exp());
        }
        let choice = clip_for(state);
        let step = dt * c.fade_rate;
        self.fade += (choice.alpha - self.fade).clamp(-step, step);
        self.pitch += (choice.pitch - self.pitch) * (1.0 - (-c.swim_pitch_rate * dt).exp());
        let rig = self.rig.clone();
        if choice.paced {
            let (clip, forward) = pick(Some(&rig), &choice.clip, self.speed);
            let goal = if choice.backward { -forward } else { forward };
            // a still body snaps to 0; a moving one eases, so a turn-around runs the scale through zero
            let scale = if goal == 0.0 {
                goal
            } else {
                let prev = self.pace.unwrap_or(goal);
                prev + (goal - prev) * (1.0 - (-c.scale_smoothing * dt).exp())
            };
            self.pace = Some(scale);
            self.play(&rig, &clip, choice.blend, true);
            self.player.speed_scale = scale;
        } else {
            self.pace = None;
            self.play(&rig, &choice.clip, choice.blend, false);
            self.player.speed_scale = 1.0;
        }
        // a remote jump is placed on its air clip by the wire's height, so the clip spans the real airtime
        match state.air_phase.filter(|_| matches!(state.pose(), Pose::Jump | Pose::Fall)) {
            Some(p) => {
                let len = rig.length(&self.player.assigned);
                self.player.seek(p.clamp(0.0, 1.0) * len);
                self.player.advance(&rig, 0.0);
            }
            None => self.player.advance(&rig, dt),
        }
    }

    /// ModelSlot.play_if_available; a switch between walk / sprint / chase keeps the stride phase.
    fn play(&mut self, rig: &Rig, clip: &str, blend: f32, paced: bool) {
        // a one-shot clip (the jump) that has run out holds its last frame until the state changes
        let held = !self.player.playing && rig.clips.get(clip).is_some_and(|c| !c.looping);
        if !rig.has_clip(clip) || (self.player.assigned == clip && (self.player.playing || held)) {
            return;
        }
        let mut phase = None;
        if paced
            && self.player.playing
            && config().paced_clips.iter().any(|p| p == &self.player.assigned)
        {
            phase = Some(self.player.position() / rig.length(&self.player.assigned).max(0.01));
        }
        self.player.play(rig, clip, blend);
        if let Some(p) = phase {
            self.player.seek(p * rig.length(clip));
        }
    }

    /// The mixed local pose of every rig bone, in rig bone order, with the head turned by its aim.
    pub fn locals(&self) -> Vec<Local> {
        let mut pose = self.player.pose(&self.rig);
        if let Some(head) = self.rig.bone("Head").and_then(|i| pose.get_mut(i)) {
            // yaw about the head's own up axis, then pitch about its side axis: the head's rest frame is the body's
            head.r *= Quat::from_rotation_y(self.head_yaw) * Quat::from_rotation_x(self.head_pitch);
        }
        pose
    }
}

/// Steps `view` and returns the pose as (bone name, local transform), the renderer's bone list.
pub fn pose(view: &mut RunnerView, state: &RunnerState, dt: f32) -> Vec<(String, Local)> {
    view.update(state, dt);
    view.rig
        .bones
        .iter()
        .map(|b| b.name.clone())
        .zip(view.locals())
        .collect()
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::Quat;

    fn st() -> RunnerState {
        RunnerState::default()
    }

    #[test]
    fn each_state_picks_its_clip() {
        let c = config();
        let walk = RunnerState { moving: true, ..st() };
        let sprint = RunnerState { moving: true, sprinting: true, ..st() };
        assert_eq!(clip_for(&st()).clip, c.clip("idle"));
        assert_eq!(clip_for(&walk).clip, c.clip("walk"));
        assert_eq!(clip_for(&sprint).clip, c.clip("sprint"));
        assert_eq!(
            clip_for(&RunnerState { chased: true, ..sprint }).clip,
            c.clip("chased")
        );
        assert_eq!(clip_for(&RunnerState { airborne_up: true, ..walk }).clip, c.clip("jump"));
        assert_eq!(clip_for(&RunnerState { airborne_down: true, ..walk }).clip, c.clip("fall"));
        assert_eq!(clip_for(&RunnerState { climbing: true, ..walk }).clip, c.clip("climb"));
        assert!(clip_for(&walk).paced && clip_for(&sprint).paced);
        assert!(!clip_for(&st()).paced);
    }

    #[test]
    fn standard_names_match_the_game() {
        assert_eq!(clip_for(&st()).clip, "idle");
        assert_eq!(clip_for(&RunnerState { moving: true, ..st() }).clip, "walk");
        assert_eq!(
            clip_for(&RunnerState { moving: true, sprinting: true, chased: true, ..st() }).clip,
            "chase"
        );
    }

    #[test]
    fn crouch_plays_idle_at_the_fade_alpha() {
        let ch = clip_for(&RunnerState { moving: true, crouching: true, ..st() });
        assert_eq!(ch.clip, config().clip("idle"));
        assert!((ch.alpha - 0.55).abs() < 1e-6, "Godot crouch fade is 0.55");
        assert_eq!(clip_for(&st()).alpha, 0.0);
    }

    #[test]
    fn walk_speed_scale_follows_ground_speed() {
        let r = ref_speed(None, "walk").0;
        let (c1, s1) = pick(None, "walk", r);
        let (c2, s2) = pick(None, "walk", r * 1.5);
        assert_eq!(c1, "walk");
        assert_eq!(c2, "walk");
        assert!((s1 - 1.0).abs() < 1e-4, "reference speed plays at scale 1");
        assert!((s2 / s1 - 1.5).abs() < 1e-3, "scale follows speed");
        // a still body holds its pose
        assert_eq!(pick(None, "walk", 0.0).1, 0.0);
        // far faster than a walk swaps to the sprint clip
        assert_eq!(pick(None, "walk", ref_speed(None, "sprint").0).0, "sprint");
        // and the scale never leaves the hard limits
        let c = config();
        let (_, hi) = pick(None, "sprint", 100.0);
        assert!(hi <= c.hard_max + 1e-6);
    }

    #[test]
    fn bad_config_key_is_named() {
        let bad = CONFIG_TEXT.replace("\"crouch_alpha\"", "\"crouch_alfa\"");
        let e = Config::parse(&bad).unwrap_err();
        assert!(e.contains("crouch_alpha"), "{e}");
        let bad = CONFIG_TEXT.replace("\"still_mps\": 0.05", "\"still_mps\": \"x\"");
        let e = Config::parse(&bad).unwrap_err();
        assert!(e.contains("speed.still_mps"), "{e}");
        let bad = CONFIG_TEXT.replace("\"climb\": \"climb\"", "\"clamb\": \"climb\"");
        let e = Config::parse(&bad).unwrap_err();
        assert!(e.contains("clips.climb"), "{e}");
        assert!(Config::parse(CONFIG_TEXT).is_ok());
    }

    #[test]
    fn both_rigs_have_the_thirteen_clips() {
        for slim in [false, true] {
            let rig = player_rig(slim).expect("player rig asset is embedded");
            assert_eq!(rig.clips.len(), 13, "slim={slim}");
            for name in &config().expected_clips {
                assert!(rig.has_clip(name), "slim={slim}: clip {name}");
            }
        }
    }

    fn bone_rot(p: &[(String, Local)], needle: &str) -> Quat {
        p.iter()
            .find(|(n, _)| n.to_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("no bone containing {needle}"))
            .1
            .r
    }

    #[test]
    fn walk_pose_swings_opposite_and_bends_the_knee() {
        let mut view = RunnerView::load(false).expect("rig");
        let walk = RunnerState {
            moving: true,
            speed: ref_speed(Some(&view.rig), "walk").0,
            ..st()
        };
        view.update(&walk, 0.0);
        // The Godot walk clip crosses neutral at different times for different bones (thighs at 0.25 s, the knee
        // bends most at 0 s), so each check samples the cycle where its bone is actually moving.
        let mut pose = |t: f32| -> Vec<(String, Local)> {
            view.player.seek(t);
            view.rig.bones.iter().map(|b| b.name.clone()).zip(view.locals()).collect()
        };
        let swing = |q: Quat| q.to_euler(macroquad::prelude::EulerRot::XYZ).0;
        let p = pose(0.125);
        let (lt, rt) = (swing(bone_rot(&p, "l-thigh")), swing(bone_rot(&p, "r-thigh")));
        let (la, ra) = (swing(bone_rot(&p, "l-arm")), swing(bone_rot(&p, "r-arm")));
        assert!(lt * rt < 0.0 && lt.abs() > 0.05, "thighs swing opposite ({lt} {rt})");
        assert!(la * ra < 0.0 && la.abs() > 0.05, "arms swing opposite ({la} {ra})");
        let knee = (0..8)
            .map(|i| {
                let p = pose(i as f32 * 0.125);
                bone_rot(&p, "l-calf")
                    .angle_between(Quat::IDENTITY)
                    .max(bone_rot(&p, "r-calf").angle_between(Quat::IDENTITY))
            })
            .fold(0.0f32, f32::max);
        assert!(knee > 0.05, "a knee bends ({knee})");
    }

    #[test]
    fn a_backwards_walk_selects_the_reversed_walk_and_a_forward_walk_does_not() {
        let speed = ref_speed(None, "walk").0;
        let back = RunnerState { moving: true, speed, backward: true, ..st() };
        let fwd = RunnerState { moving: true, speed, ..st() };
        assert!(clip_for(&back).backward, "a backwards walk picks the backwards gait");
        assert_eq!(clip_for(&back).clip, config().clip("walk"), "the backwards gait is the walk clip, time-reversed");
        assert!(!clip_for(&fwd).backward, "a forward walk does not");
        assert!(!clip_for(&RunnerState { backward: true, ..st() }).backward, "a still body is not walking backwards");
        // played, the backwards walk runs the walk clip at a negative scale; the forward walk runs it forwards
        let mut back_view = RunnerView::load(false).expect("rig");
        let mut fwd_view = RunnerView::load(false).expect("rig");
        for _ in 0..30 {
            back_view.update(&back, 1.0 / 30.0);
            fwd_view.update(&fwd, 1.0 / 30.0);
        }
        assert_eq!(back_view.player.current_animation(), "walk");
        assert!(back_view.player.speed_scale < 0.0, "backwards: {}", back_view.player.speed_scale);
        assert_eq!(fwd_view.player.current_animation(), "walk");
        assert!(fwd_view.player.speed_scale > 0.0, "forwards: {}", fwd_view.player.speed_scale);
    }

    #[test]
    fn a_turn_around_passes_the_scale_through_zero() {
        let mut view = RunnerView::load(false).expect("rig");
        let speed = ref_speed(Some(&view.rig), "walk").0;
        let fwd = RunnerState { moving: true, speed, ..st() };
        let back = RunnerState { backward: true, ..fwd };
        for _ in 0..30 {
            view.update(&fwd, 1.0 / 30.0);
        }
        assert!(view.player.speed_scale > 0.5);
        view.update(&back, 1.0 / 30.0);
        assert!(view.player.speed_scale > 0.0 && view.player.speed_scale < 1.0, "eases, not a snap: {}", view.player.speed_scale);
        for _ in 0..60 {
            view.update(&back, 1.0 / 30.0);
        }
        assert!(view.player.speed_scale < -0.5, "settles reversed: {}", view.player.speed_scale);
    }

    #[test]
    fn stopped_runner_drops_to_idle_after_the_blend() {
        let mut view = RunnerView::load(false).expect("rig");
        let speed = ref_speed(Some(&view.rig), "walk").0;
        let walk = RunnerState { moving: true, speed, ..st() };
        for _ in 0..30 {
            view.update(&walk, 1.0 / 30.0);
        }
        assert_eq!(view.player.current_animation(), "walk");
        let stopped = st();
        view.update(&stopped, 1.0 / 30.0);
        assert_eq!(view.player.current_animation(), "idle");
        // run past the blend: the pose is the idle clip alone
        for _ in 0..60 {
            view.update(&stopped, 1.0 / 30.0);
        }
        let mut reference = AnimPlayer::default();
        let rig = view.rig.clone();
        reference.play(&rig, "idle", 0.0);
        reference.seek(view.player.position());
        let a = view.locals();
        let b = reference.pose(&rig);
        for (x, y) in a.iter().zip(b.iter()) {
            assert!(x.r.dot(y.r).abs() > 0.9999 && (x.t - y.t).length() < 1e-3);
        }
    }

    #[test]
    fn crouch_fades_the_body_toward_the_alpha() {
        let mut view = RunnerView::load(false).expect("rig");
        let crouch = RunnerState { crouching: true, ..st() };
        view.update(&crouch, 0.1);
        assert!(view.fade > 0.0 && view.fade < config().crouch_alpha);
        for _ in 0..60 {
            view.update(&crouch, 0.1);
        }
        assert!((view.fade - config().crouch_alpha).abs() < 1e-5);
        for _ in 0..60 {
            view.update(&st(), 0.1);
        }
        assert_eq!(view.fade, 0.0);
    }

    #[test]
    fn an_aimed_head_tilts_the_head_bone_up_and_clamps() {
        let mut v = RunnerView::load(false).expect("the player rig is embedded");
        let head = v.rig.bone("Head").expect("the rig has a Head bone");
        let rest = v.locals()[head].r;
        // a level head is the rest pose
        assert!(v.locals()[head].r.angle_between(rest) < 1e-6);
        // aimed up: the head's forward (the body's -Z) rises
        for _ in 0..200 {
            v.aim_head(0.6, 0.0, 0.0, false, 1.0 / 60.0);
        }
        let forward = v.locals()[head].r * macroquad::prelude::vec3(0.0, 0.0, -1.0);
        assert!(forward.y > 0.4, "the head looks up: {forward:?}");
        // a hostile aim is clamped to the limit, so the head never flips over
        for _ in 0..600 {
            v.aim_head(9.0, 0.0, 0.0, false, 1.0 / 60.0);
        }
        assert!(v.head_pitch <= config().head_pitch_limit + 1e-4);
        assert!((v.head_pitch - config().head_pitch_limit).abs() < 1e-2, "{}", v.head_pitch);
    }

    #[test]
    fn the_head_trails_a_turn_then_comes_back_to_the_front() {
        let mut v = RunnerView::load(false).expect("the player rig is embedded");
        for _ in 0..30 {
            v.aim_head(0.0, 0.0, 4.0, false, 1.0 / 60.0); // turning left fast
        }
        assert!(v.head_yaw < -0.05, "the head lags the turn, looking back: {}", v.head_yaw);
        assert!(v.head_yaw >= -config().head_yaw_limit - 1e-4, "the turn is capped");
        for _ in 0..300 {
            v.aim_head(0.0, 0.0, 0.0, false, 1.0 / 60.0);
        }
        assert!(v.head_yaw.abs() < 1e-3, "a still body brings the head to the front");
    }

    fn frame() -> f32 {
        1.0 / 60.0
    }

    #[test]
    fn the_head_turns_before_the_body_within_the_free_range() {
        let mut v = RunnerView::load(false).expect("the player rig is embedded");
        // 0.5 rad (29 degrees) is inside the standing free range (55 degrees): the head takes it, the body stays
        for _ in 0..120 {
            v.aim_head(0.0, 0.5, 0.0, false, frame());
        }
        assert!((v.head_yaw - 0.5).abs() < 0.02, "the head looks at the aim: {}", v.head_yaw);
        assert_eq!(v.body_yaw, 0.0, "the body does not turn inside its free range");
        // walking narrows the free range to 20 degrees: 0.3 rad (17 degrees) still stays head-only
        let mut w = RunnerView::load(false).expect("rig");
        for _ in 0..120 {
            w.aim_head(0.0, 0.3, 0.0, true, frame());
        }
        assert_eq!(w.body_yaw, 0.0, "a walking body keeps its facing for a 17 degree look");
        assert!((w.head_yaw - 0.3).abs() < 0.02);
    }

    #[test]
    fn the_head_is_capped_at_the_aim_limit_and_the_body_follows_after_the_delay() {
        let c = config();
        let mut v = RunnerView::load(false).expect("rig");
        let aim = 1.5; // 86 degrees: past the standing free range (55 degrees)
        for _ in 0..10 {
            v.aim_head(0.0, aim, 0.0, false, frame());
        }
        assert_eq!(v.body_yaw, 0.0, "the body waits out the follow delay");
        assert!(v.head_yaw <= c.head_aim_yaw_limit + 1e-4, "the head is capped: {}", v.head_yaw);
        for _ in 0..240 {
            v.aim_head(0.0, aim, 0.0, false, frame());
        }
        let target = aim - c.body_still_free;
        assert!((v.body_yaw - target).abs() < 0.02, "the body turns until the aim is at its free edge: {} vs {target}", v.body_yaw);
    }

    #[test]
    fn the_follow_timer_resets_when_the_aim_comes_back_in_range() {
        let mut v = RunnerView::load(false).expect("rig");
        for _ in 0..5 {
            v.aim_head(0.0, 1.5, 0.0, false, frame());
        }
        assert!(v.follow_timer > 0.0, "past the free range the timer runs");
        v.aim_head(0.0, 0.2, 0.0, false, frame());
        assert_eq!(v.follow_timer, 0.0, "back in range the timer resets");
        // a new overshoot waits the whole delay again
        for _ in 0..10 {
            v.aim_head(0.0, 1.5, 0.0, false, frame());
        }
        assert_eq!(v.body_yaw, 0.0, "the delay starts over after a reset");
    }

    #[test]
    fn seeding_puts_the_body_at_the_aim_and_clears_the_timer() {
        let mut v = RunnerView::load(false).expect("rig");
        for _ in 0..5 {
            v.aim_head(0.0, 1.5, 0.0, false, frame());
        }
        v.seed_yaw(-0.7);
        assert_eq!(v.body_yaw, -0.7);
        assert_eq!(v.follow_timer, 0.0, "a reseeded body starts a fresh delay");
    }

    #[test]
    fn wrap_takes_the_short_way_round() {
        use std::f32::consts::PI;
        assert!((wrap_angle(3.5) - (3.5 - 2.0 * PI)).abs() < 1e-5);
        assert!((wrap_angle(-PI - 0.1) - (PI - 0.1)).abs() < 1e-5);
        assert!((wrap_angle(0.25) - 0.25).abs() < 1e-6);
        // an aim just across the seam from the body is a small turn, not a full circle
        let mut v = RunnerView::load(false).expect("rig");
        v.body_yaw = PI - 0.1;
        v.aim_head(0.0, -PI + 0.1, 0.0, false, frame());
        assert!(v.head_yaw.abs() < 0.2, "the head turns the short way: {}", v.head_yaw);
    }

    #[test]
    fn the_air_phase_maps_the_height_onto_the_rise_and_the_fall() {
        let peak = config().jump_peak;
        assert_eq!(air_phase(0.0, 2.0), 0.0, "at takeoff the clip starts");
        assert!((air_phase(peak, 0.5) - 0.5).abs() < 1e-6, "at the peak the clip is halfway, rising");
        assert!((air_phase(peak, -0.5) - 0.5).abs() < 1e-6, "and halfway, falling");
        assert!((air_phase(0.0, -2.0) - 1.0).abs() < 1e-6, "back on the floor the clip ends");
        assert!(air_phase(peak * 3.0, 1.0) <= 0.5 + 1e-6, "a height past the peak stays at the peak");
    }

    #[test]
    fn a_remote_jump_plays_the_air_clip_at_its_phase_not_in_time() {
        let mut v = RunnerView::load(false).expect("rig");
        let clip = config().clip("jump");
        let len = v.rig.length(&clip);
        let up = |phase| RunnerState { airborne_up: true, air_phase: Some(phase), ..st() };
        v.update(&up(0.25), 0.0);
        assert!((v.player.position() - 0.25 * len).abs() < 1e-3, "the clip is at the phase: {}", v.player.position());
        // a second of wire time does not move the clip on: the phase drives it
        v.update(&up(0.25), 1.0);
        assert!((v.player.position() - 0.25 * len).abs() < 1e-3, "the phase holds, time does not move it");
        v.update(&up(0.75), 1.0 / 60.0);
        assert!((v.player.position() - 0.75 * len).abs() < 1e-3, "the phase moves it on: {}", v.player.position());
    }

    #[test]
    fn a_local_jump_without_a_phase_still_plays_in_time() {
        let mut v = RunnerView::load(false).expect("rig");
        let clip = config().clip("jump");
        let len = v.rig.length(&clip);
        let jump = RunnerState { airborne_up: true, ..st() };
        v.update(&jump, 0.0);
        v.update(&jump, len * 0.5);
        assert!((v.player.position() - len * 0.5).abs() < 1e-3, "time plays the clip: {}", v.player.position());
    }

    #[test]
    fn speed_from_positions() {
        let mut view = RunnerView::load(false).expect("rig");
        assert_eq!(view.speed_from((0.0, 0.0), 0.1), 0.0);
        assert!((view.speed_from((0.3, 0.4), 0.1) - 5.0).abs() < 1e-4);
    }
}
