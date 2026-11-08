//! F2 saves the game window as it is on screen, from any screen (screenshot.gd): a PNG named for the
//! UTC date and time in the user folder's `screenshots` directory (`KYTTU_YYYY-MM-DD_HH-MM-SS.png`; a
//! second press in the same second gets `_2`, `_3` ...). A short line under the bottom-left corner names
//! the file once it is saved. The loop draws that line after the capture, so a picture never shows it.

use super::paths;
use crate::app::assets::Assets;
use crate::ui::style::{alpha, label_outline, lh, pal, rgba, view, F};
use macroquad::prelude::*;
use std::path::{Path, PathBuf};

/// How long the saved line stays fully shown before it fades (screenshot.gd SHOW_SECONDS).
const SHOW_SECONDS: f32 = 1.8;
/// The fade-out at the end of the line (screenshot.gd's 0.4 s tween).
const FADE_SECONDS: f32 = 0.4;
const LINE_SIZE: f32 = 7.0;
/// The gap between the line and the window's bottom-left corner (screenshot.gd offsets of 16).
const MARGIN: f32 = 16.0;

/// The line under the corner: its text and the seconds it has left on screen.
#[derive(Default)]
pub struct Screenshots {
    line: Option<(String, f32)>,
}

impl Screenshots {
    /// Saves the window now and sets the line to name the file, or to say that nothing was written.
    pub fn capture(&mut self) {
        let text = match save_capture() {
            Some(path) => {
                let name = path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                crate::lr_info!("screenshot: {}", path.display());
                format!("SCREENSHOT  {name}")
            }
            None if super::platform::is_metal() => "SCREENSHOT NEEDS OPENGL (SETTINGS, GRAPHICS API)".to_owned(),
            None => "SCREENSHOT FAILED: nothing was written".to_owned(),
        };
        self.line = Some((text, SHOW_SECONDS + FADE_SECONDS));
    }

    /// Draws the line for this frame and lets it fade. Call it after `capture` and the rest of the frame.
    pub fn draw(&mut self, a: &Assets, dt: f32) {
        let Some((text, left)) = self.line.as_mut() else {
            return;
        };
        *left -= dt;
        if *left <= 0.0 {
            self.line = None;
            return;
        }
        let fade = (*left / FADE_SECONDS).min(1.0);
        let top = view().y - MARGIN - lh(F::Body, LINE_SIZE);
        label_outline(
            a,
            F::Body,
            text.as_str(),
            MARGIN,
            top,
            LINE_SIZE,
            alpha(pal().body, fade),
            rgba(0.0, 0.0, 0.0, 0.85 * fade),
            2.0,
        );
    }
}

/// Writes what the window shows to a new file in the captures folder. None when nothing could be written.
fn save_capture() -> Option<PathBuf> {
    if super::platform::is_metal() {
        crate::lr_warn!("screenshot skipped: the Metal window cannot be read back (use --gfx gl)");
        return None;
    }
    let dir = paths::user_dir()?.join("screenshots");
    std::fs::create_dir_all(&dir).ok()?;
    let path = unused_path(&dir, &file_stem(stamp_now()));
    let mut img = get_screen_data();
    // the swapchain's alpha is not reliable, so the picture is opaque (as `save_screen` does)
    for px in img.bytes.chunks_mut(4) {
        px[3] = 255;
    }
    img.export_png(path.to_str()?);
    Some(path)
}

/// `dir/<stem>.png`, or the first `<stem>_2.png`, `<stem>_3.png` ... that does not exist yet.
fn unused_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.png"));
    if !first.exists() {
        return first;
    }
    let mut n = 2;
    loop {
        let path = dir.join(format!("{stem}_{n}.png"));
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

/// A UTC date and time, as the file name is written.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    year: u16,
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
}

fn file_stem(s: Stamp) -> String {
    format!(
        "KYTTU_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        s.year, s.month, s.day, s.hour, s.minute, s.second
    )
}

/// The UTC time, on every platform: screenshot file names stay UTC and never follow the player's time zone.
fn stamp_now() -> Stamp {
    let secs = crate::shared::clock::SystemTime::now()
        .duration_since(crate::shared::clock::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    stamp_from_unix(secs)
}

/// The UTC date and time of a Unix time (days to civil dates, Howard Hinnant's algorithm).
fn stamp_from_unix(secs: i64) -> Stamp {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let rem = secs.rem_euclid(86_400);
    Stamp {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        hour: (rem / 3_600) as u16,
        minute: (rem % 3_600 / 60) as u16,
        second: (rem % 60) as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_time_becomes_the_utc_date_and_time() {
        assert_eq!(
            file_stem(stamp_from_unix(0)),
            "KYTTU_1970-01-01_00-00-00"
        );
        assert_eq!(
            file_stem(stamp_from_unix(1_000_000_000)),
            "KYTTU_2001-09-09_01-46-40"
        );
        assert_eq!(
            file_stem(stamp_from_unix(1_709_164_799)),
            "KYTTU_2024-02-28_23-59-59"
        );
    }

    #[test]
    fn leap_day_is_a_day() {
        assert_eq!(file_stem(stamp_from_unix(1_709_164_800)), "KYTTU_2024-02-29_00-00-00");
    }
}
