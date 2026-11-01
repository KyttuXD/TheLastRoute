//! Port of the original's procedural pixel-art surfaces (game/look/ref_textures.gd, `RefTextures`) and of the tables
//! that pick them per district / room purpose (ref_look.gd, floor_sets.gd, data/themes/**, data/regions/*.json).
//!
//! Every texel is painted from a seeded Godot `RandomNumberGenerator` (PCG32, seeded with `String.hash()` of a key such
//! as "metal0"), so the textures are byte-identical to the Godot game's (verified against Godot 4.7.2 dumps, NOTES.md).
//! 32 texels per metre, sampled nearest-neighbour with repeat.
//!
//! * floors / ceilings: 32 x 32, one texture per 1 m grid cell (u = world x, v = world z).
//! * walls: 32 x 128, ONE storey of wall (cornice at row 0, skirting at the bottom rows), stretched over the room
//!   height; see `wall_v` and NOTES.md.
//!
//! Usage: `paint("wall", "paper0", 0)` or `paint("wall", "paper", 0)` -> `(32, 128, rgba8)`.
//! Not yet wired into the crate: add `pub mod textures;` to src/render/mod.rs to use it.

#![allow(dead_code)]

pub mod beach;
pub mod canvas;
pub mod ceilings;
pub mod ducts;
pub mod floors;
pub mod rng;
pub mod sets;
pub mod walls;

pub use canvas::{Img, PX, WALL_H};
pub use sets::{region_set, TexRef, TextureChoice};

use beach::*;
use ceilings::*;
use floors::*;
use walls::*;

type Painter = fn(i32) -> Img;

/// RefTextures.FLOOR_PAINTERS in its declaration order (= the layer order of the original's floor texture array,
/// which is FLOOR_PAINTERS then CEILING_PAINTERS): (kind key, painter, variant passed to the painter).
pub const FLOOR_PAINTERS: &[(&str, Painter, i32)] = &[
    ("metal0", paint_metal, 0),
    ("metal1", paint_metal, 1),
    ("metal2", paint_metal, 2),
    ("metal3", paint_metal, 3),
    ("floral0", paint_floral, 0),
    ("floral1", paint_floral, 1),
    ("star0", paint_star, 0),
    ("star1", paint_star, 1),
    ("parquet0", paint_parquet, 0),
    ("parquet1", paint_parquet, 1),
    ("stone0", paint_stone, 0),
    ("stone1", paint_stone, 1),
    ("stone2", paint_stone, 2),
    ("check0", paint_check, 0),
    ("check1", paint_check, 1),
    ("hazard", paint_hazard, 0),
    ("runner", paint_runner, 0),
    ("grate", paint_grate, 0),
    ("runnerz", paint_runner_z, 0),
    ("safe", paint_safe, 0),
    ("safe_plain", paint_safe_plain, 0),
    ("safe_mark", paint_safe_mark, 0),
    ("runner_l", runner_l, 0),
    ("runner_c", runner_c, 0),
    ("runner_r", runner_r, 0),
    ("carpet_red", paint_carpet, 0),
    ("carpet_teal", paint_carpet, 1),
    ("carpet_green", paint_carpet, 2),
    ("carpet_plum", paint_carpet, 3),
    ("carpet_tile0", paint_carpet_tile, 0),
    ("carpet_tile1", paint_carpet_tile, 1),
    ("vinyl0", paint_vinyl, 0),
    ("vinyl1", paint_vinyl, 1),
    ("concrete0", paint_concrete, 0),
    ("concrete1", paint_concrete, 1),
    ("runnerz_l", runnerz_l, 0),
    ("runnerz_c", runnerz_c, 0),
    ("runnerz_r", runnerz_r, 0),
    ("terrazzo0", paint_terrazzo, 0),
    ("terrazzo1", paint_terrazzo, 1),
    ("epoxy0", paint_epoxy, 0),
    ("epoxy1", paint_epoxy, 1),
    ("terracotta0", paint_terracotta, 0),
    ("terracotta1", paint_terracotta, 1),
    ("rock0", paint_rock_floor, 0),
    ("rock1", paint_rock_floor, 1),
    ("rock2", paint_rock_floor, 2),
    ("sand0", paint_sand, 0),
    ("sand1", paint_sand, 1),
    ("wetsand0", paint_sand, 2),
    ("driftwood0", paint_driftwood, 0),
    ("driftwood1", paint_driftwood, 1),
];

pub const WALL_PAINTERS: &[(&str, Painter, i32)] = &[
    ("panel0", paint_wall_panel, 0),
    ("panel1", paint_wall_panel, 1),
    ("panel2", paint_wall_panel, 2),
    ("paper0", paint_wall_wallpaper, 0),
    ("paper1", paint_wall_wallpaper, 1),
    ("brick0", paint_wall_brick, 0),
    ("brick1", paint_wall_brick, 1),
    ("stone0", paint_wall_stone, 0),
    ("stone1", paint_wall_stone, 1),
    ("hedge0", paint_wall_hedge, 0),
    ("hedge1", paint_wall_hedge, 1),
    ("office0", paint_wall_office, 0),
    ("office1", paint_wall_office, 1),
    ("corridor0", paint_wall_corridor, 0),
    ("corridor1", paint_wall_corridor, 1),
    ("inferno0", paint_wall_inferno, 0),
    ("rockwall0", paint_wall_rock, 0),
    ("rockwall1", paint_wall_rock, 1),
    ("rockwall2", paint_wall_rock, 2),
    ("rockwall3", paint_wall_rock, 3),
    ("rockwall4", paint_wall_rock, 4),
    ("rockwall5", paint_wall_rock, 5),
    ("rockwall6", paint_wall_rock, 6),
    ("rockwall7", paint_wall_rock, 7),
    ("hospital0", paint_wall_hospital, 0),
    ("hospital1", paint_wall_hospital, 1),
    ("lab0", paint_wall_lab, 0),
    ("lab1", paint_wall_lab, 1),
    ("mall0", paint_wall_mall, 0),
    ("mall1", paint_wall_mall, 1),
    ("glasshouse0", paint_wall_glasshouse, 0),
    ("glasshouse1", paint_wall_glasshouse, 1),
    ("seawall0", paint_wall_seawall, 0),
    ("seawall1", paint_wall_seawall, 1),
    ("whitewash0", paint_wall_whitewash, 0),
    ("whitewash1", paint_wall_whitewash, 1),
    ("tealpanel0", paint_wall_tealpanel, 0),
    ("tealpanel1", paint_wall_tealpanel, 1),
];

pub const CEILING_PAINTERS: &[(&str, Painter, i32)] = &[
    ("ceil0", paint_ceiling, 0),
    ("ceil1", paint_ceiling, 1),
    ("glass", paint_glass, 0),
    ("rockceil", paint_rock_ceiling, 0),
    ("vault", paint_vault, 0),
    ("skylight0", paint_skylight, 0),
];

/// RefTextures.DUCT_PAINTERS (the painters take no variant).
pub const DUCT_PAINTERS: &[(&str, Painter, i32)] = &[
    ("sheet", duct_sheet, 0),
    ("flange", duct_flange, 0),
    ("grille", duct_grille, 0),
];

fn runner_l(_: i32) -> Img {
    paint_runner_part(0, false)
}
fn runner_c(_: i32) -> Img {
    paint_runner_part(1, false)
}
fn runner_r(_: i32) -> Img {
    paint_runner_part(2, false)
}
fn runnerz_l(_: i32) -> Img {
    paint_runner_part(0, true)
}
fn runnerz_c(_: i32) -> Img {
    paint_runner_part(1, true)
}
fn runnerz_r(_: i32) -> Img {
    paint_runner_part(2, true)
}
fn duct_sheet(_: i32) -> Img {
    ducts::paint_duct_sheet()
}
fn duct_flange(_: i32) -> Img {
    ducts::paint_duct_flange()
}
fn duct_grille(_: i32) -> Img {
    ducts::paint_duct_grille()
}

/// The painter table of a group: "floor", "wall", "ceiling" or "duct".
pub fn table(group: &str) -> Option<&'static [(&'static str, Painter, i32)]> {
    match group {
        "floor" => Some(FLOOR_PAINTERS),
        "wall" => Some(WALL_PAINTERS),
        "ceiling" => Some(CEILING_PAINTERS),
        "duct" => Some(DUCT_PAINTERS),
        _ => None,
    }
}

/// Kind keys of a group in the original's order (the texture-array layer order).
pub fn kinds(group: &str) -> Vec<&'static str> {
    table(group)
        .map(|t| t.iter().map(|e| e.0).collect())
        .unwrap_or_default()
}

fn find(group: &str, kind: &str, variant: i32) -> Option<&'static (&'static str, Painter, i32)> {
    let t = table(group)?;
    t.iter().find(|e| e.0 == kind).or_else(|| {
        let key = format!("{kind}{variant}");
        t.iter().find(|e| e.0 == key)
    })
}

/// Paints one surface: `(width, height, RGBA8 bytes)`, row-major from the top-left texel.
///
/// `kind` is either a full kind key of the group's table ("metal3", "paper0", "carpet_red", "runner_l", "wetsand0";
/// `variant` is then ignored) or a family prefix that `variant` completes ("metal" + 3, "paper" + 0, "rockwall" + 5).
/// `None` for an unknown group or kind. (The original fell back to the table's first kind for an unknown kind; use
/// `paint_or_first` for that.)
pub fn paint(group: &str, kind: &str, variant: i32) -> Option<(u16, u16, Vec<u8>)> {
    let entry = find(group, kind, variant)?;
    let img = (entry.1)(entry.2);
    Some((img.w as u16, img.h as u16, img.px))
}

/// `RefTextures.texture()` semantics: an unknown kind paints the group's first kind (metal0 / panel0 / ceil0 / sheet).
pub fn paint_or_first(group: &str, kind: &str) -> Option<(u16, u16, Vec<u8>)> {
    paint(group, kind, 0).or_else(|| {
        let first = table(group)?.first()?;
        let img = (first.1)(first.2);
        Some((img.w as u16, img.h as u16, img.px))
    })
}

/// Painter variant number of a kind key (e.g. "carpet_teal" -> 1, "wetsand0" -> 2).
pub fn variant_of(group: &str, kind: &str) -> Option<i32> {
    table(group)?.iter().find(|e| e.0 == kind).map(|e| e.2)
}

// -- how the original draws them ------------------------------------------------------------------------------------

/// Metres of wall one repeat of the wall texture is aimed at (RefLook.WALL_TEXTURE_SPAN).
pub const WALL_TEXTURE_SPAN: f32 = 4.0;

/// RefLook.wall_tiers: how many copies of the 128 px wall are stacked over a face of this storey height.
pub fn wall_tiers(height: f32) -> i32 {
    ((height / WALL_TEXTURE_SPAN).round() as i32).max(1)
}

/// RefSurfaceMesh.add_wall's v for a point at height `y` on a wall whose room runs `room_floor`..`room_height`
/// (texture v = 0 is the cornice row at the ceiling, v = tiers at the floor; sample with repeat). u runs 0..1 across
/// the 1 m face, left to right as seen from the room.
pub fn wall_v(y: f32, room_floor: f32, room_height: f32) -> f32 {
    let full = room_height - room_floor;
    wall_tiers(full) as f32 * (1.0 - (y - room_floor) / full)
}

/// Surface response the original gives a kind (MaterialLook.floor_response / RefTextures.params):
/// (roughness, metallic, emission share of the texel).
pub fn surface_params(group: &str, kind: &str) -> (f32, f32, f32) {
    let mut rough = 0.92;
    let mut metal = 0.0;
    let mut emit = 0.0;
    if group == "floor" {
        if kind.starts_with("metal")
            || kind == "hazard"
            || kind == "grate"
            || kind.starts_with("safe")
        {
            rough = 0.62;
            metal = 0.18;
        } else if kind.starts_with("stone")
            || kind.starts_with("parquet")
            || kind.starts_with("check")
        {
            rough = 0.82;
        }
    } else if group == "ceiling" && kind == "vault" {
        emit = 0.16;
    }
    let beach = match group {
        "floor" => ["sand0", "sand1", "wetsand0", "driftwood0", "driftwood1"].contains(&kind),
        "wall" => [
            "seawall0",
            "seawall1",
            "whitewash0",
            "whitewash1",
            "tealpanel0",
            "tealpanel1",
        ]
        .contains(&kind),
        _ => false,
    };
    if beach {
        emit = 0.6;
    }
    if group == "ceiling" && kind == "skylight0" {
        emit = 0.9;
    }
    (rough, metal, emit)
}

/// Colour multiplier the original applies to the texels (the chunk texture-array shader's `tint`). The painted values
/// are deliberately dark; this lifts them so a lamp pool reads. (Single-material path: walls/floors 1.4,1.4,1.45,
/// metal floors 1.2,1.2,1.25, ceilings .8,.78,.85.)
pub const ARRAY_TINT: [f32; 3] = [1.25, 1.22, 1.3];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        for g in ["floor", "ceiling", "duct"] {
            for k in kinds(g) {
                let (w, h, px) = paint(g, k, 0).unwrap();
                assert_eq!((w, h, px.len()), (32, 32, 32 * 32 * 4), "{g}:{k}");
            }
        }
        for k in kinds("wall") {
            let (w, h, _) = paint("wall", k, 0).unwrap();
            assert_eq!((w, h), (32, 128), "wall:{k}");
        }
        assert!(paint("wall", "rockwall", 5).is_some());
        assert!(paint("floor", "nope", 0).is_none());
    }

    /// Byte-compare against a Godot dump when LR_TEX_DUMP points at one (`<group>_<kind>.rgba`, see NOTES.md).
    #[test]
    fn matches_godot_dump() {
        let Ok(dir) = std::env::var("LR_TEX_DUMP") else {
            return;
        };
        let mut bad = Vec::new();
        for g in ["floor", "wall", "ceiling", "duct"] {
            for k in kinds(g) {
                let path = format!("{dir}/{g}_{k}.rgba");
                let Ok(want) = std::fs::read(&path) else {
                    bad.push(format!("{g}:{k} (no dump)"));
                    continue;
                };
                let (_, _, got) = paint(g, k, 0).unwrap();
                let diff = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                if diff != 0 || got.len() != want.len() {
                    bad.push(format!("{g}:{k} {diff} bytes differ"));
                }
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }
}
