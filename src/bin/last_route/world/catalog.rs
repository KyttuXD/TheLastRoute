//! RoomCatalog (game/config/room_catalog.gd) as data: data/rooms/catalog.json was extracted
//! from the GDScript constants. Footprints are f32 like Godot's Vector2.

use crate::content::embedded;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct RoomType {
    pub id: String,
    pub label: String,
    pub region: String,
    pub special: i64,
    pub large_only: bool,
    pub streamed_only: bool,
    pub landmark: bool,
    pub props: Vec<String>,
    /// The order populate_rooms iterates (Godot's unstable introsort, precomputed).
    pub props_sorted: Vec<String>,
}

pub struct Catalog {
    pub types: Vec<RoomType>,
    footprints: HashMap<String, (f32, f32)>,
    non_solid: HashSet<String>,
    surface_parents: HashMap<String, Vec<String>>,
    surface_heights: HashMap<String, f64>,
    wall_backed: HashSet<String>,
    walkable: HashSet<String>,
    centerpiece: HashSet<String>,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

impl Catalog {
    pub fn load() -> Catalog {
        let text = embedded::data_under("rooms/catalog.json")
            .next()
            .map(|x| x.1)
            .unwrap_or("{}");
        let j: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let types = j["types"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| RoomType {
                        id: t["id"].as_str().unwrap_or("").into(),
                        label: t["label"].as_str().unwrap_or("").into(),
                        region: t["region"].as_str().unwrap_or("").into(),
                        special: t["special"].as_i64().unwrap_or(0),
                        large_only: t["large_only"].as_bool().unwrap_or(false),
                        streamed_only: t["streamed_only"].as_bool().unwrap_or(false),
                        landmark: t["landmark"].as_bool().unwrap_or(false),
                        props: strs(&t["props"]),
                        props_sorted: strs(&t["props_sorted_for_populate"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut footprints = HashMap::new();
        for key in ["footprint_table_full", "footprints"] {
            if let Some(o) = j[key].as_object() {
                for (k, v) in o {
                    let x = v["x"]
                        .as_f64()
                        .or_else(|| v.get(0).and_then(|x| x.as_f64()));
                    let y = v["y"]
                        .as_f64()
                        .or_else(|| v.get(1).and_then(|x| x.as_f64()));
                    if let (Some(x), Some(y)) = (x, y) {
                        footprints.insert(k.clone(), (x as f32, y as f32));
                    }
                }
            }
        }
        let surface_parents = j["surface_props"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), strs(v))).collect())
            .unwrap_or_default();
        let surface_heights = j["surface_heights"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v["value"].as_f64().map(|h| (k.clone(), h)))
                    .collect()
            })
            .unwrap_or_default();
        let set = |k: &str| -> HashSet<String> { strs(&j[k]).into_iter().collect() };
        Catalog {
            types,
            footprints,
            non_solid: set("non_solid_types"),
            surface_parents,
            surface_heights,
            wall_backed: set("wall_backed"),
            walkable: set("walkable_structures"),
            centerpiece: set("centerpiece_types"),
        }
    }

    /// Half extents in cells; odd rotations swap x and y.
    pub fn footprint(&self, t: &str, rotation: i32) -> (f32, f32) {
        let v = self.footprints.get(t).copied().unwrap_or((0.32, 0.32));
        if rotation.abs() % 2 == 1 {
            (v.1, v.0)
        } else {
            v
        }
    }

    pub fn is_solid(&self, t: &str) -> bool {
        !self.non_solid.contains(t)
    }

    pub fn is_surface_prop(&self, t: &str) -> bool {
        self.surface_parents.contains_key(t)
    }

    pub fn surface_parent_types(&self, t: &str) -> &[String] {
        self.surface_parents
            .get(t)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn surface_height(&self, parent: &str) -> f64 {
        self.surface_heights.get(parent).copied().unwrap_or(0.8)
    }

    pub fn is_wall_backed(&self, t: &str) -> bool {
        self.wall_backed.contains(t)
    }

    pub fn is_walkable_structure(&self, t: &str) -> bool {
        self.walkable.contains(t)
    }

    pub fn is_centerpiece_room(&self, room_type: &str) -> bool {
        self.centerpiece.contains(room_type)
    }

    pub fn type_by_id(&self, id: &str) -> Option<&RoomType> {
        self.types.iter().find(|t| t.id == id)
    }

    pub fn room_has_walkable_structure(&self, room_type: &str) -> bool {
        self.type_by_id(room_type)
            .map(|t| t.props.iter().any(|p| self.is_walkable_structure(p)))
            .unwrap_or(false)
    }
}
