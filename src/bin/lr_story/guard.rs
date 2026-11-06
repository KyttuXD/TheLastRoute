//! Import guard. lr_story's code was copied from last_route and adapted to stand alone, so it must never reach back into
//! the game: no `crate::app`, `crate::world`, `crate::hunters` or `crate::game` paths, and nothing that reads the game's
//! embedded data. This test reads every `.rs` file under this folder (recursively) and fails on a banned line.
//!
//! Comment lines (`//`, `///`, `//!`) are skipped: they describe the game's data in prose and import nothing. This file
//! is skipped too, because it holds the list.

use std::fs;
use std::path::{Path, PathBuf};

const BANNED: &[&str] = &[
    "use crate::app",
    "use crate::world",
    "use crate::hunters",
    "crate::world::",
    "crate::hunters::",
    "crate::app::",
    "crate::game::",
    "embedded",
];

fn lr_story_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("bin").join("lr_story")
}

/// Every `.rs` file under `dir`, recursively, except this guard.
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for path in read.filter_map(|e| e.ok().map(|e| e.path())) {
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().map_or(false, |x| x == "rs") && path.file_name().map_or(true, |n| n != "guard.rs") {
            out.push(path);
        }
    }
}

/// Every banned code line under `root`, as `path:line: text` (path relative to `root`). Pure, so tests can run it on a
/// temp folder.
fn violations(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    rs_files(root, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let shown = file.strip_prefix(root).unwrap_or(&file).display().to_string().replace('\\', "/");
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if BANNED.iter().any(|b| line.contains(b)) {
                found.push(format!("{shown}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn no_lr_story_file_reaches_into_the_game() {
    let found = violations(&lr_story_dir());
    assert!(found.is_empty(), "the import guard found {} line(s):\n  {}", found.len(), found.join("\n  "));
}

#[test]
fn the_guard_flags_code_lines_and_skips_comments_and_nested_folders() {
    let dir = std::env::temp_dir().join(format!("lr_story_guard_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.rs"), "// embedded in prose\nuse crate::world::Map;\nlet s = \"embedded\";\n").unwrap();
    fs::write(dir.join("sub").join("b.rs"), "use crate::hunters::Wolf;\n").unwrap();
    fs::write(dir.join("guard.rs"), "use crate::app::Thing;\n").unwrap();
    fs::write(dir.join("notes.txt"), "use crate::app::Thing;\n").unwrap();

    let found = violations(&dir);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().any(|f| f.starts_with("a.rs:2: ")), "{found:?}");
    assert!(found.iter().any(|f| f.starts_with("a.rs:3: ")), "{found:?}");
    assert!(found.iter().any(|f| f.starts_with("sub/b.rs:1: ")), "{found:?}");
    let _ = fs::remove_dir_all(&dir);
}
