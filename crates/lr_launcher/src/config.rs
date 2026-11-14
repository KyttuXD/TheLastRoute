//! Everything the launcher flow needs to know, in one value, so tests can run the whole flow in-process with a fake
//! stand-in for the GitHub nightly release, a temp folder, their own keys and a fake browser.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The live website: its root page is the web build (opened when no native build can be used).
pub const DEFAULT_SITE: &str = "https://lastrouterust.games.kyttuslab.org";

/// Where the signed native builds come from by default: the GitHub release tagged "nightly".
pub const DEFAULT_BASE: &str = lr_update::fetch::NIGHTLY_URL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Install if needed, start the game.
    Run,
    /// Report what is installed and what the site offers; change nothing.
    Check,
    /// Like Run, but only say what would happen; change nothing and start nothing.
    DryRun,
    /// Mark the current build bad and go back to the previous one (by hand).
    Rollback,
}

/// Opens an address in the default browser (replaced by a recorder in tests and `--test-mode`).
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct Config {
    /// The site that holds `native/<platform>/...` (https, or http on loopback for tests).
    pub base_url: String,
    /// The web version, opened when no native build can run.
    pub web_url: String,
    /// "" is the default channel; others live under `native-<channel>` on the site and in the user folder.
    pub channel: String,
    pub platform: String,
    /// The per-user install folder (builds/, tmp/, state.json, launcher.log).
    pub dir: PathBuf,
    /// ed25519 public keys that may sign builds.
    pub trusted: Vec<[u8; 32]>,
    /// The running OS version, for `min_os`; None when unknown (never blocks).
    pub os_version: Option<String>,
    pub mode: Mode,
    /// Install even when up to date or marked bad (never a lower build than one installed before).
    pub force: bool,
    /// Check and install before starting instead of in the background (so the first start after a publish is the new build).
    pub update_first: bool,
    /// Never look for an update while a build is installed (`--no-update`): the installed build starts as it is. A first
    /// start with nothing installed still installs one.
    pub no_update: bool,
    /// Install, then stop without starting the game.
    pub no_launch: bool,
    /// Machine-readable and quiet: no browser, no console tricks, and a final `LAUNCHER_RESULT {json}` line.
    pub test_mode: bool,
    /// Check every file's sha256 of the installed build before starting it (the normal check is sizes only).
    pub verify_deep: bool,
    /// Arguments passed through to the game.
    pub game_args: Vec<OsString>,
    /// A launch that fails inside this window counts as a failed start.
    pub startup_window: Duration,
    /// Failed starts in a row before the build is marked bad and the launcher rolls back.
    pub fail_limit: u32,
    /// Timeout for latest.json, the manifest and the signature.
    pub quick_seconds: u64,
    /// How long to keep waiting for a background download after the game has ended.
    pub background_wait: Duration,
    pub opener: Opener,
    /// Print progress and messages to the console.
    pub echo: bool,
    /// Called once the game process is running (Windows: lets go of the console window).
    pub on_game_started: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Config {{ site: {}, platform: {}, channel: {:?}, dir: {} }}", self.base_url, self.platform, self.channel, self.dir.display())
    }
}

impl Config {
    /// A configuration with the production defaults for `dir`.
    pub fn new(dir: PathBuf) -> Config {
        Config {
            base_url: DEFAULT_BASE.to_string(),
            web_url: DEFAULT_SITE.to_string(),
            channel: String::new(),
            platform: lr_update::manifest::this_platform().to_string(),
            dir,
            trusted: lr_update::verify::trusted_keys(),
            os_version: lr_update::osver::detect(),
            mode: Mode::Run,
            force: false,
            update_first: false,
            no_update: false,
            no_launch: false,
            test_mode: false,
            verify_deep: false,
            game_args: Vec::new(),
            startup_window: Duration::from_secs(30),
            fail_limit: 2,
            quick_seconds: 4,
            background_wait: Duration::from_secs(900),
            opener: Arc::new(|url| lr_update::browser::open_url(url)),
            echo: true,
            on_game_started: None,
        }
    }
}
