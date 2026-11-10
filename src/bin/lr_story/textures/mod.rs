//! Port of the original's procedural pixel-art surfaces (game/look/ref_textures.gd, `RefTextures`) and of the tables
//! that pick them per district / room purpose (ref_look.gd, floor_sets.gd, data/themes/**, src/bin/last_route/data/regions/*.json).
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
//!
//! lr_story copy (2026-11-05). Files copied by value from last_route's `src/bin/last_route/render/textures/`; the
//! last_route copy is not edited. Changes from the source:
//! * `sets.rs`: the maze variants come from the local `MAZE_VARIANTS` table, not `crate::world::route::maze_regions`
//!   (which needs last_route's embedded data). Same four rows as `src/bin/last_route/data/regions/maze_*.json`.
//! * `sets.rs`: "office" maps to the maze set, not the lab set. This differs from last_route's `world/mod.rs`
//!   `region_texture` (office -> lab) on purpose; see the doc comment on `FINITE_DISTRICT`.
//! * Upload: the source's `render/surfaces.rs` (lines 15-24) sets `FilterMode::Nearest` on each painted texture. Any
//!   GPU upload of these buffers in lr_story must do the same (main.rs already does for map pictures).
//! The painter itself is unchanged.

// Kept whole from last_route so every surface kind can be painted; lr_story's maps use only part of the table so far.
#![allow(dead_code)]

pub mod beach;
pub mod canvas;
pub mod ceilings;
pub mod ducts;
pub mod flat;
pub mod floors;
pub mod medieval;
pub mod rng;
pub mod sets;
pub mod sheet;
pub mod walls;

#[allow(unused_imports)] // re-exported API
pub use canvas::{Img, PX, WALL_H};
#[allow(unused_imports)] // re-exported API
pub use sets::{region_set, TexRef, TextureChoice};

use beach::*;
use ceilings::*;
use floors::*;
use walls::*;
use std::sync::atomic::{AtomicBool, Ordering};

type Painter = fn(i32) -> Img;

/// How a surface is painted. `Painted` is the original look (floors.rs, walls.rs, ceilings.rs). `Flat` swaps the lab
/// kinds for their flat versions (flat.rs): the same patterns in 3-6 flat colours, no bevel, no grain. Kinds without a
/// flat version paint the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Painted,
    Flat,
}

/// Set by `--flat-textures`: `paint()` paints the lab kinds `Flat` instead of `Painted`. Off by default.
static FLAT: AtomicBool = AtomicBool::new(false);

pub fn set_flat_textures(on: bool) {
    FLAT.store(on, Ordering::Relaxed);
}

pub fn flat_textures() -> bool {
    FLAT.load(Ordering::Relaxed)
}

/// The lab kinds that have flat versions, as (group, kind): the contact sheet's columns.
pub const LAB_KINDS: &[(&str, &str)] = &[
    ("floor", "epoxy0"),
    ("floor", "epoxy1"),
    ("floor", "grate"),
    ("floor", "hazard"),
    ("floor", "metal1"),
    ("wall", "lab0"),
    ("wall", "lab1"),
    ("wall", "timber0"),
    ("wall", "ashlar0"),
    ("floor", "cobble0"),
    ("floor", "dirt0"),
    ("ceiling", "beams0"),
    ("ceiling", "ceil0"),
];

/// The flat painter of a kind, when it has one.
fn flat_painter(group: &str, key: &str) -> Option<fn(i32) -> Img> {
    match (group, key) {
        ("floor", "metal0" | "metal1" | "metal2" | "metal3") => Some(flat::paint_metal),
        ("floor", "epoxy0" | "epoxy1") => Some(flat::paint_epoxy),
        ("floor", "grate") => Some(flat::paint_grate),
        ("floor", "hazard") => Some(flat::paint_hazard),
        ("wall", "lab0" | "lab1") => Some(flat::paint_wall_lab),
        ("ceiling", "ceil0" | "ceil1") => Some(flat::paint_ceiling),
        ("wall", "timber0" | "timber1") => Some(medieval::flat_timber_wall),
        ("wall", "ashlar0" | "ashlar1") => Some(medieval::flat_stone_wall),
        ("floor", "cobble0" | "cobble1") => Some(medieval::flat_cobble_floor),
        ("floor", "dirt0") => Some(medieval::flat_dirt_floor),
        ("ceiling", "beams0") => Some(medieval::flat_beam_ceiling),
        _ => None,
    }
}

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
    ("cobble0", medieval::paint_cobble_floor, 0),
    ("cobble1", medieval::paint_cobble_floor, 1),
    ("dirt0", medieval::paint_dirt_floor, 0),
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
    ("timber0", medieval::paint_timber_wall, 0),
    ("timber1", medieval::paint_timber_wall, 1),
    // "ashlar", not "stone": the "stone0" wall above is the older one. `medieval::paint_named("stone0")` is this one.
    ("ashlar0", medieval::paint_stone_wall, 0),
    ("ashlar1", medieval::paint_stone_wall, 1),
];

pub const CEILING_PAINTERS: &[(&str, Painter, i32)] = &[
    ("ceil0", paint_ceiling, 0),
    ("ceil1", paint_ceiling, 1),
    ("glass", paint_glass, 0),
    ("rockceil", paint_rock_ceiling, 0),
    ("vault", paint_vault, 0),
    ("skylight0", paint_skylight, 0),
    ("beams0", medieval::paint_beam_ceiling, 0),
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
/// `paint_or_first` for that.) The style is `Flat` when `--flat-textures` is set, else `Painted`.
pub fn paint(group: &str, kind: &str, variant: i32) -> Option<(u16, u16, Vec<u8>)> {
    let style = if flat_textures() { Style::Flat } else { Style::Painted };
    paint_styled(group, kind, variant, style)
}

/// `paint` with the style given. The same bytes for the same key and style, every time.
pub fn paint_styled(group: &str, kind: &str, variant: i32, style: Style) -> Option<(u16, u16, Vec<u8>)> {
    let entry = find(group, kind, variant)?;
    Some(render(group, entry, style))
}

/// Paints one table entry in a style: the flat painter when the style is `Flat` and the kind has one, else the original.
fn render(group: &str, entry: &(&'static str, Painter, i32), style: Style) -> (u16, u16, Vec<u8>) {
    let flat = match style {
        Style::Flat => flat_painter(group, entry.0),
        Style::Painted => None,
    };
    let img = match flat {
        Some(paint_flat) => paint_flat(entry.2),
        None => (entry.1)(entry.2),
    };
    (img.w as u16, img.h as u16, img.px)
}

/// `RefTextures.texture()` semantics: an unknown kind paints the group's first kind (metal0 / panel0 / ceil0 / sheet).
/// The fallback goes through the same style as `paint`, so `--flat-textures` holds for it too.
pub fn paint_or_first(group: &str, kind: &str) -> Option<(u16, u16, Vec<u8>)> {
    let style = if flat_textures() { Style::Flat } else { Style::Painted };
    paint_or_first_styled(group, kind, style)
}

/// `paint_or_first` with the style given.
pub fn paint_or_first_styled(group: &str, kind: &str, style: Style) -> Option<(u16, u16, Vec<u8>)> {
    paint_styled(group, kind, 0, style).or_else(|| {
        let first = table(group)?.first()?;
        Some(render(group, first, style))
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

    /// The same key always paints the same bytes (the RNG is seeded from the key), and two RNGs with the same key
    /// give the same stream.
    #[test]
    fn same_seed_gives_same_bytes_twice() {
        for g in ["floor", "wall", "ceiling", "duct"] {
            for k in kinds(g) {
                let a = paint(g, k, 0).unwrap();
                let b = paint(g, k, 0).unwrap();
                assert!(a == b, "{g}:{k} painted differently on the second call");
            }
        }
        let mut r1 = rng::GodotRng::from_key("epoxy0");
        let mut r2 = rng::GodotRng::from_key("epoxy0");
        for _ in 0..16 {
            assert_eq!(r1.next_u32(), r2.next_u32());
        }
    }

    /// The lab's own kinds: floors 32 x 32, walls 32 x 128, ceiling 32 x 32.
    #[test]
    fn lab_floor_is_32x32_and_lab_wall_is_32x128() {
        for k in ["epoxy0", "epoxy1", "grate", "hazard", "metal1"] {
            let (w, h, px) = paint("floor", k, 0).unwrap();
            assert_eq!((w, h), (32u16, 32u16), "floor:{k}");
            assert_eq!(px.len(), 32 * 32 * 4, "floor:{k} byte count");
        }
        for k in ["lab0", "lab1"] {
            let (w, h, px) = paint("wall", k, 0).unwrap();
            assert_eq!((w, h), (32u16, 128u16), "wall:{k}");
            assert_eq!(px.len(), 32 * 128 * 4, "wall:{k} byte count");
        }
        let (w, h, _) = paint("ceiling", "ceil0", 0).unwrap();
        assert_eq!((w, h), (32u16, 32u16));
    }

    /// The PNG fallback `assets/environment/wall_lab.png` is one storey of wall like the painter's: 32 x 128.
    #[test]
    fn the_wall_lab_png_fallback_is_32x128_like_the_painter() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets/environment/wall_lab.png");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        let (pw, ph, _) = paint("wall", "lab0", 0).unwrap();
        assert_eq!((w, h), (pw as u32, ph as u32));
        assert_eq!((w, h), (32, 128));
    }

    /// "lab" and "office" are different districts on purpose: the lab set for "lab", the maze (office) set for
    /// "office". last_route's world/mod.rs maps office to lab; this port does not.
    #[test]
    fn lab_and_office_stay_distinct() {
        let lab = region_set("lab", "", true);
        assert_eq!(lab.district, "lab");
        assert_eq!(lab.floor.kind, "epoxy0");
        let office = region_set("office", "", true);
        assert_eq!(office.district, "maze");
        assert_eq!(office.wall.kind, "panel0");
        assert_ne!(lab.district, office.district);
    }

    /// The painted style is the original painter's output, byte for byte (the flat painters do not touch it).
    #[test]
    fn painted_style_still_gives_the_original_bytes() {
        let (w, h, px) = paint_styled("floor", "epoxy0", 0, Style::Painted).unwrap();
        let direct = floors::paint_epoxy(0);
        assert_eq!((w as i32, h as i32), (direct.w, direct.h));
        assert!(px == direct.px, "painted epoxy0 changed");
        let (_, _, wall) = paint_styled("wall", "lab1", 0, Style::Painted).unwrap();
        assert!(wall == walls::paint_wall_lab(1).px, "painted lab1 changed");
        let (_, _, ceil) = paint_styled("ceiling", "ceil0", 0, Style::Painted).unwrap();
        assert!(ceil == ceilings::paint_ceiling(0).px, "painted ceil0 changed");
    }

    /// The fallback for an unknown kind honours the style: flat gives the first kind's flat texture.
    #[test]
    fn the_fallback_painter_honours_the_flat_style() {
        let first = table("floor").unwrap().first().unwrap().0;
        let flat = paint_or_first_styled("floor", "no_such_kind", Style::Flat).unwrap();
        assert!(flat == paint_styled("floor", first, 0, Style::Flat).unwrap(), "fallback is the first kind, flat");
        let painted = paint_or_first_styled("floor", "no_such_kind", Style::Painted).unwrap();
        assert!(painted == paint_styled("floor", first, 0, Style::Painted).unwrap());
        assert!(flat.2 != painted.2, "the styles differ for {first}");
        assert!(paint_or_first_styled("nope", "x", Style::Flat).is_none());
    }

    /// Every lab kind has a flat version that differs from the painted one, and the flat bytes are the same twice.
    #[test]
    fn flat_lab_kinds_differ_from_painted_and_repeat_exactly() {
        for (group, kind) in LAB_KINDS {
            let painted = paint_styled(group, kind, 0, Style::Painted).unwrap();
            let flat = paint_styled(group, kind, 0, Style::Flat).unwrap();
            let again = paint_styled(group, kind, 0, Style::Flat).unwrap();
            assert_eq!((painted.0, painted.1), (flat.0, flat.1), "{group}:{kind} size");
            assert!(flat == again, "{group}:{kind} flat bytes differ on the second call");
            assert!(flat.2 != painted.2, "{group}:{kind} flat is the painted texture");
        }
    }

    /// Kinds without a flat version paint the same in both styles.
    #[test]
    fn kinds_without_a_flat_version_are_unchanged() {
        for (group, kind) in [("floor", "carpet_red"), ("wall", "paper0"), ("ceiling", "vault")] {
            assert!(paint_styled(group, kind, 0, Style::Flat) == paint_styled(group, kind, 0, Style::Painted));
        }
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
