//! Everything drawn from files: the original's region wall/floor pictures, the hunters'
//! Blockbench models and portraits, the props and interactables (Blockbench models too), the
//! inventory icons and the app icon. Fonts are the system faces the original asks for (Courier New bold
//! for text, Impact for titles).

use crate::content::{embedded, packs, Content};
use crate::render::bbmodel::Model;
use crate::render::builder::white_texture;
use crate::render::prop_model::{AtlasCache, PropModel};
use crate::world::TEXTURE_SETS;
use crate::net::world_sync::SKIN_DEFAULT;
use crate::shared::skins;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The runner body (remote runners and NPC teammates): the blocky mannequin from the Godot build, embedded under
/// assets/player/. Its atlas is also embedded as a data URI, but the file name is what the model's `find` resolves first.
const RUNNER_BODY_MODEL: &str = "player/player_mannequin.bbmodel";
const RUNNER_BODY_ATLAS: &str = "player/player_mannequin_atlas.png";

pub struct Assets {
    pub walls: HashMap<&'static str, Texture2D>,
    pub floors: HashMap<&'static str, Texture2D>,
    pub white: Texture2D,
    pub hunters: HashMap<String, Model>,
    /// The runner body (remote runners and NPC teammates): the blocky mannequin parsed at the runner's height.
    pub npc_body: Option<Model>,
    pub portraits: HashMap<String, Texture2D>,
    /// Every prop, fixture and hand-tool model by the path the prop data names it under: props/<set>/<name>.bbmodel
    /// and interactables/<name>.bbmodel (and "pack:<id>:<path>" for pack props).
    pub models: HashMap<String, PropModel>,
    /// flashlight, taser, crowbar, axe, battery, stim
    pub icons: Vec<Texture2D>,
    pub logo: Option<Texture2D>,
    pub font: Option<Font>,
    pub title_font: Option<Font>,
    /// Courier New italic: the death moment's caption lines.
    pub caption_font: Option<Font>,
    /// Runner skin textures by skin id, read from the skin folders when first needed (see [`Assets::skin_texture`]).
    skin_textures: RefCell<HashMap<String, SkinSlot>>,
}

/// One skin id's texture: loaded, or still waiting for a file (checked again later, and fetched from the website once in a while).
enum SkinSlot {
    Ready(Texture2D),
    Waiting { check_at: Instant, fetch_at: Instant },
}

/// How often a missing skin file is looked for again on disk.
const SKIN_RECHECK: Duration = Duration::from_secs(2);
/// How often a missing skin is asked for again from the website.
const SKIN_REFETCH: Duration = Duration::from_secs(30);

pub fn tex(bytes: &[u8]) -> Texture2D {
    let t = Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png));
    t.set_filter(FilterMode::Nearest);
    t
}

fn asset_tex(path: &str) -> Option<Texture2D> {
    embedded::asset(path).map(tex)
}

/// The runner body: the embedded mannequin model, its atlas by file name, scaled so its top sits at PLAYER_HEIGHT.
fn load_runner_body() -> Option<Model> {
    let text = embedded::asset(RUNNER_BODY_MODEL).and_then(|b| std::str::from_utf8(b).ok())?;
    let atlas = asset_tex(RUNNER_BODY_ATLAS);
    let find = |name: &str| embedded::asset(&format!("player/{name}")).map(|b| b.to_vec());
    Model::parse(text, atlas, &find, Some(crate::config::PLAYER_HEIGHT))
}

fn system_font(names: &[&str]) -> Option<Font> {
    let mut dirs: Vec<String> = [
        "C:/Windows/Fonts/",
        "/System/Library/Fonts/Supplemental/",
        "/Library/Fonts/",
        "/usr/share/fonts/truetype/msttcorefonts/",
        "/usr/share/fonts/TTF/",
        "/usr/share/fonts/msttcore/",
        "/usr/share/fonts/truetype/liberation/",
        "/usr/share/fonts/liberation/",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/Library/Fonts/"));
        dirs.push(format!("{home}/.local/share/fonts/"));
        dirs.push(format!("{home}/.fonts/"));
    }
    for d in &dirs {
        for n in names {
            if let Ok(b) = std::fs::read(format!("{d}{n}")) {
                if let Ok(f) = load_ttf_font_from_bytes(&b) {
                    return Some(f);
                }
            }
        }
    }
    None
}

/// Reads a text file out of a model's folder for a bbmodel that names its atlas by file: the folder itself, else the one
/// above it (the blocky props share `props/blocky_atlas.png`).
fn embedded_sibling(dir: &str) -> impl Fn(&str) -> Option<Vec<u8>> + '_ {
    move |file: &str| {
        let up = dir.rsplit_once('/').map(|x| x.0);
        embedded::asset(&format!("{dir}/{file}"))
            .or_else(|| up.and_then(|u| embedded::asset(&format!("{u}/{file}"))))
            .map(|b| b.to_vec())
    }
}

/// The prop, fixture and hand-tool models: every interactable, every model the prop data names, and the models
/// mod packs bring (a pack's `.bbmodel`, or its `.glb`, the one format still read besides bbmodel).
fn load_models(content: &Content) -> HashMap<String, PropModel> {
    let mut models = HashMap::new();
    let mut cache = AtlasCache::default();
    let embedded_models = embedded::assets_under("interactables/")
        .map(|(p, _)| p)
        .filter(|p| p.ends_with(".bbmodel"))
        .chain(
            content
                .props
                .values()
                .flat_map(|d| d.models.iter())
                .map(|m| m.as_str())
                .filter(|m| !m.starts_with("pack:")),
        );
    for path in embedded_models {
        if models.contains_key(path) {
            continue;
        }
        let Some(text) = embedded::asset(path).and_then(|b| std::str::from_utf8(b).ok()) else {
            continue;
        };
        let dir = path.rsplit_once('/').map(|x| x.0).unwrap_or("");
        let soft = path.starts_with("props/blocky/");
        if let Some(m) = PropModel::parse(text, &embedded_sibling(dir), dir, &mut cache, soft) {
            models.insert(path.to_string(), m);
        }
    }
    let white = white_texture();
    for def in content.props.values() {
        for m in def.models.iter().filter(|m| m.starts_with("pack:")) {
            let mut it = m.splitn(3, ':');
            let (_, pid, rel) = (it.next(), it.next().unwrap_or(""), it.next().unwrap_or(""));
            let Some(pack) = content.packs.iter().find(|p| p.id == pid) else {
                continue;
            };
            let dir = rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
            let sibling = |file: &str| packs::read_pack_file(&pack.dir, &format!("{dir}/{file}"));
            let Some(bytes) = packs::read_pack_file(&pack.dir, rel) else {
                continue;
            };
            let model = if rel.ends_with(".bbmodel") {
                String::from_utf8(bytes)
                    .ok()
                    .and_then(|t| PropModel::parse(&t, &sibling, &format!("pack:{pid}/{dir}"), &mut cache, false))
            } else if rel.ends_with(".glb") {
                PropModel::parse_glb(&bytes, &sibling, &white)
            } else {
                None
            };
            if let Some(model) = model {
                models.insert(m.clone(), model);
            }
        }
    }
    models
}

impl Assets {
    pub fn load(content: &Content) -> Assets {
        let mut walls = HashMap::new();
        let mut floors = HashMap::new();
        for set in TEXTURE_SETS {
            if let Some(t) = asset_tex(&format!("environment/wall_{set}.png")) {
                walls.insert(set, t);
            }
            if let Some(t) = asset_tex(&format!("environment/floor_{set}.png")) {
                floors.insert(set, t);
            }
        }
        let mut hunters = HashMap::new();
        let npc_body = load_runner_body();
        let mut portraits = HashMap::new();
        for h in &content.hunters {
            let pack_dir = content
                .packs
                .iter()
                .find(|p| p.id == h.pack)
                .map(|p| p.dir.clone());
            let read = |rel: &str| -> Option<Vec<u8>> {
                match &pack_dir {
                    Some(d) => packs::read_pack_file(d, rel),
                    None => embedded::asset(rel).map(|b| b.to_vec()),
                }
            };
            if let Some(model_path) = h.model.as_deref().filter(|m| m.ends_with(".bbmodel")) {
                if let Some(text) = read(model_path).and_then(|b| String::from_utf8(b).ok()) {
                    let atlas = h.atlas.as_deref().and_then(|a| read(a)).map(|b| tex(&b));
                    let find = |name: &str| -> Option<Vec<u8>> {
                        let dir = model_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
                        read(&format!("{dir}/{name}"))
                    };
                    if let Some(m) =
                        Model::parse(&text, atlas, &find, Some(crate::config::HUNTER_HEIGHT))
                    {
                        hunters.insert(h.id.clone(), m);
                    }
                }
            }
            if let Some(p) = h.portrait.as_deref().and_then(|p| read(p)) {
                portraits.insert(h.id.clone(), tex(&p));
            }
        }
        let models = load_models(content);
        let icons = [
            "flashlight_item",
            "taser_item",
            "crowbar",
            "rescue_axe",
            "battery",
            "stim_pack",
        ]
        .iter()
        .map(|n| asset_tex(&format!("inventory/{n}.png")).unwrap_or_else(white_texture))
        .collect();
        Assets {
            walls,
            floors,
            white: white_texture(),
            hunters,
            npc_body,
            portraits,
            models,
            icons,
            logo: asset_tex("branding/logo_128.png"),
            // Courier New Bold / Impact as the original asks; metric-compatible stand-ins on Linux
            font: system_font(&[
                "courbd.ttf",
                "Courier New Bold.ttf",
                "Courier_New_Bold.ttf",
                "LiberationMono-Bold.ttf",
            ]),
            title_font: system_font(&[
                "impact.ttf",
                "Impact.ttf",
                "ariblk.ttf",
                "Arial Black.ttf",
                "LiberationSans-Bold.ttf",
            ]),
            caption_font: system_font(&[
                "couri.ttf",
                "Courier New Italic.ttf",
                "Courier_New_Italic.ttf",
                "LiberationMono-Italic.ttf",
            ]),
            skin_textures: RefCell::new(HashMap::new()),
        }
    }

    pub fn wall(&self, set: &str) -> &Texture2D {
        self.walls
            .get(set)
            .or_else(|| self.walls.get("maze"))
            .unwrap_or(&self.white)
    }

    pub fn floor(&self, set: &str) -> &Texture2D {
        self.floors
            .get(set)
            .or_else(|| self.floors.get("maze"))
            .unwrap_or(&self.white)
    }

    /// A hunter's body (or the base body when a pack hunter brings none).
    pub fn hunter_model(&self, id: &str) -> Option<&Model> {
        self.hunters.get(id).or_else(|| self.hunters.get("dovelle"))
    }

    /// The texture a runner's skin id draws with. DEFAULT is the mannequin atlas, so it gives None; so does an id whose
    /// skin is not on disk yet (the caller then draws the atlas too). A found skin is cached as one texture per id.
    pub fn skin_texture(&self, id: &str) -> Option<Texture2D> {
        if id == SKIN_DEFAULT {
            return None;
        }
        let now = Instant::now();
        let mut slots = self.skin_textures.borrow_mut();
        if let Some(SkinSlot::Ready(t)) = slots.get(id) {
            return Some(t.clone());
        }
        let (check_at, fetch_at) = match slots.get(id) {
            Some(SkinSlot::Waiting { check_at, fetch_at }) => (*check_at, *fetch_at),
            _ => (now, now),
        };
        if now < check_at {
            return None;
        }
        if let Some(bytes) = skins::read_skin(id) {
            let t = tex(&bytes);
            slots.insert(id.to_string(), SkinSlot::Ready(t.clone()));
            return Some(t);
        }
        let fetch_at = if now >= fetch_at {
            skins::fetch_skin_in_background(id);
            now + SKIN_REFETCH
        } else {
            fetch_at
        };
        slots.insert(id.to_string(), SkinSlot::Waiting { check_at: now + SKIN_RECHECK, fetch_at });
        None
    }
}

/// The window icon from the original's branding PNGs.
pub fn window_icon() -> Option<macroquad::miniquad::conf::Icon> {
    let load = |size: u32| -> Option<Vec<u8>> {
        let bytes = embedded::asset(&format!("branding/logo_{size}.png"))?;
        let img = Image::from_file_with_format(bytes, Some(ImageFormat::Png)).ok()?;
        if img.width as u32 != size || img.height as u32 != size {
            return None;
        }
        Some(img.bytes)
    };
    let small: [u8; 1024] = load(16)?.try_into().ok()?;
    let medium: [u8; 4096] = load(32)?.try_into().ok()?;
    let big: [u8; 16384] = load(64)?.try_into().ok()?;
    Some(macroquad::miniquad::conf::Icon { small, medium, big })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::bbmodel::texture_bytes;
    use serde_json::Value;

    fn runner_json() -> Value {
        let text = embedded::asset(RUNNER_BODY_MODEL).expect("the runner body is embedded");
        serde_json::from_slice(text).expect("the runner body is JSON")
    }

    #[test]
    fn the_runner_body_is_the_mannequin_and_its_atlas_resolves() {
        let j = runner_json();
        let find = |name: &str| embedded::asset(&format!("player/{name}")).map(|b| b.to_vec());
        let (name, png) = texture_bytes(&j, &find).expect("the body names a texture");
        assert_eq!(name.as_deref(), Some("player_mannequin_atlas.png"));
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the atlas is a PNG");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (64, 64));
        assert!(embedded::asset(RUNNER_BODY_ATLAS).is_some(), "the atlas file is embedded too");
    }

    #[test]
    fn the_runner_body_stands_at_player_height() {
        let j = runner_json();
        // the top of the visible cubes, in model units (the parser's scale is height / top)
        let top = j["elements"]
            .as_array()
            .expect("elements")
            .iter()
            .filter(|e| e["visibility"].as_bool() != Some(false) && e["export"].as_bool() != Some(false))
            .filter(|e| e["type"].as_str().unwrap_or("cube") == "cube")
            .map(|e| e["to"][1].as_f64().unwrap_or(0.0) as f32)
            .fold(f32::MIN, f32::max);
        let scale = crate::config::PLAYER_HEIGHT / top.max(1.0);
        assert!((top * scale - crate::config::PLAYER_HEIGHT).abs() < 0.01, "top {} at {}", top * scale, crate::config::PLAYER_HEIGHT);
    }
}
