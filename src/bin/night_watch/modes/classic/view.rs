//! Classic's presentation numbers and texts (data/classic/presentation.json) and the pure choices the scene makes from them:
//! the end cards, the hunter pose state, the hint line. No drawing, no GL.
#![allow(dead_code)]

use super::stalker::{door_index, DoorPose, Stalker};
use crate::core::{EndCard, NIGHTS};
use crate::hunters::PoseState;
use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;

pub const VIEW_JSON: &str = include_str!("../../data/classic/presentation.json");

#[derive(Clone, Debug)]
pub struct View {
    /// Seconds of the flinch pose after a hunter flinches from the light, and of the retreat pose after a shut door or the
    /// light turned her away.
    pub flinch_s: f32,
    pub retreat_s: f32,
    /// Seconds a static flash lasts.
    pub static_s: f32,
    pub note_s: f32,
    pub stare_note_s: f32,
    /// A stare pulse of strength 1 keeps this many seconds of static alive (the static alpha is left / static_s * 0.9).
    pub stare_static_scale: f32,
    pub win_clock: String,
    pub win_text: String,
    pub win_more: String,
    pub win_last: String,
    pub win_title: String,
    pub caught_title: String,
    pub caught_body: String,
    pub power_out_call: String,
    pub meet_met: String,
    pub meet_miss: String,
    pub meet_locked: String,
    pub hum_pan: f32,
    pub hum_volume: f32,
    pub hum_muffled_volume: f32,
    pub nyxie_door: String,
    pub nyxie_to: Vec3,
    /// Walkable floor of the office (min x, min z, max x, max z).
    pub footprint: (f32, f32, f32, f32),
    pub pitch: f32,
    pub seat: Vec3,
    pub hint_actions: Vec<String>,
    /// Hunters further than this on -z are in the building, not in the office halls.
    pub office_view_z: f32,
}

impl Default for View {
    fn default() -> View {
        View {
            flinch_s: 0.7,
            retreat_s: 1.4,
            static_s: 0.14,
            note_s: 2.5,
            stare_note_s: 3.5,
            stare_static_scale: 0.14,
            win_clock: "6:00 AM".to_string(),
            win_text: "You held the desk until 6 AM.".to_string(),
            win_more: "Night {next} is open.".to_string(),
            win_last: "You cleared every night.".to_string(),
            win_title: "NIGHT {n} COMPLETE".to_string(),
            caught_title: "CAUGHT".to_string(),
            caught_body: "{name} reached you on night {n}.".to_string(),
            power_out_call: "The power is out.".to_string(),
            meet_met: "Her eyes meet yours. She is gone.".to_string(),
            meet_miss: "Nothing there.".to_string(),
            meet_locked: "Wait.".to_string(),
            hum_pan: 0.8,
            hum_volume: 0.9,
            hum_muffled_volume: 0.35,
            nyxie_door: "doorL".to_string(),
            nyxie_to: vec3(-0.7, 0.0, -0.9),
            footprint: (-2.5, -2.0, 2.5, 2.0),
            pitch: -0.1,
            seat: vec3(0.0, 0.0, -0.6),
            hint_actions: ["nw_door_left", "nw_door_right", "nw_light_left", "nw_light_right", "nw_lamp", "nw_monitor"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            office_view_z: -12.0,
        }
    }
}

fn f(v: &Value, key: &str, d: f32) -> f32 {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d)
}

fn s(v: &Value, key: &str, d: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(d).to_string()
}

fn arr3(v: &Value, key: &str, d: Vec3) -> Vec3 {
    match v.get(key).and_then(|x| x.as_array()) {
        Some(a) if a.len() >= 3 => vec3(
            a[0].as_f64().unwrap_or(d.x as f64) as f32,
            a[1].as_f64().unwrap_or(d.y as f64) as f32,
            a[2].as_f64().unwrap_or(d.z as f64) as f32,
        ),
        _ => d,
    }
}

impl View {
    pub fn parse(src: &str) -> Result<View, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("classic presentation json: {e}"))?;
        let d = View::default();
        let notes = v.get("meet_notes").cloned().unwrap_or(Value::Null);
        let hum = v.get("hum").cloned().unwrap_or(Value::Null);
        let nyx = v.get("nyxie").cloned().unwrap_or(Value::Null);
        let player = v.get("player").cloned().unwrap_or(Value::Null);
        let footprint = match player.get("footprint").and_then(|x| x.as_array()) {
            Some(a) if a.len() >= 4 => {
                let g = |i: usize, dv: f32| a[i].as_f64().map(|x| x as f32).unwrap_or(dv);
                (g(0, d.footprint.0), g(1, d.footprint.1), g(2, d.footprint.2), g(3, d.footprint.3))
            }
            _ => d.footprint,
        };
        let hint_actions: Vec<String> = match v.get("hint_actions").and_then(|x| x.as_array()) {
            Some(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
            None => d.hint_actions.clone(),
        };
        Ok(View {
            flinch_s: f(&v, "flinch_s", d.flinch_s),
            retreat_s: f(&v, "retreat_s", d.retreat_s),
            static_s: f(&v, "static_s", d.static_s),
            note_s: f(&v, "note_s", d.note_s),
            stare_note_s: f(&v, "stare_note_s", d.stare_note_s),
            stare_static_scale: f(&v, "stare_static_scale", d.stare_static_scale),
            win_clock: s(&v, "win_clock", &d.win_clock),
            win_text: s(&v, "win_text", &d.win_text),
            win_more: s(&v, "win_more", &d.win_more),
            win_last: s(&v, "win_last", &d.win_last),
            win_title: s(&v, "win_title", &d.win_title),
            caught_title: s(&v, "caught_title", &d.caught_title),
            caught_body: s(&v, "caught_body", &d.caught_body),
            power_out_call: s(&v, "power_out_call", &d.power_out_call),
            meet_met: s(&notes, "met", &d.meet_met),
            meet_miss: s(&notes, "miss", &d.meet_miss),
            meet_locked: s(&notes, "locked", &d.meet_locked),
            hum_pan: f(&hum, "pan", d.hum_pan),
            hum_volume: f(&hum, "volume", d.hum_volume),
            hum_muffled_volume: f(&hum, "muffled_volume", d.hum_muffled_volume),
            nyxie_door: s(&nyx, "door", &d.nyxie_door),
            nyxie_to: arr3(&nyx, "to", d.nyxie_to),
            footprint,
            pitch: f(&player, "pitch", d.pitch),
            seat: arr3(&player, "seat", d.seat),
            hint_actions,
            office_view_z: f(&v, "office_view_z", d.office_view_z),
        })
    }

    /// The embedded file; the shipped defaults when it fails to parse (`--check` reports that through `parse`).
    pub fn embedded() -> View {
        View::parse(VIEW_JSON).unwrap_or_default()
    }

    /// The card of a won night: "6:00 AM", the win line and what opens next.
    pub fn win_card(&self, night: i32, is_custom: bool) -> EndCard {
        let more = has_next_level(night, is_custom);
        let tail = if more { self.win_more.replace("{next}", &(night + 1).to_string()) } else { self.win_last.clone() };
        EndCard {
            title: self.win_title.replace("{n}", &night.to_string()),
            body: format!("{}\n{}\n{}", self.win_clock, self.win_text, tail),
            can_next: more,
        }
    }

    /// The card of a lost night.
    pub fn caught_card(&self, by: &str, night: i32) -> EndCard {
        EndCard {
            title: self.caught_title.clone(),
            body: self.caught_body.replace("{name}", &crate::core::display_name(by)).replace("{n}", &night.to_string()),
            can_next: false,
        }
    }
}

/// Classic's nights run as far as `NIGHTS`; a Custom Night has no next.
pub fn has_next_level(night: i32, is_custom: bool) -> bool {
    !is_custom && night < NIGHTS
}

/// GDScript `String.capitalize()` for the plain words the menu uses ("NIGHT 1" -> "Night 1").
pub fn capitalize_words(text: &str) -> String {
    text.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The status line under the power bar: "1. Classic - Night 3".
pub fn mode_text(num: i32, title: &str, level_name: &str, night: i32) -> String {
    let level = if level_name.is_empty() { format!("Night {night}") } else { capitalize_words(level_name) };
    format!("{}. {} \u{b7} {}", num, capitalize_words(title), level)
}

// ---- the pose of a hunter ------------------------------------------------------------------------------------------------

/// Who owns a hunter's pose this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    /// The sim shows its own pose for her (door, flinch, retreat, attack).
    Sim(PoseState),
    /// The rig's own walk / sprint / idle; the idle behaviour may show a pose of its own while she stands.
    Loco,
    /// Standing in a camera room: the Godot wait pose, unless the idle behaviour has a pose for her.
    Wait(PoseState),
}

pub struct PoseIn<'a> {
    /// The sim's hunter (None: not on the floor).
    pub hunter: Option<&'a Stalker>,
    pub shown: bool,
    pub id: &'a str,
    pub door_shut: [bool; 2],
    pub monitor_up: bool,
    pub monitor_room: &'a str,
    pub attacker: &'a str,
    pub lost: bool,
    pub flinching: bool,
    pub retreating: bool,
    pub moving: bool,
}

/// `_hunter_pose_state` of the GDScript scene, with the idle behaviour left room to act when she merely waits.
pub fn pose_of(p: &PoseIn) -> Pose {
    let hunter = match p.hunter {
        Some(h) if p.shown => h,
        _ => return Pose::Loco,
    };
    if p.id == p.attacker && p.lost {
        return Pose::Sim(PoseState::Attack);
    }
    if p.flinching {
        return Pose::Sim(PoseState::LightFlinch);
    }
    if p.retreating {
        return Pose::Sim(PoseState::Retreat);
    }
    if p.moving {
        return Pose::Loco;
    }
    if hunter.is_at_door() {
        if p.id == "kathy" {
            return Pose::Sim(PoseState::Waiting);
        }
        let d = door_index(hunter.door()).unwrap_or(0);
        if p.door_shut[d] {
            return Pose::Sim(PoseState::AtDoor); // blocked by the shut door: pounding on it
        }
        return Pose::Sim(match hunter.door_pose() {
            DoorPose::Peek => {
                if d == 0 {
                    PoseState::PeekLeft
                } else {
                    PoseState::PeekRight
                }
            }
            DoorPose::Stand => PoseState::DoorGrace,
            DoorPose::Lean => PoseState::Reaching, // leaning in at the end of the wait
        });
    }
    if p.monitor_up && p.monitor_room == hunter.room {
        return Pose::Wait(PoseState::Camera);
    }
    Pose::Wait(if p.id == "kathy" { PoseState::Listening } else { PoseState::Idle })
}

/// Nyxie's walk in the dark: where she stands `t` (0..1) of the way from the left door to the desk.
pub fn nyxie_at(view: &View, door_stand: Vec3, t: f32) -> Vec3 {
    door_stand.lerp(view.nyxie_to, t.clamp(0.0, 1.0))
}

/// Why this mode cannot start, if so: Classic cannot start: the view or the props file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    View::parse(VIEW_JSON).err().or_else(super::world::load_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{cast, CastDef};

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(View::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn def(id: &str) -> &'static CastDef {
        cast().iter().find(|c| c.id == id).expect("cast")
    }

    fn inp<'a>(h: Option<&'a Stalker>) -> PoseIn<'a> {
        PoseIn {
            hunter: h,
            shown: true,
            id: h.map(|h| h.id.as_str()).unwrap_or(""),
            door_shut: [false, false],
            monitor_up: false,
            monitor_room: "",
            attacker: "",
            lost: false,
            flinching: false,
            retreating: false,
            moving: false,
        }
    }

    #[test]
    fn the_embedded_presentation_file_parses_to_the_godot_numbers() {
        let v = View::parse(VIEW_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.flinch_s, 0.7);
        assert_eq!(v.retreat_s, 1.4);
        assert_eq!(v.static_s, 0.14);
        assert_eq!(v.footprint, (-2.5, -2.0, 2.5, 2.0));
        assert_eq!(v.nyxie_to, vec3(-0.7, 0.0, -0.9));
        assert_eq!(v.seat, vec3(0.0, 0.0, -0.6));
        assert_eq!(v.hint_actions.len(), 6);
        assert_eq!(v.power_out_call, "The power is out.");
        assert_eq!((v.meet_met.as_str(), v.meet_miss.as_str(), v.meet_locked.as_str()), ("Her eyes meet yours. She is gone.", "Nothing there.", "Wait."));
        assert_eq!(v.hum_muffled_volume, 0.35);
        let d = View::default();
        assert_eq!(d.flinch_s, v.flinch_s);
        assert_eq!(d.win_text, v.win_text);
    }

    #[test]
    fn a_bad_file_falls_back_to_the_defaults() {
        assert!(View::parse("{").is_err());
        let v = View::parse("{}").unwrap_or_default();
        assert_eq!(v.retreat_s, 1.4);
    }

    #[test]
    fn the_win_card_names_the_next_night_or_the_end() {
        let v = View::embedded();
        let c = v.win_card(3, false);
        assert_eq!(c.title, "NIGHT 3 COMPLETE");
        assert_eq!(c.body, "6:00 AM\nYou held the desk until 6 AM.\nNight 4 is open.");
        assert!(c.can_next);
        let last = v.win_card(7, false);
        assert_eq!(last.body, "6:00 AM\nYou held the desk until 6 AM.\nYou cleared every night.");
        assert!(!last.can_next);
        let custom = v.win_card(2, true);
        assert!(!custom.can_next);
        assert!(custom.body.ends_with("You cleared every night."));
    }

    #[test]
    fn the_caught_card_names_the_hunter() {
        let v = View::embedded();
        let c = v.caught_card("dovelle", 2);
        assert_eq!((c.title.as_str(), c.body.as_str(), c.can_next), ("CAUGHT", "Dovelle reached you on night 2.", false));
        assert_eq!(v.caught_card("nyxie", 5).body, "Nyxie reached you on night 5.");
        assert_eq!(v.caught_card("kathy", 7).body, "Kathy reached you on night 7.");
    }

    #[test]
    fn the_status_line_follows_the_menu_data() {
        assert_eq!(mode_text(1, "CLASSIC", "NIGHT 3", 3), "1. Classic \u{b7} Night 3");
        assert_eq!(mode_text(1, "CLASSIC", "", 5), "1. Classic \u{b7} Night 5");
        assert_eq!(capitalize_words("DEAD AIR"), "Dead Air");
    }

    #[test]
    fn next_exists_only_for_the_first_six_nights_of_a_normal_run() {
        assert!(has_next_level(1, false) && has_next_level(6, false));
        assert!(!has_next_level(7, false));
        assert!(!has_next_level(1, true));
    }

    #[test]
    fn the_sim_owns_the_pose_at_a_door() {
        let mut h = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        h.admin_send("doorL");
        let mut i = inp(Some(&h));
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::PeekLeft));
        let mut r = Stalker::make(def("ravelle"), 5, 5200.0, 1.0, 3000.0);
        r.admin_send("doorR");
        assert_eq!(pose_of(&inp(Some(&r))), Pose::Sim(PoseState::PeekRight));
        h.at_door_ms = 2100.0;
        assert_eq!(pose_of(&inp(Some(&h))), Pose::Sim(PoseState::DoorGrace));
        h.at_door_ms = 4000.0;
        assert_eq!(pose_of(&inp(Some(&h))), Pose::Sim(PoseState::Reaching));
        i = inp(Some(&h));
        i.door_shut = [true, false];
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::AtDoor));
        let mut k = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        k.id = "kathy".to_string();
        k.admin_send("doorL");
        assert_eq!(pose_of(&inp(Some(&k))), Pose::Sim(PoseState::Waiting));
    }

    #[test]
    fn flinch_retreat_and_attack_beat_walking() {
        let h = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        let mut i = inp(Some(&h));
        i.moving = true;
        assert_eq!(pose_of(&i), Pose::Loco);
        i.retreating = true;
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::Retreat));
        i.flinching = true;
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::LightFlinch));
        i.attacker = "dovelle";
        i.lost = true;
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::Attack));
        // attacking needs the loss
        i.lost = false;
        assert_eq!(pose_of(&i), Pose::Sim(PoseState::LightFlinch));
    }

    #[test]
    fn a_hunter_in_a_camera_room_waits_or_stares_at_the_watched_feed() {
        let h = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        let mut i = inp(Some(&h));
        assert_eq!(pose_of(&i), Pose::Wait(PoseState::Idle));
        i.monitor_up = true;
        i.monitor_room = "stage";
        assert_eq!(pose_of(&i), Pose::Wait(PoseState::Camera));
        i.monitor_room = "dining";
        assert_eq!(pose_of(&i), Pose::Wait(PoseState::Idle));
        let mut k = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        k.id = "kathy".to_string();
        let mut ki = inp(Some(&k));
        ki.id = "kathy";
        assert_eq!(pose_of(&ki), Pose::Wait(PoseState::Listening));
        // a hunter who is not on the floor, or not shown, has no sim pose
        let mut hidden = inp(Some(&h));
        hidden.shown = false;
        assert_eq!(pose_of(&hidden), Pose::Loco);
        assert_eq!(pose_of(&inp(None)), Pose::Loco);
    }

    #[test]
    fn nyxie_creeps_from_the_left_door_to_the_desk() {
        let v = View::embedded();
        let from = vec3(-4.9, 0.0, 0.3);
        assert_eq!(nyxie_at(&v, from, 0.0), from);
        assert!((nyxie_at(&v, from, 1.0) - v.nyxie_to).length() < 1e-4);
        let mid = nyxie_at(&v, from, 0.5);
        assert!(mid.x > from.x && mid.x < v.nyxie_to.x);
        assert!((nyxie_at(&v, from, 5.0) - v.nyxie_to).length() < 1e-4);
    }
}
