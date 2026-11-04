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
        };
        for key in ["idle", "walk", "sprint", "chased", "crouch", "jump", "fall", "climb"] {
            if !cfg.clips.contains_key(key) {
                return Err(format!("anim.json: missing key \"clips.{key}\""));
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

/// What a runner is doing, as the clip choice reads it (local controller state, or a remote's replicated flags).
#[derive(Debug, Clone, Copy, Default)]
pub struct RunnerState {
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
    pub airborne_up: bool,
    pub airborne_down: bool,
    pub climbing: bool,
    /// a hunter is chasing this runner: the sprint becomes the chase clip
    pub chased: bool,
    /// real ground speed, metres per second
    pub speed: f32,
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
}

/// player_controller_3d.gd 1923-1935 / remote_players.gd 736-763.
pub fn clip_for(state: &RunnerState) -> ClipChoice {
    let c = config();
    let (key, alpha, blend) = if state.climbing {
        ("climb", 0.0, c.blend_climb)
    } else if state.airborne_down {
        ("fall", 0.0, c.blend_air)
    } else if state.airborne_up {
        ("jump", 0.0, c.blend_air)
    } else if state.crouching {
        ("crouch", c.crouch_alpha, c.blend_idle)
    } else if state.moving && state.sprinting {
        (if state.chased { "chased" } else { "sprint" }, 0.0, c.blend_travel)
    } else if state.moving {
        ("walk", 0.0, c.blend_travel)
    } else {
        ("idle", 0.0, c.blend_idle)
    };
    let clip = c.clip(key);
    ClipChoice {
        paced: c.paced_clips.contains(&clip),
        clip,
        alpha,
        blend,
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
            primed: false,
        }
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
        let rig = self.rig.clone();
        if choice.paced {
            let (clip, goal) = pick(Some(&rig), &choice.clip, self.speed);
            let scale = if goal <= 0.0 {
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
        self.player.advance(&rig, dt);
    }

    /// ModelSlot.play_if_available; a switch between walk / sprint / chase keeps the stride phase.
    fn play(&mut self, rig: &Rig, clip: &str, blend: f32, paced: bool) {
        if !rig.has_clip(clip) || (self.player.assigned == clip && self.player.playing) {
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

    /// The mixed local pose of every rig bone, in rig bone order.
    pub fn locals(&self) -> Vec<Local> {
        self.player.pose(&self.rig)
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
    fn speed_from_positions() {
        let mut view = RunnerView::load(false).expect("rig");
        assert_eq!(view.speed_from((0.0, 0.0), 0.1), 0.0);
        assert!((view.speed_from((0.3, 0.4), 0.1) - 5.0).abs() < 1e-4);
    }
}
