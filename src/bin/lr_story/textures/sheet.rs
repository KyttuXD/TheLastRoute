//! Contact sheet of the lab textures, to check the flat look by eye: one column per lab kind (`LAB_KINDS`), the painted
//! texture on the top row and the flat one under it, each texel drawn `SCALE` x `SCALE` pixels. `--texture-sheet PATH`
//! writes it as a PNG and exits.

use super::canvas::{PX, WALL_H};
use super::{paint_styled, Style, LAB_KINDS};
use macroquad::prelude::Image;
use std::path::Path;

/// Screen pixels per texel.
pub const SCALE: i32 = 3;
/// Gap between cells and around the sheet, in screen pixels.
const GAP: i32 = 8;
/// The sheet's background, opaque.
const BACK: [u8; 4] = [0x30, 0x30, 0x34, 0xff];

/// The sheet as RGBA8, rows top-down: `(width, height, bytes)`. Every cell is as big as the tallest texture (a 32 x 128
/// wall) and holds its texture top-left; the top row is `Style::Painted`, the bottom row `Style::Flat`.
pub fn lab_sheet() -> (u16, u16, Vec<u8>) {
    let cell_w = PX * SCALE;
    let cell_h = WALL_H * SCALE;
    let cols = LAB_KINDS.len() as i32;
    let w = GAP + cols * (cell_w + GAP);
    let h = GAP + 2 * (cell_h + GAP);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for p in px.chunks_exact_mut(4) {
        p.copy_from_slice(&BACK);
    }
    for (row, style) in [Style::Painted, Style::Flat].into_iter().enumerate() {
        for (col, (group, kind)) in LAB_KINDS.iter().enumerate() {
            let Some((tw, th, tpx)) = paint_styled(group, kind, 0, style) else {
                continue;
            };
            let (tw, th) = (tw as i32, th as i32);
            let ox = GAP + col as i32 * (cell_w + GAP);
            let oy = GAP + row as i32 * (cell_h + GAP);
            for y in 0..th {
                for x in 0..tw {
                    let s = ((y * tw + x) * 4) as usize;
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let (sx, sy) = (ox + x * SCALE + dx, oy + y * SCALE + dy);
                            let d = ((sy * w + sx) * 4) as usize;
                            px[d..d + 4].copy_from_slice(&tpx[s..s + 4]);
                        }
                    }
                }
            }
        }
    }
    (w as u16, h as u16, px)
}

/// Writes the lab sheet to `path` as a PNG, creating its folder first.
///
/// `Image::export_png` flips the rows (it expects GL's bottom-up order), so the rows go in bottom-up here and come out
/// top-down in the file.
pub fn write_lab_sheet(path: &Path) -> Result<(), String> {
    let (w, h, px) = lab_sheet();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let row = w as usize * 4;
    let mut bottom_up = Vec::with_capacity(px.len());
    for y in (0..h as usize).rev() {
        bottom_up.extend_from_slice(&px[y * row..(y + 1) * row]);
    }
    let img = Image {
        bytes: bottom_up,
        width: w,
        height: h,
    };
    img.export_png(&path.to_string_lossy());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_has_a_painted_and_a_flat_row_of_every_lab_kind() {
        let (w, h, px) = lab_sheet();
        assert_eq!(px.len(), w as usize * h as usize * 4);
        let cell_w = PX * SCALE;
        let cell_h = WALL_H * SCALE;
        assert_eq!(w as i32, GAP + LAB_KINDS.len() as i32 * (cell_w + GAP));
        assert_eq!(h as i32, GAP + 2 * (cell_h + GAP));
        // every cell's top-left texel is painted, not background
        for row in 0..2 {
            for col in 0..LAB_KINDS.len() as i32 {
                let (x, y) = (GAP + col * (cell_w + GAP), GAP + row * (cell_h + GAP));
                let o = ((y * w as i32 + x) * 4) as usize;
                assert_ne!(&px[o..o + 4], &BACK[..], "cell ({row}, {col}) is background");
            }
        }
    }

    #[test]
    fn sheet_is_the_same_twice() {
        assert!(lab_sheet() == lab_sheet());
    }
}
