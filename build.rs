//! Build steps:
//! 1. Embeds the game's data (src/bin/last_route/data, including regions/) and assets (src/bin/last_route/assets) (pictures, Blockbench models)
//!    into the executable, so the game runs from anywhere; mod packs still load from disk.
//! 2. Gives the Windows executable the original app icon (src/bin/last_route/assets/branding/logo_*.png, the same
//!    octagon the Godot export presets use), packed into an .ico at build time.

use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, p.canonicalize().unwrap()));
        }
    }
}

/// One static list of the files under each `(folder, key prefix)` root with an extension in `exts` that `skip` (given
/// the key) does not reject: the key is the prefix plus the path under the folder, so the keys do not change when a
/// folder moves.
fn embed(name: &str, roots: &[(&str, &str)], exts: &[&str], skip: &dyn Fn(&str) -> bool, text: bool, code: &mut String) {
    let ty = if text { "&str" } else { "&[u8]" };
    let mac = if text {
        "include_str!"
    } else {
        "include_bytes!"
    };
    code.push_str(&format!("pub static {name}: &[(&str, {ty})] = &[\n"));
    for (folder, prefix) in roots {
        let root = Path::new(folder);
        let mut files = vec![];
        walk(root, root, &mut files);
        for (rel, abs) in files {
            let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            let key = format!("{prefix}{rel}");
            if !exts.contains(&ext.as_str()) || skip(&key) {
                continue;
            }
            code.push_str(&format!(
                "    ({:?}, {mac}({:?})),\n",
                key,
                abs.to_string_lossy()
            ));
        }
    }
    code.push_str("];\n");
}

/// The commit this build comes from: GITHUB_SHA in CI, else `git rev-parse HEAD`, else "unknown".
fn revision() -> String {
    let ci = std::env::var("GITHUB_SHA").ok().map(|sha| sha.trim().to_string()).filter(|sha| !sha.is_empty());
    let git = || {
        let out = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
        let sha = String::from_utf8(out.stdout).ok()?;
        out.status.success().then(|| sha.trim().to_string()).filter(|sha| !sha.is_empty())
    };
    ci.or_else(git).unwrap_or_else(|| "unknown".into())
}

fn main() {
    println!("cargo:rerun-if-changed=src/bin/last_route/assets");
    // the CI run number is this build's number for the update check; a local build has 0 and never updates
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    let build = std::env::var("GITHUB_RUN_NUMBER").ok().filter(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())).unwrap_or_else(|| "0".into());
    println!("cargo:rustc-env=LR_BUILD_NUMBER={build}");
    // the commit this build comes from, so the update check can tell a rebuild of the same commit from a new one
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rustc-env=LR_REVISION={}", revision());
    println!("cargo:rerun-if-changed=src/bin/last_route/data");
    let mut code = String::new();
    // the game's data, region files (regions/) and pictures all live in its bin folder
    embed(
        "DATA_FILES",
        &[("src/bin/last_route/data", "")],
        &["json"],
        &|_| false,
        true,
        &mut code,
    );
    embed(
        "ASSET_FILES",
        &[("src/bin/last_route/assets", "")],
        // ttf: the open-licence stand-in fonts (assets/fonts/, OFL 1.1) the browser build falls back to
        &["png", "bbmodel", "json", "ttf"],
        // The game reads every model through the prop data (props/<set>/<name>.bbmodel, props/blocky/...) and
        // interactables/; the one-per-prop props/<id>.bbmodel rebuilds from before the data named variants are not
        // loaded by anything, so they stay on disk and out of the executable. GLBs are not embedded either (the
        // converter's sources live in tools/props/source_glb).
        &|key| key.strip_prefix("props/").is_some_and(|rest| !rest.contains('/') && rest.ends_with(".bbmodel")),
        false,
        &mut code,
    );
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("embedded.rs"), code).unwrap();

    // Windows executables each carry their own icon. `compile_for` links a resource into that one
    // binary only (cargo:rustc-link-arg-bin), so the server and Night Watch do not share the game's.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for (bin, ico) in [
            ("lr_server", "packaging/windows/lr_server.ico"),
        ] {
            println!("cargo:rerun-if-changed={ico}");
            let mut res = winresource::WindowsResource::new();
            res.set_icon(ico);
            if let Err(e) = res.compile_for(bin) {
                println!("cargo:warning={bin} icon not embedded: {e}");
            }
        }
    }

    // the game's .exe icon only when building for Windows (macOS and Linux get theirs from packaging/)
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let sizes = [16u32, 32, 64, 128, 256];
        let images: Vec<(u32, Vec<u8>)> = sizes
            .iter()
            .filter_map(|s| {
                std::fs::read(format!("src/bin/last_route/assets/branding/logo_{s}.png"))
                    .ok()
                    .map(|b| (*s, b))
            })
            .collect();
        if images.is_empty() {
            return;
        }
        let mut ico = vec![0u8, 0, 1, 0];
        ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * images.len() as u32;
        for (size, data) in &images {
            let dim = if *size >= 256 { 0 } else { *size as u8 };
            ico.extend_from_slice(&[dim, dim, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in &images {
            ico.extend_from_slice(data);
        }
        let out = out_dir.join("last_route.ico");
        std::fs::write(&out, ico).unwrap();
        let mut res = winresource::WindowsResource::new();
        res.set_icon(out.to_str().unwrap());
        if let Err(e) = res.compile_for("last_route") {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
