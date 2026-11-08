//! Every sewer prop and ceiling fixture fits under the sewer's 4.0 m wall (`data/regions/sewer.json`
//! `geometry.wall_height`, capped by `seams::wall_top`). The tests place props the way `render/world_mesh.rs` does:
//! `visual_y` (the mount height, a ceiling seat, or the floor), the model lowered by its lowest vertex when mounted, the
//! per-type draw scale from `data/render/prop_draw_scale.json`, and the model's own height from its embedded bbmodel.
//! A prop that does not fit is fixed in data (its draw scale, or its mount height in `data/regions/sewer.json`), never
//! in code. The model bounds are read here from the bbmodel elements (cubes and meshes, with their rotation), because
//! `PropModel` needs a graphics context to load.

use super::catalog::Catalog;
use super::infinite::{self, ChunkKey};
use super::route::sewer::core::{crawl_pipe_mount_y, feature_mount_y, LEVEL, LOWER_LEVEL};
use crate::content::loot::LootTables;
use crate::content::props::{ceiling_top, visual_y};
use crate::content::{embedded, Content};
use crate::render::{prop_scale, seams};
use macroquad::prelude::{EulerRot, Mat3, Vec3};

const WORLDS: [&str; 4] = ["wired-sewer", "1426541270", "water-a", "water-b"];
/// Rounding slack for a model authored flush with its seat.
const EPS: f32 = 0.02;

fn cat() -> &'static Catalog {
    static CAT: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    CAT.get_or_init(Catalog::load)
}

fn content() -> &'static Content {
    static C: std::sync::OnceLock<Content> = std::sync::OnceLock::new();
    C.get_or_init(Content::load)
}

fn v3(j: &serde_json::Value) -> Vec3 {
    let n = |i: usize| j.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    Vec3::new(n(0), n(1), n(2))
}

/// The model's lowest and highest y in metres (16 bbmodel units to the metre, as `Model::from_json` scales props).
fn model_y_range(path: &str) -> (f32, f32) {
    let bytes = embedded::asset(path).unwrap_or_else(|| panic!("{path} is embedded"));
    let j: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for e in j["elements"].as_array().unwrap_or_else(|| panic!("{path} has elements")) {
        if e["visibility"].as_bool() == Some(false) || e["export"].as_bool() == Some(false) {
            continue;
        }
        let origin = v3(&e["origin"]);
        let r = v3(&e["rotation"]);
        let rot = Mat3::from_euler(EulerRot::ZYX, r.z.to_radians(), r.y.to_radians(), r.x.to_radians());
        let points: Vec<Vec3> = match e["type"].as_str().unwrap_or("cube") {
            "cube" => {
                let (f, t) = (v3(&e["from"]), v3(&e["to"]));
                let mut out = vec![];
                for x in [f.x, t.x] {
                    for y in [f.y, t.y] {
                        for z in [f.z, t.z] {
                            out.push(Vec3::new(x, y, z));
                        }
                    }
                }
                out
            }
            "mesh" => e["vertices"].as_object().map(|m| m.values().map(|v| v3(v) + origin).collect()).unwrap_or_default(),
            _ => continue,
        };
        for p in points {
            let y = (rot * (p - origin) + origin).y / 16.0;
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    assert!(lo <= hi, "{path} has no drawable element");
    (lo, hi)
}

/// The tallest top of a prop type's models (every variant, the worst case) when drawn as `world_mesh` draws it.
/// `mount_y` is the prop's own mount height (0 when it has none), `ceiling` the seat `fixture_ceiling` gives it.
fn drawn_top(ptype: &str, mount_y: f32, ceiling: f32) -> f32 {
    let def = content().props.get(ptype).unwrap_or_else(|| panic!("{ptype} is a prop type"));
    let scale = prop_scale::draw_scale(ptype);
    def.models
        .iter()
        .map(|m| {
            let (lo, hi) = model_y_range(m);
            let mut y = visual_y(ptype, ceiling, mount_y);
            if mount_y > 0.0 {
                y -= lo * scale;
            }
            y + hi * scale
        })
        .fold(f32::MIN, f32::max)
}

/// A region's wall top, read like `world_mesh::region_wall_height` does (an unknown region gets the sewer's).
fn wall_of(region: &str) -> f32 {
    let table = crate::content::regions::table();
    let def = table.def(region).or_else(|| table.def("sewer")).expect("the sewer region exists");
    seams::wall_top(def.geometry.map_or(crate::config::WALL_HEIGHT, |g| g.wall_height))
}

#[test]
fn the_sewer_wall_top_is_four_metres() {
    for region in ["sewer", "sewer2"] {
        assert_eq!(wall_of(region), 4.0, "{region}");
    }
}

/// Each sewer prop type, at the mount height its feature kind gets (or the floor), fits under the wall.
#[test]
fn every_sewer_prop_type_fits_under_the_wall() {
    let wall = wall_of("sewer");
    let (mut seen, mut bad) = (0, Vec::new());
    for ptype in content().props.keys().filter(|t| t.starts_with("sewer")) {
        // the mount heights a type is placed at: its feature mount, or the floor; the crawl pipe (a `sewerPipeRun`
        // feature) also hangs at the crawl mount
        let mut mounts = vec![feature_mount_y(ptype).unwrap_or(0.0) as f32];
        if ptype.as_str() == "sewerPipeRun" {
            mounts.push(crawl_pipe_mount_y() as f32);
        }
        for mount in mounts {
            let top = drawn_top(ptype, mount, seams::ceiling_y(wall));
            if top > wall + EPS {
                bad.push(format!("{ptype} mounted at {mount} m reaches {top} m"));
            }
            seen += 1;
        }
    }
    assert!(bad.is_empty(), "over the {wall} m wall: {bad:?}");
    assert!(seen >= 15, "checked {seen} placements");
}

/// Every prop that stands in a generated sewer chunk (the maze, the extra rooms) fits under its cell's ceiling.
#[test]
fn every_prop_in_a_sample_sewer_fits_under_its_ceiling() {
    let mut count = 0;
    let mut bad = std::collections::BTreeSet::new();
    let mut types = std::collections::BTreeSet::new();
    for world in WORLDS {
        for level in [LEVEL, LOWER_LEVEL] {
            for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1), (2, 1), (-2, 0)] {
                let (f, _) = infinite::chunk(world, ChunkKey { cx, cy, level }, cat(), &LootTables::default());
                for p in &f.props {
                    // a type without a definition (the room templates name a plain `barrel`) has no model to measure
                    if content().props.get(&p.ptype).is_none() {
                        continue;
                    }
                    let wall = wall_of(f.region_at(p.cell.x, p.cell.y));
                    let top = drawn_top(&p.ptype, p.mount_y.unwrap_or(0.0) as f32, seams::ceiling_y(wall));
                    if top > wall + EPS {
                        bad.insert(format!("{} reaches {top} m, over the {wall} m wall", p.ptype));
                    }
                    types.insert(p.ptype.clone());
                    count += 1;
                }
            }
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
    assert!(count > 40, "checked {count} props");
    assert!(types.iter().any(|t| t == "sewerLamp"), "the sample has lamps: {types:?}");
}

/// A ceiling-hung prop type is seated by its authored top: hung in a sewer, its drawn top never passes the wall.
#[test]
fn ceiling_fixtures_hang_from_the_sewer_ceiling_and_stay_under_the_wall() {
    let wall = wall_of("sewer");
    let seat = seams::ceiling_y(wall);
    // `surgicalLight` is left out: it is a hospital fixture and no sewer template places it
    let mut bad = Vec::new();
    for ptype in ["diningChandelier", "ceilingLight", "ceilingFan", "infernoChainLantern", "hangingVines"] {
        assert!(ceiling_top(ptype).is_some(), "{ptype} is a ceiling type");
        if content().props.get(ptype).is_none() {
            continue;
        }
        let top = drawn_top(ptype, 0.0, seat);
        if top > wall + EPS {
            bad.push(format!("{ptype} hung in a sewer reaches {top} m"));
        }
    }
    assert!(bad.is_empty(), "over the {wall} m wall: {bad:?}");
    // the switchable room lamp is a 0.08 m slab drawn at the ceiling quad less 0.04 m, so its top is the quad
    assert!(seat <= wall, "the ceiling quad sits under the wall top");
}

/// The room lamps of the sample sewers sit under the wall.
#[test]
fn sewer_room_lights_sit_under_the_wall() {
    for world in WORLDS {
        for level in [LEVEL, LOWER_LEVEL] {
            let (f, _) = infinite::chunk(world, ChunkKey { cx: 0, cy: 0, level }, cat(), &LootTables::default());
            for l in &f.room_lights {
                let wall = wall_of(f.region_at(l.center.x, l.center.y));
                assert!(seams::ceiling_y(wall) <= wall, "{world}: a room lamp pokes through the {wall} m ceiling");
            }
        }
    }
}

/// The limit the fit tests use is the sewer region file's own `geometry.wall_height`, not a number in a test: reading the
/// raw JSON gives the same 4.0 m, so a data change moves both the wall and the check.
#[test]
fn the_fit_limit_is_read_from_the_region_data() {
    for (region, file) in [("sewer", "regions/sewer.json"), ("sewer2", "regions/sewer2.json")] {
        let (_, text) = embedded::data_under(file).next().unwrap_or_else(|| panic!("{file} is embedded"));
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        let raw = v["geometry"]["wall_height"].as_f64().expect("geometry.wall_height") as f32;
        assert_eq!(wall_of(region), seams::wall_top(raw), "{region}");
    }
}

/// Every mounted sewer feature kind hangs under the wall at its own mount height (the lamp, the valve wheel, the pipes):
/// the mount itself leaves room for a model of 5 cm or more.
#[test]
fn every_sewer_mount_height_is_under_the_wall() {
    let wall = wall_of("sewer");
    for ptype in content().props.keys().filter(|t| t.starts_with("sewer")) {
        if let Some(mount) = feature_mount_y(ptype) {
            assert!(mount as f32 + 0.05 <= wall, "{ptype} mounts at {mount} m under a {wall} m wall");
        }
    }
    assert!(crawl_pipe_mount_y() as f32 + 0.05 <= wall);
}
