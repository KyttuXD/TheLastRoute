//! Extra dressing for the classic building: bbmodel props from `../last_route/assets/props/` placed in the eight camera rooms
//! (data/classic/props.json), drawn with the building in every camera feed. The rooms themselves, their walls, doorways,
//! vents, lockers, counters, tables and the camera mounts are `gfx::building`; the security room is `gfx::office`.
//!
//! `parse_props` and `prop_text` are pure. `Props::build` needs the GL context (textures).
#![allow(dead_code)]

use crate::gfx::building::Building;
use crate::gfx::{Gfx, LightSet};
use crate::hunters::bbmodel::Model;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

pub const PROPS_JSON: &str = include_str!("../../data/classic/props.json");

#[derive(Clone, Debug, PartialEq)]
pub struct PropDef {
    pub room: String,
    pub model: String,
    /// Local to the room centre: x, y (base of the prop), z.
    pub pos: Vec3,
    pub yaw_deg: f32,
    /// Multiplies the native 1/16 size.
    pub scale: f32,
}

impl PropDef {
    /// Where the prop's base centre stands in the world.
    pub fn world_pos(&self) -> Vec3 {
        Building::room_centre(&self.room) + self.pos
    }
}

/// The props file. An entry without a room or model is an error.
pub fn parse_props(src: &str) -> Result<Vec<PropDef>, String> {
    let root: Value = serde_json::from_str(src).map_err(|e| format!("classic props json: {e}"))?;
    let list = root.get("props").and_then(|p| p.as_array()).ok_or_else(|| "classic props json: no props".to_string())?;
    let mut out = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let room = p.get("room").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let model = p.get("model").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if room.is_empty() || model.is_empty() {
            return Err(format!("classic props json: entry {i} needs a room and a model"));
        }
        let pos = match p.get("pos").and_then(|x| x.as_array()) {
            Some(a) if a.len() >= 3 => vec3(
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
            ),
            _ => return Err(format!("classic props json: entry {i} ({model}) has no pos")),
        };
        out.push(PropDef {
            room,
            model,
            pos,
            yaw_deg: p.get("yaw").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
            scale: p.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32,
        });
    }
    Ok(out)
}

/// The embedded bbmodel of a prop (every name props.json uses).
pub fn prop_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "ampStack" => include_str!("../../../last_route/assets/props/ampStack.bbmodel"),
        "comboAmp" => include_str!("../../../last_route/assets/props/comboAmp.bbmodel"),
        "floorLamp" => include_str!("../../../last_route/assets/props/floorLamp.bbmodel"),
        "coffeeStation" => include_str!("../../../last_route/assets/props/coffeeStation.bbmodel"),
        "controlConsole" => include_str!("../../../last_route/assets/props/controlConsole.bbmodel"),
        "cableCoil" => include_str!("../../../last_route/assets/props/cableCoil.bbmodel"),
        "breakerCabinet" => include_str!("../../../last_route/assets/props/breakerCabinet.bbmodel"),
        "clothesRack" => include_str!("../../../last_route/assets/props/clothesRack.bbmodel"),
        "bakeryRack" => include_str!("../../../last_route/assets/props/bakeryRack.bbmodel"),
        "arcadeCabinet" => include_str!("../../../last_route/assets/props/arcadeCabinet.bbmodel"),
        "magazineRack" => include_str!("../../../last_route/assets/props/magazineRack.bbmodel"),
        _ => return None,
    })
}

/// The baked props: one mesh per placed prop, in world space.
pub struct Props {
    meshes: Vec<Mesh>,
    placed: Vec<PropDef>,
}

impl Props {
    pub fn empty() -> Props {
        Props { meshes: Vec::new(), placed: Vec::new() }
    }

    /// Parse the models and bake them into meshes. Needs the GL context. A prop whose model does not parse is skipped.
    pub fn build() -> Props {
        let defs = parse_props(PROPS_JSON).unwrap_or_default();
        let mut models: HashMap<String, Option<Model>> = HashMap::new();
        let mut meshes = Vec::new();
        let mut placed = Vec::new();
        for d in defs {
            let entry = models.entry(d.model.clone()).or_insert_with(|| prop_text(&d.model).and_then(|t| Model::parse(t, None, &|_| None, None)));
            let model = match entry {
                Some(m) => m,
                None => continue,
            };
            let (mut verts, mut idx) = (Vec::new(), Vec::new());
            let s = model.scale * d.scale;
            model.build_scaled(&mut verts, &mut idx, d.world_pos(), d.yaw_deg.to_radians(), Vec3::splat(s), 0.0, 0.0, WHITE);
            if verts.is_empty() {
                continue;
            }
            meshes.push(Mesh { vertices: verts, indices: idx, texture: Some(model.texture.clone()) });
            placed.push(d);
        }
        Props { meshes, placed }
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// The props of one room (for tests and the admin rows).
    pub fn in_room(&self, room: &str) -> Vec<&PropDef> {
        self.placed.iter().filter(|p| p.room == room).collect()
    }

    /// Draw every prop with the lit shader (inside an already begun camera).
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3) {
        if self.meshes.is_empty() {
            return;
        }
        gfx.shader.begin(lights, cam_pos);
        for m in &self.meshes {
            draw_mesh(m);
        }
        gfx.shader.end();
    }
}

/// Why this mode cannot start, if so: the Classic props file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    parse_props(PROPS_JSON).err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cam_rooms;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(parse_props("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    #[test]
    fn the_props_file_parses_and_every_model_is_embedded() {
        let defs = parse_props(PROPS_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert!(defs.len() >= 12, "{} props", defs.len());
        for d in &defs {
            assert!(prop_text(&d.model).is_some(), "{} is not embedded", d.model);
            assert!(d.scale > 0.0 && d.yaw_deg.is_finite());
        }
    }

    #[test]
    fn every_prop_stands_inside_a_camera_room() {
        let rooms = cam_rooms();
        let defs = parse_props(PROPS_JSON).unwrap_or_default();
        for d in &defs {
            assert!(rooms.contains(&d.room), "{} is not a camera room", d.room);
            assert!(d.pos.x.abs() <= 3.0 && d.pos.z.abs() <= 3.0, "{} {:?} leaves the room", d.model, d.pos);
            assert!(d.pos.y >= 0.0 && d.pos.y < 1.0);
            let c = Building::room_centre(&d.room);
            assert!((d.world_pos() - c - d.pos).length() < 1e-4);
        }
        // every room has at least one extra prop besides the built dressing, except the two halls share cabinets
        for r in &rooms {
            if r == "easthall" || r == "westhall" {
                continue;
            }
            assert!(defs.iter().any(|d| &d.room == r), "{r} has no extra prop");
        }
    }

    #[test]
    fn the_models_are_blockbench_documents_with_a_texture() {
        for d in parse_props(PROPS_JSON).unwrap_or_default() {
            let text = prop_text(&d.model).unwrap_or("");
            let j: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            assert!(j.get("elements").and_then(|e| e.as_array()).map_or(false, |a| !a.is_empty()), "{} has no elements", d.model);
            let src = j.get("textures").and_then(|t| t.get(0)).and_then(|t| t.get("source")).and_then(|s| s.as_str()).unwrap_or("");
            assert!(src.starts_with("data:image/png;base64,"), "{} has no embedded texture", d.model);
        }
    }

    #[test]
    fn a_bad_entry_is_an_error() {
        assert!(parse_props("{").is_err());
        assert!(parse_props("{\"props\": [{\"room\": \"stage\"}]}").is_err());
        assert!(parse_props("{\"props\": [{\"room\": \"stage\", \"model\": \"x\"}]}").is_err());
        let ok = parse_props("{\"props\": [{\"room\": \"stage\", \"model\": \"x\", \"pos\": [1, 0, 2], \"yaw\": 90, \"scale\": 2}]}").unwrap_or_default();
        assert_eq!(ok.len(), 1);
        assert_eq!((ok[0].pos, ok[0].yaw_deg, ok[0].scale), (vec3(1.0, 0.0, 2.0), 90.0, 2.0));
    }
}
