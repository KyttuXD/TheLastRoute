#![allow(dead_code)]
//! Masked Hours' prop models: the Blockbench props of `last_route/assets/props/` that fit the wing (arcade cabinets, the
//! prize counter, party tables, crates, a bench, a toy chest, speakers, the office chair, a clock, ceiling lights). They are
//! read-only inputs (`include_str!`). The list is pure data (`specs()`), so tests can check it; `load()` builds one static
//! mesh per placement and needs the GL context (the atlas texture).
//!
//! Orientation: a model faces -Z at yaw 0 (the hunters' convention); `yaw` turns it like `Actor::yaw`. Models are placed by
//! the centre of their footprint, standing on `y` (or hanging from it when `hang`).

use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

/// One placed prop.
#[derive(Clone, Debug, PartialEq)]
pub struct PropSpec {
    pub file: &'static str,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// Uniform scale over Blockbench's 16 units per metre.
    pub scale: f32,
    /// Hang from `y` (ceiling lights) instead of standing on it.
    pub hang: bool,
}

fn stand(file: &'static str, x: f32, y: f32, z: f32, yaw: f32, scale: f32) -> PropSpec {
    PropSpec { file, x, y, z, yaw, scale, hang: false }
}

fn hang(file: &'static str, x: f32, y: f32, z: f32, scale: f32) -> PropSpec {
    PropSpec { file, x, y, z, yaw: 0.0, scale, hang: true }
}

/// The embedded text of a prop model by file stem.
pub fn text(file: &str) -> Option<&'static str> {
    Some(match file {
        "arcadeCabinet" => include_str!("../../../last_route/assets/props/arcadeCabinet.bbmodel"),
        "prizeCounter" => include_str!("../../../last_route/assets/props/prizeCounter.bbmodel"),
        "restaurantPartyTable" => include_str!("../../../last_route/assets/props/restaurantPartyTable.bbmodel"),
        "supplyCrates" => include_str!("../../../last_route/assets/props/supplyCrates.bbmodel"),
        "labBench" => include_str!("../../../last_route/assets/props/labBench.bbmodel"),
        "toyChest" => include_str!("../../../last_route/assets/props/toyChest.bbmodel"),
        "stageSpeaker" => include_str!("../../../last_route/assets/props/stageSpeaker.bbmodel"),
        "officeChair" => include_str!("../../../last_route/assets/props/officeChair.bbmodel"),
        "bedroomWallClock" => include_str!("../../../last_route/assets/props/bedroomWallClock.bbmodel"),
        "ceilingLight" => include_str!("../../../last_route/assets/props/ceilingLight.bbmodel"),
        _ => return None,
    })
}

/// Every placement, in world coordinates (the office is at the origin).
pub fn specs() -> Vec<PropSpec> {
    let mut v = Vec::new();
    // arcade cabinets along the game area's south wall, facing into the room (north)
    for x in [-6.5f32, -4.5, 4.5, 6.5] {
        v.push(stand("arcadeCabinet", x, 0.0, -18.8, 0.0, 1.05));
    }
    // the prize counter along the prize corner's west wall, facing east into the room
    for z in [-27.0f32, -25.85, -24.7, -23.55] {
        v.push(stand("prizeCounter", -17.0, 0.0, z, -FRAC_PI_2, 1.0));
    }
    // party tables in the four party rooms off the hall
    for side in [-1.0f32, 1.0] {
        for z in [-12.8f32, -7.2] {
            v.push(stand("restaurantPartyTable", side * 5.2, 0.0, z, 0.0, 0.7));
        }
    }
    // speakers on the show stage platform, facing the audience
    for x in [-5.2f32, 5.2] {
        v.push(stand("stageSpeaker", x, 0.7, -36.4, PI, 1.0));
    }
    // the vent hub's crates (the hunters' nest)
    for x in [-9.0f32, -3.0, 4.0, 10.0] {
        v.push(stand("supplyCrates", x, 0.0, 8.7, 0.0, 1.2));
    }
    // the parts room: a repair bench facing the room, a chest for the limbs
    v.push(stand("labBench", 14.0, 0.0, -29.2, PI, 1.0));
    v.push(stand("toyChest", 13.0, 0.0, -21.2, PI, 1.1));
    // the office: the chair behind the desk and the clock on the back wall
    v.push(stand("officeChair", 0.0, 0.0, 0.7, 0.0, 1.2));
    v.push(stand("bedroomWallClock", 2.5, 2.1, 3.5, 0.0, 0.8));
    // ceiling lights: down the main hall and over the show floor
    for i in 0..5 {
        v.push(hang("ceilingLight", 0.0, 3.6, -3.0 - i as f32 * 2.9, 1.2));
    }
    for (x, z) in [(-8.0f32, -21.0f32), (8.0, -21.0), (0.0, -26.0), (-8.0, -29.0), (8.0, -29.0)] {
        v.push(hang("ceilingLight", x, 3.6, z, 1.6));
    }
    v
}

/// Build one static, lit mesh per placement. Needs the GL context (every model decodes its atlas). A model that does not
/// parse is skipped.
pub fn load() -> Vec<Mesh> {
    let mut models: HashMap<&'static str, Option<Model>> = HashMap::new();
    let mut out = Vec::new();
    for s in specs() {
        if !models.contains_key(s.file) {
            let m = text(s.file).and_then(|t| Model::parse(t, None, &|_| None, None));
            models.insert(s.file, m);
        }
        let model = match models.get(s.file) {
            Some(Some(m)) => m,
            _ => continue,
        };
        let sc = model.scale * s.scale;
        let centre = vec3((model.min.x + model.max.x) * 0.5, if s.hang { model.max.y } else { model.min.y }, (model.min.z + model.max.z) * 0.5);
        let rot = Mat3::from_rotation_y(s.yaw);
        let pos = vec3(s.x, s.y, s.z) - rot * (centre * sc);
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        model.build_scaled(&mut vertices, &mut indices, pos, s.yaw, Vec3::splat(sc), 0.0, 0.0, WHITE);
        if vertices.is_empty() || vertices.len() > 60_000 {
            continue;
        }
        out.push(Mesh { vertices, indices, texture: Some(model.texture.clone()) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placement_names_an_embedded_model_that_is_json() {
        let list = specs();
        assert!(list.len() >= 30, "the wing is dressed: {} props", list.len());
        let mut seen: Vec<&str> = Vec::new();
        for s in &list {
            assert!(s.scale > 0.1 && s.scale < 4.0, "{}", s.file);
            assert!(s.x.is_finite() && s.y.is_finite() && s.z.is_finite() && s.yaw.is_finite());
            if !seen.contains(&s.file) {
                seen.push(s.file);
                let t = text(s.file).unwrap_or_else(|| panic!("{} is not embedded", s.file));
                let j: serde_json::Value = serde_json::from_str(t).unwrap_or_else(|e| panic!("{}: {e}", s.file));
                assert!(j["elements"].is_array(), "{}", s.file);
            }
        }
        assert_eq!(text("nope"), None);
    }

    #[test]
    fn props_stand_inside_the_rooms_they_dress() {
        for s in specs() {
            // everything is inside the wing's bounds: show floor x -19..19, z -40..-17; halls and hub to z 10; the office
            assert!(s.x.abs() <= 19.0 && s.z >= -40.0 && s.z <= 10.0, "{} at {},{}", s.file, s.x, s.z);
            if s.hang {
                assert!((s.y - 3.6).abs() < 1e-6);
            }
        }
    }
}
