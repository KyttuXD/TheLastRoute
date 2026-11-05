//! The item a runner holds, as other runners see it: the optional `held` key of the net sample and the transform that
//! puts the item's model on the mannequin's right hand.
//!
//! Wire: `"held": "<id>"` with the stable Godot ids (`flashlight`, `taser`, `crowbar`, `rescue_axe`, `battery`,
//! `stim_pack`). The key is absent when nothing is held. An older peer never sends it and ignores it; an id this build
//! does not know (a newer peer's, or Godot's lockpick and revive kit) reads as no item, never as a bad sample.
//!
//! Pose: `data/render/remote_held.json` (Godot `HeldItemPoses.THIRD_PERSON` and `REMOTE_HELD_SCALES`). The item hangs on
//! the `R-Hand` bone, so it follows every clip. Swing replication is not done (the held item stays in its rest pose).

use crate::content::embedded;
use crate::game::Item;
use macroquad::prelude::*;
use std::sync::OnceLock;

const FILE: &str = "render/remote_held.json";

/// The wire id of a held item.
pub fn wire(item: Item) -> &'static str {
    match item {
        Item::Flashlight => "flashlight",
        Item::Taser => "taser",
        Item::Crowbar => "crowbar",
        Item::Axe => "rescue_axe",
        Item::Battery => "battery",
        Item::Stim => "stim_pack",
    }
}

/// The item a wire id names; None for an id this build does not know.
pub fn from_wire(id: &str) -> Option<Item> {
    [Item::Flashlight, Item::Taser, Item::Crowbar, Item::Axe, Item::Battery, Item::Stim].into_iter().find(|i| wire(*i) == id)
}

/// How one item sits in the hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPose {
    pub scale: f32,
    pub upright: bool,
    pub tilt_deg: f32,
    pub grip: f32,
}

/// Where the item's anchor sits on the hand bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// Model units below the bone's pivot (the hand's grip point).
    pub offset_units: Vec3,
    pub roll_deg: f32,
    /// Metres in the anchor's own frame.
    pub shift_m: Vec3,
    pub rig_scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeldData {
    pub bone: String,
    pub anchor: Anchor,
    items: Vec<(Item, ItemPose)>,
}

impl HeldData {
    pub fn parse(src: &str) -> Result<HeldData, String> {
        let v: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        let num = |o: &serde_json::Value, k: &str| o[k].as_f64().map(|n| n as f32).ok_or_else(|| format!("{k} is missing or not a number"));
        let vec3_of = |o: &serde_json::Value, k: &str| -> Result<Vec3, String> {
            let a = o[k].as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{k} is not a list of three numbers"))?;
            let n = |i: usize| a[i].as_f64().map(|n| n as f32).ok_or_else(|| format!("{k} holds a non-number"));
            Ok(vec3(n(0)?, n(1)?, n(2)?))
        };
        let a = &v["anchor"];
        let anchor = Anchor {
            offset_units: vec3_of(a, "offset_units").map_err(|e| format!("anchor.{e}"))?,
            roll_deg: num(a, "roll_deg").map_err(|e| format!("anchor.{e}"))?,
            shift_m: vec3_of(a, "shift_m").map_err(|e| format!("anchor.{e}"))?,
            rig_scale: num(a, "rig_scale").map_err(|e| format!("anchor.{e}"))?,
        };
        let bone = a["bone"].as_str().ok_or("anchor.bone is missing or not a string")?.to_string();
        let mut items = vec![];
        for item in [Item::Flashlight, Item::Taser, Item::Crowbar, Item::Axe, Item::Battery, Item::Stim] {
            let o = &v["items"][wire(item)];
            let e = |m: String| format!("items.{}: {m}", wire(item));
            items.push((
                item,
                ItemPose {
                    scale: num(o, "scale").map_err(e)?,
                    upright: o["upright"].as_bool().ok_or_else(|| e("upright is missing or not a boolean".into()))?,
                    tilt_deg: num(o, "tilt_deg").map_err(e)?,
                    grip: num(o, "grip").map_err(e)?,
                },
            ));
        }
        Ok(HeldData { bone, anchor, items })
    }

    pub fn pose(&self, item: Item) -> ItemPose {
        self.items.iter().find(|(i, _)| *i == item).map(|(_, p)| *p).expect("every item has a pose")
    }
}

/// The shipped pose table, read once. None when the file is bad (the error is printed once): no item is drawn then.
pub fn data() -> Option<&'static HeldData> {
    static DATA: OnceLock<Option<HeldData>> = OnceLock::new();
    DATA.get_or_init(|| {
        let (path, src) = embedded::data_under(FILE).next()?;
        match HeldData::parse(src) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("{path}: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// The item's model-to-world transform. `hand` is the hand bone's matrix in the mannequin's model space and `root` the
/// figure's root (feet, yaw, and the metres-per-unit scale), so `root * hand` maps the hand's local model units to the
/// world. The anchor is the grip point below the bone pivot, turned so the model's +Y points along the hand's forward
/// (-Z at rest, Godot's R-Attachment convention), rolled a little, then the item sits on it by its pose.
pub fn item_transform(d: &HeldData, item: Item, root: Mat4, hand: Mat4, unit: f32) -> Mat4 {
    let p = d.pose(item);
    let a = &d.anchor;
    // the figure's model units back to metres: the anchor and the item are authored in metres
    let metres = Mat4::from_scale(Vec3::splat(1.0 / unit.max(1e-6)));
    let anchor = Mat4::from_translation(a.offset_units)
        * Mat4::from_rotation_x(-90f32.to_radians())
        * Mat4::from_rotation_z(a.roll_deg.to_radians())
        * metres
        * Mat4::from_translation(a.shift_m);
    // the model's basis in the anchor: upright items stand handle forward and blade up, then tilt about X
    let basis = if p.upright {
        Mat4::from_rotation_x(p.tilt_deg.to_radians()) * Mat4::from_cols(Vec4::Y, Vec4::Z, Vec4::X, Vec4::W)
    } else {
        Mat4::IDENTITY
    } * Mat4::from_scale(Vec3::splat(p.scale * a.rig_scale));
    let grip = -basis.transform_vector3(vec3(0.0, p.grip, 0.0));
    root * hand * anchor * Mat4::from_translation(grip) * basis
}

/// Whether a runner's held item shows: not while it is in a locker, down, stunned-flat or on a ladder (both hands busy),
/// and not once its body has faded out.
pub fn shown(held: Option<Item>, hiding: bool, pose: crate::render::anim::runner::Pose, alpha: f32) -> Option<Item> {
    use crate::render::anim::runner::Pose;
    if hiding || alpha < 0.05 || matches!(pose, Pose::Down | Pose::Climb) {
        return None;
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::anim::runner::Pose;

    fn shipped() -> HeldData {
        HeldData::parse(embedded::data_under(FILE).next().expect("remote_held.json is embedded").1).expect("parses")
    }

    #[test]
    fn every_item_has_a_wire_id_that_reads_back() {
        for i in [Item::Flashlight, Item::Taser, Item::Crowbar, Item::Axe, Item::Battery, Item::Stim] {
            assert_eq!(from_wire(wire(i)), Some(i));
        }
        assert_eq!(from_wire("lockpick"), None, "a Godot-only item is no item");
        assert_eq!(from_wire(""), None);
    }

    #[test]
    fn the_shipped_table_parses_and_names_every_item() {
        let d = shipped();
        assert_eq!(d.bone, "R-Hand");
        assert_eq!(d.pose(Item::Axe).tilt_deg, -55.0);
        assert!(d.pose(Item::Crowbar).upright && !d.pose(Item::Flashlight).upright);
        assert!(HeldData::parse("{}").is_err());
        assert!(HeldData::parse("not json").is_err());
    }

    #[test]
    fn the_item_hangs_at_the_hand_and_points_forward() {
        let d = shipped();
        let unit = 1.067 / 64.0;
        // a hand at the model origin, the figure at the world origin facing -Z
        let xf = item_transform(&d, Item::Flashlight, Mat4::from_scale(Vec3::splat(unit)), Mat4::IDENTITY, unit);
        let pose = d.pose(Item::Flashlight);
        let axis = xf.transform_vector3(Vec3::Y).normalize();
        // the model's +Y is the hand's forward, rolled 8 degrees about it
        assert!(axis.z < -0.95, "the flashlight points forward, got {axis:?}");
        // the model origin sits within a forearm of the hand pivot
        assert!(xf.transform_point3(Vec3::ZERO).length() < 0.5);
        let len = xf.transform_vector3(Vec3::Y).length();
        assert!((len - pose.scale * d.anchor.rig_scale).abs() < 1e-4, "the item is drawn at its scale, got {len}");
        // a hand that turned with the figure carries the item with it
        let turned = item_transform(&d, Item::Flashlight, Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2) * Mat4::from_scale(Vec3::splat(unit)), Mat4::IDENTITY, unit);
        assert!(turned.transform_vector3(Vec3::Y).normalize().x < -0.95, "turning the figure turns the item");
    }

    #[test]
    fn an_upright_tool_stands_on_its_handle() {
        let d = shipped();
        let unit = 1.067 / 64.0;
        let xf = item_transform(&d, Item::Crowbar, Mat4::from_scale(Vec3::splat(unit)), Mat4::IDENTITY, unit);
        let handle = xf.transform_vector3(Vec3::Y).normalize();
        assert!(handle.z.abs() > 0.5, "the handle runs along the forward axis, tilted: {handle:?}");
    }

    #[test]
    fn the_item_is_hidden_while_hiding_down_climbing_or_faded() {
        let it = Some(Item::Taser);
        assert_eq!(shown(it, false, Pose::Idle, 1.0), it);
        assert_eq!(shown(it, true, Pose::Idle, 1.0), None);
        assert_eq!(shown(it, false, Pose::Down, 1.0), None);
        assert_eq!(shown(it, false, Pose::Climb, 1.0), None);
        assert_eq!(shown(it, false, Pose::Crouch, 0.0), None);
        assert_eq!(shown(it, false, Pose::Crouch, 0.45), it, "a crouch fade keeps it");
        assert_eq!(shown(None, false, Pose::Idle, 1.0), None);
    }
}
