#![allow(dead_code)]
//! The 2D game's office colours and signature pieces on the 3D office (port of night_watch_palette.gd and the classic, mixed
//! and day-shift parts of night_watch_signature.gd).
//!
//! Both passes work on `Vec<Group>` before the world is baked, so they are pure (no GL):
//!  * `apply` re-tints wall, floor and ceiling cubes with the office palette and lays the checkerboard floor on top of each
//!    cube named exactly "Floor" (the Godot FloorChecker plane; here real thin cubes, because a cube only carries a kind and a tint).
//!  * `signature` adds the orange and checker bands, the five posters and the warm ceiling lamps (classic), the lamp boards
//!    (hub) or the red rails, tile lines and posters (cargo bay).

use macroquad::prelude::Vec3;

use super::mesh::Group;
use super::office::{mul3, put, O};

pub struct PaletteDef {
    pub id: &'static str,
    pub wall: [u8; 3],
    pub floor: [u8; 3],
    pub ceiling: [u8; 3],
    pub trim: [u8; 3],
}

pub const PALETTES: [PaletteDef; 9] = [
    PaletteDef { id: "classic_security", wall: [32, 62, 78], floor: [24, 34, 50], ceiling: [50, 79, 92], trim: [224, 122, 48] },
    PaletteDef { id: "front_office", wall: [50, 38, 104], floor: [28, 22, 73], ceiling: [13, 10, 41], trim: [210, 70, 120] },
    PaletteDef { id: "broadcast_booth", wall: [20, 54, 78], floor: [16, 34, 52], ceiling: [41, 78, 91], trim: [224, 122, 48] },
    PaletteDef { id: "child_bedroom", wall: [38, 30, 82], floor: [47, 31, 67], ceiling: [66, 54, 68], trim: [240, 220, 120] },
    PaletteDef { id: "control_room", wall: [33, 50, 77], floor: [23, 37, 59], ceiling: [52, 75, 103], trim: [232, 190, 40] },
    PaletteDef { id: "lift_landing", wall: [36, 39, 51], floor: [29, 31, 40], ceiling: [77, 69, 61], trim: [232, 190, 40] },
    PaletteDef { id: "cargo_bay", wall: [190, 135, 92], floor: [86, 90, 72], ceiling: [240, 205, 150], trim: [190, 60, 50] },
    PaletteDef { id: "watch_post", wall: [25, 31, 50], floor: [24, 29, 45], ceiling: [34, 41, 79], trim: [110, 130, 200] },
    PaletteDef { id: "hub_office", wall: [40, 52, 62], floor: [24, 35, 51], ceiling: [59, 70, 78], trim: [224, 122, 48] },
];

pub const GAIN: f32 = 3.0;
pub const MAX_TINT: f32 = 1.5;
/// The warm orange of the signature bands.
pub const ORANGE: [f32; 3] = [1.5, 0.62, 0.2];

pub fn palette(office_id: &str) -> Option<&'static PaletteDef> {
    PALETTES.iter().find(|p| p.id == office_id)
}

fn unit(c: [u8; 3]) -> [f32; 3] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

/// Tint for "wall", "floor" or "ceiling" of an office: min(colour * GAIN, MAX_TINT); white if unknown.
pub fn tint_for(office_id: &str, part: &str) -> [f32; 3] {
    let p = match palette(office_id) {
        Some(p) => p,
        None => return [1.0, 1.0, 1.0],
    };
    let c = match part {
        "wall" => p.wall,
        "floor" => p.floor,
        "ceiling" => p.ceiling,
        "trim" => p.trim,
        _ => return [1.0, 1.0, 1.0],
    };
    let u = unit(c);
    [(u[0] * GAIN).min(MAX_TINT), (u[1] * GAIN).min(MAX_TINT), (u[2] * GAIN).min(MAX_TINT)]
}

/// The mid tone of a material kind's ramp, used to turn a wanted flat colour into a tint (the textures are not flat).
fn ref_tone(kind: &str) -> [f32; 3] {
    let c: [u8; 3] = match kind {
        "wall" => [0x58, 0x5f, 0x70],
        "floor" => [0x4b, 0x46, 0x50],
        "paper" => [0xc9, 0xc2, 0xae],
        "fabric" => [0x63, 0x3d, 0x4c],
        _ => [0x80, 0x80, 0x80],
    };
    unit(c)
}

/// The tint that makes a cube of `kind` read as the flat colour `rgb` (0..255), capped at 3.
pub fn flat_tint(kind: &str, rgb: [u8; 3]) -> [f32; 3] {
    let r = ref_tone(kind);
    let u = unit(rgb);
    [(u[0] / r[0]).min(3.0), (u[1] / r[1]).min(3.0), (u[2] / r[2]).min(3.0)]
}

/// Which palette part a cube belongs to, if any: the slab named exactly "Floor" (any kind), wall-kind cubes, floor-kind cubes,
/// and concrete cubes whose name contains "ceiling". Hazard tape is never re-tinted.
pub fn part_of(name: &str, kind: &str, hazard: &str) -> Option<&'static str> {
    if !hazard.is_empty() {
        return None;
    }
    if name == "Floor" {
        Some("floor")
    } else if kind == "wall" {
        Some("wall")
    } else if kind == "floor" {
        Some("floor")
    } else if kind == "concrete" && name.to_lowercase().contains("ceiling") {
        Some("ceiling")
    } else {
        None
    }
}

/// Tint the shell of an office and lay the checker floor. Returns the number of re-tinted cubes. Call once per group list.
pub fn apply(groups: &mut Vec<Group>, office_id: &str) -> usize {
    let pal = match palette(office_id) {
        Some(p) => p,
        None => return 0,
    };
    let mut changed = 0;
    let mut floors: Vec<(Vec3, Vec3)> = Vec::new();
    for g in groups.iter_mut() {
        let origin = g.origin;
        for c in g.cubes.iter_mut() {
            let part = match part_of(&c.name, &c.kind, &c.hazard) {
                Some(p) => p,
                None => continue,
            };
            c.tint = mul3(c.tint, tint_for(office_id, part));
            changed += 1;
            if part == "floor" && c.name == "Floor" {
                floors.push((origin + c.pos, c.size));
            }
        }
    }
    if !floors.is_empty() {
        let mut checker = Group::new("FloorChecker", Vec3::ZERO);
        for (centre, size) in floors {
            checker_floor(&mut checker, centre, size, pal.floor);
        }
        groups.push(checker);
    }
    changed
}

/// The 2D floors are checkerboards: the floor colour lightened and a darker one, a metre a square. A light slab with dark
/// squares on it, on top of the floor slab.
pub fn checker_floor(g: &mut Group, centre: Vec3, size: Vec3, base: [u8; 3]) {
    let b = unit(base);
    let light = [(b[0] * 1.9).min(1.0), (b[1] * 1.9).min(1.0), (b[2] * 1.9).min(1.0)];
    let dark = [b[0] * 1.1, b[1] * 1.1, b[2] * 1.1];
    let to8 = |c: [f32; 3]| [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
    let light_t = flat_tint("floor", to8(light));
    let dark_t = flat_tint("floor", to8(dark));
    let top = centre.y + size.y * 0.5;
    put(
        g,
        "FloorCheckerLight",
        [size.x, 0.008, size.z],
        [centre.x, top + 0.004, centre.z],
        O::new().k("floor").t(light_t).ao(1.0),
    );
    let x0 = centre.x - size.x * 0.5;
    let z0 = centre.z - size.z * 0.5;
    let nx = size.x.ceil() as i32;
    let nz = size.z.ceil() as i32;
    for i in 0..nx {
        for j in 0..nz {
            if (i + j) % 2 == 1 {
                let tw = (size.x - i as f32).min(1.0);
                let td = (size.z - j as f32).min(1.0);
                put(
                    g,
                    "FloorCheckerDark",
                    [tw, 0.010, td],
                    [x0 + i as f32 + tw * 0.5, top + 0.005, z0 + j as f32 + td * 0.5],
                    O::new().k("floor").t(dark_t).ao(1.0),
                );
            }
        }
    }
}

// ---- signature pieces --------------------------------------------------------------------------------------------------------

/// Add the office's signature groups. Unknown office ids add nothing.
pub fn signature(groups: &mut Vec<Group>, office_id: &str) {
    match office_id {
        "classic_security" => classic(groups),
        "hub_office" => {
            classic(groups);
            mixed(groups);
        }
        "cargo_bay" => dayshift(groups),
        _ => {}
    }
}

const HW: f32 = super::office::HALF_W;
const HD: f32 = super::office::HALF_D;
const DOOR_HALF: f32 = super::office::DOOR_W * 0.5;

/// Classic (and the Hub): the orange band over a navy-and-white checker band round the walls, five posters, warm ceiling lamps.
fn classic(groups: &mut Vec<Group>) {
    let mut root = Group::new("Signature", Vec3::ZERO);
    // (centre x, centre z, length, runs along x)
    let seg_len = HD - DOOR_HALF - 0.1;
    let segments: [(f32, f32, f32, bool); 5] = [
        (0.0, HD - 0.05, HW * 2.0 - 0.2, true),
        (-HW + 0.05, -(HD + DOOR_HALF) * 0.5, seg_len, false),
        (-HW + 0.05, (HD + DOOR_HALF) * 0.5, seg_len, false),
        (HW - 0.05, -(HD + DOOR_HALF) * 0.5, seg_len, false),
        (HW - 0.05, (HD + DOOR_HALF) * 0.5, seg_len, false),
    ];
    let navy = flat_tint("wall", [24, 38, 62]);
    let pale = flat_tint("wall", [190, 205, 215]);
    for (cx, cz, length, along_x) in segments {
        let band = if along_x { [length, 0.1, 0.03] } else { [0.03, 0.1, length] };
        put(&mut root, "OrangeBand", band, [cx, 1.3, cz], O::new().k("wall").t(ORANGE));
        // the checker band: 0.2 m squares alternating navy and pale
        let n = (length / 0.2).ceil() as i32;
        for i in 0..n {
            let start = i as f32 * 0.2;
            let len = (length - start).min(0.2);
            let off = -length * 0.5 + start + len * 0.5;
            let (size, pos) = if along_x { ([len, 0.2, 0.03], [cx + off, 1.12, cz]) } else { ([0.03, 0.2, len], [cx, 1.12, cz + off]) };
            put(&mut root, "CheckerBand", size, pos, O::new().k("wall").t(if i % 2 == 0 { navy } else { pale }));
        }
    }
    // the five posters on the front wall, over the bands: (x, width, height, picture, background)
    let specs: [(f32, f32, f32, &str, [u8; 3]); 5] = [
        (-1.7, 0.5, 0.68, "star", [110, 60, 160]),
        (-0.95, 0.52, 0.72, "balloons", [190, 50, 50]),
        (0.0, 0.8, 1.0, "triangle", [190, 50, 60]),
        (0.95, 0.52, 0.72, "face", [60, 130, 80]),
        (1.7, 0.5, 0.68, "star2", [190, 120, 40]),
    ];
    for (x, w, h, kind, base) in specs {
        put(&mut root, &format!("PosterFrame{kind}"), [w + 0.06, h + 0.06, 0.03], [x, 1.85, HD - 0.05], O::new().k("wall").t([0.4, 0.3, 0.2]));
        poster(&mut root, kind, base, x, 1.85, HD - 0.075, w, h);
    }
    groups.push(root);
    // warm yellow ceiling lamps
    for g in groups.iter_mut() {
        for c in g.cubes.iter_mut() {
            if c.name == "CeilingLens" {
                c.kind = "light".to_string();
                c.tint = [1.5, 1.3, 0.6];
            }
        }
    }
}

/// Mixed Shift: the amber framed lamp boards on both side walls by the back corners.
fn mixed(groups: &mut Vec<Group>) {
    let mut root = Group::new("SignatureMixed", Vec3::ZERO);
    for side in [-1.0f32, 1.0] {
        let x = side * (HW - 0.05);
        put(&mut root, "BoardFrame", [0.03, 0.8, 0.56], [x, 1.75, -1.15], O::new().k("wall").t([0.5, 0.32, 0.18]));
        put(&mut root, "BoardLamp", [0.04, 0.32, 0.16], [x - side * 0.01, 1.75, -1.15], O::new().k("light").t([2.0, 1.4, 0.4]));
    }
    groups.push(root);
}

/// Day shift (cargo bay): a red rail round the room, tile lines on the ceiling, a bright ceiling panel and two posters.
fn dayshift(groups: &mut Vec<Group>) {
    let mut root = Group::new("Signature", Vec3::ZERO);
    let height = super::office::HEIGHT;
    let red = [1.8, 0.28, 0.25];
    for side in [-1.0f32, 1.0] {
        put(&mut root, "RedRail", [0.03, 0.1, HD * 2.0 - 0.2], [side * (HW - 0.04), 1.0, 0.0], O::new().k("wall").t(red));
    }
    put(&mut root, "RedRail", [HW * 2.0 - 0.2, 0.1, 0.03], [0.0, 1.0, HD - 0.04], O::new().k("wall").t(red));
    put(&mut root, "RedRail", [HW * 2.0 - 0.2, 0.1, 0.03], [0.0, 1.0, -HD + 0.04], O::new().k("wall").t(red));
    let tile = [1.6 * 0.6, 1.5 * 0.6, 1.25 * 0.6];
    for i in -(HW as i32)..=(HW as i32) {
        put(&mut root, "CeilingTileLine", [0.03, 0.02, HD * 2.0 - 0.2], [i as f32, height - 0.03, 0.0], O::new().k("wall").t(tile));
    }
    for i in -(HD as i32)..=(HD as i32) {
        put(&mut root, "CeilingTileLine", [HW * 2.0 - 0.2, 0.02, 0.03], [0.0, height - 0.03, i as f32], O::new().k("wall").t(tile));
    }
    put(&mut root, "CeilingPanelDay", [1.6, 0.04, 0.5], [0.0, height - 0.03, 0.0], O::new().k("light").t([2.2, 2.0, 1.5]));
    // two posters either side of the window wall
    for (p, rgb) in [(-0.5f32, [60u8, 130, 80]), (0.5f32, [235u8, 200, 150])] {
        let x = p + (HW - 0.6) * if p > 0.0 { 1.0 } else { -1.0 };
        put(&mut root, "PosterFrameDay", [0.5, 0.7, 0.03], [x, 1.9, HD - 0.05], O::new().k("wall").t([0.45, 0.3, 0.2]));
        let t = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
        put(&mut root, "PosterDay", [0.42, 0.62, 0.02], [x, 1.9, HD - 0.07], O::new().k("fabric").t(t));
    }
    groups.push(root);
}

// ---- poster pictures ---------------------------------------------------------------------------------------------------------

pub const POSTER_W: usize = 16;
pub const POSTER_H: usize = 20;

/// The painted 16 x 20 picture of a poster as pixels; None keeps the background colour. Row 0 is the top.
pub fn poster_bitmap(kind: &str) -> Vec<Option<[u8; 3]>> {
    let cream = [240u8, 225, 170];
    let gold = [235u8, 190, 60];
    let mut px: Vec<Option<[u8; 3]>> = vec![None; POSTER_W * POSTER_H];
    let mut set = |x: i32, y: i32, c: [u8; 3]| {
        if x >= 0 && y >= 0 && (x as usize) < POSTER_W && (y as usize) < POSTER_H {
            px[y as usize * POSTER_W + x as usize] = Some(c);
        }
    };
    match kind {
        "star" | "star2" => {
            for i in 3..13 {
                set(8, i + 2, gold);
                set(i, 10, gold);
            }
            for i in 5..11 {
                set(i, 8, gold);
                set(i, 12, gold);
            }
        }
        "balloons" => {
            for (cx, cy) in [(5i32, 6i32), (10, 6), (8, 9)] {
                for dx in -2i32..=2 {
                    for dy in -2i32..=2 {
                        if dx.abs() + dy.abs() <= 3 {
                            set(cx + dx, cy + dy, if cy == 6 { cream } else { gold });
                        }
                    }
                }
            }
            for y in 12..19 {
                set(8, y, [60, 40, 30]);
            }
        }
        "triangle" => {
            for y in 4..16 {
                let half = ((y - 4) as f32 * 0.55) as i32;
                for x in (8 - half)..(9 + half) {
                    set(x.clamp(0, 15), y, cream);
                }
            }
            for x in 5..11 {
                set(x, 17, [30, 20, 20]);
            }
        }
        "face" => {
            for dx in -4i32..=4 {
                for dy in -4i32..=4 {
                    if dx * dx + dy * dy <= 17 {
                        set(8 + dx, 9 + dy, [150, 220, 130]);
                    }
                }
            }
            set(6, 8, [30, 50, 30]);
            set(10, 8, [30, 50, 30]);
            for x in 6..11 {
                set(x, 11, [30, 50, 30]);
            }
        }
        _ => {}
    }
    let edge = [cream[0] / 2, cream[1] / 2, cream[2] / 2];
    for x in 0..POSTER_W as i32 {
        set(x, 0, edge);
        set(x, 19, edge);
    }
    px
}

/// Horizontal runs of equal colour per row: (row, first column, run length, colour).
pub fn poster_runs(kind: &str) -> Vec<(usize, usize, usize, [u8; 3])> {
    let px = poster_bitmap(kind);
    let mut out = Vec::new();
    for y in 0..POSTER_H {
        let mut x = 0;
        while x < POSTER_W {
            match px[y * POSTER_W + x] {
                None => x += 1,
                Some(c) => {
                    let mut e = x + 1;
                    while e < POSTER_W && px[y * POSTER_W + e] == Some(c) {
                        e += 1;
                    }
                    out.push((y, x, e - x, c));
                    x = e;
                }
            }
        }
    }
    out
}

/// A poster on the front wall facing -z (into the room): background panel plus one thin cube per pixel run. The picture is
/// mirrored in x because the Godot quad faces the room rotated by PI.
fn poster(g: &mut Group, kind: &str, base: [u8; 3], cx: f32, cy: f32, z: f32, w: f32, h: f32) {
    put(g, &format!("Poster{kind}"), [w, h, 0.01], [cx, cy, z], O::new().k("paper").t(flat_tint("paper", base)).ao(1.0));
    for (row, x0, len, c) in poster_runs(kind) {
        let mid = x0 as f32 + len as f32 * 0.5;
        let x = cx - (mid - POSTER_W as f32 * 0.5) / POSTER_W as f32 * w;
        let y = cy + h * 0.5 - (row as f32 + 0.5) / POSTER_H as f32 * h;
        put(
            g,
            "PosterPixel",
            [len as f32 / POSTER_W as f32 * w, h / POSTER_H as f32, 0.01],
            [x, y, z - 0.008],
            O::new().k("paper").t(flat_tint("paper", c)).ao(1.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec3;

    fn cube<'a>(groups: &'a [Group], gname: &str, cname: &str) -> Option<&'a super::super::mesh::Cube> {
        groups.iter().filter(|g| g.name == gname).flat_map(|g| g.cubes.iter()).find(|c| c.name == cname)
    }

    #[test]
    fn tint_for_applies_gain_and_cap() {
        let t = tint_for("classic_security", "wall");
        assert!((t[0] - 32.0 / 255.0 * 3.0).abs() < 1e-5);
        assert!((t[2] - 78.0 / 255.0 * 3.0).abs() < 1e-5);
        // 224 * 3 / 255 is above the cap
        let trim = tint_for("classic_security", "trim");
        assert_eq!(trim[0], MAX_TINT);
        assert_eq!(tint_for("nope", "wall"), [1.0, 1.0, 1.0]);
        assert_eq!(tint_for("classic_security", "nope"), [1.0, 1.0, 1.0]);
        assert_eq!(PALETTES.len(), 9);
    }

    #[test]
    fn part_rules_match_godot() {
        assert_eq!(part_of("Floor", "carpet", ""), Some("floor"));
        assert_eq!(part_of("HallLFloor", "floor", ""), Some("floor"));
        assert_eq!(part_of("WallBack", "wall", ""), Some("wall"));
        assert_eq!(part_of("Ceiling", "concrete", ""), Some("ceiling"));
        assert_eq!(part_of("PillarBody", "concrete", ""), None);
        assert_eq!(part_of("FloorStripe", "hazard", "y"), None);
        assert_eq!(part_of("DeskTop", "wood", ""), None);
    }

    #[test]
    fn apply_tints_the_shell_once_and_adds_a_checker() {
        use super::super::office::{Office, OfficeConfig};
        let raw = Office::parts(&OfficeConfig { palette: false, signature: false, ..OfficeConfig::default() });
        let tinted = Office::parts(&OfficeConfig { signature: false, ..OfficeConfig::default() });
        let before = cube(&raw.groups, "Shell", "WallBack").map(|c| c.tint).unwrap_or([0.0; 3]);
        let after = cube(&tinted.groups, "Shell", "WallBack").map(|c| c.tint).unwrap_or([0.0; 3]);
        let t = tint_for("classic_security", "wall");
        for i in 0..3 {
            assert!((after[i] - before[i] * t[i]).abs() < 1e-5);
        }
        // props are untouched
        let d0 = cube(&raw.groups, "Desk", "DeskTop").map(|c| c.tint);
        let d1 = cube(&tinted.groups, "Desk", "DeskTop").map(|c| c.tint);
        assert_eq!(d0, d1);
        // the checker exists and covers the floor slab (6.4 x 5.4)
        let checker = tinted.groups.iter().find(|g| g.name == "FloorChecker").map(|g| g.cubes.len()).unwrap_or(0);
        assert!(checker > 10);
        assert!(raw.groups.iter().all(|g| g.name != "FloorChecker"));
    }

    #[test]
    fn checker_squares_alternate_and_fit() {
        let mut g = Group::new("c", Vec3::ZERO);
        checker_floor(&mut g, vec3(0.0, -0.1, 0.0), vec3(3.0, 0.2, 2.0), [24, 34, 50]);
        // one light slab plus 3 dark squares of the 6 (i + j odd)
        assert_eq!(g.cubes.len(), 1 + 3);
        for c in &g.cubes {
            assert!(c.pos.x - c.size.x * 0.5 >= -1.5 - 1e-5 && c.pos.x + c.size.x * 0.5 <= 1.5 + 1e-5);
            assert!(c.pos.z - c.size.z * 0.5 >= -1.0 - 1e-5 && c.pos.z + c.size.z * 0.5 <= 1.0 + 1e-5);
            assert!(c.pos.y > 0.0, "above the floor top");
        }
    }

    #[test]
    fn classic_signature_has_bands_posters_and_warm_lamps() {
        use super::super::office::{Office, OfficeConfig};
        let p = Office::parts(&OfficeConfig::default());
        let sig = p.groups.iter().find(|g| g.name == "Signature").expect("signature group");
        assert_eq!(sig.cubes.iter().filter(|c| c.name == "OrangeBand").count(), 5);
        assert!(sig.cubes.iter().filter(|c| c.name == "CheckerBand").count() > 20);
        assert_eq!(sig.cubes.iter().filter(|c| c.name.starts_with("PosterFrame")).count(), 5);
        assert!(sig.cubes.iter().any(|c| c.name == "PosterPixel"));
        let lens = cube(&p.groups, "Shell", "CeilingLens").expect("lens");
        assert_eq!(lens.kind, "light");
        assert_eq!(lens.tint, [1.5, 1.3, 0.6]);
        // the hub adds the lamp boards, the cargo bay gets rails
        let hub = Office::parts(&OfficeConfig { variant: super::super::office::OfficeVariant::Hub, ..OfficeConfig::default() });
        assert!(hub.groups.iter().any(|g| g.name == "SignatureMixed"));
        let cargo = Office::parts(&OfficeConfig { variant: super::super::office::OfficeVariant::Cargo, ..OfficeConfig::default() });
        let sig = cargo.groups.iter().find(|g| g.name == "Signature").expect("cargo signature");
        assert!(sig.cubes.iter().any(|c| c.name == "RedRail"));
        assert!(sig.cubes.iter().any(|c| c.name == "CeilingPanelDay"));
    }

    #[test]
    fn poster_bitmaps_follow_the_godot_drawing() {
        let tri = poster_bitmap("triangle");
        assert!(tri[4 * POSTER_W + 8].is_some(), "apex row 4 at x 8");
        assert!(tri[4 * POSTER_W + 7].is_none());
        assert!(tri[15 * POSTER_W + 8 - 5].is_some(), "row 15 is 8 pixels wider each side: half = 6");
        assert_eq!(tri[17 * POSTER_W + 5], Some([30, 20, 20]));
        // the border rows are cream * 0.5
        assert_eq!(tri[0], Some([120, 112, 85]));
        assert_eq!(tri[19 * POSTER_W + 15], Some([120, 112, 85]));
        let face = poster_bitmap("face");
        assert_eq!(face[8 * POSTER_W + 6], Some([30, 50, 30]));
        assert_eq!(face[9 * POSTER_W + 8], Some([150, 220, 130]));
        let runs = poster_runs("star");
        assert!(!runs.is_empty());
        for (row, x0, len, _) in runs {
            assert!(row < POSTER_H && x0 + len <= POSTER_W && len >= 1);
        }
    }
}
