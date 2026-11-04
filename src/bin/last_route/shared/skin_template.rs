//! The skin template a player paints over (Godot PlayerSkin.template_png, player_skin.gd): the 64x64 default skin with
//! every overlay rectangle cleared. The rectangles are data (data/skins/rules.json `overlay_rects`, each [x, y, w, h]).

use crate::shared::paths;
use macroquad::prelude::{Image, ImageFormat};
use std::path::{Path, PathBuf};

/// The file the SKIN tab's TEMPLATE button writes.
pub const TEMPLATE_FILE: &str = "lastroute_skin_template.png";
const SIZE: usize = 64;
const DEFAULT_ATLAS: &[u8] = include_bytes!("../assets/player/player_mannequin_atlas.png");
const RULES: &str = include_str!("../data/skins/rules.json");

/// The overlay rectangles as (x, y, w, h), from the rules data. Accepts [x, y, w, h] arrays or {x, y, w, h} objects.
pub fn overlay_rects() -> Vec<(usize, usize, usize, usize)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(RULES) else {
        return Vec::new();
    };
    let Some(list) = v.get("layout").and_then(|l| l.get("overlay_rects")).or_else(|| v.get("overlay_rects")).and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|r| {
            if let Some(a) = r.as_array() {
                let n = |i: usize| a.get(i).and_then(|x| x.as_u64()).map(|x| x as usize);
                Some((n(0)?, n(1)?, n(2)?, n(3)?))
            } else {
                let n = |k: &str| r.get(k).and_then(|x| x.as_u64()).map(|x| x as usize);
                Some((n("x")?, n("y")?, n("w")?, n("h")?))
            }
        })
        .collect()
}

/// The default skin as 64x64 RGBA bytes (the mannequin atlas), or a flat fill like Godot's fallback.
fn default_rgba() -> Vec<u8> {
    if let Ok(img) = Image::from_file_with_format(DEFAULT_ATLAS, Some(ImageFormat::Png)) {
        if img.width as usize == SIZE && img.height as usize == SIZE && img.bytes.len() == SIZE * SIZE * 4 {
            return img.bytes;
        }
    }
    [158u8, 128, 82, 255].repeat(SIZE * SIZE)
}

/// The template: the default skin with every overlay rectangle fully transparent, as a PNG.
pub fn template_png() -> Vec<u8> {
    let mut px = default_rgba();
    for (x, y, w, h) in overlay_rects() {
        for yy in y..(y + h).min(SIZE) {
            for xx in x..(x + w).min(SIZE) {
                let i = (yy * SIZE + xx) * 4;
                px[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    encode_png(&px, SIZE as u32, SIZE as u32)
}

/// The Downloads folder, else the user data folder.
pub fn downloads_dir() -> Option<PathBuf> {
    let home = if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" };
    std::env::var(home)
        .ok()
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join("Downloads"))
        .filter(|d| d.is_dir())
        .or_else(paths::user_dir)
}

/// Writes the template into `dir` (created if missing) and returns the file path.
pub fn save_template(dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not open {}: {e}", dir.display()))?;
    let path = dir.join(TEMPLATE_FILE);
    std::fs::write(&path, template_png()).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// Saves the template to the Downloads folder (user folder fallback).
pub fn save_template_to_downloads() -> Result<PathBuf, String> {
    let dir = downloads_dir().ok_or_else(|| "no folder to save into".to_string())?;
    save_template(&dir)
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut tagged = kind.to_vec();
    tagged.extend_from_slice(body);
    out.extend_from_slice(&tagged);
    out.extend_from_slice(&crc32(&tagged).to_be_bytes());
}

/// A minimal RGBA8 PNG (stored deflate blocks; a skin is tiny).
fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(b) = blocks.next() {
        z.push(if blocks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut s) = (1u32, 0u32);
    for &b in &raw {
        a = (a + b as u32) % 65521;
        s = (s + a) % 65521;
    }
    z.extend_from_slice(&((s << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Image {
        Image::from_file_with_format(bytes, Some(ImageFormat::Png)).expect("the template decodes")
    }

    #[test]
    fn template_is_64x64_with_clear_overlays_and_opaque_base() {
        let rects = overlay_rects();
        assert!(!rects.is_empty(), "rules.json overlay_rects is the data this reads");
        let img = decode(&template_png());
        assert_eq!((img.width as usize, img.height as usize), (64, 64));
        let alpha = |x: usize, y: usize| img.bytes[(y * 64 + x) * 4 + 3];
        for (x, y, w, h) in &rects {
            for yy in *y..y + h {
                for xx in *x..x + w {
                    assert_eq!(alpha(xx, yy), 0, "overlay pixel {xx},{yy} is clear");
                }
            }
        }
        let inside = |x: usize, y: usize| rects.iter().any(|(rx, ry, w, h)| x >= *rx && x < rx + w && y >= *ry && y < ry + h);
        let mut base = 0;
        for y in 0..64 {
            for x in 0..64 {
                if !inside(x, y) && alpha(x, y) == 255 {
                    base += 1;
                }
            }
        }
        assert!(base > 0, "the base layer keeps opaque pixels");
    }

    #[test]
    fn saved_file_has_the_name_and_the_same_bytes() {
        let dir = std::env::temp_dir().join(format!("lr_skin_template_{}", std::process::id()));
        let path = save_template(&dir).expect("saves");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "lastroute_skin_template.png");
        assert_eq!(std::fs::read(&path).unwrap(), template_png());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encoder_round_trips_pixels() {
        let px: Vec<u8> = (0..SIZE * SIZE * 4).map(|i| (i * 7 % 256) as u8).collect();
        let img = decode(&encode_png(&px, 64, 64));
        assert_eq!(img.bytes, px);
    }
}
