//! A check that the PNG decoder gives the same pixels as it did before a change to it: every `.png` in the repository,
//! every picture embedded in a `.glb` and every base64 texture of a `.bbmodel` is decoded and its size and pixels are
//! hashed. Run by hand (it reads about two thousand files):
//!   LR_PNG_HASHES=/tmp/before.txt <test binary> --ignored png_parity   writes the list
//!   LR_PNG_HASHES=/tmp/before.txt LR_PNG_CHECK=1 <test binary> --ignored png_parity   compares with it

use macroquad::prelude::Image;
use sha2::{Digest, Sha256};
use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if name != "target" && name != ".git" && name != "dist" {
                walk(&p, out);
            }
        } else if name.ends_with(".png") || name.ends_with(".glb") || name.ends_with(".bbmodel") {
            out.push(p);
        }
    }
}

fn line(tag: &str, bytes: &[u8]) -> String {
    match Image::from_file_with_format(bytes, None) {
        Ok(img) => {
            let mut h = Sha256::new();
            h.update(img.width.to_le_bytes());
            h.update(img.height.to_le_bytes());
            h.update(&img.bytes);
            let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            format!("{tag} {}x{} {hex}", img.width, img.height)
        }
        Err(_) => format!("{tag} UNREADABLE"),
    }
}

#[test]
#[ignore = "reads every picture in the repository; see the top of the file"]
fn png_parity() {
    let Ok(list) = std::env::var("LR_PNG_HASHES") else { return };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    walk(root, &mut files);
    files.sort();
    let mut lines = vec![];
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy().to_string();
        let Ok(bytes) = std::fs::read(f) else { continue };
        if rel.ends_with(".png") {
            lines.push(line(&rel, &bytes));
        } else if rel.ends_with(".glb") {
            if let Some(glb) = crate::render::glb::Glb::parse(&bytes) {
                let buffers = glb.buffers(&|_| None).unwrap_or_default();
                for (i, img) in glb.images().into_iter().enumerate() {
                    if let Some(crate::render::glb::ImageSource::View { buffer, offset, length }) = img {
                        if let Some(png) = buffers.get(buffer).and_then(|b| b.get(offset..offset + length)) {
                            lines.push(line(&format!("{rel}#{i}"), png));
                        }
                    }
                }
            }
        } else if let Ok(text) = std::str::from_utf8(&bytes) {
            let mut n = 0;
            for part in text.split("data:image/png;base64,").skip(1) {
                let b64: String = part.chars().take_while(|c| *c != '"').collect();
                lines.push(line(&format!("{rel}#{n}"), &crate::content::embedded::base64_decode(&b64)));
                n += 1;
            }
        }
    }
    let text = lines.join("\n") + "\n";
    if std::env::var("LR_PNG_CHECK").is_ok() {
        let before = std::fs::read_to_string(&list).expect("the list written before the change");
        let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), text.lines().collect());
        assert_eq!(a.len(), b.len(), "the same pictures are found");
        let differ: Vec<String> = a.iter().zip(&b).filter(|(x, y)| x != y).take(10).map(|(x, y)| format!("{x}\n  now {y}")).collect();
        assert!(differ.is_empty(), "{} pictures decode differently, the first:\n{}", a.iter().zip(&b).filter(|(x, y)| x != y).count(), differ.join("\n"));
        eprintln!("png parity: {} pictures decode as before", a.len());
    } else {
        std::fs::write(&list, text).expect("the list is written");
        eprintln!("png parity: {} pictures listed, {} unreadable", lines.len(), lines.iter().filter(|l| l.ends_with("UNREADABLE")).count());
    }
}
