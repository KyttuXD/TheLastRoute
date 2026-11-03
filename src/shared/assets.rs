//! Everything drawn from files: the original's region wall/floor pictures, the hunters'
//! Blockbench models and portraits, the props rebuilt as Blockbench models, the inventory
//! icons and the app icon. Fonts are the system faces the original asks for (Courier New bold
//! for text, Impact for titles).

use crate::content::{embedded, packs, Content};
use crate::render::bbmodel::Model;
use crate::render::builder::white_texture;
use crate::render::glb::GlbModel;
use crate::world::TEXTURE_SETS;
use macroquad::prelude::*;
use std::collections::HashMap;

/// The hunter model the NPC teammates wear (the base body; there is no mannequin asset in this tree).
const NPC_BODY_HUNTER: &str = "dovelle";

pub struct Assets {
    pub walls: HashMap<&'static str, Texture2D>,
    pub floors: HashMap<&'static str, Texture2D>,
    pub white: Texture2D,
    pub hunters: HashMap<String, Model>,
    /// The NPC teammates' body (game/npc.rs): the base hunter model (dovelle) parsed at the runner's height.
    pub npc_body: Option<Model>,
    pub portraits: HashMap<String, Texture2D>,
    pub props: HashMap<String, Model>,
    /// The original's Blender exports by asset path: props/<set>/<name>.glb and interactables/<name>.glb
    /// (and "pack:<id>:<path>" for pack props).
    pub glb: HashMap<String, GlbModel>,
    /// flashlight, taser, crowbar, axe, battery, stim
    pub icons: Vec<Texture2D>,
    pub logo: Option<Texture2D>,
    pub font: Option<Font>,
    pub title_font: Option<Font>,
    /// Courier New italic: the death moment's caption lines.
    pub caption_font: Option<Font>,
}

pub fn tex(bytes: &[u8]) -> Texture2D {
    let t = Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png));
    t.set_filter(FilterMode::Nearest);
    t
}

fn asset_tex(path: &str) -> Option<Texture2D> {
    embedded::asset(path).map(tex)
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
        let mut npc_body = None;
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
                    if h.id == NPC_BODY_HUNTER {
                        // the NPC teammates' body: the same model at the runner's height (game/npc.rs)
                        npc_body =
                            Model::parse(&text, atlas.clone(), &find, Some(crate::config::PLAYER_HEIGHT));
                    }
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
        let prop_atlas = asset_tex("props/blocky_atlas.png");
        let mut props = HashMap::new();
        for (path, bytes) in embedded::assets_under("props/") {
            let Some(name) = path
                .strip_prefix("props/")
                .and_then(|n| n.strip_suffix(".bbmodel"))
            else {
                continue;
            };
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some(m) = Model::parse(text, prop_atlas.clone(), &|_| None, None) {
                    props.insert(name.to_string(), m);
                }
            }
        }
        let white = white_texture();
        let mut glb = HashMap::new();
        for (path, bytes) in
            embedded::assets_under("props/").chain(embedded::assets_under("interactables/"))
        {
            if !path.ends_with(".glb") {
                continue;
            }
            let dir = path.rsplit_once('/').map(|x| x.0).unwrap_or("");
            let sibling = |uri: &str| embedded::asset(&format!("{dir}/{uri}")).map(|b| b.to_vec());
            if let Some(m) = GlbModel::parse(bytes, &sibling, &white) {
                glb.insert(path.to_string(), m);
            }
        }
        for def in content.props.values() {
            for m in def
                .models
                .iter()
                .filter(|m| m.starts_with("pack:") && m.ends_with(".glb"))
            {
                let mut it = m.splitn(3, ':');
                let (_, pid, rel) = (it.next(), it.next().unwrap_or(""), it.next().unwrap_or(""));
                let Some(pack) = content.packs.iter().find(|p| p.id == pid) else {
                    continue;
                };
                let dir = rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
                let sibling = |uri: &str| packs::read_pack_file(&pack.dir, &format!("{dir}/{uri}"));
                if let Some(model) = packs::read_pack_file(&pack.dir, rel)
                    .and_then(|b| GlbModel::parse(&b, &sibling, &white))
                {
                    glb.insert(m.clone(), model);
                }
            }
        }
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
            props,
            glb,
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
