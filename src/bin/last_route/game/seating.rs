//! Sit and lie on furniture (TODO "Sit, lie and ride on furniture"). Seat anchors per furniture kind are data
//! (`data/seating.json`, `lr.seating/1`); this file is the behaviour: pick the anchor, lock the pose, and put the body
//! back where it stood when the player gets up. Riding is parsed (`ride_enabled`) but stays off.
//!
//! Hooks (the integrate agent): register `pub mod seating;` in game/mod.rs (render/anim/emote_pose.rs reads its emote extras), hold a [`SeatState`] on the player, and in
//! interact.rs call [`Seating::kind`] for a prop kind name to offer E (the kind `label`, "SIT" or "LIE DOWN"),
//! then [`SeatState::sit`]. While [`SeatState::locked`] the move input is ignored; E, jump or crouch calls
//! [`SeatState::exit`] and teleports the body to the returned position.

use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.seating/1";
pub(super) const TEXT: &str = include_str!("../data/seating.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    Sit,
    Lie,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeatKind {
    pub pose: Pose,
    /// Metres along the furniture facing from its centre.
    pub offset: f32,
    /// Feet height above the floor while seated.
    pub height: f32,
    /// Metres in front of the furniture to stand up at when the old spot is gone.
    pub exit: f32,
    /// Prompt text (upper case).
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seating {
    pub ride_enabled: bool,
    pub default_reach: f32,
    pub kinds: Vec<(String, SeatKind)>,
}

/// The doorway pose, blocky face and emote coverage settings (`emote_extras`).
#[derive(Clone, Debug, PartialEq)]
pub struct EmoteExtras {
    pub doorway_pose: Value,
    pub doorway_idle_seconds: f32,
    pub doorway_half_width: f32,
    pub doorway_depth: f32,
    pub blocky_enabled: bool,
    pub blocky_while_emoting: bool,
    pub blocky_texture: String,
    pub required_emotes: Vec<String>,
}

fn f(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .map(|n| n as f32)
        .ok_or_else(|| format!("seating.json: '{key}' missing or not a number"))
}

fn s(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("seating.json: '{key}' missing or empty"))
}

pub fn parse(text: &str) -> Result<(Seating, EmoteExtras), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("seating.json: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("seating.json: 'schema' must be {SCHEMA}"));
    }
    let mut kinds = vec![];
    for (name, k) in v["kinds"].as_object().ok_or("seating.json: 'kinds' missing")? {
        let pose = match k["pose"].as_str() {
            Some("sit") => Pose::Sit,
            Some("lie") => Pose::Lie,
            _ => return Err(format!("seating.json: kind '{name}' pose must be sit or lie")),
        };
        let exit = f(k, "exit")?;
        if exit < 0.0 {
            return Err(format!("seating.json: kind '{name}' exit must not be negative"));
        }
        kinds.push((name.clone(), SeatKind { pose, offset: f(k, "offset")?, height: f(k, "height")?, exit, label: s(k, "label")? }));
    }
    if kinds.is_empty() {
        return Err("seating.json: no kinds".into());
    }
    let x = &v["emote_extras"];
    let blocky = &x["blocky_face"];
    let extras = EmoteExtras {
        doorway_pose: x["doorway_pose"].clone(),
        doorway_idle_seconds: f(x, "doorway_idle_seconds")?,
        doorway_half_width: f(x, "doorway_half_width")?,
        doorway_depth: f(x, "doorway_depth")?,
        blocky_enabled: blocky["enabled"].as_bool().unwrap_or(false),
        blocky_while_emoting: blocky["swap_while_emoting"].as_bool().unwrap_or(false),
        blocky_texture: s(blocky, "texture")?,
        required_emotes: x["required_emotes"]
            .as_array()
            .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
    };
    Ok((
        Seating { ride_enabled: v["ride_enabled"].as_bool().unwrap_or(false), default_reach: f(&v, "default_reach")?, kinds },
        extras,
    ))
}

fn loaded() -> &'static (Seating, EmoteExtras) {
    static C: OnceLock<(Seating, EmoteExtras)> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

pub fn seating() -> &'static Seating {
    &loaded().0
}

pub fn emote_extras() -> &'static EmoteExtras {
    &loaded().1
}

impl Seating {
    pub fn kind(&self, name: &str) -> Option<&SeatKind> {
        self.kinds.iter().find(|(n, _)| n == name).map(|(_, k)| k)
    }

    /// Whether a player `dist` metres from the furniture can use it.
    pub fn in_reach(&self, dist: f32) -> bool {
        dist.is_finite() && dist <= self.default_reach
    }
}

/// A body on furniture.
#[derive(Clone, Debug, PartialEq)]
pub struct Seated {
    pub kind: String,
    pub pose: Pose,
    /// Where the body stood before sitting: exit puts it back here.
    pub from: [f32; 3],
    /// Where the body is while seated.
    pub anchor: [f32; 3],
    /// The furniture facing, the body yaw lock.
    pub yaw: f32,
    pub furniture: (f32, f32),
    pub facing: (f32, f32),
    pub exit: f32,
    pub floor_y: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeatState {
    seated: Option<Seated>,
}

impl SeatState {
    pub fn seated(&self) -> Option<&Seated> {
        self.seated.as_ref()
    }

    /// True while the pose holds the body (move input is ignored).
    pub fn locked(&self) -> bool {
        self.seated.is_some()
    }

    /// Sits or lies on a `kind` of furniture centred at `centre` (grid x, z) facing `facing`, from `player` (x, y, z).
    /// None for an unknown kind, a player out of reach, or a body that is already seated.
    pub fn sit(&mut self, cfg: &Seating, kind: &str, centre: (f32, f32), facing: (f32, f32), player: [f32; 3], floor_y: f32) -> Option<&Seated> {
        if self.seated.is_some() {
            return None;
        }
        let k = cfg.kind(kind)?;
        let dist = ((player[0] - centre.0).powi(2) + (player[2] - centre.1).powi(2)).sqrt();
        if !cfg.in_reach(dist) {
            return None;
        }
        let n = (facing.0 * facing.0 + facing.1 * facing.1).sqrt().max(1e-6);
        let (fx, fz) = (facing.0 / n, facing.1 / n);
        self.seated = Some(Seated {
            kind: kind.to_string(),
            pose: k.pose,
            from: player,
            anchor: [centre.0 + fx * k.offset, floor_y + k.height, centre.1 + fz * k.offset],
            yaw: (-fx).atan2(-fz),
            furniture: centre,
            facing: (fx, fz),
            exit: k.exit,
            floor_y,
        });
        self.seated.as_ref()
    }

    /// Gets up: the position to put the body back at (where it stood). `blocked` says that spot is now unusable, then
    /// the body stands `exit` metres in front of the furniture instead.
    pub fn exit(&mut self, blocked: bool) -> Option<[f32; 3]> {
        let s = self.seated.take()?;
        if blocked {
            Some([s.furniture.0 + s.facing.0 * s.exit, s.floor_y, s.furniture.1 + s.facing.1 * s.exit])
        } else {
            Some(s.from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_data_parses_and_riding_is_off() {
        let (s, e) = parse(TEXT).expect("seating.json parses");
        assert!(!s.ride_enabled);
        assert_eq!(s.kind("bed").unwrap().pose, Pose::Lie);
        assert_eq!(s.kind("chair").unwrap().pose, Pose::Sit);
        assert!(s.kinds.iter().all(|(_, k)| k.label == k.label.to_uppercase()));
        assert!(!e.required_emotes.is_empty());
        assert_eq!(seating(), &s);
    }

    #[test]
    fn sit_and_exit_restores_the_position() {
        let cfg = seating();
        let mut st = SeatState::default();
        let from = [3.0, 0.0, 4.0];
        let seat = st.sit(cfg, "chair", (3.5, 4.0), (0.0, 1.0), from, 0.0).expect("sits").clone();
        assert!(st.locked());
        assert_eq!(seat.pose, Pose::Sit);
        assert!((seat.anchor[1] - cfg.kind("chair").unwrap().height).abs() < 1e-6);
        assert!(st.sit(cfg, "chair", (3.5, 4.0), (0.0, 1.0), from, 0.0).is_none(), "already seated");
        assert_eq!(st.exit(false), Some(from));
        assert!(!st.locked());
        assert_eq!(st.exit(false), None);
    }

    #[test]
    fn blocked_exit_stands_in_front_and_far_or_unknown_is_refused() {
        let cfg = seating();
        let mut st = SeatState::default();
        assert!(st.sit(cfg, "bed", (0.0, 0.0), (0.0, 1.0), [20.0, 0.0, 0.0], 0.0).is_none(), "out of reach");
        assert!(st.sit(cfg, "lava", (0.0, 0.0), (0.0, 1.0), [0.0, 0.0, 0.5], 0.0).is_none(), "unknown kind");
        st.sit(cfg, "bed", (0.0, 0.0), (0.0, 1.0), [0.0, 0.0, 0.5], 0.0).unwrap();
        let out = st.exit(true).unwrap();
        assert!((out[2] - cfg.kind("bed").unwrap().exit).abs() < 1e-5);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn changed_data_changes_behaviour() {
        let text = TEXT.replace("\"reach\"", "\"reach\"").replace("\"default_reach\": 1.6", "\"default_reach\": 0.2");
        let (cfg, _) = parse(&text).unwrap();
        assert!(!cfg.in_reach(1.0));
        assert!(seating().in_reach(1.0));
        assert!(parse("{}").is_err());
    }
}
