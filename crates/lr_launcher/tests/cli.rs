//! The real `lr_launcher` program, started as a process: arguments, exit codes, the headless test mode.

use lr_update::manifest::this_platform;
use lr_update::testsite::{BuildSpec, TestSite};
use lr_update::verify::{test_keys, to_hex};
use std::path::PathBuf;
use std::process::{Command, Output};

fn run_name() -> &'static str {
    if cfg!(windows) {
        "bin/game.exe"
    } else {
        "bin/game"
    }
}

struct Cli {
    web: TestSite,
    dir: PathBuf,
    pubkey: String,
}

impl Cli {
    fn new(name: &str, behavior: &str) -> Cli {
        let dir = std::env::temp_dir().join(format!("lr-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = test_keys::pair(4);
        let web = TestSite::start();
        let stub = std::fs::read(env!("CARGO_BIN_EXE_lr_stub_game")).unwrap();
        let mut spec = BuildSpec::simple(this_platform(), 1, stub, &key);
        spec.files = vec![(run_name(), std::fs::read(env!("CARGO_BIN_EXE_lr_stub_game")).unwrap(), 0o755), ("data/behavior.txt", behavior.as_bytes().to_vec(), 0o644)];
        spec.run = run_name();
        web.publish(&spec);
        Cli { web, dir, pubkey: to_hex(&test_keys::public(&key)) }
    }

    fn run(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_lr_launcher"))
            .args(["--test-mode", "--base-url", &self.web.url, "--dir"])
            .arg(&self.dir)
            .args(extra)
            .env("LR_NATIVE_PUBKEY", &self.pubkey)
            .env_remove("LR_NATIVE_DIR")
            .output()
            .expect("the launcher starts")
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn result_line(out: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let line = text.lines().find_map(|l| l.strip_prefix("LAUNCHER_RESULT ")).unwrap_or_else(|| panic!("no result line in {text:?}"));
    serde_json::from_str(line).unwrap()
}

#[test]
fn the_launcher_installs_runs_the_game_with_its_arguments_and_passes_its_exit_code_on() {
    let cli = Cli::new("run", "exit 0");
    let out = cli.run(&["--headless", "--test-suite", "x.json"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let r = result_line(&out);
    assert_eq!((r["kind"].as_str(), r["build"].as_u64()), (Some("Ran"), Some(1)));
    let ran = std::fs::read_to_string(lr_update::state::build_dir(&cli.dir, 1).join("ran.txt")).unwrap();
    assert!(ran.contains("--headless") && ran.contains("--test-suite") && ran.contains("x.json") && ran.contains("launcher=1"), "{ran}");
}

#[test]
fn a_games_failing_exit_code_is_the_launchers_exit_code() {
    let cli = Cli::new("code", "exit 7");
    let out = cli.run(&[]);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(result_line(&out)["exit"], 7);
}

#[test]
fn check_exits_10_when_an_update_is_available_and_0_when_current() {
    let cli = Cli::new("check", "exit 0");
    let out = cli.run(&["--check"]);
    assert_eq!(out.status.code(), Some(10));
    assert!(String::from_utf8_lossy(&out.stdout).contains("a newer build is available"));
    assert!(!cli.dir.exists(), "--check changes nothing");
    assert_eq!(cli.run(&["--no-launch"]).status.code(), Some(0));
    assert_eq!(cli.run(&["--check"]).status.code(), Some(0));
    let dry = cli.run(&["--dry-run"]);
    assert!(String::from_utf8_lossy(&dry.stdout).contains("would: start build 1"), "{}", String::from_utf8_lossy(&dry.stdout));
}

#[test]
fn bad_arguments_exit_64_and_help_exits_0() {
    let cli = Cli::new("args", "");
    let out = Command::new(env!("CARGO_BIN_EXE_lr_launcher")).args(["--base-url", "http://example.org"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("https"));
    let help = Command::new(env!("CARGO_BIN_EXE_lr_launcher")).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--rollback"));
    drop(cli);
}

#[test]
fn offline_with_nothing_installed_exits_2_and_never_opens_a_browser() {
    let mut cli = Cli::new("offline", "");
    cli.web.stop();
    let out = cli.run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(result_line(&out)["kind"], "Offline");
}

#[test]
fn no_native_build_prints_the_web_address_it_would_open() {
    let cli = Cli::new("web", "");
    let out = cli.run(&["--platform", "macos-universal", "--web-url", "https://web.example.test/"]);
    let r = if this_platform() == "macos-universal" { None } else { Some(result_line(&out)) };
    if let Some(r) = r {
        assert_eq!(r["kind"], "Web");
        assert_eq!(r["opened"], "https://web.example.test/");
        assert_eq!(out.status.code(), Some(0));
    }
}
