//! Build steps:
//! 1. Embeds everything under data/ (JSON content) and assets/ (pictures, Blockbench models)
//!    into the executable, so the game runs from anywhere; mod packs still load from disk.
//! 2. Gives the Windows executable the original app icon (assets/branding/logo_*.png, the same
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

fn embed(name: &str, folder: &str, exts: &[&str], text: bool, code: &mut String) {
    let root = Path::new(folder);
    let mut files = vec![];
    walk(root, root, &mut files);
    let ty = if text { "&str" } else { "&[u8]" };
    let mac = if text {
        "include_str!"
    } else {
        "include_bytes!"
    };
    code.push_str(&format!("pub static {name}: &[(&str, {ty})] = &[\n"));
    for (rel, abs) in files {
        let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if !exts.contains(&ext.as_str()) {
            continue;
        }
        code.push_str(&format!(
            "    ({rel:?}, {mac}({:?})),\n",
            abs.to_string_lossy()
        ));
    }
    code.push_str("];\n");
}

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=data");
    let mut code = String::new();
    embed("DATA_FILES", "data", &["json"], true, &mut code);
    embed(
        "ASSET_FILES",
        "assets",
        &["png", "bbmodel", "json", "glb"],
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
            ("night_watch", "packaging/windows/night_watch.ico"),
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
                std::fs::read(format!("assets/branding/logo_{s}.png"))
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
