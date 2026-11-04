//! Hunter sprites: the front and back cut-outs the Night Watch modes draw. The PNGs are embedded in `data.rs`
//! (`HUNTER_SPRITE_FILES`, one list with the other embeds) and decoded once per thread on first use. Each hunter's
//! scale and feet anchor come from `data/sprites.json`. The shape drawing stays in the modes until the drivers switch.

// The drivers call `draw_hunter` next; until then only the tests reach most of this.
#![allow(dead_code)]

use crate::data;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Which cut-out to draw: the hunter seen from the front (facing the player) or from behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Facing {
    Front,
    Back,
}

/// How one hunter is placed: `scale` multiplies the fitted size (1.0 fills the rect), `anchor` is the feet point as a
/// fraction of the PNG (x from the left, y from the top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteSpec {
    pub scale: f32,
    pub anchor: [f32; 2],
}

const BUILT_IN_SPEC: SpriteSpec = SpriteSpec { scale: 1.0, anchor: [0.5, 1.0] };

/// Every decoded cut-out, keyed by hunter id: index 0 is the front, index 1 the back.
pub struct Sprites {
    faces: HashMap<&'static str, [Texture2D; 2]>,
    specs: HashMap<String, SpriteSpec>,
    default: SpriteSpec,
}

thread_local! {
    // Leaked on purpose: the textures live for the whole run, and a `&'static` is Copy, so callers can hold it freely.
    static SPRITES: &'static Sprites = Box::leak(Box::new(Sprites::load()));
}

/// The decoded sprites. The first call on a thread decodes them, so call it after the window exists.
pub fn sprites() -> &'static Sprites {
    SPRITES.with(|s| *s)
}

/// The texture for `id` and `facing`. Panics when the hunter has no sprite: a missing embed is a content bug, and the
/// registry test catches it before a build ships.
pub fn sprite(id: &str, facing: Facing) -> &'static Texture2D {
    sprites().try_sprite(id, facing).unwrap_or_else(|| panic!("night_watch: no hunter sprite for '{id}'"))
}

/// Draws the hunter's cut-out inside `rect`, keeping its aspect ratio, with the feet on the bottom-centre of `rect`.
/// Returns false (and draws nothing) when there is no sprite for `id`, so the caller can fall back to its shapes.
pub fn draw_hunter(id: &str, facing: Facing, rect: Rect, tint: Color) -> bool {
    let sprites = sprites();
    let Some(tex) = sprites.try_sprite(id, facing) else { return false };
    let spec = sprites.spec(id);
    let (tw, th) = (tex.width(), tex.height());
    let fit = (rect.w / tw).min(rect.h / th) * spec.scale;
    let (dw, dh) = (tw * fit, th * fit);
    let feet_x = rect.x + rect.w * 0.5;
    let feet_y = rect.y + rect.h;
    let x = feet_x - spec.anchor[0] * dw;
    let y = feet_y - spec.anchor[1] * dh;
    draw_texture_ex(tex, x, y, tint, DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() });
    true
}

impl Sprites {
    fn load() -> Sprites {
        let faces = data::HUNTER_SPRITE_FILES.iter().map(|(id, front, back)| (*id, [decode(front), decode(back)])).collect();
        let (default, specs) = parse_specs(data::sprites_config());
        Sprites { faces, specs, default }
    }

    /// The texture for `id` and `facing`, or None when the hunter has no sprite.
    pub fn try_sprite(&self, id: &str, facing: Facing) -> Option<&Texture2D> {
        self.faces.get(id).map(|pair| &pair[index(facing)])
    }

    /// The scale and anchor for `id`; a hunter without its own entry gets the file's default.
    pub fn spec(&self, id: &str) -> SpriteSpec {
        self.specs.get(id).copied().unwrap_or(self.default)
    }
}

fn index(facing: Facing) -> usize {
    match facing {
        Facing::Front => 0,
        Facing::Back => 1,
    }
}

fn decode(png: &[u8]) -> Texture2D {
    Texture2D::from_file_with_format(png, Some(ImageFormat::Png))
}

/// The embedded PNG bytes for `id` and `facing`, without touching the GPU.
fn embedded_png(id: &str, facing: Facing) -> Option<&'static [u8]> {
    data::HUNTER_SPRITE_FILES.iter().find(|(h, _, _)| *h == id).map(|(_, front, back)| match facing {
        Facing::Front => *front,
        Facing::Back => *back,
    })
}

/// Reads the file's `default` and `hunters` entries. A missing field keeps the value above it, so a partial file still loads.
fn parse_specs(file: &Value) -> (SpriteSpec, HashMap<String, SpriteSpec>) {
    let default = parse_spec(&file["default"], BUILT_IN_SPEC);
    let specs = file["hunters"]
        .as_object()
        .map(|m| m.iter().map(|(id, v)| (id.clone(), parse_spec(v, default))).collect())
        .unwrap_or_default();
    (default, specs)
}

fn parse_spec(v: &Value, fallback: SpriteSpec) -> SpriteSpec {
    let scale = v["scale"].as_f64().map_or(fallback.scale, |s| s as f32);
    let anchor = match v["anchor"].as_array() {
        Some(a) if a.len() == 2 => [a[0].as_f64().map_or(fallback.anchor[0], |x| x as f32), a[1].as_f64().map_or(fallback.anchor[1], |y| y as f32)],
        _ => fallback.anchor,
    };
    SpriteSpec { scale, anchor }
}

/// Hunter ids a mode file names: the `hunters` list or object, `hunter`/`rusher`/`with` strings, and the
/// `hall_hunters`/`vent_hunters` lists. Walks the whole file, so nested entries count too.
fn hunter_ids_in(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("hunters", Value::Object(m)) => out.extend(m.keys().cloned()),
                    ("hunters", Value::Array(items)) => out.extend(items.iter().filter_map(|i| i["id"].as_str()).map(String::from)),
                    ("hunter" | "rusher" | "with", Value::String(s)) => {
                        out.insert(s.clone());
                    }
                    ("hall_hunters" | "vent_hunters", Value::Array(items)) => out.extend(items.iter().filter_map(Value::as_str).map(String::from)),
                    _ => {}
                }
                hunter_ids_in(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| hunter_ids_in(i, out)),
        _ => {}
    }
}

/// Every hunter id the mode files name, plus the ones the mode drivers name only in code.
fn hunters_in_modes() -> BTreeSet<String> {
    // `elise` is the Descent hunter: named in modes_play/descent.rs, not in its cast or rules JSON.
    const NAMED_IN_CODE: [&str; 1] = ["elise"];
    let mut ids = BTreeSet::new();
    for (mode, _) in data::CAST_FILES {
        hunter_ids_in(data::cast(mode), &mut ids);
    }
    for (mode, _) in data::RULES_FILES {
        hunter_ids_in(data::rules(mode), &mut ids);
    }
    ids.extend(NAMED_IN_CODE.iter().map(|s| s.to_string()));
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn every_hunter_a_mode_uses_has_both_facings() {
        let used = hunters_in_modes();
        assert!(used.len() >= 20, "the mode files name too few hunters: {used:?}");
        for id in &used {
            for facing in [Facing::Front, Facing::Back] {
                let png = embedded_png(id, facing).unwrap_or_else(|| panic!("hunter '{id}' has no {facing:?} cut-out embedded"));
                assert!(png.starts_with(&PNG_SIGNATURE), "{id} {facing:?} is not a PNG");
            }
        }
    }

    #[test]
    fn every_embedded_hunter_has_a_front_and_back_and_a_spec() {
        let (_, specs) = parse_specs(data::sprites_config());
        for (id, _, _) in data::HUNTER_SPRITE_FILES {
            assert!(embedded_png(id, Facing::Front).is_some() && embedded_png(id, Facing::Back).is_some(), "{id}");
            assert!(specs.contains_key(id), "data/sprites.json has no entry for '{id}'");
        }
    }

    #[test]
    fn sprite_scale_and_anchor_load_from_the_file() {
        let (default, specs) = parse_specs(data::sprites_config());
        assert_eq!(default, BUILT_IN_SPEC, "the file's default is the built-in one");
        assert_eq!(specs.len(), data::HUNTER_SPRITE_FILES.len());
        for (id, spec) in &specs {
            assert!(spec.scale > 0.0, "{id} scale must be positive");
            assert!(spec.anchor.iter().all(|a| (0.0..=1.0).contains(a)), "{id} anchor must sit inside the PNG: {:?}", spec.anchor);
        }
    }

    #[test]
    fn a_partial_spec_keeps_the_default_for_the_missing_field() {
        let file = serde_json::json!({
            "default": { "scale": 0.8, "anchor": [0.4, 0.95] },
            "hunters": { "x": { "scale": 1.2 }, "y": { "anchor": [0.5, 1.0] } }
        });
        let (default, specs) = parse_specs(&file);
        assert_eq!(default, SpriteSpec { scale: 0.8, anchor: [0.4, 0.95] });
        assert_eq!(specs["x"], SpriteSpec { scale: 1.2, anchor: [0.4, 0.95] });
        assert_eq!(specs["y"], SpriteSpec { scale: 0.8, anchor: [0.5, 1.0] });
    }

    #[test]
    fn the_walk_finds_ids_in_each_shape_a_mode_file_uses() {
        let mut ids = BTreeSet::new();
        hunter_ids_in(
            &serde_json::json!({
                "hunters": [{ "id": "opalie" }],
                "watches": [{ "id": "rooftop", "hunter": "verrine" }],
                "combo": { "with": "ravelle" },
                "rusher": "umbriel",
                "vent_hunters": ["haelith"],
                "hunters_obj": {},
            }),
            &mut ids,
        );
        let got: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(got, ["haelith", "opalie", "ravelle", "umbriel", "verrine"]);
    }
}
