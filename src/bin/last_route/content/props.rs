//! Prop definitions (`lr.prop/1`, data/props/*.json): each catalogue type's model variants
//! (Blockbench models) and its fixture light, as
//! PropModelCatalog.MODEL_PATHS / LIGHT_PROFILES resolve them.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Light {
    pub color: (f32, f32, f32),
    pub energy: f32,
    pub range: f32,
    pub offset: (f32, f32, f32),
    /// Optional wobble of the energy with time (render/lamp_flicker.rs): cycles a second and 0..1 swing either way.
    /// Zero for either is steady.
    pub flicker_hz: f32,
    pub flicker_depth: f32,
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub id: String,
    /// The model variants, as asset paths relative to assets/ ("props/maze/locker.bbmodel"); a room draws one of them
    /// (PropModelCatalog's variant pick). A mod pack's are "pack:<id>:<path>".
    pub models: Vec<String>,
    pub light: Option<Light>,
}

fn hex(s: &str) -> (f32, f32, f32) {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    (
        ((v >> 16) & 255) as f32 / 255.0,
        ((v >> 8) & 255) as f32 / 255.0,
        (v & 255) as f32 / 255.0,
    )
}

pub fn parse_file(v: &Value) -> Vec<PropDef> {
    let mut out = vec![];
    for p in v["props"].as_array().into_iter().flatten() {
        let Some(id) = p["id"].as_str() else { continue };
        let models = p["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str())
            .map(|m| m.trim_start_matches("res://assets/").to_string())
            .collect();
        let light = p.get("light").filter(|l| l.is_object()).map(|l| {
            let o = l["offset"].as_array();
            let g = |i: usize, d: f64| {
                o.and_then(|a| a.get(i))
                    .and_then(|x| x.as_f64())
                    .unwrap_or(d) as f32
            };
            Light {
                color: hex(l["color"].as_str().unwrap_or("ffffff")),
                energy: l["energy"].as_f64().unwrap_or(0.4) as f32,
                range: l["range"].as_f64().unwrap_or(3.5) as f32,
                offset: (g(0, 0.0), g(1, 0.5), g(2, 0.0)),
                flicker_hz: l["flicker_hz"].as_f64().unwrap_or(0.0).max(0.0) as f32,
                flicker_depth: l["flicker_depth"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32,
            }
        });
        out.push(PropDef {
            id: id.to_string(),
            models,
            light,
        });
    }
    out
}

/// PropModelCatalog._stable_index: a 31-hash of the key's bytes.
pub fn stable_index(value: &str, size: usize) -> usize {
    let mut r: i64 = 0;
    for b in value.bytes() {
        r = (r * 31 + b as i64) & 0x7fff_ffff;
    }
    (r as usize) % size.max(1)
}

/// CEILING_MODEL_TOP: authored tops seated against the ceiling.
pub fn ceiling_top(t: &str) -> Option<f32> {
    match t {
        "diningChandelier" => Some(0.90),
        "ceilingLight" => Some(0.30),
        "ceilingFan" => Some(0.59),
        "infernoChainLantern" => Some(1.59),
        "surgicalLight" => Some(0.90),
        "hangingVines" => Some(1.25),
        _ => None,
    }
}

pub const FLOOR_DRESSING: [&str; 14] = [
    "mansionAreaRug",
    "corridorCarpet",
    "hotelCarpet",
    "apartmentCarpet",
    "beachMat",
    "bedroomRug",
    "gothicBedroomRug",
    "houseBathMat",
    "arcadeCarpetCircle",
    "arcadeCarpetRunner",
    "frozenTidelinePool",
    "overgrowthPatch",
    "cableCoil",
    "cableRun",
];

pub const WALL_MOUNTED: [&str; 30] = [
    "normalPictureFrameEmpty",
    "normalPaintingFrameEmpty",
    "luxuryPictureFrameEmpty",
    "luxuryPaintingFrameEmpty",
    "gothicPictureFrameEmpty",
    "gothicPaintingFrameEmpty",
    "bathroomMirror",
    "framedWallPortrait",
    "bedroomWallClock",
    "publicHandDryer",
    "publicSoapDispenser",
    "publicPaperTowelDispenser",
    "publicBabyChangeStation",
    "publicMensRoomSign",
    "publicWomensRoomSign",
    "houseMedicineCabinet",
    "houseTowelRack",
    "houseBathroomShelf",
    "xrayLightbox",
    "hospitalWardSign",
    "monitorWall",
    "labWhiteboard",
    "diveHeadsetShelf",
    "hazardSign",
    "menuBoard",
    "donutSign",
    "cinemaScreen",
    "acousticPanel",
    "guitarWallHanger",
    "onAirSign",
];

/// PropModelCatalog.visual_y: where the model's origin sits (wall height 3.6).
pub fn visual_y(t: &str, wall_height: f32, mount_y: f32) -> f32 {
    if mount_y > 0.0 {
        mount_y
    } else if FLOOR_DRESSING.contains(&t) {
        0.012
    } else if WALL_MOUNTED.contains(&t) {
        wall_height * 0.42
    } else if let Some(top) = ceiling_top(t) {
        wall_height - top
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use crate::content::{embedded, Content};

    /// Every model a prop type names is a bbmodel the build embeds, and the game's interactables are all there: the game
    /// reads no other model format from its own assets.
    #[test]
    fn every_prop_model_is_an_embedded_bbmodel() {
        let content = Content::load();
        assert!(content.props.len() > 300, "{} prop types", content.props.len());
        for def in content.props.values() {
            assert!(!def.models.is_empty(), "{} names no model", def.id);
            for m in &def.models {
                assert!(m.ends_with(".bbmodel"), "{}: {m} is not a bbmodel", def.id);
                assert!(embedded::asset(m).is_some(), "{}: {m} is not embedded", def.id);
            }
        }
        for name in ["relay", "battery", "coin", "hiding_spot", "lift", "vent", "safe_zone", "music_box", "heater_station", "wall_light_switch", "flashlight_item", "taser_item", "crowbar", "rescue_axe", "stim_pack"] {
            assert!(embedded::asset(&format!("interactables/{name}.bbmodel")).is_some(), "{name}");
        }
        assert!(!embedded::assets_under("").any(|(p, _)| p.ends_with(".glb")), "a .glb is embedded");
    }

    /// The two models the game splits by outliner group name carry those groups, with the pivots door.rs and the
    /// locker draw expect.
    #[test]
    fn door_and_locker_models_carry_their_groups() {
        let group_names = |path: &str| -> Vec<String> {
            let j: serde_json::Value = serde_json::from_slice(embedded::asset(path).expect(path)).expect(path);
            j["outliner"]
                .as_array()
                .expect(path)
                .iter()
                .filter(|n| n.is_object())
                .map(|n| n["name"].as_str().unwrap_or("").to_string())
                .collect()
        };
        let single = group_names("props/mansion/normal_room_door.bbmodel");
        for g in ["DoorHinge", "DoorFrameL", "DoorFrameR", "DoorFrameTop"] {
            assert!(single.iter().any(|n| n == g), "normal_room_door lacks {g}");
        }
        assert!(group_names("props/mansion/normal_double_room_door.bbmodel").iter().any(|n| n == "DoorHingeRight"));
        assert!(group_names("interactables/hiding_spot.bbmodel").iter().any(|n| n == "LockerDoor"));
    }
}
