//! The launcher's plain log file (`launcher.log` in the install folder, rotated at 256 KB to `launcher.log.old`), and
//! what goes to the console.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

const MAX_LOG_BYTES: u64 = 256 * 1024;

pub struct Log {
    file: Mutex<Option<File>>,
    echo: bool,
    lines: Mutex<Vec<String>>,
}

/// `2026-11-07T10:00:00Z` from seconds since the Unix epoch (days-to-civil, no date crate).
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

impl Log {
    /// Opens the log in `dir`; `None` keeps lines in memory only (check and dry-run write nothing).
    pub fn open(dir: Option<&Path>, echo: bool) -> Log {
        let file = dir.and_then(|d| {
            let path = d.join("launcher.log");
            if std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
                let _ = std::fs::rename(&path, d.join("launcher.log.old"));
            }
            std::fs::create_dir_all(d).ok()?;
            std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
        });
        Log { file: Mutex::new(file), echo, lines: Mutex::new(Vec::new()) }
    }

    /// A line for the log file (and the console when it is enabled).
    pub fn info(&self, text: &str) {
        let stamp = iso_utc(lr_update::state::now_unix());
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{stamp} {text}");
            }
        }
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(text.to_string());
        }
        if self.echo {
            eprintln!("{text}");
        }
    }

    /// Every line written so far (tests read this).
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_is_right() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso_utc(1_788_775_200), "2026-09-07T10:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z", "a leap day");
    }
}
