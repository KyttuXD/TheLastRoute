//! A port of Godot's `CoplanarScan` (game/look/coplanar_scan.gd): z-fighting between same-facing, axis-aligned faces of
//! DIFFERENT owners (props, decals, rugs, door frames, the level shell) that lie in the same plane and overlap.
//!
//! `scan` takes triangles with their owner ids, keeps the axis-aligned ones, buckets them by plane and reports the
//! overlapping pairs, largest first. Every limit (tolerance, minimum overlap, axis epsilon, the buried-slab skip, the
//! additive decor and the owners that are hidden by design) is read from `data/world/coplanar.json`.
//!
//! The scan itself is checked on made-up boxes (a rug laid on a rug, a decal on a floor, a door frame against a wall).
//! The placements the game makes are checked where the numbers are exact: floor dressing sits above the floor plane by
//! more than the tolerance, so a rug, a decal or a mat can never fight the floor.

use std::collections::HashMap;

use serde_json::Value;

type P3 = [f32; 3];

/// The scan settings (`coplanar.json`).
#[derive(Clone, Debug, PartialEq)]
struct Config {
    tolerance: f32,
    min_overlap: f32,
    axis_epsilon: f32,
    slab_period: f32,
    slab_tolerance: f32,
    additive_decor: Vec<String>,
    skipped_owners: Vec<String>,
}

impl Config {
    fn parse(text: &str) -> Result<Config, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("coplanar.json: {e}"))?;
        let num = |k: &str| -> Result<f32, String> {
            let x = v.get(k).and_then(Value::as_f64).ok_or_else(|| format!("coplanar.json: {k} must be a number"))? as f32;
            if x.is_finite() && x > 0.0 {
                Ok(x)
            } else {
                Err(format!("coplanar.json: {k} must be above 0"))
            }
        };
        let names = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        Ok(Config {
            tolerance: num("tolerance")?,
            min_overlap: num("min_overlap")?,
            axis_epsilon: num("axis_epsilon")?,
            slab_period: num("slab_period")?,
            slab_tolerance: num("slab_tolerance")?,
            additive_decor: names("additive_decor"),
            skipped_owners: names("skipped_owners"),
        })
    }

    fn shipped() -> Config {
        let (_, text) = crate::content::embedded::data_under("world/coplanar.json").next().expect("coplanar.json is embedded");
        Config::parse(text).unwrap_or_else(|e| panic!("{e}"))
    }
}

/// One axis-aligned face: its plane on `axis`, the facing `sign`, and its extent on the other two axes.
#[derive(Clone, Debug)]
struct Face {
    axis: usize,
    sign: i32,
    plane: f32,
    u0: f32,
    u1: f32,
    v0: f32,
    v1: f32,
    owner: String,
}

/// An overlapping pair, `a` before `b` in sorted order.
#[derive(Clone, Debug, PartialEq)]
struct Pair {
    a: String,
    b: String,
    axis: usize,
    plane: f32,
    overlap: f32,
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `_add_triangle`: the face of an axis-aligned triangle, or nothing (slanted, degenerate or buried in a slab).
fn face_of(cfg: &Config, a: P3, b: P3, c: P3, owner: &str) -> Option<Face> {
    let n = cross(sub(b, a), sub(c, a));
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-9 {
        return None;
    }
    let n = [n[0] / len, n[1] / len, n[2] / len];
    let mut axis = None;
    for (k, nk) in n.iter().enumerate() {
        if nk.abs() > 1.0 - cfg.axis_epsilon {
            axis = Some(k);
        }
    }
    let axis = axis?;
    if (a[axis] - b[axis]).abs() > cfg.axis_epsilon || (a[axis] - c[axis]).abs() > cfg.axis_epsilon {
        return None;
    }
    // a face looking down from the floor plane is buried in the slab: never seen
    if axis == 1 && n[1] < 0.0 && (a[1] - (a[1] / cfg.slab_period).round() * cfg.slab_period).abs() < cfg.slab_tolerance {
        return None;
    }
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let lo = |i: usize| a[i].min(b[i]).min(c[i]);
    let hi = |i: usize| a[i].max(b[i]).max(c[i]);
    Some(Face {
        axis,
        sign: if n[axis] > 0.0 { 1 } else { -1 },
        plane: (a[axis] + b[axis] + c[axis]) / 3.0,
        u0: lo(u),
        u1: hi(u),
        v0: lo(v),
        v1: hi(v),
        owner: owner.to_string(),
    })
}

/// The 12 triangles of a box, each quad counter-clockwise seen from outside.
fn box_triangles(min: P3, max: P3) -> Vec<[P3; 3]> {
    let p = |x: usize, y: usize, z: usize| [[min[0], max[0]][x], [min[1], max[1]][y], [min[2], max[2]][z]];
    let quads = [
        [p(1, 0, 0), p(1, 1, 0), p(1, 1, 1), p(1, 0, 1)], // +x
        [p(0, 0, 1), p(0, 1, 1), p(0, 1, 0), p(0, 0, 0)], // -x
        [p(0, 1, 0), p(0, 1, 1), p(1, 1, 1), p(1, 1, 0)], // +y
        [p(0, 0, 1), p(0, 0, 0), p(1, 0, 0), p(1, 0, 1)], // -y
        [p(0, 0, 1), p(1, 0, 1), p(1, 1, 1), p(0, 1, 1)], // +z
        [p(1, 0, 0), p(0, 0, 0), p(0, 1, 0), p(1, 1, 0)], // -z
    ];
    quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect()
}

/// The scan: each triangle comes with its owner; `parent_of` gives an owner's parent (a prop's pieces share one) for
/// `ignore_siblings`. Port of `scan` and `_pairs`.
fn scan(cfg: &Config, tris: &[(String, [P3; 3])], ignore_siblings: bool, parent_of: &dyn Fn(&str) -> String) -> Vec<Pair> {
    let mut faces: Vec<Face> = Vec::new();
    let mut buckets: HashMap<(usize, i32, i64), Vec<usize>> = HashMap::new();
    for (owner, t) in tris {
        let hidden = cfg.skipped_owners.iter().chain(cfg.additive_decor.iter()).any(|s| owner.contains(s.as_str()));
        if hidden {
            continue;
        }
        if let Some(f) = face_of(cfg, t[0], t[1], t[2], owner) {
            let key = (f.axis, f.sign, (f.plane / cfg.tolerance).round() as i64);
            faces.push(f);
            buckets.entry(key).or_default().push(faces.len() - 1);
        }
    }
    let mut found: HashMap<String, Pair> = HashMap::new();
    for (key, here) in &buckets {
        let mut lists = vec![here];
        if let Some(next) = buckets.get(&(key.0, key.1, key.2 + 1)) {
            lists.push(next);
        }
        for (li, other) in lists.iter().enumerate() {
            for (i, ia) in here.iter().enumerate() {
                let fa = &faces[*ia];
                let start = if li == 0 { i + 1 } else { 0 };
                for ib in other.iter().skip(start) {
                    let fb = &faces[*ib];
                    if fa.owner == fb.owner || (fa.plane - fb.plane).abs() > cfg.tolerance {
                        continue;
                    }
                    if ignore_siblings && parent_of(&fa.owner) == parent_of(&fb.owner) {
                        continue;
                    }
                    let du = fa.u1.min(fb.u1) - fa.u0.max(fb.u0);
                    let dv = fa.v1.min(fb.v1) - fa.v0.max(fb.v0);
                    if du <= 0.0 || dv <= 0.0 || du * dv < cfg.min_overlap {
                        continue;
                    }
                    let mut owners = [fa.owner.clone(), fb.owner.clone()];
                    owners.sort();
                    let id = format!("{}|{}|{}", owners[0], owners[1], key.0);
                    if found.get(&id).map_or(true, |p| p.overlap < du * dv) {
                        let [a, b] = owners;
                        found.insert(id, Pair { a, b, axis: fa.axis, plane: fa.plane, overlap: du * dv });
                    }
                }
            }
        }
    }
    let mut out: Vec<Pair> = found.into_values().collect();
    out.sort_by(|x, y| y.overlap.total_cmp(&x.overlap).then_with(|| x.a.cmp(&y.a)).then_with(|| x.b.cmp(&y.b)));
    out
}

fn boxes(items: &[(&str, P3, P3)]) -> Vec<(String, [P3; 3])> {
    items.iter().flat_map(|(o, lo, hi)| box_triangles(*lo, *hi).into_iter().map(move |t| (o.to_string(), t))).collect()
}

fn flat(_: &str) -> String {
    String::new()
}

#[test]
fn the_shipped_settings_are_godots() {
    let c = Config::shipped();
    assert_eq!(c.tolerance, 0.002);
    assert_eq!(c.min_overlap, 0.0004);
    assert_eq!(c.axis_epsilon, 0.0005);
    assert_eq!(c.slab_period, 4.0);
    assert!(c.additive_decor.iter().any(|s| s == "Ref_halo") && c.skipped_owners.iter().any(|s| s == "LockerSeal"));
}

#[test]
fn bad_settings_are_refused() {
    assert!(Config::parse("{}").is_err());
    assert!(Config::parse(r#"{"tolerance":0,"min_overlap":1,"axis_epsilon":1,"slab_period":4,"slab_tolerance":1}"#).is_err());
}

#[test]
fn a_rug_laid_on_a_rug_fights() {
    let tris = boxes(&[("rug_a", [0.0, 0.0, 0.0], [2.0, 0.012, 2.0]), ("rug_b", [1.0, 0.0, 1.0], [3.0, 0.012, 3.0])]);
    let pairs = scan(&Config::shipped(), &tris, false, &flat);
    let top = pairs.iter().find(|p| p.axis == 1 && (p.plane - 0.012).abs() < 1e-6).expect("the tops fight");
    assert!((top.overlap - 1.0).abs() < 1e-4, "a 1 m x 1 m overlap: {}", top.overlap);
    assert_eq!((top.a.as_str(), top.b.as_str()), ("rug_a", "rug_b"));
}

#[test]
fn a_decal_lifted_past_the_tolerance_does_not_fight() {
    let cfg = Config::shipped();
    let lifted = 0.012 + cfg.tolerance * 2.0;
    let tris = boxes(&[("rug", [0.0, 0.0, 0.0], [2.0, 0.012, 2.0]), ("decal", [0.5, 0.0, 0.5], [1.5, lifted, 1.5])]);
    assert!(scan(&cfg, &tris, false, &flat).iter().all(|p| p.axis != 1 || (p.plane - 0.012).abs() > 1e-6));
}

#[test]
fn a_door_frame_flush_with_the_wall_fights_and_one_set_back_does_not() {
    let cfg = Config::shipped();
    let wall = ("wall", [0.0, 0.0, 0.0], [4.0, 3.0, 0.2]);
    let flush = boxes(&[wall, ("frame", [1.0, 0.0, 0.0], [2.0, 2.1, 0.2])]);
    let pairs = scan(&cfg, &flush, false, &flat);
    assert!(pairs.iter().any(|p| p.axis == 2 && p.overlap > 1.0), "{pairs:?}");
    let set_back = boxes(&[wall, ("frame", [1.0, 0.0, -0.05], [2.0, 2.1, 0.25])]);
    assert!(scan(&cfg, &set_back, false, &flat).iter().all(|p| p.axis != 2));
}

#[test]
fn faces_that_face_opposite_ways_do_not_fight() {
    let cfg = Config::shipped();
    // two boxes side by side share the plane x = 1 but face away from each other
    let side = boxes(&[("a", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), ("b", [1.0, 0.0, 0.0], [2.0, 1.0, 1.0])]);
    assert!(scan(&cfg, &side, false, &flat).iter().all(|p| p.axis != 0));
}

#[test]
fn an_overlap_under_the_minimum_area_is_not_reported() {
    let cfg = Config::shipped();
    // the tops share y = 1 and overlap by 1 cm x 1 cm = 0.0001 m2, under the 0.0004 m2 minimum
    let tris = boxes(&[("a", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), ("b", [0.99, 0.5, 0.99], [2.0, 1.0, 2.0])]);
    assert!(scan(&cfg, &tris, false, &flat).iter().all(|p| p.axis != 1));
    let wide = Config { min_overlap: 0.00005, ..cfg };
    assert!(scan(&wide, &tris, false, &flat).iter().any(|p| p.axis == 1));
}

#[test]
fn a_changed_tolerance_changes_what_fights() {
    let tris = boxes(&[("a", [0.0, 0.0, 0.0], [1.0, 0.010, 1.0]), ("b", [0.0, 0.0, 0.0], [1.0, 0.0125, 1.0])]);
    let tight = Config::shipped();
    let loose = Config { tolerance: 0.01, ..tight.clone() };
    let count = |c: &Config| scan(c, &tris, false, &flat).iter().filter(|p| p.axis == 1 && p.plane > 0.005).count();
    assert_eq!(count(&tight), 0, "2.5 mm apart is past the 2 mm tolerance");
    assert_eq!(count(&loose), 1);
}

#[test]
fn the_underside_of_the_floor_slab_is_buried_and_skipped() {
    let cfg = Config::shipped();
    // a slab's bottom face lies on the 4 m grid looking down; it does not fight another slab's bottom there
    let tris = boxes(&[("slab_a", [0.0, 4.0, 0.0], [2.0, 5.0, 2.0]), ("slab_b", [0.0, 4.0, 0.0], [2.0, 5.5, 2.0])]);
    let pairs = scan(&cfg, &tris, false, &flat);
    assert!(pairs.iter().all(|p| !(p.axis == 1 && (p.plane - 4.0).abs() < 1e-3)), "{pairs:?}");
}

#[test]
fn siblings_additive_decor_and_hidden_owners_are_skipped() {
    let cfg = Config::shipped();
    let parent = |o: &str| o.split('/').next().unwrap_or("").to_string();
    let tris = boxes(&[("chair/seat", [0.0, 0.0, 0.0], [1.0, 0.5, 1.0]), ("chair/back", [0.0, 0.0, 0.0], [1.0, 0.5, 1.0])]);
    assert!(!scan(&cfg, &tris, false, &parent).is_empty());
    assert!(scan(&cfg, &tris, true, &parent).is_empty(), "parts of one prop are modelling detail");
    let glow = boxes(&[("Ref_halo", [0.0, 0.0, 0.0], [1.0, 0.1, 1.0]), ("lamp", [0.0, 0.0, 0.0], [1.0, 0.1, 1.0])]);
    assert!(scan(&cfg, &glow, false, &flat).is_empty(), "additive decor draws without depth writes");
    let seal = boxes(&[("LockerSeal", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), ("locker", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0])]);
    assert!(scan(&cfg, &seal, false, &flat).is_empty());
}

#[test]
fn slanted_faces_are_not_scanned() {
    let cfg = Config::shipped();
    assert!(face_of(&cfg, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0], "ramp").is_none());
}

/// Floor dressing (rugs, mats, decals) is drawn above the floor plane by more than the tolerance, so it never fights the
/// floor, and below a footstep so it still reads as flat.
#[test]
fn floor_dressing_never_sits_on_the_floor_plane() {
    use crate::content::props::{visual_y, FLOOR_DRESSING};
    let cfg = Config::shipped();
    for t in FLOOR_DRESSING {
        let y = visual_y(t, 4.0, 0.0);
        assert!(y > cfg.tolerance, "{t} lies {y} m up, inside the {} m tolerance of the floor", cfg.tolerance);
        assert!(y < 0.05, "{t} floats {y} m above the floor");
    }
}
