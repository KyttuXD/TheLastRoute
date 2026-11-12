//! Spectator mode for a downed or dead runner in a room with other runners (multiplayer and public lobbies).
//!
//! A downed runner (game/revive.rs) or a dead one (the run card, ui/report.rs) may choose SPECTATE when another runner is
//! in the room. A spectator keeps watching: it follows another runner's view, cycled with LEFT and RIGHT
//! ([`Spectate::cycle`]), and it cannot move, interact or be caught. The labels are in data/ui/spectate.json.
//!
//! The camera sits behind the watched runner ([`Game::spectate_camera`], on from the start; [F] goes back to the
//! spectator's own eyes). It can only show what this client has loaded: a runner outside this runner's window of
//! chunks, or more than a storey above or below, is named on screen but the view stays with the spectator's body.
//!
//! A downed runner starts watching with the prompt button or with [E] (its mouse is held by the look, so a key is
//! needed). A teammate's revive ends the watching and gives the controls back; a dead runner watches until the run ends.

use super::{Controls, Game};
use serde_json::Value;
use std::sync::OnceLock;

/// The embedded file (data/ui/spectate.json), by its path under the data folder.
pub const SPECTATE_PATH: &str = "ui/spectate.json";

/// The spectator labels, from data/ui/spectate.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Labels {
    pub button: String,
    pub downed_prompt: String,
    /// Contains `{name}`, replaced by the watched runner's name.
    pub watching: String,
    pub no_one: String,
    pub hint: String,
}

impl Labels {
    pub fn parse(text: &str) -> Result<Labels, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|x| !x.trim().is_empty())
                .ok_or_else(|| format!("{k} must be a non-empty string"))
        };
        let watching = s("watching")?;
        if !watching.contains("{name}") {
            return Err("watching must contain {name}".into());
        }
        Ok(Labels {
            button: s("button")?,
            downed_prompt: s("downed_prompt")?,
            watching,
            no_one: s("no_one")?,
            hint: s("hint")?,
        })
    }

    /// The watching line for a runner called `name`.
    pub fn watching_line(&self, name: &str) -> String {
        self.watching.replace("{name}", name)
    }
}

/// The embedded labels (data/ui/spectate.json), parsed once.
pub fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == SPECTATE_PATH)
            .map(|(_, t)| t)
            .expect("data/ui/spectate.json is embedded by build.rs");
        Labels::parse(text).expect("data/ui/spectate.json is valid")
    })
}

/// The follow camera numbers, from data/ui/spectate_camera.json.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraCfg {
    /// How far behind the watched runner the eye sits.
    pub distance: f32,
    /// Eye height above the runner's feet.
    pub height: f32,
    /// Height of the point the camera looks at.
    pub look_height: f32,
    pub follow_label: String,
    /// Shown in place of `follow_label` while the watched runner is too far from this runner's floor to be shown.
    pub out_of_sight: String,
    pub cycle_hint: String,
}

impl CameraCfg {
    pub fn parse(text: &str) -> Result<CameraCfg, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let n = |k: &str| {
            v.get(k)
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite() && *x >= 0.0)
                .map(|x| x as f32)
                .ok_or_else(|| format!("{k} must be a number of 0 or more"))
        };
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|x| !x.trim().is_empty())
                .ok_or_else(|| format!("{k} must be a non-empty string"))
        };
        Ok(CameraCfg { distance: n("distance")?, height: n("height")?, look_height: n("look_height")?, follow_label: s("follow_label")?, out_of_sight: s("out_of_sight")?, cycle_hint: s("cycle_hint")? })
    }
}

/// The embedded follow camera numbers (data/ui/spectate_camera.json), parsed once.
pub fn camera_cfg() -> &'static CameraCfg {
    static CFG: OnceLock<CameraCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == "ui/spectate_camera.json")
            .map(|(_, t)| t)
            .expect("data/ui/spectate_camera.json is embedded by build.rs");
        CameraCfg::parse(text).expect("data/ui/spectate_camera.json is valid")
    })
}

/// The follow camera: the eye `distance` behind a runner at `pos` (x, y feet, z) facing `facing` (x, z), raised to
/// `height`, and the point it looks at. A zero facing puts the eye straight above.
pub fn follow_pose(pos: [f32; 3], facing: [f32; 2], cfg: &CameraCfg) -> ([f32; 3], [f32; 3]) {
    let len = (facing[0] * facing[0] + facing[1] * facing[1]).sqrt();
    let (fx, fz) = if len > 1e-4 { (facing[0] / len, facing[1] / len) } else { (0.0, 0.0) };
    let eye = [pos[0] - fx * cfg.distance, pos[1] + cfg.height, pos[2] - fz * cfg.distance];
    (eye, [pos[0], pos[1] + cfg.look_height, pos[2]])
}

/// The peers still alive, in roster order. `roster` pairs a peer with whether it is alive.
pub fn living_peers(roster: &[(u32, bool)]) -> Vec<u32> {
    roster.iter().filter(|(_, alive)| *alive).map(|(p, _)| *p).collect()
}

/// The spectator state: the room peer being watched, None when no other runner is left to watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Spectate {
    pub target: Option<u32>,
    /// True while the camera sits behind the watched runner ([F] toggles it).
    pub follow: bool,
}

/// Where the watched runner stands, as this client last heard: its place in this window's cells, its feet height, its
/// yaw. The app sets it each frame (`Game::spectate_watched`) from the runners near enough to be drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watched {
    pub at: (f32, f32),
    pub h: f32,
    pub yaw: f32,
}

/// Whether the player may choose SPECTATE: it is downed or dead, and another runner is in the room to watch.
pub fn may_spectate(downed: bool, over: bool, peers: &[u32]) -> bool {
    (downed || over) && !peers.is_empty()
}

impl Spectate {
    /// The transition into spectator mode: watching the first other runner. None when the player may not spectate.
    pub fn enter(downed: bool, over: bool, peers: &[u32]) -> Option<Spectate> {
        may_spectate(downed, over, peers).then(|| Spectate { target: peers.first().copied(), follow: true })
    }

    /// Moves the watched runner `step` places along `peers` (wrapping at both ends). A target no longer in the room, or
    /// none, goes to the first runner. With no runner left the target is None.
    pub fn cycle(&mut self, peers: &[u32], step: i32) {
        if peers.is_empty() {
            self.target = None;
            return;
        }
        let n = peers.len() as i32;
        let i = match self.target.and_then(|t| peers.iter().position(|p| *p == t)) {
            Some(i) => (i as i32 + step).rem_euclid(n) as usize,
            None => 0,
        };
        self.target = Some(peers[i]);
    }
}

impl Game {
    /// True while this runner watches the room (game/spectate.rs).
    pub fn spectating(&self) -> bool {
        self.spectate.is_some()
    }

    /// The controls the run actually gets this frame: a spectator gets none, so it cannot move, interact, use items or
    /// open shops. The one key a downed spectator keeps is the use key, which spends a revive kit (game/revive.rs).
    /// The player's own keys pass through otherwise.
    pub fn effective_controls(&self, c: &Controls) -> Controls {
        if self.spectating() {
            Controls { use_item: self.downed.is_some() && c.use_item, ..Controls::default() }
        } else {
            c.clone()
        }
    }

    /// Ends the watching of a runner who is back on its feet (a revive): the controls and the view are its own again.
    /// A dead runner keeps watching.
    pub fn spectate_leave_if_up(&mut self) {
        if self.downed.is_none() && self.report.is_none() {
            self.spectate = None;
            self.spectate_watched = None;
        }
    }

    /// The follow camera this frame: the eye behind the watched runner and the point it looks at, in the scene's
    /// coordinates. None when not following, or when the watched runner is not near enough to be drawn here.
    pub fn spectate_camera(&self) -> Option<([f32; 3], [f32; 3])> {
        if !self.spectate_following() {
            return None;
        }
        let w = self.spectate_watched?;
        let inside = w.at.0 >= 0.0 && w.at.1 >= 0.0 && w.at.0 < self.fac.cols as f32 && w.at.1 < self.fac.rows as f32;
        if !inside {
            return None;
        }
        Some(follow_pose([w.at.0, w.h, w.at.1], [-w.yaw.sin(), -w.yaw.cos()], camera_cfg()))
    }

    /// [F]: toggles the camera behind the watched runner. No effect when not spectating.
    pub fn spectate_toggle_follow(&mut self) {
        if let Some(s) = self.spectate.as_mut() {
            s.follow = !s.follow;
        }
    }

    /// True when the camera should sit behind the watched runner this frame.
    pub fn spectate_following(&self) -> bool {
        self.spectate.is_some_and(|s| s.follow && s.target.is_some())
    }

    /// [E] (+1) and [Q] (-1): like [`Game::spectate_cycle`] but over the living runners only (`roster`: peer, alive).
    pub fn spectate_cycle_living(&mut self, roster: &[(u32, bool)], step: i32) {
        self.spectate_cycle(&living_peers(roster), step);
    }

    /// The peer the spectator watches, if any.
    pub fn spectate_target(&self) -> Option<u32> {
        self.spectate.and_then(|s| s.target)
    }

    /// The downed or dead runner chooses SPECTATE. False when it may not (alive, or no other runner in the room).
    pub fn spectate_enter(&mut self, peers: &[u32]) -> bool {
        if self.spectating() {
            return true;
        }
        match Spectate::enter(self.downed.is_some(), self.report.is_some(), peers) {
            Some(s) => {
                self.spectate = Some(s);
                true
            }
            None => false,
        }
    }

    /// LEFT (-1) or RIGHT (+1): the spectator watches the next or previous runner of `peers`. No effect when not spectating.
    pub fn spectate_cycle(&mut self, peers: &[u32], step: i32) {
        if let Some(s) = self.spectate.as_mut() {
            s.cycle(peers, step);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn game() -> Game {
        let content = Content::load();
        let catalog = Catalog::load();
        Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-SPECTATE-7", 0, &content, &catalog)
    }

    #[test]
    fn the_labels_load_and_the_name_goes_into_the_watching_line() {
        let l = labels();
        assert_eq!(l.button, "SPECTATE");
        assert_eq!(l.watching_line("ANNA"), "WATCHING ANNA");
        assert!(Labels::parse("{\"button\":\"S\",\"downed_prompt\":\"D\",\"watching\":\"W\",\"no_one\":\"N\",\"hint\":\"H\"}").is_err(), "watching needs {{name}}");
    }

    #[test]
    fn a_living_runner_cannot_spectate() {
        assert!(Spectate::enter(false, false, &[2, 3]).is_none(), "alive: no spectator mode");
        assert!(!may_spectate(false, false, &[2, 3]));
    }

    #[test]
    fn a_downed_or_dead_runner_with_someone_to_watch_goes_to_spectate() {
        assert_eq!(Spectate::enter(true, false, &[2, 3]), Some(Spectate { target: Some(2), follow: true }), "downed: watches the first runner");
        assert_eq!(Spectate::enter(false, true, &[5]), Some(Spectate { target: Some(5), follow: true }), "dead: watches the first runner");
        assert!(Spectate::enter(true, false, &[]).is_none(), "alone in the room: nobody to watch");
    }

    #[test]
    fn spectate_cycles_through_the_runners_both_ways_and_wraps() {
        let peers = [2, 3, 7];
        let mut s = Spectate { target: Some(2), ..Default::default() };
        s.cycle(&peers, 1);
        assert_eq!(s.target, Some(3));
        s.cycle(&peers, 1);
        assert_eq!(s.target, Some(7));
        s.cycle(&peers, 1);
        assert_eq!(s.target, Some(2), "RIGHT past the last runner wraps to the first");
        s.cycle(&peers, -1);
        assert_eq!(s.target, Some(7), "LEFT past the first runner wraps to the last");
    }

    #[test]
    fn a_watched_runner_who_leaves_moves_the_view_to_the_first_one_left() {
        let mut s = Spectate { target: Some(3), ..Default::default() };
        s.cycle(&[2, 7], 1);
        assert_eq!(s.target, Some(2), "the target left the room: the first runner is watched");
        let mut none = Spectate { target: Some(3), ..Default::default() };
        none.cycle(&[], 1);
        assert_eq!(none.target, None, "nobody left: no target");
    }

    #[test]
    fn follow_targets_only_living_teammates_and_cycling_wraps() {
        let roster = [(2, true), (3, false), (7, true)];
        assert_eq!(living_peers(&roster), vec![2, 7], "the dead runner is skipped");
        let mut g = game();
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&living_peers(&roster)));
        assert_eq!(g.spectate_target(), Some(2));
        g.spectate_cycle_living(&roster, 1);
        assert_eq!(g.spectate_target(), Some(7), "3 is dead: E goes 2 to 7");
        g.spectate_cycle_living(&roster, 1);
        assert_eq!(g.spectate_target(), Some(2), "E wraps");
        g.spectate_cycle_living(&roster, -1);
        assert_eq!(g.spectate_target(), Some(7), "Q wraps back");
        // the watched runner dies: the view moves to a living one
        g.spectate_cycle_living(&[(2, true), (7, false)], 1);
        assert_eq!(g.spectate_target(), Some(2));
    }

    #[test]
    fn follow_toggles_only_while_spectating_and_needs_a_target() {
        let mut g = game();
        g.spectate_toggle_follow();
        assert!(!g.spectate_following(), "a runner who is not spectating cannot follow");
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        assert!(g.spectate_following(), "the follow camera is on from the start");
        g.spectate_toggle_follow();
        assert!(!g.spectate_following(), "[F] goes back to the spectator's own eyes");
        g.spectate_toggle_follow();
        assert!(g.spectate_following());
        g.spectate_cycle(&[], 1);
        assert!(!g.spectate_following(), "nobody left to follow");
    }

    #[test]
    fn the_follow_camera_sits_behind_and_above_by_the_data_numbers() {
        let cfg = camera_cfg();
        assert!(cfg.distance > 0.0 && cfg.height > 0.0);
        let (eye, look) = follow_pose([10.0, 0.0, 5.0], [0.0, 2.0], cfg);
        assert!((eye[2] - (5.0 - cfg.distance)).abs() < 1e-4 && (eye[0] - 10.0).abs() < 1e-4, "behind a runner facing +z");
        assert!((eye[1] - cfg.height).abs() < 1e-4);
        assert!((look[1] - cfg.look_height).abs() < 1e-4 && look[2] == 5.0);
        let (above, _) = follow_pose([0.0, 0.0, 0.0], [0.0, 0.0], cfg);
        assert!(above[0] == 0.0 && above[2] == 0.0, "no facing: straight above");
        assert!(CameraCfg::parse("{\"distance\":-1}").is_err());
    }

    #[test]
    fn a_downed_runner_becomes_a_spectator_and_cycles_on_the_game() {
        let mut g = game();
        // another runner in the room could revive: the capture downs the runner (game/revive.rs)
        g.room_teammates = 1;
        g.end_run(0, "caught");
        assert!(g.runner_downed());
        assert!(!g.spectate_enter(&[]), "no other runner: stays downed, not spectating");
        assert!(!g.spectating());
        assert!(g.spectate_enter(&[2, 3]));
        assert!(g.spectating());
        assert_eq!(g.spectate_target(), Some(2));
        g.spectate_cycle(&[2, 3], 1);
        assert_eq!(g.spectate_target(), Some(3));
    }

    #[test]
    fn a_spectator_cannot_move_interact_or_use_anything() {
        let mut g = game();
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        let pressed = Controls { fwd: 1.0, strafe: 1.0, sprint: true, interact: true, interact_held: true, use_item: true, flashlight: true, ..Controls::default() };
        let given = g.effective_controls(&pressed);
        assert!(given.fwd == 0.0 && given.strafe == 0.0 && !given.interact && !given.interact_held && !given.use_item && !given.flashlight && !given.sprint);
        let before = g.p.pos;
        g.step(0.5, &given, &Content::load(), &Catalog::load(), &crate::audio::Sounds::default());
        assert_eq!(g.p.pos, before, "a spectator body does not walk");
    }

    #[test]
    fn a_downed_runner_cannot_move_before_or_while_it_watches() {
        let (content, catalog, sounds) = (Content::load(), Catalog::load(), crate::audio::Sounds::default());
        let mut g = game();
        g.room_teammates = 1;
        g.end_run(0, "caught");
        assert!(g.runner_downed());
        let pressed = Controls { fwd: 1.0, strafe: 1.0, sprint: true, jump: true, ..Controls::default() };
        let before = g.p.pos;
        for _ in 0..30 {
            g.step(1.0 / 60.0, &pressed, &content, &catalog, &sounds);
        }
        assert_eq!(g.p.pos, before, "a downed runner lies still");
        assert!(g.spectate_enter(&[2]));
        for _ in 0..30 {
            g.step(1.0 / 60.0, &pressed, &content, &catalog, &sounds);
        }
        assert_eq!(g.p.pos, before, "and still lies there while it watches");
    }

    #[test]
    fn a_revive_ends_the_watching_and_gives_the_controls_back() {
        let mut g = game();
        g.room_teammates = 1;
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        g.spectate_watched = Some(Watched { at: (5.0, 5.0), h: 0.0, yaw: 0.0 });
        g.spectate_leave_if_up();
        assert!(g.spectating(), "still down: still watching");
        g.downed = None;
        g.spectate_leave_if_up();
        assert!(!g.spectating() && g.spectate_watched.is_none(), "back up: the view and the keys are its own");
        let pressed = Controls { fwd: 1.0, ..Controls::default() };
        assert_eq!(g.effective_controls(&pressed).fwd, 1.0);
        // a dead runner keeps watching
        let mut dead = game();
        dead.end_run(0, "caught");
        assert!(dead.report.is_some() && dead.spectate_enter(&[2]));
        dead.spectate_leave_if_up();
        assert!(dead.spectating());
    }

    #[test]
    fn a_downed_spectator_keeps_only_the_revive_kit_key() {
        let mut g = game();
        g.room_teammates = 1;
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        let pressed = Controls { fwd: 1.0, interact: true, use_item: true, ..Controls::default() };
        let given = g.effective_controls(&pressed);
        assert!(given.use_item && given.fwd == 0.0 && !given.interact);
    }

    #[test]
    fn the_follow_camera_needs_a_watched_runner_inside_the_window() {
        let mut g = game();
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        assert!(g.spectate_camera().is_none(), "nothing heard of the runner yet");
        g.spectate_watched = Some(Watched { at: (20.0, 30.0), h: 0.0, yaw: 0.0 });
        let (eye, look) = g.spectate_camera().expect("in the window: followed");
        let cfg = camera_cfg();
        // yaw 0 faces -z (game/player.rs forward2), so the eye is behind at +z
        assert!((eye[0] - 20.0).abs() < 1e-4 && (eye[2] - (30.0 + cfg.distance)).abs() < 1e-4 && (look[2] - 30.0).abs() < 1e-4);
        g.spectate_watched = Some(Watched { at: (-4.0, 30.0), h: 0.0, yaw: 0.0 });
        assert!(g.spectate_camera().is_none(), "outside this window: not drawn here, so not followed");
        g.spectate_watched = Some(Watched { at: (20.0, 30.0), h: 0.0, yaw: 0.0 });
        g.spectate_toggle_follow();
        assert!(g.spectate_camera().is_none(), "[F] off: the spectator's own eyes");
    }

    #[test]
    fn a_spectator_is_never_caught_again() {
        let mut g = game();
        // another runner in the room: the catch downs the runner, who may then spectate
        g.room_teammates = 1;
        g.end_run(0, "caught");
        assert!(g.spectate_enter(&[2]));
        assert!(g.spectating());
        // a further catch while spectating does not end the run or move the state on
        g.end_run(0, "caught");
        assert!(g.spectating(), "spectator stays spectator");
        assert!(g.report.is_none(), "a spectator's catch does not end the run");
    }
}
