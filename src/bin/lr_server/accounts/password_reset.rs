//! Lost-password recovery for the account store (`accounts.json`).
//!
//! Three ways to set another account's password exist; this file holds what two of them share:
//!
//! * `lr_server reset-password USERNAME` (the offline command, [`run_cli`]): for the worst case where nobody can sign in
//!   as an admin and only shell access is left. It edits the file itself.
//! * `POST /api/admin/account/reset-password` (`AccountService::admin_reset_password`): a signed-in full admin owner.
//! * The admin menu's RESET PASSWORD field in the game, which calls that endpoint.
//!
//! The running server keeps every account in memory and rewrites the whole file on each change. An edit made behind its
//! back is therefore overwritten by the next save, and the old password stays valid for as long as the server runs. So
//! the server holds an exclusive lock on `<accounts file>.lock` while it runs ([`lock_store`]), and the offline command
//! refuses to touch the file while that lock is held unless `--force` is given. After a reset the server has to be
//! restarted to load the new file.
//!
//! No password is ever written to a log: this file prints a generated password to stdout once and nothing else about it.

use crate::account_service::{
    password_is_valid, password_record, username_of_str, valid_username, WEAK_PASSWORD_MESSAGE,
};
use crate::util::{field, iso_time, random_bytes, str_or_empty, stringify, truthy};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Letters and digits that cannot be mistaken for each other when read aloud or typed: no `I`, `O`, `0`, `1`. Exactly 32
/// symbols, so one random byte masked with 31 picks one without bias.
const TEMP_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const TEMP_GROUPS: usize = 4;
const TEMP_GROUP_LEN: usize = 4;

/// A one-time temporary password: 16 random symbols from [`TEMP_ALPHABET`] (80 bits) in four dash-separated groups of
/// four, like `K7QF-2MXD-H9TA-WP4C`. It meets the register rule (6 to 200 characters).
pub fn generate_temporary_password() -> String {
    let bytes = random_bytes(TEMP_GROUPS * TEMP_GROUP_LEN);
    let mut out = String::with_capacity(TEMP_GROUPS * (TEMP_GROUP_LEN + 1));
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && i % TEMP_GROUP_LEN == 0 {
            out.push('-');
        }
        out.push(TEMP_ALPHABET[(byte & 31) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The store, as parsed JSON.

#[derive(Debug, PartialEq, Eq)]
pub enum ResetError {
    /// The username is not 3 to 24 letters, digits or underscores.
    BadUsername,
    /// The new password breaks the register rule.
    WeakPassword,
    /// There is no accounts file at this path.
    NoStore(PathBuf),
    /// The file is not an account store (not JSON, or no `accounts` list).
    BadStore(String),
    /// No account of that (canonical) name is in the file.
    NoSuchAccount(String),
    /// Reading or writing failed.
    Io(String),
}

impl std::fmt::Display for ResetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResetError::BadUsername => write!(f, "Username must be 3-24 characters: letters, numbers, underscore"),
            ResetError::WeakPassword => write!(f, "{WEAK_PASSWORD_MESSAGE}"),
            ResetError::NoStore(path) => write!(f, "No accounts file at {}", path.display()),
            ResetError::BadStore(why) => write!(f, "The accounts file is not usable: {why}"),
            ResetError::NoSuchAccount(name) => write!(f, "No account named {name} (nothing was changed; --list shows the names)"),
            ResetError::Io(why) => write!(f, "{why}"),
        }
    }
}

/// What the store edit changed.
#[derive(Debug, PartialEq, Eq)]
pub struct StoreReset {
    pub username: String,
    /// Account entries given the new salt and hash (normally 1; a hand-edited file could name the account twice).
    pub accounts_updated: usize,
    /// Sessions of that account removed from the file.
    pub sessions_removed: usize,
}

/// The canonical name of a stored account entry, when the server would load it: an object with a valid username and a
/// non-empty salt and hash (`AccountService::load`). Anything else in the list is junk the server skips.
fn loadable_name(entry: &Value) -> Option<String> {
    let name = username_of_str(&str_or_empty(field(entry, "username")));
    (valid_username(&name) && truthy(field(entry, "salt")) && truthy(field(entry, "hash"))).then_some(name)
}

/// The account names in a parsed store, canonical (UPPERCASE), in file order, without repeats. No hashes.
pub fn list_account_names(store: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(Value::Array(accounts)) = field(store, "accounts") {
        for name in accounts.iter().filter_map(loadable_name) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Gives the account `username` this salt and hash and removes that account's sessions, leaving every other account,
/// every other field and every other session as they were. Pure: it takes the finished record, so it can be tested
/// without the slow key derivation.
pub fn reset_password_in_store(store: &mut Value, username: &str, salt: &str, hash: &str) -> Result<StoreReset, ResetError> {
    let target = username_of_str(username);
    if !valid_username(&target) {
        return Err(ResetError::BadUsername);
    }
    let Value::Object(root) = store else {
        return Err(ResetError::BadStore("the top level is not an object".into()));
    };
    let Some(Value::Array(accounts)) = root.get_mut("accounts") else {
        return Err(ResetError::BadStore("there is no accounts list".into()));
    };
    let mut updated = 0;
    for entry in accounts.iter_mut() {
        if loadable_name(entry).as_deref() == Some(target.as_str()) {
            if let Value::Object(account) = entry {
                account.insert("salt".into(), Value::String(salt.to_string()));
                account.insert("hash".into(), Value::String(hash.to_string()));
                updated += 1;
            }
        }
    }
    if updated == 0 {
        return Err(ResetError::NoSuchAccount(target));
    }
    let mut removed = 0;
    if let Some(Value::Array(sessions)) = root.get_mut("sessions") {
        let before = sessions.len();
        sessions.retain(|session| username_of_str(&str_or_empty(field(session, "username"))) != target);
        removed = before - sessions.len();
    }
    Ok(StoreReset { username: target, accounts_updated: updated, sessions_removed: removed })
}

// ---------------------------------------------------------------------------------------------------------------
// The file.

/// `accounts.json.20261106T123456Z.bak`: the backup written next to the file before a reset.
pub fn backup_path(store: &Path, now_ms: i64, attempt: u32) -> PathBuf {
    let iso = iso_time(now_ms);
    let stamp: String = iso.chars().take(19).filter(|c| *c != '-' && *c != ':').collect();
    let mut name = store.as_os_str().to_owned();
    name.push(format!(".{stamp}Z"));
    if attempt > 0 {
        name.push(format!(".{attempt}"));
    }
    name.push(".bak");
    PathBuf::from(name)
}

/// Writes `bytes` to a NEW file (fails when it exists) that never has looser permissions than `like`: it is created
/// owner-only, then given `like`'s mode, and (on Unix) its owner and group when they differ and we may. Returns a
/// warning when the ownership could not be copied.
fn write_new_file(path: &Path, bytes: &[u8], like: &std::fs::Metadata) -> std::io::Result<Option<String>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[allow(unused_mut)]
    let mut warning = None;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::set_permissions(path, like.permissions())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let made = std::fs::metadata(path)?;
            if (made.uid(), made.gid()) != (like.uid(), like.gid())
                && std::os::unix::fs::chown(path, Some(like.uid()), Some(like.gid())).is_err()
            {
                warning = Some(format!(
                    "{} is owned by a different user than the old file; run this command as the user the server runs as",
                    path.display()
                ));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(warning)
}

/// What a reset of the file did.
#[derive(Debug)]
pub struct FileReset {
    pub store: StoreReset,
    /// The copy of the file as it was before the edit.
    pub backup: PathBuf,
    pub warnings: Vec<String>,
}

/// Sets the password of `username` in the accounts file at `path`: validates the name and password with the register
/// rules, writes a timestamped `.bak` copy of the old file next to it, then replaces the file atomically (a temp file
/// in the same folder, renamed over it) with the new salt and hash and that account's sessions gone. Nothing is
/// written when the account does not exist. The caller prints the result; this never logs a password.
pub fn reset_password_file(path: &Path, username: &str, password: &str, now_ms: i64) -> Result<FileReset, ResetError> {
    let target = username_of_str(username);
    if !valid_username(&target) {
        return Err(ResetError::BadUsername);
    }
    if !password_is_valid(password) {
        return Err(ResetError::WeakPassword);
    }
    let io = |what: &str, error: std::io::Error| ResetError::Io(format!("{what} {}: {error}", path.display()));
    let original = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(ResetError::NoStore(path.to_path_buf())),
        Err(error) => return Err(io("Could not read", error)),
    };
    let mut store: Value = serde_json::from_slice(&original).map_err(|error| ResetError::BadStore(error.to_string()))?;
    let (salt, hash) = password_record(password);
    let reset = reset_password_in_store(&mut store, &target, &salt, &hash)?;
    let like = std::fs::metadata(path).map_err(|error| io("Could not stat", error))?;
    let mut warnings = Vec::new();

    let mut backup = None;
    for attempt in 0..100 {
        let candidate = backup_path(path, now_ms, attempt);
        match write_new_file(&candidate, &original, &like) {
            Ok(warning) => {
                warnings.extend(warning);
                backup = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ResetError::Io(format!("Could not write the backup {}: {error}", candidate.display()))),
        }
    }
    let backup = backup.ok_or_else(|| ResetError::Io("Could not find a free backup name".into()))?;

    let mut temp = path.as_os_str().to_owned();
    temp.push(".reset-tmp");
    let temp = PathBuf::from(temp);
    let _ = std::fs::remove_file(&temp);
    match write_new_file(&temp, stringify(&store).as_bytes(), &like) {
        Ok(warning) => warnings.extend(warning),
        Err(error) => return Err(ResetError::Io(format!("Could not write {}: {error}", temp.display()))),
    }
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(io("Could not replace", error));
    }
    Ok(FileReset { store: reset, backup, warnings })
}

// ---------------------------------------------------------------------------------------------------------------
// The lock the running server holds.

/// `<accounts file>.lock`.
pub fn store_lock_path(store: &Path) -> PathBuf {
    let mut name = store.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

pub enum StoreLock {
    /// We hold the lock for as long as the file is kept open.
    Held(File),
    /// Another process holds it: a server is running on this account file.
    InUse,
    /// The lock file could not be opened or locked (read-only folder, a file system without locks): carry on without it.
    Unavailable(String),
}

/// Takes the exclusive lock on the account store once, without waiting. The lock goes when the process ends, however
/// it ends.
pub fn lock_store(store: &Path) -> StoreLock {
    let lock = store_lock_path(store);
    if let Some(parent) = lock.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = match OpenOptions::new().create(true).write(true).truncate(false).open(&lock) {
        Ok(file) => file,
        // A lock file made by another user is still lockable when it can only be read.
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => match File::open(&lock) {
            Ok(file) => file,
            Err(error) => return StoreLock::Unavailable(error.to_string()),
        },
        Err(error) => return StoreLock::Unavailable(error.to_string()),
    };
    match file.try_lock() {
        Ok(()) => StoreLock::Held(file),
        Err(TryLockError::WouldBlock) => StoreLock::InUse,
        Err(TryLockError::Error(error)) => StoreLock::Unavailable(error.to_string()),
    }
}

/// [`lock_store`], retrying for up to `wait` while it is in use (a reset command that is finishing, or the old server
/// still shutting down during a restart).
pub fn lock_store_waiting(store: &Path, wait: Duration) -> StoreLock {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match lock_store(store) {
            StoreLock::InUse if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            other => return other,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The command line.

#[derive(Debug, PartialEq, Eq)]
pub enum PasswordSource {
    /// No password given: make a temporary one and print it once.
    Generate,
    /// `--password VALUE`.
    Literal(String),
    /// `--password-stdin`: the first line of standard input.
    Stdin,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CliArgs {
    pub username: Option<String>,
    pub password: PasswordSource,
    pub data_dir: Option<String>,
    pub force: bool,
    pub list: bool,
    pub help: bool,
}

pub const USAGE: &str = "\
usage: lr_server reset-password USERNAME [--password NEWPASS | --password-stdin] [--data-dir DIR] [--force]
       lr_server reset-password --list [--data-dir DIR]

Sets an account's password in accounts.json while the server is stopped. Without a password option a temporary one
is generated and printed once on stdout. The account must already exist (nothing is created).

  USERNAME          the account (any case; stored in UPPERCASE)
  --password P      use this password (6-200 characters). It shows up in the process list and shell history:
                    prefer --password-stdin or the generated one
  --password-stdin  read the password from the first line of standard input
  --data-dir DIR    the server's data directory (the folder holding accounts.json); default: the same
                    LASTROUTE_DATA_DIR / LASTROUTE_ACCOUNT_STORE settings the server reads, else ./.data
  --list            print the account names (no hashes) and exit
  --force           edit the file although a running server holds it. The running server keeps the old
                    password in memory and overwrites this change on its next save: restart it at once
  -h, --help        this text
";

/// Parses the words after `reset-password`.
pub fn parse_cli_args(args: &[String]) -> Result<CliArgs, String> {
    let mut parsed =
        CliArgs { username: None, password: PasswordSource::Generate, data_dir: None, force: false, list: false, help: false };
    let mut i = 0;
    let value_of = |flag: &str, inline: Option<&str>, i: &mut usize| -> Result<String, String> {
        match inline {
            Some(value) => Ok(value.to_string()),
            None => {
                *i += 1;
                args.get(*i).cloned().ok_or_else(|| format!("{flag} needs a value"))
            }
        }
    };
    while i < args.len() {
        let word = args[i].as_str();
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (word, None),
        };
        match flag {
            "-h" | "--help" => parsed.help = true,
            "--list" => parsed.list = true,
            "--force" => parsed.force = true,
            "--password" => {
                if parsed.password != PasswordSource::Generate {
                    return Err("give the password once: --password or --password-stdin".into());
                }
                parsed.password = PasswordSource::Literal(value_of(flag, inline, &mut i)?);
            }
            "--password-stdin" => {
                if parsed.password != PasswordSource::Generate {
                    return Err("give the password once: --password or --password-stdin".into());
                }
                parsed.password = PasswordSource::Stdin;
            }
            "--data-dir" => parsed.data_dir = Some(value_of(flag, inline, &mut i)?),
            other if other.starts_with('-') && other.len() > 1 => return Err(format!("unknown option {other}")),
            name => {
                if parsed.username.replace(name.to_string()).is_some() {
                    return Err("give one username".into());
                }
            }
        }
        i += 1;
    }
    Ok(parsed)
}

/// `lr_server reset-password ...`. `args` are the words after the subcommand. The generated password (and nothing else)
/// goes to `out`; every message goes to `err`. Returns the exit status: 0 done, 1 failed or bad usage, 2 refused because
/// a server is running.
pub fn run_cli(
    args: &[String],
    env: &HashMap<String, String>,
    cwd: &Path,
    now_ms: i64,
    stdin: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let parsed = match parse_cli_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            let _ = writeln!(err, "reset-password: {message}\n\n{USAGE}");
            return 1;
        }
    };
    if parsed.help {
        let _ = write!(out, "{USAGE}");
        return 0;
    }
    let store_path = match &parsed.data_dir {
        Some(dir) => cwd.join(dir).join("accounts.json"),
        None if env.get("LASTROUTE_ACCOUNT_UPSTREAM").is_some_and(|v| !v.trim().is_empty()) => {
            let _ = writeln!(
                err,
                "reset-password: LASTROUTE_ACCOUNT_UPSTREAM is set, so the website owns the accounts and accounts.json is not used. \
                 Reset the password on the website, or pass --data-dir to edit a local file anyway."
            );
            return 1;
        }
        None => crate::server::account_store_path_from(env, cwd),
    };

    if parsed.list {
        return match read_store(&store_path) {
            Ok(store) => {
                let names = list_account_names(&store);
                for name in &names {
                    let _ = writeln!(out, "{name}");
                }
                let _ = writeln!(err, "{} account(s) in {}", names.len(), store_path.display());
                0
            }
            Err(error) => {
                let _ = writeln!(err, "reset-password: {error}");
                1
            }
        };
    }

    let Some(username) = parsed.username.as_deref() else {
        let _ = writeln!(err, "reset-password: give the username to reset (or --list)\n\n{USAGE}");
        return 1;
    };
    if !valid_username(&username_of_str(username)) {
        let _ = writeln!(err, "reset-password: {}", ResetError::BadUsername);
        return 1;
    }
    let (password, generated) = match parsed.password {
        PasswordSource::Generate => (generate_temporary_password(), true),
        PasswordSource::Literal(password) => (password, false),
        PasswordSource::Stdin => {
            let mut line = String::new();
            if let Err(error) = stdin.read_line(&mut line) {
                let _ = writeln!(err, "reset-password: could not read the password from standard input: {error}");
                return 1;
            }
            (line.trim_end_matches(['\n', '\r']).to_string(), false)
        }
    };
    if !password_is_valid(&password) {
        let _ = writeln!(err, "reset-password: {}", ResetError::WeakPassword);
        return 1;
    }
    if !store_path.is_file() {
        let _ = writeln!(err, "reset-password: {}", ResetError::NoStore(store_path));
        return 1;
    }

    let lock = lock_store(&store_path);
    let mut server_may_be_running = false;
    match &lock {
        StoreLock::InUse if !parsed.force => {
            let _ = writeln!(
                err,
                "reset-password: the server is running on {}. It keeps the accounts in memory and would overwrite this change \
                 (and keep the old password working).\nStop it first (systemctl stop lastroute-rust), run this command again, \
                 then start it. --force edits the file anyway.",
                store_path.display()
            );
            return 2;
        }
        StoreLock::InUse => server_may_be_running = true,
        StoreLock::Unavailable(why) => {
            let _ = writeln!(err, "warning: could not check whether the server is running ({why}); make sure it is stopped");
        }
        StoreLock::Held(_) => {}
    }

    match reset_password_file(&store_path, username, &password, now_ms) {
        Ok(done) => {
            let _ = writeln!(
                err,
                "Password reset for {} ({} account entr{}, {} session(s) signed out).",
                done.store.username,
                done.store.accounts_updated,
                if done.store.accounts_updated == 1 { "y" } else { "ies" },
                done.store.sessions_removed
            );
            let _ = writeln!(err, "Backup of the old file: {}", done.backup.display());
            for warning in &done.warnings {
                let _ = writeln!(err, "warning: {warning}");
            }
            if server_may_be_running {
                let _ = writeln!(
                    err,
                    "WARNING: a server is running on this file. It still holds the old password and sessions in memory and will \
                     overwrite this change the next time it saves. Restart it NOW (systemctl restart lastroute-rust)."
                );
            } else {
                let _ = writeln!(err, "Start (or restart) the server now: it loads accounts.json only at startup.");
            }
            if generated {
                let _ = writeln!(err, "Temporary password for {} (shown once; tell them to change it after signing in):", done.store.username);
                let _ = writeln!(out, "{password}");
            }
            0
        }
        Err(error) => {
            let _ = writeln!(err, "reset-password: {error}");
            1
        }
    }
}

fn read_store(path: &Path) -> Result<Value, ResetError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(ResetError::NoStore(path.to_path_buf())),
        Err(error) => return Err(ResetError::Io(format!("Could not read {}: {error}", path.display()))),
    };
    serde_json::from_slice(&bytes).map_err(|error| ResetError::BadStore(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_service::{password_matches, AccountService, AccountServiceOptions, MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH};
    use crate::util::{hex, manual_clock};
    use serde_json::json;

    // Obviously fake passwords for tests only.
    const OLD: &str = "fake-old-password-1";
    const NEW: &str = "fake-new-password-2";

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Dir {
            let dir = std::env::temp_dir().join(format!("lr_reset_{}", hex(&random_bytes(6))));
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
        fn store(&self) -> PathBuf {
            self.0.join("accounts.json")
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A real store written by the account service: ADA (progress, two sessions), BOB (progress, one session), AYTTU
    /// (an owner) and a limited admin. Returns the dir and ADA's and BOB's session tokens.
    fn seeded() -> (Dir, String, String) {
        let dir = Dir::new();
        let (clock, _) = manual_clock(1_700_000_000_000);
        let accounts =
            AccountService::new(AccountServiceOptions { store_path: Some(dir.store()), now: clock, ..Default::default() });
        let ada = accounts.register(&json!({ "username": "ada", "password": OLD }), false).1;
        let ada_token = ada["sessionToken"].as_str().unwrap().to_string();
        accounts.login(&json!({ "username": "ADA", "password": OLD }));
        accounts.save(&json!({ "sessionToken": ada_token, "progress": { "floor": 7, "note": "keep me" } }));
        let bob = accounts.register(&json!({ "username": "bob", "password": OLD }), false).1;
        let bob_token = bob["sessionToken"].as_str().unwrap().to_string();
        accounts.save(&json!({ "sessionToken": bob_token, "progress": { "floor": 2 } }));
        let owner = accounts.register(&json!({ "username": "AYTTU", "password": OLD }), true).1;
        accounts.register(&json!({ "username": "MOD", "password": OLD }), false);
        accounts.set_admin_roster(&json!({
            "sessionToken": owner["sessionToken"], "username": "MOD", "permissions": ["world", "visual"],
        }));
        (dir, ada_token, bob_token)
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn account<'a>(store: &'a Value, name: &str) -> &'a Value {
        store["accounts"].as_array().unwrap().iter().find(|a| a["username"] == name).unwrap()
    }

    fn checks(store: &Value, name: &str, password: &str) -> bool {
        let a = account(store, name);
        password_matches(password, a.get("salt"), a.get("hash"))
    }

    #[test]
    fn generated_passwords_meet_the_rules_use_the_unambiguous_alphabet_and_differ() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let password = generate_temporary_password();
            assert!(password_is_valid(&password), "{password}");
            assert_eq!(password.len(), 19, "{password}");
            assert!(password.len() >= 12);
            let symbols: Vec<char> = password.chars().filter(|c| *c != '-').collect();
            assert_eq!(symbols.len(), 16);
            assert!(symbols.iter().all(|c| TEMP_ALPHABET.contains(&(*c as u8))), "{password}");
            assert!(!password.contains(['0', 'O', '1', 'I', 'l']), "{password}");
            assert!(seen.insert(password), "a repeat in 200 draws");
        }
        assert_eq!(TEMP_ALPHABET.len(), 32);
        assert!((MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&19));
    }

    #[test]
    fn the_store_edit_changes_only_the_target_and_its_sessions() {
        let (dir, ada_token, bob_token) = seeded();
        let before = read_json(&dir.store());
        assert_eq!(before["sessions"].as_array().unwrap().len(), 5);
        let mut store = before.clone();
        let done = reset_password_in_store(&mut store, " ada ", "newsalt", "newhash").unwrap();
        assert_eq!(done, StoreReset { username: "ADA".into(), accounts_updated: 1, sessions_removed: 2 });
        // the target: new salt and hash, everything else of the entry as it was
        let (was, now) = (account(&before, "ADA"), account(&store, "ADA"));
        assert_eq!((now["salt"].clone(), now["hash"].clone()), (json!("newsalt"), json!("newhash")));
        for key in ["username", "progress", "savedAt", "adminPermissions"] {
            assert_eq!(was[key], now[key], "{key}");
        }
        // every other account is identical, byte for byte as serialized
        for name in ["BOB", "AYTTU", "MOD"] {
            assert_eq!(stringify(account(&before, name)), stringify(account(&store, name)), "{name}");
        }
        // sessions: ADA's two are gone, the others stay as they were
        let tokens = |s: &Value| -> Vec<String> {
            s["sessions"].as_array().unwrap().iter().map(|x| stringify(x)).collect()
        };
        let kept: Vec<String> = before["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["username"] != "ADA")
            .map(|s| stringify(s))
            .collect();
        assert_eq!(tokens(&store), kept);
        assert_eq!(kept.len(), 3);
        let _ = (ada_token, bob_token);
    }

    #[test]
    fn unknown_junk_and_bad_input_fail_without_touching_the_store() {
        let mut store = json!({
            "version": 2,
            "accounts": [
                { "username": "ada", "salt": "s", "hash": "h" },
                { "username": "ghost", "salt": "", "hash": "h" },
                { "username": "no", "salt": "s", "hash": "h" },
                "junk"
            ],
            "sessions": [{ "tokenHash": "x", "username": "GHOST" }]
        });
        let before = store.clone();
        assert_eq!(reset_password_in_store(&mut store, "nobody", "s", "h"), Err(ResetError::NoSuchAccount("NOBODY".into())));
        // an entry the server would skip is not an account
        assert_eq!(reset_password_in_store(&mut store, "ghost", "s", "h"), Err(ResetError::NoSuchAccount("GHOST".into())));
        assert_eq!(reset_password_in_store(&mut store, "no", "s", "h"), Err(ResetError::BadUsername));
        assert_eq!(reset_password_in_store(&mut store, "has space", "s", "h"), Err(ResetError::BadUsername));
        assert_eq!(store, before);
        assert!(matches!(reset_password_in_store(&mut json!([]), "ada", "s", "h"), Err(ResetError::BadStore(_))));
        assert!(matches!(reset_password_in_store(&mut json!({ "version": 2 }), "ada", "s", "h"), Err(ResetError::BadStore(_))));
        assert_eq!(list_account_names(&store), ["ADA"], "only entries the server would load are listed");
    }

    #[test]
    fn the_file_reset_verifies_like_login_writes_a_backup_and_signs_the_account_out() {
        let (dir, ada_token, bob_token) = seeded();
        let before_text = std::fs::read_to_string(dir.store()).unwrap();
        let before = read_json(&dir.store());
        let done = reset_password_file(&dir.store(), "Ada", NEW, 1_700_000_123_000).unwrap();
        assert_eq!((done.store.username.as_str(), done.store.sessions_removed), ("ADA", 2));
        // the backup is the old file, named with the time, next to it
        assert_eq!(done.backup.file_name().unwrap().to_str().unwrap(), "accounts.json.20231114T221523Z.bak");
        assert_eq!(done.backup.parent(), dir.store().parent());
        assert_eq!(std::fs::read_to_string(&done.backup).unwrap(), before_text);
        // no temp file is left behind
        assert!(!dir.0.join("accounts.json.reset-tmp").exists());

        let after = read_json(&dir.store());
        assert!(checks(&after, "ADA", NEW), "the new password verifies with the login check");
        assert!(!checks(&after, "ADA", OLD), "the old one no longer does");
        assert_ne!(account(&after, "ADA")["salt"], account(&before, "ADA")["salt"]);
        assert_ne!(account(&after, "ADA")["hash"], account(&before, "ADA")["hash"]);
        for name in ["BOB", "AYTTU", "MOD"] {
            assert_eq!(stringify(account(&before, name)), stringify(account(&after, name)), "{name}");
            assert!(checks(&after, name, OLD), "{name} still has the old password");
        }
        assert_eq!(account(&after, "ADA")["progress"], json!({ "floor": 7, "note": "keep me" }));
        assert_eq!(account(&after, "MOD")["adminPermissions"], json!(["visual", "world"]));

        // a server started on the new file: the new password signs in, the old does not, the old sessions are dead,
        // the other account's session survives
        let (clock, _) = manual_clock(1_700_000_200_000);
        let server =
            AccountService::new(AccountServiceOptions { store_path: Some(dir.store()), now: clock, ..Default::default() });
        assert_eq!(server.login(&json!({ "username": "ADA", "password": NEW })).0, 200);
        assert_eq!(server.login(&json!({ "username": "ADA", "password": OLD })).0, 401);
        assert_eq!(server.current_session(&json!({ "sessionToken": ada_token })).0, 401);
        assert_eq!(server.current_session(&json!({ "sessionToken": bob_token })).0, 200);
        // and the stored text never holds a password
        let text = std::fs::read_to_string(dir.store()).unwrap();
        assert!(!text.contains(NEW) && !text.contains(OLD));
    }

    #[test]
    fn the_file_reset_refuses_unknown_users_weak_passwords_and_missing_files_and_changes_nothing() {
        let (dir, ..) = seeded();
        let text = std::fs::read_to_string(dir.store()).unwrap();
        let listing = |dir: &Dir| std::fs::read_dir(&dir.0).unwrap().count();
        let files = listing(&dir);
        assert_eq!(reset_password_file(&dir.store(), "nobody", NEW, 0).unwrap_err(), ResetError::NoSuchAccount("NOBODY".into()));
        assert_eq!(reset_password_file(&dir.store(), "ada", "short", 0).unwrap_err(), ResetError::WeakPassword);
        assert_eq!(reset_password_file(&dir.store(), "ada", &"x".repeat(201), 0).unwrap_err(), ResetError::WeakPassword);
        assert_eq!(reset_password_file(&dir.store(), "a!", NEW, 0).unwrap_err(), ResetError::BadUsername);
        assert_eq!(std::fs::read_to_string(dir.store()).unwrap(), text);
        assert_eq!(listing(&dir), files, "no backup or temp file for a refused reset");
        let missing = dir.0.join("none").join("accounts.json");
        assert!(matches!(reset_password_file(&missing, "ada", NEW, 0), Err(ResetError::NoStore(_))));
        assert!(!missing.parent().unwrap().exists());
        std::fs::write(dir.store(), "{").unwrap();
        assert!(matches!(reset_password_file(&dir.store(), "ada", NEW, 0), Err(ResetError::BadStore(_))));
    }

    #[test]
    fn two_resets_in_the_same_second_keep_both_backups() {
        let (dir, ..) = seeded();
        let first = reset_password_file(&dir.store(), "ada", NEW, 5_000).unwrap();
        let second = reset_password_file(&dir.store(), "bob", NEW, 5_000).unwrap();
        assert_ne!(first.backup, second.backup);
        assert!(first.backup.exists() && second.backup.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_new_file_and_the_backup_keep_the_old_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, ..) = seeded();
        std::fs::set_permissions(dir.store(), std::fs::Permissions::from_mode(0o640)).unwrap();
        let done = reset_password_file(&dir.store(), "ada", NEW, 1).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.store()), 0o640);
        assert_eq!(mode(&done.backup), 0o640);
    }

    #[test]
    fn the_server_lock_is_exclusive_and_the_command_refuses_while_it_is_held() {
        let (dir, ..) = seeded();
        let StoreLock::Held(server) = lock_store(&dir.store()) else { panic!("the first lock is ours") };
        assert!(matches!(lock_store(&dir.store()), StoreLock::InUse));
        assert!(matches!(lock_store_waiting(&dir.store(), Duration::from_millis(150)), StoreLock::InUse));
        let before = std::fs::read_to_string(dir.store()).unwrap();

        let (code, _, err) = cli(&dir, &["ada", "--data-dir", dir.0.to_str().unwrap()], "");
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("server is running"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.store()).unwrap(), before, "nothing was written");

        // --force goes ahead and says what to do about the running server
        let (code, _, err) = cli(&dir, &["ada", "--password", NEW, "--force", "--data-dir", dir.0.to_str().unwrap()], "");
        assert_eq!(code, 0, "{err}");
        assert!(err.contains("Restart it NOW"), "{err}");
        assert!(checks(&read_json(&dir.store()), "ADA", NEW));

        drop(server);
        assert!(matches!(lock_store(&dir.store()), StoreLock::Held(_)), "the lock goes with the server");
    }

    fn cli(dir: &Dir, args: &[&str], stdin: &str) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_cli(&args, &HashMap::new(), &dir.0, 1_700_000_000_000, &mut stdin.as_bytes(), &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn the_command_generates_a_password_prints_it_once_on_stdout_only_and_never_logs_it() {
        let (dir, ..) = seeded();
        let (code, out, err) = cli(&dir, &["ada", "--data-dir", "."], "");
        assert_eq!(code, 0, "{err}");
        let password = out.trim_end().to_string();
        assert_eq!(out, format!("{password}\n"), "stdout is the password and nothing else");
        assert!(password_is_valid(&password) && password.len() >= 12);
        assert_eq!(err.matches(&password).count(), 0, "the messages never repeat the password: {err}");
        assert!(err.contains("Password reset for ADA") && err.contains("Backup of the old file") && err.contains("restart"), "{err}");
        assert!(checks(&read_json(&dir.store()), "ADA", &password));
        // a second run makes a different one
        let (_, again, _) = cli(&dir, &["ada", "--data-dir", "."], "");
        assert_ne!(again, out);
    }

    #[test]
    fn the_command_takes_a_given_password_from_the_option_or_standard_input_and_does_not_echo_it() {
        let (dir, ..) = seeded();
        let (code, out, err) = cli(&dir, &["BOB", "--password", NEW, "--data-dir", "."], "");
        assert_eq!(code, 0, "{err}");
        assert_eq!(out, "");
        assert!(!err.contains(NEW) && !format!("{err}{out}").contains(NEW));
        assert!(checks(&read_json(&dir.store()), "BOB", NEW));
        let (code, out, err) = cli(&dir, &["ada", "--password-stdin", "--data-dir=."], "fake-stdin-password-3\r\n");
        assert_eq!((code, out.as_str()), (0, ""), "{err}");
        assert!(checks(&read_json(&dir.store()), "ADA", "fake-stdin-password-3"));
    }

    #[test]
    fn the_command_fails_clearly_and_creates_nothing() {
        let (dir, ..) = seeded();
        let text = std::fs::read_to_string(dir.store()).unwrap();
        let (code, out, err) = cli(&dir, &["nobody", "--data-dir", "."], "");
        assert_eq!((code, out.as_str()), (1, ""));
        assert!(err.contains("No account named NOBODY"), "{err}");
        let (code, _, err) = cli(&dir, &["ada", "--password", "short", "--data-dir", "."], "");
        assert_eq!(code, 1);
        assert!(err.contains("6-200"), "{err}");
        assert_eq!(cli(&dir, &["--data-dir", "."], "").0, 1, "a username is required");
        assert_eq!(cli(&dir, &["ada", "bob", "--data-dir", "."], "").0, 1);
        assert_eq!(cli(&dir, &["ada", "--nope"], "").0, 1);
        assert_eq!(cli(&dir, &["ada", "--password"], "").0, 1);
        assert_eq!(cli(&dir, &["ada", "--password", "x", "--password-stdin"], "").0, 1);
        let empty = Dir::new();
        let (code, _, err) = cli(&empty, &["ada", "--data-dir", "."], "");
        assert_eq!(code, 1);
        assert!(err.contains("No accounts file"), "{err}");
        assert_eq!(std::fs::read_dir(&empty.0).unwrap().count(), 0, "no account, lock or backup file is made");
        assert_eq!(std::fs::read_to_string(dir.store()).unwrap(), text);
        let mut names: Vec<String> =
            std::fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["accounts.json", "accounts.json.lock"], "a refused reset leaves no backup or temp file");
    }

    #[test]
    fn the_command_finds_the_file_the_server_would_and_refuses_when_the_website_owns_the_accounts() {
        let (dir, ..) = seeded();
        let nested = dir.0.join("data");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::rename(dir.store(), nested.join("accounts.json")).unwrap();
        let run = |env: &[(&str, &str)], args: &[&str]| {
            let env: HashMap<String, String> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_cli(&args, &env, &dir.0, 0, &mut "".as_bytes(), &mut out, &mut err);
            (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
        };
        let (code, out, _) = run(&[("LASTROUTE_DATA_DIR", "data")], &["--list"]);
        assert_eq!((code, out.as_str()), (0, "ADA\nBOB\nAYTTU\nMOD\n"));
        let (code, out, _) = run(&[("KYTTU_DATA_DIR", "data")], &["--list"]);
        assert_eq!((code, out.as_str()), (0, "ADA\nBOB\nAYTTU\nMOD\n"));
        let (code, out, _) = run(&[("LASTROUTE_ACCOUNT_STORE", "data/accounts.json")], &["--list"]);
        assert_eq!((code, out.as_str()), (0, "ADA\nBOB\nAYTTU\nMOD\n"));
        assert!(!out.contains("hash") && !out.contains("salt"));
        let (code, _, err) = run(&[("LASTROUTE_DATA_DIR", "data"), ("LASTROUTE_ACCOUNT_UPSTREAM", "https://site.example")], &["ada"]);
        assert_eq!(code, 1);
        assert!(err.contains("website owns the accounts"), "{err}");
        // --data-dir still reaches a local file when the upstream is set
        let (code, _, _) = run(&[("LASTROUTE_ACCOUNT_UPSTREAM", "https://site.example")], &["--list", "--data-dir", "data"]);
        assert_eq!(code, 0);
        assert_eq!(run(&[], &["--help"]).0, 0);
    }
}
