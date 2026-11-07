//! The whole launcher flow against a fake website on 127.0.0.1 and a stub game (src/bin/lr_stub_game.rs).
//! Each test has its own site, its own install folder and its own fake browser.

use lr_launcher::{run, Config, Kind, Mode};
use lr_update::manifest::this_platform;
use lr_update::state;
use lr_update::testsite::{BuildSpec, Fault, TestSite};
use lr_update::verify::test_keys;
use ring::signature::Ed25519KeyPair;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WEB: &str = "https://web.example.test/";

struct Env {
    web: TestSite,
    dir: PathBuf,
    key: Ed25519KeyPair,
    opened: Arc<Mutex<Vec<String>>>,
}

fn run_name() -> &'static str {
    if cfg!(windows) {
        "bin/game.exe"
    } else {
        "bin/game"
    }
}

fn stub() -> Vec<u8> {
    std::fs::read(env!("CARGO_BIN_EXE_lr_stub_game")).expect("the stub game is built")
}

impl Env {
    fn new(name: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("lr-launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Env { web: TestSite::start(), dir, key: test_keys::pair(1), opened: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Publishes build `n` whose stub game does what `behavior` says (see lr_stub_game.rs).
    fn publish(&self, n: u64, behavior: &str) {
        self.publish_with(n, behavior, |_| {});
    }

    fn publish_with(&self, n: u64, behavior: &str, tweak: impl FnOnce(&mut BuildSpec)) {
        self.publish_signed(n, behavior, &self.key, tweak);
    }

    fn publish_signed(&self, n: u64, behavior: &str, key: &Ed25519KeyPair, tweak: impl FnOnce(&mut BuildSpec)) {
        let mut spec = BuildSpec {
            platform: this_platform(),
            channel: "",
            build: n,
            files: vec![(run_name(), stub(), 0o755), ("data/behavior.txt", behavior.as_bytes().to_vec(), 0o644)],
            run: run_name(),
            min_os: None,
            launcher_min: 1,
            key,
        };
        tweak(&mut spec);
        self.web.publish(&spec);
    }

    fn cfg(&self) -> Config {
        let mut c = Config::new(self.dir.clone());
        c.base_url = self.web.url.clone();
        c.web_url = WEB.to_string();
        c.platform = this_platform().to_string();
        c.trusted = vec![test_keys::public(&self.key)];
        c.os_version = Some("12.7.6".to_string());
        c.echo = false;
        // Long enough that a slow first start of a new exe (virus scanners) is still inside the window.
        c.startup_window = Duration::from_secs(8);
        c.background_wait = Duration::from_secs(20);
        c.quick_seconds = 3;
        let opened = self.opened.clone();
        c.opener = Arc::new(move |url| {
            opened.lock().unwrap().push(url.to_string());
            Ok(())
        });
        c
    }

    fn state(&self) -> state::State {
        state::load(&self.dir).0
    }

    fn build_dir(&self, n: u64) -> PathBuf {
        state::build_dir(&self.dir, n)
    }

    fn ran(&self, n: u64) -> String {
        std::fs::read_to_string(self.build_dir(n).join("ran.txt")).unwrap_or_default()
    }

    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }

    fn downloads(&self) -> Vec<String> {
        self.web.hits().into_iter().filter(|h| h.ends_with(".z")).collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(state::tmp_dir(dir)).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()
}

#[test]
fn a_first_start_downloads_verifies_installs_and_runs_the_game() {
    let env = Env::new("first");
    env.publish(1, "");
    let mut cfg = env.cfg();
    cfg.game_args = vec!["--hello".into(), "world".into()];
    let out = run(&cfg);
    assert_eq!((out.kind, out.exit_code, out.build), (Kind::Ran, 0, Some(1)), "{out:?}");
    let ran = env.ran(1);
    assert!(ran.contains("--hello") && ran.contains("launcher=1") && ran.contains("build=1") && ran.contains("cwd_ok=true"), "{ran}");
    let s = env.state();
    assert_eq!((s.current, s.previous, s.failures), (Some(1), None, 0));
    assert!(env.dir.join("launcher.log").is_file());
    assert!(leftovers(&env.dir).is_empty(), "tmp is clean: {:?}", leftovers(&env.dir));
    assert!(env.opened().is_empty(), "no browser");
}

#[test]
fn an_up_to_date_start_downloads_no_files() {
    let env = Env::new("uptodate");
    env.publish(1, "");
    assert_eq!(run(&env.cfg()).exit_code, 0);
    env.web.clear_hits();
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.build), (Kind::Ran, Some(1)));
    assert!(env.downloads().is_empty(), "{:?}", env.web.hits());
    assert!(env.web.hits().iter().any(|h| h.ends_with("latest.json")), "it still looked at the pointer");
}

#[test]
fn a_new_build_is_installed_in_the_background_and_runs_next_start() {
    let env = Env::new("background");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "");
    let out = run(&env.cfg());
    assert_eq!(out.build, Some(1), "this start still runs the installed build");
    let s = env.state();
    assert_eq!((s.current, s.previous), (Some(2), Some(1)), "build 2 is ready, build 1 is kept for a rollback");
    assert!(env.build_dir(1).is_dir() && env.build_dir(2).is_dir());
    let out = run(&env.cfg());
    assert_eq!(out.build, Some(2));
    assert!(env.ran(2).contains("build=2"));
}

#[test]
fn update_first_runs_the_new_build_at_once() {
    let env = Env::new("updatefirst");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "");
    let mut cfg = env.cfg();
    cfg.update_first = true;
    let out = run(&cfg);
    assert_eq!(out.build, Some(2));
    assert!(env.ran(2).contains("build=2") && env.ran(1).lines().count() == 1);
}

#[test]
fn a_bad_download_is_refused_and_the_old_build_stays() {
    let env = Env::new("badsha");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "");
    env.web.fault(&format!("native/{}/builds/2/f0.z", this_platform()), Fault::FlipByte);
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.build), (Kind::Ran, Some(1)));
    let s = env.state();
    assert_eq!((s.current, s.previous), (Some(1), None));
    assert!(!env.build_dir(2).exists());
    assert!(leftovers(&env.dir).is_empty(), "{:?}", leftovers(&env.dir));
    // the next start works as before, and a fixed site then installs
    env.web.clear_faults();
    run(&env.cfg());
    assert_eq!(env.state().current, Some(2));
}

#[test]
fn a_build_signed_with_another_key_is_refused() {
    let env = Env::new("badsig");
    env.publish(1, "");
    run(&env.cfg());
    let evil = test_keys::pair(9);
    env.publish_signed(2, "", &evil, |_| {});
    let out = run(&env.cfg());
    assert_eq!(out.build, Some(1));
    assert_eq!(env.state().current, Some(1));
    assert!(!env.build_dir(2).exists());
}

#[test]
fn an_interrupted_download_leaves_the_old_build_runnable_and_partials_are_cleaned() {
    let env = Env::new("interrupted");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "");
    env.web.fault(&format!("native/{}/builds/2/f0.z", this_platform()), Fault::TruncateAfter(100));
    assert_eq!(run(&env.cfg()).build, Some(1));
    assert_eq!(env.state().current, Some(1));
    assert!(!env.build_dir(2).exists() && leftovers(&env.dir).is_empty());
    // a download that was killed half way (the launcher itself gone) left files behind: the next start removes them
    let partial = state::tmp_dir(&env.dir).join("2.partial");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("half"), b"x").unwrap();
    std::fs::create_dir_all(state::tmp_dir(&env.dir).join("2.dl")).unwrap();
    env.web.clear_faults();
    run(&env.cfg());
    assert!(!partial.exists());
    assert_eq!(env.state().current, Some(2));
}

#[test]
fn an_offline_start_runs_the_last_good_build() {
    let mut env = Env::new("offline");
    env.publish(1, "");
    run(&env.cfg());
    env.web.stop();
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.exit_code, out.build), (Kind::Ran, 0, Some(1)));
    assert!(env.opened().is_empty());
    assert_eq!(env.state().current, Some(1));
}

#[test]
fn offline_with_nothing_installed_stops_with_a_message_and_no_browser() {
    let mut env = Env::new("offline-empty");
    env.web.stop();
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.exit_code), (Kind::Offline, 2));
    assert!(out.message.contains("Connect to the internet"), "{}", out.message);
    assert!(env.opened().is_empty());
}

#[test]
fn no_native_build_for_this_system_opens_the_web_version() {
    let env = Env::new("nonative");
    env.publish_with(1, "", |s| s.platform = "linux-aarch64-other");
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.exit_code), (Kind::Web, 0), "{out:?}");
    assert_eq!(env.opened(), vec![WEB.to_string()]);
    assert_eq!(out.opened.as_deref(), Some(WEB));
}

#[test]
fn a_system_that_is_too_old_gets_the_web_version_but_keeps_its_installed_build() {
    let env = Env::new("oldos");
    env.publish_with(1, "", |s| s.min_os = Some("13.0"));
    let out = run(&env.cfg()); // os_version is 12.7.6
    assert_eq!(out.kind, Kind::Web, "{out:?}");
    assert!(out.message.contains("needs system version 13.0"), "{}", out.message);
    // with a newer system the same build installs; then a later build that needs more than this system is skipped
    let mut cfg = env.cfg();
    cfg.os_version = Some("14.1".into());
    assert_eq!(run(&cfg).build, Some(1));
    env.publish_with(2, "", |s| s.min_os = Some("15.0"));
    assert_eq!(run(&cfg).build, Some(1));
    assert_eq!(env.state().current, Some(1), "build 2 needs system 15.0");
    assert!(!env.build_dir(2).exists());
}

#[test]
fn a_launcher_that_is_too_old_does_not_install_the_build() {
    let env = Env::new("oldlauncher");
    env.publish_with(1, "", |s| s.launcher_min = 99);
    let out = run(&env.cfg());
    assert_eq!(out.kind, Kind::Web);
    assert!(env.state().current.is_none());
}

#[test]
fn a_build_that_fails_to_start_twice_is_rolled_back() {
    let env = Env::new("crashloop");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "exit 3");
    let mut cfg = env.cfg();
    cfg.update_first = true;
    let first = run(&cfg);
    assert_eq!((first.kind, first.exit_code, first.build), (Kind::Ran, 3, Some(2)));
    assert_eq!((env.state().current, env.state().failures), (Some(2), 1), "one failure is only counted");
    // the second failure marks build 2 bad and the same start goes on with build 1
    cfg.update_first = false;
    let second = run(&cfg);
    assert_eq!((second.kind, second.exit_code, second.build), (Kind::Ran, 0, Some(1)), "{second:?}");
    let s = env.state();
    assert_eq!((s.current, s.previous, s.bad.clone(), s.failures), (Some(1), None, vec![2], 0));
    assert!(!env.build_dir(2).exists(), "the bad build is collected");
    // the site still offers build 2: it is not downloaded again
    env.web.clear_hits();
    run(&cfg);
    assert!(env.downloads().is_empty(), "{:?}", env.web.hits());
    // a higher build is installed
    env.publish(3, "");
    run(&cfg);
    assert_eq!(env.state().current, Some(3));
}

#[test]
fn with_no_earlier_build_a_crash_loop_ends_in_the_web_version() {
    let env = Env::new("crashloop-alone");
    env.publish(1, "exit 3");
    let cfg = env.cfg();
    assert_eq!(run(&cfg).exit_code, 3);
    let second = run(&cfg);
    assert_eq!(second.kind, Kind::Web, "{second:?}");
    assert_eq!(env.opened(), vec![WEB.to_string()]);
    let s = env.state();
    assert_eq!((s.current, s.bad.clone()), (None, vec![1]));
}

#[test]
fn a_game_that_survives_the_startup_window_counts_as_a_good_start_even_if_it_crashes_later() {
    let env = Env::new("survivor");
    env.publish(1, "sleep 3500\nexit 5");
    let mut cfg = env.cfg();
    cfg.startup_window = Duration::from_millis(1000);
    let out = run(&cfg);
    assert_eq!((out.kind, out.exit_code), (Kind::Ran, 5));
    let s = env.state();
    assert_eq!((s.current, s.failures, s.bad.len()), (Some(1), 0, 0));
}

#[test]
fn a_game_that_quits_normally_at_once_is_not_a_failure() {
    let env = Env::new("quick-quit");
    env.publish(1, "exit 0");
    for _ in 0..3 {
        assert_eq!(run(&env.cfg()).exit_code, 0);
    }
    assert_eq!((env.state().current, env.state().failures), (Some(1), 0));
}

#[test]
fn a_game_that_cannot_be_started_counts_as_a_failure() {
    let env = Env::new("nostart");
    // the "program" is not an executable
    env.publish_with(1, "", |s| s.files[0].1 = b"not a program".to_vec());
    let cfg = env.cfg();
    let out = run(&cfg);
    assert_eq!(out.kind, Kind::Ran);
    assert_ne!(out.exit_code, 0);
    assert_eq!(env.state().failures, 1);
}

#[test]
fn rollback_by_hand_goes_back_and_the_bad_build_waits_for_a_higher_one() {
    let env = Env::new("rollback");
    env.publish(1, "");
    run(&env.cfg());
    env.publish(2, "");
    run(&env.cfg()); // installs 2 in the background
    assert_eq!(env.state().current, Some(2));
    let mut cfg = env.cfg();
    cfg.mode = Mode::Rollback;
    let out = run(&cfg);
    assert_eq!(out.exit_code, 0, "{}", out.message);
    let s = env.state();
    assert_eq!((s.current, s.bad.clone()), (Some(1), vec![2]));
    assert!(!env.build_dir(2).exists());
    cfg.mode = Mode::Run;
    assert_eq!(run(&cfg).build, Some(1));
    assert_eq!(env.state().current, Some(1), "build 2 stays bad");
    env.publish(3, "");
    run(&cfg);
    assert_eq!(env.state().current, Some(3));
    // nothing earlier than 3 and 1... rolling back twice reaches the end
    let mut cfg = env.cfg();
    cfg.mode = Mode::Rollback;
    assert_eq!(run(&cfg).exit_code, 0);
    assert_eq!(run(&cfg).exit_code, 1, "no earlier build");
}

#[test]
fn a_lower_build_on_the_site_is_never_installed_not_even_with_force() {
    let env = Env::new("downgrade");
    env.publish(5, "");
    run(&env.cfg());
    env.publish(3, ""); // a replayed or stale pointer
    let mut cfg = env.cfg();
    cfg.force = true;
    let out = run(&cfg);
    assert_eq!(out.build, Some(5));
    assert_eq!(env.state().current, Some(5));
    assert!(!env.build_dir(3).exists());
    assert!(!env.downloads().iter().any(|d| d.contains("/builds/3/")), "{:?}", env.downloads());
}

#[test]
fn force_reinstalls_the_current_build() {
    let env = Env::new("force");
    env.publish(1, "exit 0");
    run(&env.cfg());
    // a changed file of the same size passes the quick check, so only --force (or --verify) puts the real one back
    std::fs::write(env.build_dir(1).join("data").join("behavior.txt"), "exit 9").unwrap();
    env.web.clear_hits();
    let mut cfg = env.cfg();
    cfg.force = true;
    let out = run(&cfg);
    assert_eq!((out.kind, out.build, out.exit_code), (Kind::Ran, Some(1), 0));
    assert!(!env.downloads().is_empty(), "force downloads again");
    assert_eq!(std::fs::read_to_string(env.build_dir(1).join("data").join("behavior.txt")).unwrap(), "exit 0");
}

#[test]
fn hostile_manifests_signed_by_the_right_key_are_still_refused() {
    let env = Env::new("hostile");
    let platform = this_platform();
    let good = |path: &str, size: u64, plat: &str| {
        format!(
            r#"{{"schema":"lr.native/1","platform":"{plat}","build":1,"version":"0.1.0","revision":"abc","built":"now","run":"{path}",
            "files":[{{"path":"{path}","size":{size},"sha256":"{h}","z":"f0.z","z_size":10,"z_sha256":"{h}"}}]}}"#,
            h = "a".repeat(64)
        )
    };
    let cases = [
        ("path traversal", good("../evil.exe", 10, platform)),
        ("absolute path", good("/etc/evil", 10, platform)),
        ("drive letter", good("C:/evil.exe", 10, platform)),
        ("huge size", good("bin/game", 900_000_000_000, platform)),
        ("wrong platform", good("bin/game", 10, "freebsd-riscv")),
    ];
    for (name, text) in cases {
        env.web.publish_manifest_text(platform, "", 1, &text, &env.key);
        let out = run(&env.cfg());
        assert_eq!(out.kind, Kind::Web, "{name}: {out:?}");
        assert!(env.state().current.is_none() && !env.build_dir(1).exists(), "{name}");
        assert!(!env.dir.join("evil.exe").exists() && !env.dir.parent().unwrap().join("evil.exe").exists(), "{name}");
        assert!(leftovers(&env.dir).is_empty(), "{name}: {:?}", leftovers(&env.dir));
    }
}

#[test]
fn with_no_signing_key_built_in_nothing_installs_and_the_web_opens() {
    let env = Env::new("nokey");
    env.publish(1, "");
    let mut cfg = env.cfg();
    cfg.trusted = vec![];
    let out = run(&cfg);
    assert_eq!(out.kind, Kind::Web);
    assert!(out.message.contains("no signing key"), "{}", out.message);
    assert!(!env.build_dir(1).exists());
}

#[test]
fn a_damaged_state_file_is_rebuilt_and_the_game_still_starts() {
    let env = Env::new("badstate");
    env.publish(1, "");
    run(&env.cfg());
    std::fs::write(env.dir.join("state.json"), "{ this is not json").unwrap();
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.build), (Kind::Ran, Some(1)));
    assert_eq!(env.state().current, Some(1));
}

#[test]
fn an_installed_build_with_a_changed_file_is_dropped_and_installed_again() {
    let env = Env::new("tampered");
    env.publish(1, "");
    run(&env.cfg());
    std::fs::write(env.build_dir(1).join("data").join("behavior.txt"), "a different size").unwrap();
    let out = run(&env.cfg());
    assert_eq!((out.kind, out.build), (Kind::Ran, Some(1)));
    assert_eq!(std::fs::read_to_string(env.build_dir(1).join("data").join("behavior.txt")).unwrap(), "", "restored from the site");
}

#[test]
fn a_deep_verify_notices_a_changed_file_of_the_same_size() {
    let env = Env::new("deep");
    env.publish(1, "exit 0");
    run(&env.cfg());
    std::fs::write(env.build_dir(1).join("data").join("behavior.txt"), "exit 9").unwrap(); // same length
    let mut cfg = env.cfg();
    cfg.verify_deep = true;
    let out = run(&cfg);
    assert_eq!(out.exit_code, 0, "restored from the site and run");
    assert_eq!(std::fs::read_to_string(env.build_dir(1).join("data").join("behavior.txt")).unwrap(), "exit 0");
}

#[test]
fn check_reports_without_changing_anything() {
    let env = Env::new("check");
    env.publish(1, "");
    let mut cfg = env.cfg();
    cfg.mode = Mode::Check;
    let out = run(&cfg);
    assert_eq!((out.kind, out.exit_code), (Kind::Report, 10), "{}", out.message);
    assert!(out.message.contains("a newer build is available") && out.message.contains("build 1"), "{}", out.message);
    assert!(!env.dir.exists(), "check creates nothing");
    assert!(env.downloads().is_empty());
    // install it, then check again
    cfg.mode = Mode::Run;
    run(&cfg);
    cfg.mode = Mode::Check;
    let out = run(&cfg);
    assert_eq!(out.exit_code, 0, "{}", out.message);
    assert!(out.message.contains("up to date"), "{}", out.message);
}

#[test]
fn dry_run_says_what_it_would_do_and_does_nothing() {
    let env = Env::new("dryrun");
    env.publish(1, "");
    let mut cfg = env.cfg();
    cfg.mode = Mode::DryRun;
    let out = run(&cfg);
    assert!(out.message.contains("would: download") && out.message.contains("dry run: nothing was changed"), "{}", out.message);
    assert!(!env.dir.exists() && env.downloads().is_empty() && env.opened().is_empty());
    // no build for this platform: it says the browser would open, and does not
    let mut cfg2 = env.cfg();
    cfg2.mode = Mode::DryRun;
    cfg2.platform = "macos-universal".into();
    if this_platform() == "macos-universal" {
        cfg2.platform = "windows-x86_64".into();
    }
    let out = run(&cfg2);
    assert_eq!(out.exit_code, 3, "{}", out.message);
    assert!(out.message.contains("would: open"), "{}", out.message);
    assert!(env.opened().is_empty());
}

#[test]
fn no_launch_installs_and_stops() {
    let env = Env::new("nolaunch");
    env.publish(1, "");
    let mut cfg = env.cfg();
    cfg.no_launch = true;
    let out = run(&cfg);
    assert_eq!((out.kind, out.build), (Kind::Installed, Some(1)));
    assert!(env.ran(1).is_empty(), "the game did not run");
    assert_eq!(env.state().current, Some(1));
}

#[test]
fn another_launcher_installing_means_this_one_waits_for_the_next_start() {
    let env = Env::new("lock");
    env.publish(1, "");
    std::fs::create_dir_all(&env.dir).unwrap();
    let _held = state::InstallLock::acquire(&env.dir, 600).unwrap();
    let out = run(&env.cfg());
    assert_eq!(out.kind, Kind::Web, "{out:?}");
    assert!(env.state().current.is_none());
}

#[test]
fn channels_use_their_own_site_folder_and_install_folder() {
    let env = Env::new("channel");
    env.publish_with(1, "", |s| s.channel = "beta");
    let out = run(&env.cfg());
    assert_eq!(out.kind, Kind::Web, "the default channel has no build");
    let mut cfg = env.cfg();
    cfg.channel = "beta".into();
    let out = run(&cfg);
    assert_eq!((out.kind, out.build), (Kind::Ran, Some(1)), "{out:?}");
}
