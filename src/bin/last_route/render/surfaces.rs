//! The original's painted surfaces (render/textures paints them byte for byte) as GPU textures,
//! painted once per kind and kept for the session.

use super::textures::TexRef;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHE: RefCell<HashMap<(&'static str, &'static str), Option<Texture2D>>> = RefCell::new(HashMap::new());
}

/// Uploads one painted surface (nearest sampling; the world shader repeats UVs).
#[cfg(not(test))]
fn upload(w: u16, h: u16, px: &[u8]) -> Texture2D {
    let t = Texture2D::from_rgba8(w, h, px);
    t.set_filter(FilterMode::Nearest);
    t
}

/// Test only: a distinct raw handle that is never uploaded, so the world mesh builders run with no GL behind them
/// (they only clone and compare textures; see `Assets::headless`).
#[cfg(test)]
fn upload(_w: u16, _h: u16, _px: &[u8]) -> Texture2D {
    use macroquad::miniquad::{RawId, TextureId};
    thread_local! {
        static NEXT: std::cell::Cell<u32> = const { std::cell::Cell::new(1000) };
    }
    let n = NEXT.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    Texture2D::from_miniquad_texture(TextureId::from_raw_id(RawId::OpenGl(n)))
}

/// The texture of a painted kind (nearest sampling; the world shader repeats UVs), or None
/// for a kind the painter tables do not know.
pub fn texture(r: TexRef) -> Option<Texture2D> {
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((r.group, r.kind))
            .or_insert_with(|| {
                let (w, h, px) = r.paint()?;
                Some(upload(w, h, &px))
            })
            .clone()
    })
}

// -- surface themes (data/surface_themes.json) ---------------------------------------------------------------------

use super::textures::canvas::{hex, Col};
use std::sync::OnceLock;

const THEMES_PATH: &str = "surface_themes.json";
const AREA_LIGHTS_PATH: &str = "render/area_lights.json";

/// Surface response numbers the shader gets per kind (`textures::surface_params`).
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub rough_default: f32,
    pub metal_rough: f32,
    pub metal_metallic: f32,
    pub stone_rough: f32,
    pub vault_emit: f32,
    pub beach_emit: f32,
    pub skylight_emit: f32,
}

/// The colours the painters share and the world tint, from data/surface_themes.json. `builtin` is what the code
/// painted before the data existed (byte parity with the Godot game).
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceThemes {
    pub metal: u32,
    pub stone: u32,
    pub wood: u32,
    pub flat_seam: u32,
    pub flat_edge: u32,
    pub array_tint: [f32; 3],
    pub response: Response,
}

fn parse_hex(v: &serde_json::Value, key: &str) -> Result<u32, String> {
    let s = v.get(key).and_then(|x| x.as_str()).ok_or_else(|| format!("{key}: expected a hex colour"))?;
    if s.len() != 6 {
        return Err(format!("{key}: expected six hex digits"));
    }
    u32::from_str_radix(s, 16).map_err(|_| format!("{key}: not a hex colour"))
}

impl SurfaceThemes {
    pub fn builtin() -> SurfaceThemes {
        SurfaceThemes {
            metal: 0x2a2627,
            stone: 0x3a3532,
            wood: 0x3a2418,
            flat_seam: 0x141012,
            flat_edge: 0x564e50,
            array_tint: [1.25, 1.22, 1.3],
            response: Response {
                rough_default: 0.92,
                metal_rough: 0.62,
                metal_metallic: 0.18,
                stone_rough: 0.82,
                vault_emit: 0.16,
                beach_emit: 0.6,
                skylight_emit: 0.9,
            },
        }
    }

    /// Parses the table; a key it leaves out keeps the built-in value, a malformed one is an error.
    pub fn parse(text: &str) -> Result<SurfaceThemes, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut out = SurfaceThemes::builtin();
        if let Some(p) = v.get("palette") {
            for (key, slot) in [
                ("metal", &mut out.metal),
                ("stone", &mut out.stone),
                ("wood", &mut out.wood),
                ("flat_seam", &mut out.flat_seam),
                ("flat_edge", &mut out.flat_edge),
            ] {
                if p.get(key).is_some() {
                    *slot = parse_hex(p, key)?;
                }
            }
        }
        if let Some(t) = v.get("array_tint") {
            let a = t.as_array().filter(|a| a.len() == 3).ok_or("array_tint: expected three numbers")?;
            for (i, n) in a.iter().enumerate() {
                out.array_tint[i] = n.as_f64().filter(|n| *n > 0.0).ok_or("array_tint: expected positive numbers")? as f32;
            }
        }
        if let Some(r) = v.get("response") {
            for (key, slot) in [
                ("rough_default", &mut out.response.rough_default),
                ("metal_rough", &mut out.response.metal_rough),
                ("metal_metallic", &mut out.response.metal_metallic),
                ("stone_rough", &mut out.response.stone_rough),
                ("vault_emit", &mut out.response.vault_emit),
                ("beach_emit", &mut out.response.beach_emit),
                ("skylight_emit", &mut out.response.skylight_emit),
            ] {
                if let Some(n) = r.get(key) {
                    *slot = n.as_f64().filter(|n| (0.0..=1.0).contains(n)).ok_or_else(|| format!("{key}: expected 0..1"))? as f32;
                }
            }
        }
        Ok(out)
    }
}

/// A palette entry as a painter colour.
pub fn palette_col(rgb: u32) -> Col {
    hex(rgb)
}

/// The themes compiled into the executable; a table that fails to parse is reported and the built-in one is used.
pub fn surface_themes() -> &'static SurfaceThemes {
    static T: OnceLock<SurfaceThemes> = OnceLock::new();
    T.get_or_init(|| {
        let text = crate::content::embedded::data_under(THEMES_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        SurfaceThemes::parse(text).unwrap_or_else(|e| {
            eprintln!("{THEMES_PATH}: {e}");
            SurfaceThemes::builtin()
        })
    })
}

// -- per-area light colour (data/render/area_lights.json) -----------------------------------------------------------

/// The colour and energy factor a room role gives its lamps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaLight {
    pub color: [f32; 3],
    pub energy: f32,
}

impl AreaLight {
    pub const NEUTRAL: AreaLight = AreaLight { color: [1.0, 1.0, 1.0], energy: 1.0 };
}

#[derive(Clone, Debug, PartialEq)]
pub struct AreaLights {
    pub default: AreaLight,
    pub roles: std::collections::HashMap<String, AreaLight>,
}

fn parse_area(v: &serde_json::Value, what: &str) -> Result<AreaLight, String> {
    let c = parse_hex(v, "color").map_err(|e| format!("{what}: {e}"))?;
    let energy = v
        .get("energy")
        .and_then(|e| e.as_f64())
        .filter(|e| (0.1..=3.0).contains(e))
        .ok_or_else(|| format!("{what}: energy expected 0.1..3"))? as f32;
    let ch = |shift: u32| ((c >> shift) & 0xff) as f32 / 255.0;
    Ok(AreaLight { color: [ch(16), ch(8), ch(0)], energy })
}

impl AreaLights {
    pub fn parse(text: &str) -> Result<AreaLights, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let default = match v.get("default") {
            Some(d) => parse_area(d, "default")?,
            None => AreaLight::NEUTRAL,
        };
        let mut roles = std::collections::HashMap::new();
        if let Some(m) = v.get("roles").and_then(|r| r.as_object()) {
            for (k, d) in m {
                roles.insert(k.clone(), parse_area(d, k)?);
            }
        }
        Ok(AreaLights { default, roles })
    }

    /// The light of a room role; the default for a role the table does not list.
    pub fn light_for(&self, role: &str) -> AreaLight {
        self.roles.get(role).copied().unwrap_or(self.default)
    }
}

/// The area light table compiled into the executable (neutral white if it fails to parse).
pub fn area_lights() -> &'static AreaLights {
    static T: OnceLock<AreaLights> = OnceLock::new();
    T.get_or_init(|| {
        let text = crate::content::embedded::data_under(AREA_LIGHTS_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        AreaLights::parse(text).unwrap_or_else(|e| {
            eprintln!("{AREA_LIGHTS_PATH}: {e}");
            AreaLights { default: AreaLight::NEUTRAL, roles: Default::default() }
        })
    })
}

/// Lookup by room role (`rtype`) in the compiled table.
pub fn area_light(role: &str) -> AreaLight {
    area_lights().light_for(role)
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn shipped_themes_match_the_old_hardcoded_colours() {
        let text = include_str!("../data/surface_themes.json");
        assert_eq!(SurfaceThemes::parse(text).unwrap(), SurfaceThemes::builtin());
        assert_eq!(SurfaceThemes::builtin().array_tint, super::super::textures::ARRAY_TINT);
        assert_eq!(surface_themes(), &SurfaceThemes::builtin());
    }

    #[test]
    fn a_changed_theme_value_changes_the_palette() {
        let t = SurfaceThemes::parse(r#"{"palette":{"metal":"102030"}}"#).unwrap();
        assert_eq!(t.metal, 0x102030);
        assert_eq!(t.stone, 0x3a3532);
        assert!(SurfaceThemes::parse(r#"{"palette":{"metal":"zz"}}"#).is_err());
        assert!(SurfaceThemes::parse(r#"{"response":{"beach_emit":4}}"#).is_err());
    }

    #[test]
    fn area_light_defaults_for_unknown_roles() {
        let t = AreaLights::parse(include_str!("../data/render/area_lights.json")).unwrap();
        assert_eq!(t.light_for("no_such_role"), t.default);
        assert_eq!(t.default, AreaLight::NEUTRAL);
        assert_ne!(t.light_for("chapel"), t.default);
        assert_eq!(area_light("no_such_role"), AreaLight::NEUTRAL);
    }

    #[test]
    fn area_light_roles_are_real_room_purposes() {
        use super::super::textures::sets::{ROOM_FLOORS, ROOM_WALLS};
        let t = AreaLights::parse(include_str!("../data/render/area_lights.json")).unwrap();
        for role in t.roles.keys() {
            let known = ROOM_WALLS.iter().any(|(p, _)| p == role) || ROOM_FLOORS.iter().any(|(p, _)| p == role);
            assert!(known, "{role}: not a room purpose");
        }
    }

    #[test]
    fn a_changed_light_value_changes_the_lookup() {
        let t = AreaLights::parse(r#"{"roles":{"x":{"color":"ff0000","energy":2.0}}}"#).unwrap();
        assert_eq!(t.light_for("x"), AreaLight { color: [1.0, 0.0, 0.0], energy: 2.0 });
        assert_eq!(t.light_for("y"), AreaLight::NEUTRAL);
    }
}
