//! The command line. The launcher takes its own flags by exact name and passes everything else to the game, so
//! `lr_launcher --headless --test-suite x.json` still reaches the game (`--headless` is the game's flag, so the
//! launcher's own quiet test mode is `--test-mode`). `--` ends the launcher's flags.

use crate::config::{Config, Mode};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

pub const HELP: &str = "\
lr_launcher: installs and starts the newest native build of The Last Route from the website.

  lr_launcher [flags] [game arguments]

Flags (everything else goes to the game; \"--\" ends the flags):
  --check            report what is installed and what the site offers; change nothing
                     (exit 10: update available, 2: offline, 3: no native build, 4: refused)
  --dry-run          like a normal start, but only say what would happen
  --rollback         mark the current build bad and go back to the previous one (a higher build on the site is installed again)
  --force            install the site's build even when it is the current one or was marked bad (never a lower build)
  --update-first     check and install before starting (the default is to start at once and update for the next start)
  --no-launch        install, then stop without starting the game
  --verify           check every installed file's sha256 before starting
  --channel NAME     use the builds under native-NAME (the default channel is native)
  --dir PATH         the install folder (default: the per-user folder; LR_NATIVE_DIR also sets it)
  --base-url URL     the site (https, or http on localhost for tests)
  --web-url URL      the web version opened as the fallback
  --platform ID      pretend to be another platform (windows-x86_64, macos-universal, linux-x86_64)
  --os-version V     pretend the system version is V (checked against the manifest's min_os)
  --test-mode        quiet and machine-readable: never opens a browser, prints a LAUNCHER_RESULT line
  --version, --help

Environment: LR_NATIVE_DIR (install folder). With a localhost --base-url, LR_NATIVE_PUBKEY (64 hex digits) adds a
trusted public key for tests.
";

#[derive(Debug)]
pub enum Parsed {
    Run(Box<Config>),
    /// Print this text and exit 0.
    Info(String),
    Error(String),
}

fn take_value(it: &mut std::vec::IntoIter<OsString>, flag: &str) -> Result<String, String> {
    it.next().and_then(|v| v.into_string().ok()).ok_or_else(|| format!("{flag} needs a value"))
}

const VALUE_FLAGS: [&str; 6] = ["--channel", "--dir", "--base-url", "--web-url", "--platform", "--os-version"];

/// Parses `args` (without the program name). `env_pubkey` is `LR_NATIVE_PUBKEY`; `default_dir` gives the per-user folder
/// for a channel.
pub fn parse(args: Vec<OsString>, env_pubkey: Option<String>, default_dir: impl Fn(&str) -> Option<PathBuf>) -> Parsed {
    let mut it = args.into_iter();
    let mut game: Vec<OsString> = Vec::new();
    let mut channel = String::new();
    let (mut dir, mut base, mut web, mut platform, mut os): (Option<PathBuf>, Option<String>, Option<String>, Option<String>, Option<String>) =
        (None, None, None, None, None);
    let (mut mode, mut force, mut update_first, mut no_launch, mut test_mode, mut deep) = (Mode::Run, false, false, false, false, false);
    while let Some(arg) = it.next() {
        let Some(text) = arg.to_str().map(str::to_string) else {
            game.push(arg);
            continue;
        };
        // --flag=value
        let (flag, inline) = match text.split_once('=') {
            Some((f, v)) if VALUE_FLAGS.contains(&f) => (f.to_string(), Some(v.to_string())),
            _ => (text.clone(), None),
        };
        let mut value = || -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => take_value(&mut it, &flag),
            }
        };
        match flag.as_str() {
            "--" => {
                game.extend(it.by_ref());
            }
            "--help" | "-h" => return Parsed::Info(HELP.to_string()),
            "--version" => return Parsed::Info(format!("lr_launcher {}", crate::flow::LAUNCHER_VERSION)),
            "--check" => mode = Mode::Check,
            "--dry-run" => mode = Mode::DryRun,
            "--rollback" => mode = Mode::Rollback,
            "--force" => force = true,
            "--update-first" => update_first = true,
            "--no-launch" => no_launch = true,
            "--verify" => deep = true,
            "--test-mode" => test_mode = true,
            "--channel" | "--dir" | "--base-url" | "--web-url" | "--platform" | "--os-version" => {
                let v = match value() {
                    Ok(v) => v,
                    Err(e) => return Parsed::Error(e),
                };
                match flag.as_str() {
                    "--channel" => channel = v,
                    "--dir" => dir = Some(PathBuf::from(v)),
                    "--base-url" => base = Some(v),
                    "--web-url" => web = Some(v),
                    "--platform" => platform = Some(v),
                    _ => os = Some(v),
                }
            }
            _ => game.push(arg),
        }
    }
    if !lr_update::manifest::valid_channel(&channel) {
        return Parsed::Error("the channel must be lower case letters, digits and dashes".into());
    }
    let Some(dir) = dir.or_else(|| default_dir(&channel)) else {
        return Parsed::Error("cannot find a per-user folder; pass --dir".into());
    };
    let mut cfg = Config::new(dir);
    cfg.channel = channel;
    cfg.mode = mode;
    cfg.force = force;
    cfg.update_first = update_first;
    cfg.no_launch = no_launch;
    cfg.test_mode = test_mode;
    cfg.verify_deep = deep;
    cfg.game_args = game;
    if let Some(b) = base {
        if let Err(e) = lr_update::fetch::base_url_allowed(&b) {
            return Parsed::Error(e);
        }
        // A localhost site is a test site: a test key may be added for it (never for a real site).
        let local = lr_update::http::host_of(&b).is_some_and(|h| lr_update::http::is_loopback_host(&h));
        if let (Some(hex), true) = (env_pubkey.as_deref(), local) {
            match lr_update::verify::from_hex(hex).and_then(|k| <[u8; 32]>::try_from(k).ok()) {
                Some(key) => cfg.trusted.push(key),
                None => return Parsed::Error("LR_NATIVE_PUBKEY must be 64 hex digits".into()),
            }
        }
        if web.is_none() {
            cfg.web_url = b.clone();
        }
        cfg.base_url = b;
    }
    if let Some(w) = web {
        if let Err(e) = lr_update::fetch::base_url_allowed(&w) {
            return Parsed::Error(e);
        }
        cfg.web_url = w;
    }
    if let Some(p) = platform {
        cfg.platform = p;
    }
    if let Some(o) = os {
        cfg.os_version = Some(o);
    }
    if cfg.test_mode {
        cfg.echo = false;
        // Never open a real browser in test mode: the address is printed in the result line instead.
        cfg.opener = Arc::new(|_| Ok(()));
    }
    Parsed::Run(Box::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Parsed {
        parse(args.iter().map(OsString::from).collect(), None, |_| Some(PathBuf::from("/tmp/lr")))
    }

    fn cfg(args: &[&str]) -> Config {
        match p(args) {
            Parsed::Run(c) => *c,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn launcher_flags_are_taken_and_the_rest_goes_to_the_game() {
        let c = cfg(&["--force", "--channel", "beta", "--headless", "--test-suite", "a.json", "--check"]);
        assert!(c.force && c.channel == "beta" && c.mode == Mode::Check);
        assert_eq!(c.game_args, vec![OsString::from("--headless"), "--test-suite".into(), "a.json".into()]);
    }

    #[test]
    fn double_dash_ends_the_launcher_flags() {
        let c = cfg(&["--no-launch", "--", "--force", "--check"]);
        assert!(c.no_launch && !c.force && c.mode == Mode::Run);
        assert_eq!(c.game_args, vec![OsString::from("--force"), "--check".into()]);
    }

    #[test]
    fn equals_forms_work() {
        let c = cfg(&["--channel=kt-1", "--base-url=http://127.0.0.1:81", "--platform=linux-x86_64"]);
        assert_eq!((c.channel.as_str(), c.base_url.as_str(), c.platform.as_str()), ("kt-1", "http://127.0.0.1:81", "linux-x86_64"));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(matches!(p(&["--channel"]), Parsed::Error(_)));
        assert!(matches!(p(&["--channel", "../x"]), Parsed::Error(_)));
        assert!(matches!(p(&["--base-url", "http://evil.example"]), Parsed::Error(_)));
        assert!(matches!(p(&["--base-url", "ftp://x"]), Parsed::Error(_)));
        assert!(matches!(p(&["--help"]), Parsed::Info(_)));
    }

    #[test]
    fn a_test_key_is_accepted_only_for_a_localhost_site() {
        let key = lr_update::verify::to_hex(&[5u8; 32]);
        let run = |base: &str| parse(vec!["--base-url".into(), base.into()], Some(key.clone()), |_| Some(PathBuf::from("/x")));
        let Parsed::Run(local) = run("http://127.0.0.1:8000") else { panic!() };
        assert!(local.trusted.contains(&[5u8; 32]));
        let Parsed::Run(real) = run("https://lastrouterust.games.kyttuslab.org") else { panic!() };
        assert!(!real.trusted.contains(&[5u8; 32]), "the env key must not widen trust for the real site");
    }

    #[test]
    fn test_mode_never_opens_a_browser() {
        let c = cfg(&["--test-mode"]);
        assert!(c.test_mode && !c.echo);
        assert!((c.opener)("https://example.org").is_ok());
    }
}
