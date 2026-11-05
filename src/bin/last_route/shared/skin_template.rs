//! The skin templates a player paints over (Godot PlayerSkin.template_png, player_skin.gd, and the slim twin in
//! docs/player_skin_template_slim.png): the 64x64 default skin with every overlay rectangle cleared. The slim one also
//! clears the arms' 4th texture column, which is how a skin says "slim arms" (Alex). The rectangles are data
//! (data/skins/rules.json `overlay_rects` and `slim_probe_rects`, each [x, y, w, h]).

use crate::shared::paths;
use crate::shared::skins::{encode_png, layout};
use macroquad::prelude::{Image, ImageFormat};
use std::path::{Path, PathBuf};

/// The file the SKIN tab's TEMPLATE button writes.
pub const TEMPLATE_FILE: &str = "lastroute_skin_template.png";
/// The file the SLIM TEMPLATE button writes.
pub const TEMPLATE_SLIM_FILE: &str = "lastroute_skin_template_slim.png";
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

fn clear_rect(px: &mut [u8], (x, y, w, h): (usize, usize, usize, usize)) {
    for yy in y..(y + h).min(SIZE) {
        for xx in x..(x + w).min(SIZE) {
            let i = (yy * SIZE + xx) * 4;
            px[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
    }
}

/// The template pixels: the default skin with every overlay rectangle fully transparent; `slim` also clears the arms'
/// 4th column in both arm bases.
pub fn template_rgba(slim: bool) -> Vec<u8> {
    let mut px = default_rgba();
    for r in overlay_rects() {
        clear_rect(&mut px, r);
    }
    if slim {
        for r in layout().map(|l| l.slim_probe_rects).unwrap_or_default() {
            clear_rect(&mut px, (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize));
        }
    }
    px
}

/// The template as a PNG (written with the game's own small encoder).
pub fn template_png() -> Vec<u8> {
    encode_png(&template_rgba(false), SIZE as u32, SIZE as u32)
}

/// The slim template as a PNG.
pub fn template_slim_png() -> Vec<u8> {
    encode_png(&template_rgba(true), SIZE as u32, SIZE as u32)
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

/// A file name in `dir` that is not taken: `name.png`, else `name-2.png`, `name-3.png`, ... A template is never
/// written over a file that is already there (it may be the player's painted skin).
pub fn free_name(dir: &Path, file: &str) -> PathBuf {
    let first = dir.join(file);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, "png"));
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn write_template(dir: &Path, file: &str, png: Vec<u8>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not open {}: {e}", dir.display()))?;
    let path = free_name(dir, file);
    std::fs::write(&path, png).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// Writes the classic template into `dir` (created if missing) and returns the file path.
pub fn save_template(dir: &Path) -> Result<PathBuf, String> {
    write_template(dir, TEMPLATE_FILE, template_png())
}

/// Writes the slim template into `dir` (created if missing) and returns the file path.
pub fn save_template_slim(dir: &Path) -> Result<PathBuf, String> {
    write_template(dir, TEMPLATE_SLIM_FILE, template_slim_png())
}

/// Saves the template to the Downloads folder (user folder fallback).
pub fn save_template_to_downloads(slim: bool) -> Result<PathBuf, String> {
    let dir = downloads_dir().ok_or_else(|| "no folder to save into".to_string())?;
    if slim {
        save_template_slim(&dir)
    } else {
        save_template(&dir)
    }
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
    fn the_slim_template_clears_the_arms_fourth_column_and_reads_as_slim() {
        let classic = template_rgba(false);
        let slim = template_rgba(true);
        let probes = layout().expect("layout").slim_probe_rects;
        assert!(!probes.is_empty());
        let mut changed = 0;
        for i in (3..classic.len()).step_by(4) {
            if classic[i] != slim[i] {
                changed += 1;
            }
        }
        assert!(changed > 0, "the slim template differs from the classic one");
        for r in &probes {
            for y in r[1]..r[1] + r[3] {
                for x in r[0]..r[0] + r[2] {
                    assert_eq!(slim[((y * 64 + x) * 4 + 3) as usize], 0, "slim probe pixel {x},{y} is clear");
                }
            }
        }
        let classic_v = crate::shared::skins::validate_skin(&template_png(), 65536).expect("the classic template is a valid skin");
        let slim_v = crate::shared::skins::validate_skin(&template_slim_png(), 65536).expect("the slim template is a valid skin");
        assert!(!classic_v.slim, "the classic template reads as classic arms");
        assert!(slim_v.slim, "the slim template reads as slim arms (Alex)");
    }

    #[test]
    fn templates_are_small_pngs_through_the_shared_encoder() {
        // the old private encoder stored the pixels uncompressed (16.5 KB); the shared one compresses them like Godot's
        assert!(template_png().len() < 4096, "{} bytes", template_png().len());
        assert!(template_slim_png().len() < 4096);
        assert!(crate::shared::skin_png::check_png(
            &template_png(),
            crate::shared::skin_png::Limits { max_bytes: 65536, max_chunks: 64 },
            crate::shared::skin_png::Shape::Remote
        )
        .is_ok());
    }

    #[test]
    fn saved_file_has_the_name_and_the_same_bytes() {
        let dir = std::env::temp_dir().join(format!("lr_skin_template_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = save_template(&dir).expect("saves");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "lastroute_skin_template.png");
        assert_eq!(std::fs::read(&path).unwrap(), template_png());
        let slim = save_template_slim(&dir).expect("saves");
        assert_eq!(slim.file_name().unwrap().to_str().unwrap(), "lastroute_skin_template_slim.png");
        assert_eq!(std::fs::read(&slim).unwrap(), template_slim_png());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_template_never_overwrites_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("lr_skin_template_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = save_template(&dir).unwrap();
        std::fs::write(&first, b"my painted skin").unwrap();
        let second = save_template(&dir).unwrap();
        assert_eq!(second.file_name().unwrap().to_str().unwrap(), "lastroute_skin_template-2.png");
        assert_eq!(std::fs::read(&first).unwrap(), b"my painted skin", "the first file is untouched");
        let third = save_template(&dir).unwrap();
        assert_eq!(third.file_name().unwrap().to_str().unwrap(), "lastroute_skin_template-3.png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
