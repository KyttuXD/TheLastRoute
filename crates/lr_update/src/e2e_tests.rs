//! fetch + install against the in-process fake website (loopback http).

use crate::fetch::{FetchError, Site};
use crate::install::{install_build, verify_installed};
use crate::testsite::{BuildSpec, Fault, TestSite};
use crate::verify::test_keys;
use std::path::PathBuf;

const PLATFORM: &str = "windows-x86_64";

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("lr-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn site_for(web: &TestSite, key_seed: u8) -> Site {
    Site::new(&web.url, "", PLATFORM, vec![test_keys::public(&test_keys::pair(key_seed))]).unwrap()
}

fn manifest_of(site: &Site) -> crate::manifest::Manifest {
    site.fetch_manifest(&site.fetch_latest(None).unwrap(), None).unwrap().0
}

#[test]
fn a_published_build_downloads_verifies_and_unpacks() {
    let web = TestSite::start();
    let key = test_keys::pair(1);
    let program = (0..200_000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
    let published = web.publish(&BuildSpec::simple(PLATFORM, 7, program.clone(), &key));
    let site = site_for(&web, 1);
    let latest = site.fetch_latest(None).unwrap();
    assert_eq!(latest.build, 7);
    let (m, _, _) = site.fetch_manifest(&latest, None).unwrap();
    assert_eq!(m, published);
    let dir = temp("ok");
    let (stage, scratch) = (dir.join("stage"), dir.join("scratch"));
    let mut seen = Vec::new();
    install_build(&site, &m, &stage, &scratch, None, None, &mut |p| seen.push(p.clone())).unwrap();
    assert_eq!(std::fs::read(stage.join("bin").join("game")).unwrap(), program);
    assert_eq!(std::fs::read_to_string(stage.join("data").join("readme.txt")).unwrap(), "build 7\n");
    assert!(!scratch.exists(), "the compressed downloads are removed");
    assert!(verify_installed(&stage, &m, true).is_ok());
    assert_eq!(seen.last().unwrap().percent(), 100);
    assert_eq!(seen.first().unwrap().percent(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_chosen_files_are_downloaded() {
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple(PLATFORM, 2, vec![1, 2, 3], &key));
    let site = site_for(&web, 1);
    let m = manifest_of(&site);
    let dir = temp("only");
    web.clear_hits();
    install_build(&site, &m, &dir.join("s"), &dir.join("t"), Some(&|p: &str| p == "bin/game"), None, &mut |_| {}).unwrap();
    assert!(dir.join("s/bin/game").is_file() && !dir.join("s/data/readme.txt").exists());
    assert!(!web.hits().iter().any(|h| h.ends_with("f1.z")), "{:?}", web.hits());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_latest_json_means_no_native_build() {
    let web = TestSite::start();
    let site = site_for(&web, 1);
    assert_eq!(site.fetch_latest(None), Err(FetchError::NoNativeBuild));
}

#[test]
fn a_wrong_signature_or_key_is_refused() {
    let web = TestSite::start();
    web.publish(&BuildSpec::simple(PLATFORM, 3, vec![9; 10], &test_keys::pair(1)));
    let wrong_key = site_for(&web, 2);
    let latest = wrong_key.fetch_latest(None).unwrap();
    assert!(matches!(wrong_key.fetch_manifest(&latest, None), Err(FetchError::Untrusted(_))), "a build signed by another key");
    let no_keys = Site::new(&web.url, "", PLATFORM, vec![]).unwrap();
    assert!(matches!(no_keys.fetch_manifest(&latest, None), Err(FetchError::Untrusted(m)) if m.contains("no signing key")));
    // a manifest changed after signing: the pointer hash catches it first
    let site = site_for(&web, 1);
    let path = format!("native/{PLATFORM}/builds/3/manifest.json");
    let text = String::from_utf8(web.get(&path).unwrap()).unwrap().replace("test build", "evil build");
    web.put(&path, text.into_bytes());
    assert!(matches!(site.fetch_manifest(&latest, None), Err(FetchError::Untrusted(m)) if m.contains("hash")));
}

#[test]
fn a_manifest_for_another_platform_is_refused() {
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple("linux-x86_64", 4, vec![1], &key));
    // the Windows site path serves the Linux manifest
    for name in ["latest.json", "builds/4/manifest.json", "builds/4/manifest.json.sig"] {
        let data = web.get(&format!("native/linux-x86_64/{name}")).unwrap();
        web.put(&format!("native/{PLATFORM}/{name}"), data);
    }
    let site = site_for(&web, 1);
    let latest = site.fetch_latest(None).unwrap();
    assert!(matches!(site.fetch_manifest(&latest, None), Err(FetchError::Untrusted(m)) if m.contains("not windows-x86_64")));
}

#[test]
fn a_damaged_or_cut_off_file_is_refused_and_leaves_nothing() {
    let key = test_keys::pair(1);
    for (name, fault) in [("flipped", Fault::FlipByte), ("cutoff", Fault::TruncateAfter(5)), ("closed", Fault::Close), ("error", Fault::Status(500))] {
        let web = TestSite::start();
        web.publish(&BuildSpec::simple(PLATFORM, 5, vec![7; 50_000], &key));
        let site = site_for(&web, 1);
        let m = manifest_of(&site);
        web.fault(&format!("native/{PLATFORM}/builds/5/f0.z"), fault);
        let dir = temp(&format!("bad-{name}"));
        let result = install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {});
        assert!(result.is_err(), "{name} must fail");
        assert!(!dir.join("s").exists() && !dir.join("t").exists(), "{name}: stage and scratch are removed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn chunked_and_close_delimited_bodies_work() {
    let key = test_keys::pair(1);
    for fault in [Fault::Chunked, Fault::NoLength] {
        let web = TestSite::start();
        web.publish(&BuildSpec::simple(PLATFORM, 6, vec![3; 30_000], &key));
        web.fault(&format!("native/{PLATFORM}/builds/6/f0.z"), fault.clone());
        let site = site_for(&web, 1);
        let m = manifest_of(&site);
        let dir = temp("framing");
        install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {}).unwrap_or_else(|e| panic!("{fault:?}: {e}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_download_longer_than_listed_is_cut_off() {
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple(PLATFORM, 8, vec![5; 20_000], &key));
    let site = site_for(&web, 1);
    let m = manifest_of(&site);
    let mut longer = web.get(&format!("native/{PLATFORM}/builds/8/f0.z")).unwrap();
    longer.extend(std::iter::repeat(0u8).take(5000));
    web.put(&format!("native/{PLATFORM}/builds/8/f0.z"), longer);
    let dir = temp("longer");
    assert!(install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zip_bomb_is_stopped_at_the_listed_size() {
    // A file that unpacks to far more than its manifest size, with a correct compressed hash: refused at the limit.
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple(PLATFORM, 9, vec![1; 100], &key));
    let site = site_for(&web, 1);
    let mut m = manifest_of(&site);
    let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 10_000_000], 9);
    web.put(&format!("native/{PLATFORM}/builds/9/f0.z"), bomb.clone());
    m.files[0].z_size = bomb.len() as u64;
    m.files[0].z_sha256 = crate::verify::sha256_hex(&bomb);
    let dir = temp("bomb");
    let err = install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {}).unwrap_err();
    assert!(matches!(err, FetchError::Untrusted(ref m) if m.contains("cannot unpack")), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stopped_server_reads_as_offline() {
    let mut web = TestSite::start();
    let site = site_for(&web, 1);
    web.stop();
    assert!(matches!(site.fetch_latest(None), Err(FetchError::Offline(_))));
}

#[test]
fn cancelling_stops_the_request() {
    use std::sync::atomic::AtomicBool;
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple(PLATFORM, 10, vec![2; 1000], &key));
    let site = site_for(&web, 1);
    let cancel = AtomicBool::new(true);
    assert_eq!(site.fetch_latest(Some(&cancel)), Err(FetchError::Cancelled));
}

#[test]
fn verify_installed_notices_missing_resized_and_changed_files() {
    let web = TestSite::start();
    let key = test_keys::pair(1);
    web.publish(&BuildSpec::simple(PLATFORM, 11, vec![4; 500], &key));
    let site = site_for(&web, 1);
    let m = manifest_of(&site);
    let dir = temp("verify");
    install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {}).unwrap();
    let game = dir.join("s/bin/game");
    let mut changed = std::fs::read(&game).unwrap();
    changed[0] ^= 1;
    std::fs::write(&game, &changed).unwrap();
    assert!(verify_installed(&dir.join("s"), &m, false).is_ok(), "same size passes the quick check");
    assert!(verify_installed(&dir.join("s"), &m, true).unwrap_err().contains("sha256"));
    std::fs::write(&game, b"short").unwrap();
    assert!(verify_installed(&dir.join("s"), &m, false).unwrap_err().contains("size"));
    std::fs::remove_file(&game).unwrap();
    assert!(verify_installed(&dir.join("s"), &m, false).unwrap_err().contains("missing"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_flat_named_release_downloads_and_verifies() {
    // The same signed tree, re-served under GitHub-release style flat asset names.
    let web = TestSite::start();
    let key = test_keys::pair(1);
    let program = (0..50_000u32).map(|i| (i % 241) as u8).collect::<Vec<u8>>();
    let published = web.publish(&BuildSpec::simple(PLATFORM, 12, program.clone(), &key));
    let flat = TestSite::start();
    for rel in ["latest.json", "builds/12/manifest.json", "builds/12/manifest.json.sig", "builds/12/f0.z", "builds/12/f1.z"] {
        let data = web.get(&format!("native/{PLATFORM}/{rel}")).unwrap();
        flat.put(&format!("native-{PLATFORM}-{}", rel.replace('/', "-")), data);
    }
    let site = Site::new_flat(&flat.url, "", PLATFORM, vec![test_keys::public(&key)]).unwrap();
    let latest = site.fetch_latest(None).unwrap();
    assert_eq!(latest.build, 12);
    let (m, _, _) = site.fetch_manifest(&latest, None).unwrap();
    assert_eq!(m, published);
    let dir = temp("flat");
    install_build(&site, &m, &dir.join("s"), &dir.join("t"), None, None, &mut |_| {}).unwrap();
    assert_eq!(std::fs::read(dir.join("s/bin/game")).unwrap(), program);
    assert!(verify_installed(&dir.join("s"), &m, true).is_ok());
    assert!(flat.hits().iter().any(|h| h.ends_with("native-windows-x86_64-builds-12-f0.z")), "{:?}", flat.hits());
    // the tree-layout site is not what the flat site reads
    let tree = Site::new(&flat.url, "", PLATFORM, vec![test_keys::public(&key)]).unwrap();
    assert_eq!(tree.fetch_latest(None), Err(FetchError::NoNativeBuild));
    let _ = std::fs::remove_dir_all(&dir);
}
