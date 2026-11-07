//! The publisher (packaging/native/manifest.mjs, Node) and the launcher (Rust) must agree byte for byte on the manifest,
//! the compression and the ed25519 signature. This test makes a real signed tree with the Node script and installs it
//! with the launcher from the fake website. It is skipped (and says so) when `node` is not installed.

use lr_launcher::{run, Config, Kind};
use lr_update::manifest::this_platform;
use lr_update::testsite::TestSite;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    let full = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/native/manifest.mjs").canonicalize().expect("manifest.mjs exists");
    // Windows gives a \? path; node compares process.argv[1] with the module URL and does not like it.
    PathBuf::from(full.to_string_lossy().trim_start_matches("\\\\?\\"))
}

fn node(args: &[&str]) -> Option<std::process::Output> {
    Command::new("node").arg(script()).args(args).output().ok()
}

fn have_node() -> bool {
    Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("lr-node-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

#[test]
fn a_tree_made_by_the_node_publisher_installs_and_runs() {
    if !have_node() {
        eprintln!("SKIPPED: node is not installed");
        return;
    }
    let dir = temp("tree");
    let key = dir.join("signing.pem");
    let out = node(&["keygen", &s(&key)]).unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let public_hex = text.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap().to_string();
    assert_eq!(public_hex.len(), 64, "{text}");
    // a second keygen onto the same file refuses to overwrite it
    assert!(!node(&["keygen", &s(&key)]).unwrap().status.success());

    let platform = this_platform();
    let run_path = if cfg!(windows) { "bin/game.exe" } else { "bin/game" };
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("bin")).unwrap();
    std::fs::create_dir_all(src.join("data")).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_lr_stub_game"), src.join(run_path)).unwrap();
    std::fs::write(src.join("data/behavior.txt"), "exit 0").unwrap();
    std::fs::write(src.join("data/ünïcode name.txt"), "unicode file name").unwrap();
    let tree = dir.join("tree").join(platform);
    let build_out = tree.join("builds").join("3");
    let built = node(&[
        "build", "--platform", platform, "--build", "3", "--revision", "abcdef0123", "--version", "0.1.0", "--src", &s(&src), "--run", run_path, "--out",
        &s(&build_out), "--key", &s(&key), "--min-os", "11.0", "--notes", "node contract",
    ])
    .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    assert!(node(&["latest", "--platform-dir", &s(&tree), "--build", "3"]).unwrap().status.success());
    let verified = node(&["verify", "--platform-dir", &s(&tree), "--pub", &public_hex, "--platform", platform]).unwrap();
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));

    // the launcher installs it from the fake site
    let web = TestSite::start();
    web.load_dir(&format!("native/{platform}"), &tree);
    let dest = dir.join("install");
    let mut cfg = Config::new(dest.clone());
    cfg.base_url = web.url.clone();
    cfg.platform = platform.to_string();
    cfg.trusted = vec![lr_update::verify::from_hex(&public_hex).unwrap().try_into().unwrap()];
    cfg.os_version = Some("12.7.6".into());
    cfg.echo = false;
    let result = run(&cfg);
    assert_eq!((result.kind, result.exit_code, result.build), (Kind::Ran, 0, Some(3)), "{result:?}");
    let installed = lr_update::state::build_dir(&dest, 3);
    assert_eq!(std::fs::read_to_string(installed.join("data/ünïcode name.txt")).unwrap(), "unicode file name");
    assert!(std::fs::read_to_string(installed.join("ran.txt")).unwrap().contains("build=3"));

    // a changed compressed file fails the Node verifier too
    let z = build_out.join("f0.z");
    let mut bytes = std::fs::read(&z).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&z, bytes).unwrap();
    assert!(!node(&["verify", "--platform-dir", &s(&tree), "--pub", &public_hex]).unwrap().status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_node_publisher_refuses_what_the_launcher_would_refuse() {
    if !have_node() {
        eprintln!("SKIPPED: node is not installed");
        return;
    }
    let dir = temp("refuse");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("game"), "x").unwrap();
    std::fs::write(src.join(".hidden"), "x").unwrap();
    let common = |out: &Path, run: &str| {
        node(&["build", "--platform", "linux-x86_64", "--build", "1", "--revision", "r", "--version", "v", "--src", &s(&src), "--run", run, "--out", &s(out), "--no-sign"]).unwrap()
    };
    let hidden = common(&dir.join("o1"), "game");
    assert!(!hidden.status.success(), "a hidden file name must be refused");
    assert!(String::from_utf8_lossy(&hidden.stderr).contains("hidden"), "{}", String::from_utf8_lossy(&hidden.stderr));
    std::fs::remove_file(src.join(".hidden")).unwrap();
    assert!(!common(&dir.join("o2"), "missing").status.success(), "--run must name a file");
    assert!(common(&dir.join("o3"), "game").status.success());
    assert!(!node(&["build", "--platform", "freebsd", "--build", "1", "--revision", "r", "--version", "v", "--src", &s(&src), "--run", "game", "--out", &s(&dir.join("o4")), "--no-sign"])
        .unwrap()
        .status
        .success());
    // the private key may not be created inside the repository
    let inside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../should-not-exist.pem");
    assert!(!node(&["keygen", &s(&inside)]).unwrap().status.success());
    assert!(!inside.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
