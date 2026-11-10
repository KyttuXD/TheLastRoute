//! Night Watch hunter states -> pose names (NightWatchPoses.pose_for_state) and the vent crawl state machine
//! (NightWatchVentPose). Pure logic: no drawing, no GL.
#![allow(dead_code)]

/// The Night Watch hunter states a mode can put an actor in. `Rig` means "no Night Watch pose": the rig's own
/// idle / walk / sprint clips play (see `Actor::set_locomotion`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoseState {
    Rig,
    Idle,
    Doorway,
    AtDoor,
    DoorGrace,
    PeekLeft,
    PeekRight,
    Camera,
    Monitor,
    Glass,
    Vent,
    Approach,
    Attack,
    Retreat,
    LightFlinch,
    Glitch,
    Kneel,
    Tired,
    PointOffice,
    Greet,
    Listening,
    Reaching,
    Waiting,
}

impl PoseState {
    /// The state key used by night_watch_poses.json "states" (None for `Rig`).
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            PoseState::Rig => return None,
            PoseState::Idle => "idle",
            PoseState::Doorway => "doorway",
            PoseState::AtDoor => "at_door",
            PoseState::DoorGrace => "door_grace",
            PoseState::PeekLeft => "peek_left",
            PoseState::PeekRight => "peek_right",
            PoseState::Camera => "camera",
            PoseState::Monitor => "monitor",
            PoseState::Glass => "glass",
            PoseState::Vent => "vent",
            PoseState::Approach => "approach",
            PoseState::Attack => "attack",
            PoseState::Retreat => "retreat",
            PoseState::LightFlinch => "light_flinch",
            PoseState::Glitch => "glitch",
            PoseState::Kneel => "kneel",
            PoseState::Tired => "tired",
            PoseState::PointOffice => "point_office",
            PoseState::Greet => "greet",
            PoseState::Listening => "listening",
            PoseState::Reaching => "reaching",
            PoseState::Waiting => "waiting",
        })
    }

    /// Every state except `Rig`, in the order of poses.json.
    pub const ALL: [PoseState; 22] = [
        PoseState::Idle,
        PoseState::Doorway,
        PoseState::AtDoor,
        PoseState::DoorGrace,
        PoseState::PeekLeft,
        PoseState::PeekRight,
        PoseState::Camera,
        PoseState::Monitor,
        PoseState::Glass,
        PoseState::Vent,
        PoseState::Approach,
        PoseState::Attack,
        PoseState::Retreat,
        PoseState::LightFlinch,
        PoseState::Glitch,
        PoseState::Kneel,
        PoseState::Tired,
        PoseState::PointOffice,
        PoseState::Greet,
        PoseState::Listening,
        PoseState::Reaching,
        PoseState::Waiting,
    ];
}

/// poses.json "states" (the shared table).
pub const STATE_TABLE: [(&str, &str); 22] = [
    ("idle", "door_lurk"),
    ("doorway", "door_lurk"),
    ("at_door", "door_pound"),
    ("door_grace", "door_lurk"),
    ("peek_left", "peek_left"),
    ("peek_right", "peek_right"),
    ("camera", "stare_camera"),
    ("monitor", "stare_camera"),
    ("glass", "press_glass"),
    ("vent", "vent_crawl"),
    ("approach", "creep_walk"),
    ("attack", "sprint_lunge"),
    ("retreat", "retreat"),
    ("light_flinch", "flinch"),
    ("glitch", "twitch"),
    ("kneel", "kneel"),
    ("tired", "slump"),
    ("point_office", "point_office"),
    ("greet", "wave"),
    ("listening", "listen"),
    ("reaching", "reach_out"),
    ("waiting", "lean_wall"),
];

/// poses.json "hunter_states": (hunter, state key, pose). Only Verrine has her own vent poses.
pub const HUNTER_STATE_TABLE: [(&str, &str, &str); 3] = [
    ("verrine", "vent", "vent_crawl_verrine"),
    ("verrine", "vent_enter", "vent_drop_verrine"),
    ("verrine", "vent_exit", "vent_rise_verrine"),
];

/// The pose a state key maps to for this hunter ("vent_enter" and "vent_exit" exist for Verrine only).
pub fn pose_for_key(key: &str, hunter: &str) -> Option<&'static str> {
    for (h, k, p) in HUNTER_STATE_TABLE {
        if h == hunter && k == key {
            return Some(p);
        }
    }
    STATE_TABLE.iter().find(|(k, _)| *k == key).map(|(_, p)| *p)
}

/// The pose a Night Watch hunter state maps to (None for `Rig`).
pub fn pose_for_state(s: PoseState, hunter: &str) -> Option<&'static str> {
    pose_for_key(s.key()?, hunter)
}

// ------------------------------------------------------------------------------------------- vent crawl

pub const VENT_ENTER_S: f32 = 0.8;
pub const VENT_EXIT_S: f32 = 0.8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    None,
    Enter,
    Crawl,
    Exit,
}

/// What `VentPose::update` asks the actor to do this frame.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct VentStep {
    /// A vent pose owns the model: the caller skips its walk / idle clip.
    pub owns: bool,
    /// Play this state key (`"vent_enter"`, `"vent"`, `"vent_exit"`) at this speed scale.
    pub play: Option<(&'static str, f32)>,
    /// Stop the Night Watch pose (back to the rig's own clips).
    pub stop: bool,
}

/// NightWatchVentPose for one hunter. `has_pose(state_key)` says whether the hunter has a pose for the key (Godot:
/// `play_state` returning true).
#[derive(Clone, Debug)]
pub struct VentPose {
    phase: Phase,
    left: f32,
    speed: f32,
}

impl Default for VentPose {
    fn default() -> Self {
        VentPose::new()
    }
}

impl VentPose {
    pub fn new() -> VentPose {
        VentPose { phase: Phase::None, left: 0.0, speed: 1.0 }
    }

    /// The state key of the current phase ("" when none).
    pub fn state_of(&self) -> &'static str {
        match self.phase {
            Phase::Enter => "vent_enter",
            Phase::Crawl => "vent",
            Phase::Exit => "vent_exit",
            Phase::None => "",
        }
    }

    pub fn active(&self) -> bool {
        self.phase != Phase::None
    }

    fn start_crawl(&mut self, speed: f32) -> VentStep {
        self.phase = Phase::Crawl;
        self.speed = speed;
        VentStep { owns: true, play: Some(("vent", speed)), stop: false }
    }

    pub fn update(&mut self, in_shaft: bool, moving: bool, delta: f32, has_pose: &dyn Fn(&str) -> bool) -> VentStep {
        let speed = if moving { 1.0 } else { 0.35 };
        if in_shaft {
            if self.phase == Phase::None || self.phase == Phase::Exit {
                if has_pose("vent_enter") {
                    self.phase = Phase::Enter;
                    self.left = VENT_ENTER_S;
                    return VentStep { owns: true, play: Some(("vent_enter", 1.0)), stop: false };
                }
                return self.start_crawl(speed);
            }
            if self.phase == Phase::Enter {
                self.left -= delta;
                if self.left <= 0.0 {
                    return self.start_crawl(speed);
                }
                return VentStep { owns: true, play: None, stop: false };
            }
            if (self.speed - speed).abs() > 0.01 {
                return self.start_crawl(speed);
            }
            return VentStep { owns: true, play: None, stop: false };
        }
        if self.phase == Phase::Enter || self.phase == Phase::Crawl {
            if has_pose("vent_exit") {
                self.phase = Phase::Exit;
                self.left = VENT_EXIT_S;
                return VentStep { owns: true, play: Some(("vent_exit", 1.0)), stop: false };
            }
            self.phase = Phase::None;
            return VentStep { owns: false, play: None, stop: true };
        }
        if self.phase == Phase::Exit {
            self.left -= delta;
            if self.left <= 0.0 {
                self.phase = Phase::None;
                return VentStep { owns: false, play: None, stop: true };
            }
            return VentStep { owns: true, play: None, stop: false };
        }
        VentStep::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_is_the_sprint_lunge_and_rig_has_no_pose() {
        assert_eq!(pose_for_state(PoseState::Attack, "dovelle"), Some("sprint_lunge"));
        assert_eq!(pose_for_state(PoseState::Idle, "aki"), Some("door_lurk"));
        assert_eq!(pose_for_state(PoseState::AtDoor, "aki"), Some("door_pound"));
        assert_eq!(pose_for_state(PoseState::LightFlinch, "aki"), Some("flinch"));
        assert_eq!(pose_for_state(PoseState::Rig, "aki"), None);
    }

    #[test]
    fn every_state_has_a_pose_and_a_key() {
        for s in PoseState::ALL {
            let k = s.key().expect("key");
            assert!(STATE_TABLE.iter().any(|(sk, _)| *sk == k), "{k}");
            assert!(pose_for_state(s, "dovelle").is_some(), "{k}");
        }
    }

    #[test]
    fn verrine_has_her_own_vent_poses_others_do_not() {
        assert_eq!(pose_for_state(PoseState::Vent, "verrine"), Some("vent_crawl_verrine"));
        assert_eq!(pose_for_key("vent_enter", "verrine"), Some("vent_drop_verrine"));
        assert_eq!(pose_for_key("vent_exit", "verrine"), Some("vent_rise_verrine"));
        assert_eq!(pose_for_state(PoseState::Vent, "dovelle"), Some("vent_crawl"));
        assert_eq!(pose_for_key("vent_enter", "dovelle"), None);
    }

    #[test]
    fn states_match_the_poses_json() {
        let j: serde_json::Value = serde_json::from_str(crate::assets::POSES_JSON).expect("poses json");
        let states = j["states"].as_object().expect("states");
        assert_eq!(states.len(), STATE_TABLE.len());
        for (k, p) in STATE_TABLE {
            assert_eq!(states[k].as_str(), Some(p), "{k}");
            assert!(j["poses"][p].is_object(), "pose {p} exists");
        }
        let own = j["hunter_states"]["verrine"].as_object().expect("verrine");
        for (h, k, p) in HUNTER_STATE_TABLE {
            assert_eq!(h, "verrine");
            assert_eq!(own[k].as_str(), Some(p));
        }
    }

    #[test]
    fn vent_pose_without_drop_pose_crawls_at_once_and_stops_on_exit() {
        let none = |_: &str| false;
        let mut v = VentPose::new();
        let s = v.update(true, true, 0.016, &none);
        assert!(s.owns);
        assert_eq!(s.play, Some(("vent", 1.0)));
        assert_eq!(v.state_of(), "vent");
        // standing still slows the crawl
        let s = v.update(true, false, 0.016, &none);
        assert_eq!(s.play, Some(("vent", 0.35)));
        let s = v.update(false, false, 0.016, &none);
        assert!(!s.owns);
        assert!(s.stop);
        assert!(!v.active());
    }

    #[test]
    fn verrine_vent_pose_drops_crawls_and_rises() {
        let has = |k: &str| k == "vent_enter" || k == "vent_exit";
        let mut v = VentPose::new();
        let s = v.update(true, true, 0.1, &has);
        assert_eq!(s.play, Some(("vent_enter", 1.0)));
        assert_eq!(v.state_of(), "vent_enter");
        let mut crawled = false;
        for _ in 0..12 {
            let s = v.update(true, true, 0.1, &has);
            assert!(s.owns);
            if s.play == Some(("vent", 1.0)) {
                crawled = true;
                break;
            }
        }
        assert!(crawled);
        let s = v.update(false, true, 0.1, &has);
        assert_eq!(s.play, Some(("vent_exit", 1.0)));
        assert!(s.owns);
        let mut done = false;
        for _ in 0..12 {
            let s = v.update(false, true, 0.1, &has);
            if s.stop {
                done = true;
                assert!(!s.owns);
                break;
            }
        }
        assert!(done);
        assert!(!v.active());
    }
}
