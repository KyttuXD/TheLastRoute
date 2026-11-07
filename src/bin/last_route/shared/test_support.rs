//! Test-only helpers. The Godot parity checks read golden data that is local-only: the compact extracts in
//! `tests/parity`, or the full dumps in the directory named by `PARITY_DIR`. CI has neither, so those checks
//! skip (and pass) there instead of failing.

use std::path::{Path, PathBuf};

/// The Godot parity data directory: `PARITY_DIR` when it names an existing directory, else the repo's
/// `tests/parity`. `None` when neither exists.
pub fn parity_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PARITY_DIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("parity");
    repo.is_dir().then_some(repo)
}

/// True when the parity data is absent. Prints the skip line, so a parity test can `return` early on it.
pub fn skip_without_parity() -> bool {
    let missing = parity_dir().is_none();
    if missing {
        eprintln!("skipped: needs tests/parity (local-only)");
    }
    missing
}
