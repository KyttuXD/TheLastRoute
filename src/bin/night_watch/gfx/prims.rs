#![allow(dead_code)]
//! Shared building helpers for every mode's rooms: walls with doorways, room shells, stairs, small props.
//! All positions are local to the group (the group origin is added at bake time). Floor level is y = 0.

use super::mesh::{Cube, Group};
use macroquad::prelude::*;

/// Height of a doorway cut by `wall_with_gaps`; a lintel fills the wall above it.
pub const DOOR_HEIGHT: f32 = 2.6;

/// A straight wall from `a` to `b` along X (`along_x`, at z = `fixed`) or along Z (at x = `fixed`), with openings.
/// `gaps` are (start, end) intervals along the axis; each opening is `DOOR_HEIGHT` tall with a lintel above when the
/// wall is taller. Wall segments are solid, lintels are not. Cubes are named `name`.
pub fn wall_with_gaps(
    g: &mut Group,
    along_x: bool,
    fixed: f32,
    a: f32,
    b: f32,
    gaps: &[(f32, f32)],
    height: f32,
    thick: f32,
    tint: [f32; 3],
    name: &str,
) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut cuts: Vec<(f32, f32)> = gaps
        .iter()
        .map(|(s, e)| (s.min(*e).max(lo), s.max(*e).min(hi)))
        .filter(|(s, e)| e - s > 1e-4)
        .collect();
    cuts.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut segs: Vec<(f32, f32)> = Vec::new();
    let mut at = lo;
    for (s, e) in &cuts {
        if *s > at + 1e-4 {
            segs.push((at, *s));
        }
        at = at.max(*e);
    }
    if hi > at + 1e-4 {
        segs.push((at, hi));
    }
    let place = |len_a: f32, len_b: f32, y0: f32, y1: f32| -> (Vec3, Vec3) {
        let len = len_b - len_a;
        let mid = (len_a + len_b) * 0.5;
        let hgt = y1 - y0;
        let cy = (y0 + y1) * 0.5;
        if along_x {
            (vec3(len, hgt, thick), vec3(mid, cy, fixed))
        } else {
            (vec3(thick, hgt, len), vec3(fixed, cy, mid))
        }
    };
    for (s, e) in segs {
        let (size, pos) = place(s, e, 0.0, height);
        g.add(Cube::new(name, size, pos).tint(tint).solid());
    }
    if height > DOOR_HEIGHT + 0.05 {
        for (s, e) in &cuts {
            let (size, pos) = place(*s, *e, DOOR_HEIGHT, height);
            g.add(Cube::new(name, size, pos).tint(tint));
        }
    }
}

/// Floor, four walls (WallN, WallS, WallW, WallE) and optionally a ceiling for the rectangle x0..x1, z0..z1.
/// The floor top is at y = 0; walls are 0.2 thick, centred on the rectangle edge.
pub fn room_shell(g: &mut Group, x0: f32, x1: f32, z0: f32, z1: f32, height: f32, tint: [f32; 3], ceiling: bool) {
    let (xa, xb) = (x0.min(x1), x0.max(x1));
    let (za, zb) = (z0.min(z1), z0.max(z1));
    let cx = (xa + xb) * 0.5;
    let cz = (za + zb) * 0.5;
    g.add(Cube::new("Floor", vec3(xb - xa + 0.4, 0.2, zb - za + 0.4), vec3(cx, -0.1, cz)).kind("floor"));
    wall_with_gaps(g, true, za, xa - 0.1, xb + 0.1, &[], height, 0.2, tint, "WallN");
    wall_with_gaps(g, true, zb, xa - 0.1, xb + 0.1, &[], height, 0.2, tint, "WallS");
    wall_with_gaps(g, false, xa, za - 0.1, zb + 0.1, &[], height, 0.2, tint, "WallW");
    wall_with_gaps(g, false, xb, za - 0.1, zb + 0.1, &[], height, 0.2, tint, "WallE");
    if ceiling {
        g.add(Cube::new("Ceiling", vec3(xb - xa + 0.4, 0.2, zb - za + 0.4), vec3(cx, height + 0.1, cz)));
    }
}

/// A flight of `steps` solid blocks rising toward -Z from `origin` (the foot of the first step's front edge).
/// Step i is `rise * (i + 1)` tall and `run` deep.
pub fn stairs(g: &mut Group, origin: Vec3, steps: u32, rise: f32, run: f32, width: f32, tint: [f32; 3]) {
    for i in 0..steps {
        let h = rise * (i + 1) as f32;
        let centre = origin + vec3(0.0, h * 0.5, -run * (i as f32 + 0.5));
        g.add(Cube::new("StairStep", vec3(width, h, run), centre).kind("concrete").tint(tint).solid());
    }
}

/// Small named props built from a few cubes: table, crate, bed, bench, shelf, locker, chair, barrel, bin, lamp.
/// An unknown kind draws a crate. `origin` is the centre of the footprint on the floor.
pub fn blocky_prop(g: &mut Group, kind: &str, origin: Vec3, tint: [f32; 3]) {
    let o = origin;
    match kind {
        "table" => {
            g.add(Cube::new("TableTop", vec3(1.2, 0.08, 0.7), o + vec3(0.0, 0.76, 0.0)).kind("wood").tint(tint).solid());
            for (dx, dz) in [(-0.52, -0.27), (0.52, -0.27), (-0.52, 0.27), (0.52, 0.27)] {
                g.add(Cube::new("TableLeg", vec3(0.07, 0.72, 0.07), o + vec3(dx, 0.36, dz)).kind("wood").tint(tint));
            }
        }
        "bench" => {
            g.add(Cube::new("BenchSeat", vec3(1.4, 0.08, 0.4), o + vec3(0.0, 0.45, 0.0)).kind("wood").tint(tint).solid());
            g.add(Cube::new("BenchLeg", vec3(0.08, 0.42, 0.34), o + vec3(-0.6, 0.21, 0.0)).kind("metal"));
            g.add(Cube::new("BenchLeg", vec3(0.08, 0.42, 0.34), o + vec3(0.6, 0.21, 0.0)).kind("metal"));
        }
        "bed" => {
            g.add(Cube::new("BedFrame", vec3(1.0, 0.35, 2.0), o + vec3(0.0, 0.175, 0.0)).kind("wood").solid());
            g.add(Cube::new("BedMat", vec3(0.94, 0.14, 1.9), o + vec3(0.0, 0.42, 0.0)).kind("fabric").tint(tint));
            g.add(Cube::new("BedPillow", vec3(0.6, 0.1, 0.35), o + vec3(0.0, 0.54, -0.7)).kind("paper"));
        }
        "shelf" => {
            g.add(Cube::new("ShelfFrame", vec3(1.0, 1.8, 0.35), o + vec3(0.0, 0.9, 0.0)).kind("wood").tint(tint).solid());
            for y in [0.45, 0.9, 1.35] {
                g.add(Cube::new("ShelfBoard", vec3(0.96, 0.04, 0.36), o + vec3(0.0, y, 0.0)).kind("metal"));
            }
        }
        "locker" => {
            g.add(Cube::new("LockerBody", vec3(0.5, 1.9, 0.5), o + vec3(0.0, 0.95, 0.0)).kind("metal").tint(tint).solid());
            g.add(Cube::new("LockerHandle", vec3(0.04, 0.2, 0.04), o + vec3(0.18, 1.0, 0.27)).kind("plastic"));
        }
        "chair" => {
            g.add(Cube::new("ChairSeat", vec3(0.45, 0.08, 0.45), o + vec3(0.0, 0.45, 0.0)).kind("fabric").tint(tint));
            g.add(Cube::new("ChairBack", vec3(0.45, 0.5, 0.06), o + vec3(0.0, 0.74, 0.2)).kind("fabric").tint(tint));
            g.add(Cube::new("ChairLeg", vec3(0.06, 0.42, 0.06), o + vec3(0.0, 0.21, 0.0)).kind("metal"));
        }
        "barrel" => {
            g.add(Cube::new("BarrelBody", vec3(0.55, 0.9, 0.55), o + vec3(0.0, 0.45, 0.0)).kind("metal").tint(tint).solid());
            g.add(Cube::new("BarrelBand", vec3(0.58, 0.06, 0.58), o + vec3(0.0, 0.6, 0.0)).kind("plastic"));
        }
        "bin" => {
            g.add(Cube::new("BinBody", vec3(0.35, 0.5, 0.35), o + vec3(0.0, 0.25, 0.0)).kind("plastic").tint(tint));
        }
        "lamp" => {
            g.add(Cube::new("LampPole", vec3(0.06, 1.5, 0.06), o + vec3(0.0, 0.75, 0.0)).kind("metal"));
            g.add(Cube::new("LampShade", vec3(0.3, 0.2, 0.3), o + vec3(0.0, 1.6, 0.0)).tint(tint));
        }
        _ => {
            g.add(Cube::new("CrateBox", vec3(0.8, 0.8, 0.8), o + vec3(0.0, 0.4, 0.0)).kind("wood").tint(tint).solid());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> Group {
        Group::new("T", Vec3::ZERO)
    }

    #[test]
    fn a_wall_without_gaps_is_one_solid_cube() {
        let mut g = group();
        wall_with_gaps(&mut g, true, 0.0, -3.0, 3.0, &[], 3.0, 0.2, [1.0; 3], "Wall");
        assert_eq!(g.cubes.len(), 1);
        assert!(g.cubes[0].solid);
        assert_eq!(g.cubes[0].size, vec3(6.0, 3.0, 0.2));
        assert_eq!(g.cubes[0].pos, vec3(0.0, 1.5, 0.0));
    }

    #[test]
    fn gaps_split_the_wall_and_get_a_lintel() {
        let mut g = group();
        wall_with_gaps(&mut g, false, 2.0, -4.0, 4.0, &[(-1.0, 1.0)], 3.6, 0.2, [1.0; 3], "Wall");
        let solid: Vec<_> = g.cubes.iter().filter(|c| c.solid).collect();
        assert_eq!(solid.len(), 2);
        assert_eq!(solid[0].size, vec3(0.2, 3.6, 3.0));
        assert_eq!(solid[0].pos.z, -2.5);
        assert_eq!(solid[1].pos.z, 2.5);
        let lintel: Vec<_> = g.cubes.iter().filter(|c| !c.solid).collect();
        assert_eq!(lintel.len(), 1);
        assert!((lintel[0].size.y - 1.0).abs() < 1e-5);
        assert!((lintel[0].pos.y - 3.1).abs() < 1e-5);
        assert_eq!(lintel[0].pos.x, 2.0);
    }

    #[test]
    fn overlapping_and_out_of_range_gaps_are_clamped() {
        let mut g = group();
        wall_with_gaps(&mut g, true, 0.0, 0.0, 10.0, &[(-5.0, 1.0), (0.5, 2.0), (9.0, 20.0)], 2.5, 0.2, [1.0; 3], "Wall");
        let solid: Vec<_> = g.cubes.iter().filter(|c| c.solid).collect();
        assert_eq!(solid.len(), 1);
        assert!((solid[0].size.x - 7.0).abs() < 1e-5, "2..9 remains");
        assert_eq!(g.cubes.len(), 1, "a 2.5 m wall needs no lintel");
    }

    #[test]
    fn a_shell_has_floor_four_walls_and_optional_ceiling() {
        let mut g = group();
        room_shell(&mut g, 0.0, 6.0, 0.0, 5.0, 3.6, [1.0; 3], false);
        assert_eq!(g.cubes.len(), 5);
        let mut h = group();
        room_shell(&mut h, 0.0, 6.0, 0.0, 5.0, 3.6, [1.0; 3], true);
        assert_eq!(h.cubes.len(), 6);
        assert!(h.cubes.iter().any(|c| c.name == "Ceiling" && c.pos.y > 3.6));
        assert_eq!(g.cubes[0].kind, "floor");
    }

    #[test]
    fn stairs_rise_by_the_step_height() {
        let mut g = group();
        stairs(&mut g, vec3(0.0, 0.0, 0.0), 4, 0.2, 0.3, 1.2, [1.0; 3]);
        assert_eq!(g.cubes.len(), 4);
        let tops: Vec<f32> = g.cubes.iter().map(|c| c.pos.y + c.size.y * 0.5).collect();
        for (i, t) in tops.iter().enumerate() {
            assert!((t - 0.2 * (i + 1) as f32).abs() < 1e-5);
        }
        assert!(g.cubes[3].pos.z < g.cubes[0].pos.z, "climbs toward -Z");
        assert!(g.cubes.iter().all(|c| c.solid));
    }

    #[test]
    fn every_prop_kind_adds_cubes_with_at_least_two_looks() {
        for k in ["table", "bench", "bed", "shelf", "locker", "chair", "barrel", "lamp", "crate", "unknown"] {
            let mut g = group();
            blocky_prop(&mut g, k, Vec3::ZERO, [1.0; 3]);
            assert!(!g.cubes.is_empty(), "{k}");
            assert!(g.cubes.iter().all(|c| !c.kind.is_empty()), "{k}");
        }
    }
}
