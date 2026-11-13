//! Profiles: several players (or one player with several accounts) on one computer, each with their own settings,
//! saves, skins, account sign-in and caches (docs/PROFILES.md).
//!
//! The DEFAULT profile is the root user folder itself (`paths::root_user_dir`), so a player who never makes a second
//! profile keeps every file where it always was. Every other profile is the folder `profiles/<folder name>/` under
//! the root. `profiles.json` in the root lists them (name, created, last used) and names the one used last.
//!
//! This file is the pure part: names, the index, the folders and the instance lock, all given the root folder and
//! the time by the caller. It holds no global state; `app/instance.rs` chooses the profile at start.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The profile that is the root user folder. It always exists and cannot be deleted or renamed.
pub const DEFAULT_NAME: &str = "DEFAULT";
/// The index file, in the root user folder.
pub const INDEX_FILE: &str = "profiles.json";
/// The folder under the root that holds one folder per profile.
pub const PROFILES_DIR: &str = "profiles";
/// Deleted profiles are moved here (under `profiles/`), never removed.
pub const TRASH_DIR: &str = ".trash";
/// The scratch folder the profile screen's own process runs on (its log and graphics marker), so it touches no profile.
pub const PICKER_DIR: &str = ".picker";
/// The lock file a running instance keeps in its profile folder.
pub const LOCK_FILE: &str = "instance.lock";
/// The longest profile name, in characters.
pub const NAME_MAX: usize = 16;
const SCHEMA: &str = "lr.profiles/1";

/// Why a typed profile name is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    /// A character outside A-Z, 0-9, space, `-` and `_`.
    BadChar,
    /// A name Windows keeps for devices (CON, NUL, COM1...), which cannot be a folder there.
    Reserved,
}

/// Why a profile change is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    Name(NameError),
    /// A profile with this name already exists.
    Exists,
    /// No profile has this name.
    Missing,
    /// DEFAULT cannot be deleted or renamed.
    Default,
    /// The folder could not be created or moved.
    Io(String),
}

impl ProfileError {
    /// The key of this error's line in data/ui/profiles.json (`errors`).
    pub fn key(&self) -> &'static str {
        match self {
            ProfileError::Name(NameError::Empty) => "name_empty",
            ProfileError::Name(NameError::TooLong) => "name_long",
            ProfileError::Name(NameError::BadChar) => "name_chars",
            ProfileError::Name(NameError::Reserved) => "name_reserved",
            ProfileError::Exists => "exists",
            ProfileError::Missing => "missing",
            ProfileError::Default => "default",
            ProfileError::Io(_) => "io",
        }
    }
}

/// Whether `c` (already upper-cased) may be in a profile name.
pub fn name_char_ok(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '-' || c == '_'
}

/// A typed name as a profile name: trimmed, upper-cased, runs of spaces made one space; 1 to 16 characters from
/// A-Z, 0-9, space, `-` and `_`.
pub fn clean_name(text: &str) -> Result<String, NameError> {
    let upper = text.trim().to_uppercase();
    let mut name = String::new();
    for c in upper.chars() {
        if !name_char_ok(c) {
            return Err(NameError::BadChar);
        }
        if c == ' ' && name.ends_with(' ') {
            continue;
        }
        name.push(c);
    }
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > NAME_MAX {
        return Err(NameError::TooLong);
    }
    if reserved_on_windows(&name) {
        return Err(NameError::Reserved);
    }
    Ok(name)
}

fn reserved_on_windows(name: &str) -> bool {
    if matches!(name, "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let numbered = |prefix: &str| {
        name.strip_prefix(prefix)
            .map_or(false, |rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()))
    };
    numbered("COM") || numbered("LPT")
}

/// The folder name of a valid profile name: the name with each space as `+` (so `A B` and `A_B` stay two folders).
/// Only A-Z, 0-9, `-`, `_` and `+` come out, which every file system takes.
pub fn folder_name(name: &str) -> String {
    name.chars().map(|c| if c == ' ' { '+' } else { c }).collect()
}

/// The folder of profile `name` under `root`: the root itself for DEFAULT, `profiles/<folder name>` otherwise.
pub fn profile_dir(root: &Path, name: &str) -> PathBuf {
    if name == DEFAULT_NAME {
        root.to_path_buf()
    } else {
        root.join(PROFILES_DIR).join(folder_name(name))
    }
}

/// The scratch folder of the profile screen's process.
pub fn picker_dir(root: &Path) -> PathBuf {
    root.join(PROFILES_DIR).join(PICKER_DIR)
}

/// One profile in the index. Times are Unix seconds; `last_used` 0 is never.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub created: u64,
    pub last_used: u64,
}

/// `profiles.json`: the profiles in the order they were made (DEFAULT first) and the one used last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub profiles: Vec<Profile>,
    pub last: String,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            profiles: vec![Profile { name: DEFAULT_NAME.to_string(), created: 0, last_used: 0 }],
            last: DEFAULT_NAME.to_string(),
        }
    }
}

impl Index {
    /// The index in `text`. Damaged text, bad names and repeats are dropped; DEFAULT is always there and first.
    pub fn parse(text: &str) -> Index {
        let mut index = Index::default();
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return index;
        };
        let num = |p: &Value, k: &str| p.get(k).and_then(Value::as_u64).unwrap_or(0);
        for p in v.get("profiles").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
            let Some(Ok(name)) = p.get("name").and_then(Value::as_str).map(clean_name) else {
                continue;
            };
            let entry = Profile { name: name.clone(), created: num(p, "created"), last_used: num(p, "last_used") };
            if name == DEFAULT_NAME {
                index.profiles[0] = entry;
            } else if !index.has(&name) {
                index.profiles.push(entry);
            }
        }
        if let Some(Ok(last)) = v.get("last").and_then(Value::as_str).map(clean_name) {
            if index.has(&last) {
                index.last = last;
            }
        }
        index
    }

    /// The index as the text of `profiles.json`.
    pub fn to_text(&self) -> String {
        let profiles: Vec<Value> = self
            .profiles
            .iter()
            .map(|p| json!({ "name": p.name, "created": p.created, "last_used": p.last_used }))
            .collect();
        let v = json!({ "schema": SCHEMA, "last": self.last, "profiles": profiles });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// The index of `root` (a missing or damaged file is the index with DEFAULT alone).
    pub fn load(root: &Path) -> Index {
        match std::fs::read_to_string(root.join(INDEX_FILE)) {
            Ok(text) => Index::parse(&text),
            Err(_) => Index::default(),
        }
    }

    /// Writes `profiles.json` in `root` (to a temporary file first, then renamed over the old one).
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let tmp = root.join(format!("{INDEX_FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, root.join(INDEX_FILE))
    }

    pub fn has(&self, name: &str) -> bool {
        self.profiles.iter().any(|p| p.name == name)
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// Adds a profile made at `now` and creates its folder. Returns the cleaned name.
    pub fn add(&mut self, root: &Path, typed: &str, now: u64) -> Result<String, ProfileError> {
        let name = clean_name(typed).map_err(ProfileError::Name)?;
        if self.has(&name) {
            return Err(ProfileError::Exists);
        }
        std::fs::create_dir_all(profile_dir(root, &name)).map_err(|e| ProfileError::Io(e.to_string()))?;
        self.profiles.push(Profile { name: name.clone(), created: now, last_used: 0 });
        Ok(name)
    }

    /// The profile `typed` names, added (and its folder created) when it is new. Returns the cleaned name.
    pub fn ensure(&mut self, root: &Path, typed: &str, now: u64) -> Result<String, ProfileError> {
        let name = clean_name(typed).map_err(ProfileError::Name)?;
        if self.has(&name) {
            std::fs::create_dir_all(profile_dir(root, &name)).map_err(|e| ProfileError::Io(e.to_string()))?;
            return Ok(name);
        }
        self.add(root, &name, now)
    }

    /// Marks `name` as used at `now`, and as the last one used. False when there is no such profile.
    pub fn touch(&mut self, name: &str, now: u64) -> bool {
        match self.profiles.iter_mut().find(|p| p.name == name) {
            Some(p) => {
                p.last_used = now;
                self.last = name.to_string();
                true
            }
            None => false,
        }
    }

    /// Deletes a profile: its folder moves to `profiles/.trash/<folder name>-<now>` (nothing is removed from the
    /// disk) and it leaves the index. DEFAULT is refused. Returns where the folder went, if there was one.
    pub fn remove(&mut self, root: &Path, name: &str, now: u64) -> Result<Option<PathBuf>, ProfileError> {
        if name == DEFAULT_NAME {
            return Err(ProfileError::Default);
        }
        if !self.has(name) {
            return Err(ProfileError::Missing);
        }
        let dir = profile_dir(root, name);
        let mut moved = None;
        if dir.is_dir() {
            let trash = root.join(PROFILES_DIR).join(TRASH_DIR);
            std::fs::create_dir_all(&trash).map_err(|e| ProfileError::Io(e.to_string()))?;
            let mut to = trash.join(format!("{}-{now}", folder_name(name)));
            let mut n = 1;
            while to.exists() {
                n += 1;
                to = trash.join(format!("{}-{now}-{n}", folder_name(name)));
            }
            std::fs::rename(&dir, &to).map_err(|e| ProfileError::Io(e.to_string()))?;
            moved = Some(to);
        }
        self.profiles.retain(|p| p.name != name);
        if self.last == name {
            self.last = DEFAULT_NAME.to_string();
        }
        Ok(moved)
    }

    /// Renames a profile and its folder. DEFAULT is refused, and so is a name already taken. Returns the new name.
    pub fn rename(&mut self, root: &Path, from: &str, typed: &str) -> Result<String, ProfileError> {
        if from == DEFAULT_NAME {
            return Err(ProfileError::Default);
        }
        let to = clean_name(typed).map_err(ProfileError::Name)?;
        if !self.has(from) {
            return Err(ProfileError::Missing);
        }
        if self.has(&to) {
            return Err(ProfileError::Exists);
        }
        let (old, new) = (profile_dir(root, from), profile_dir(root, &to));
        if new.exists() {
            return Err(ProfileError::Exists);
        }
        if old.is_dir() {
            std::fs::rename(&old, &new).map_err(|e| ProfileError::Io(e.to_string()))?;
        } else {
            std::fs::create_dir_all(&new).map_err(|e| ProfileError::Io(e.to_string()))?;
        }
        for p in self.profiles.iter_mut().filter(|p| p.name == from) {
            p.name = to.clone();
        }
        if self.last == from {
            self.last = to.clone();
        }
        Ok(to)
    }
}

// -- the instance lock ----------------------------------------------------------------------

/// `instance.lock`: the process running on a profile. Times are Unix seconds; `beat` is rewritten while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lock {
    pub pid: u32,
    pub started: u64,
    pub beat: u64,
}

impl Lock {
    pub fn to_text(&self) -> String {
        json!({ "pid": self.pid, "started": self.started, "beat": self.beat }).to_string()
    }

    pub fn parse(text: &str) -> Option<Lock> {
        let v: Value = serde_json::from_str(text).ok()?;
        let num = |k: &str| v.get(k).and_then(Value::as_u64);
        Some(Lock { pid: u32::try_from(num("pid")?).ok()?, started: num("started")?, beat: num("beat").or(num("started"))? })
    }

    /// A lock is stale when its process is known to be gone, or when it was last rewritten more than `stale_after`
    /// seconds ago. `pid_alive` is `None` where the system cannot say (then only the age counts).
    pub fn is_stale(&self, now: u64, stale_after: u64, pid_alive: Option<bool>) -> bool {
        pid_alive == Some(false) || now.saturating_sub(self.beat) > stale_after
    }
}

/// Whether process `pid` exists: read from `/proc` on Linux, unknown (`None`) elsewhere (std has no portable check).
pub fn pid_alive(pid: u32) -> Option<bool> {
    if cfg!(target_os = "linux") && Path::new("/proc/self").exists() {
        Some(Path::new("/proc").join(pid.to_string()).exists())
    } else {
        None
    }
}

/// Who holds a profile folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockState {
    /// No lock file, an unreadable one, or a stale one.
    Free,
    /// This process's own lock.
    Ours,
    /// Another running instance.
    InUse(Lock),
}

/// The lock in `dir`, if it has a readable one.
pub fn read_lock(dir: &Path) -> Option<Lock> {
    Lock::parse(&std::fs::read_to_string(dir.join(LOCK_FILE)).ok()?)
}

/// [`lock_state`] with the process check given (the tests give one).
pub fn lock_state_with(dir: &Path, my_pid: u32, now: u64, stale_after: u64, alive: impl Fn(u32) -> Option<bool>) -> LockState {
    match read_lock(dir) {
        None => LockState::Free,
        Some(lock) if lock.pid == my_pid => LockState::Ours,
        Some(lock) if lock.is_stale(now, stale_after, alive(lock.pid)) => LockState::Free,
        Some(lock) => LockState::InUse(lock),
    }
}

/// Who holds the profile folder `dir`, as process `my_pid` sees it at `now`.
pub fn lock_state(dir: &Path, my_pid: u32, now: u64, stale_after: u64) -> LockState {
    lock_state_with(dir, my_pid, now, stale_after, pid_alive)
}

/// Writes (or rewrites, as the heartbeat) this process's lock in `dir`.
pub fn write_lock(dir: &Path, pid: u32, started: u64, now: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(LOCK_FILE), Lock { pid, started, beat: now }.to_text())
}

/// Removes the lock in `dir` when it is this process's own; another instance's lock is left alone.
pub fn release_lock(dir: &Path, pid: u32) {
    if read_lock(dir).map_or(false, |l| l.pid == pid) {
        let _ = std::fs::remove_file(dir.join(LOCK_FILE));
    }
}

/// The command line for starting the game again: `raw` without the profile options (`--profile NAME`,
/// `--profile=NAME`, `--profiles`, `--profile-force`), then `extra`.
pub fn restart_args(raw: &[String], extra: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut skip_value = false;
    for a in raw {
        if skip_value {
            skip_value = false;
            if !a.starts_with("--") {
                continue;
            }
        }
        if a == "--profile" {
            skip_value = true;
            continue;
        }
        if a == "--profiles" || a == "--profile-force" || a.starts_with("--profile=") {
            continue;
        }
        out.push(a.clone());
    }
    out.extend(extra.iter().cloned());
    out
}

/// The profile a command line names: `--profile NAME` or `--profile=NAME` (the text as typed, not yet checked).
pub fn profile_arg(raw: &[String]) -> Option<String> {
    for (i, a) in raw.iter().enumerate() {
        if let Some(v) = a.strip_prefix("--profile=") {
            return Some(v.to_string());
        }
        if a == "--profile" {
            return Some(raw.get(i + 1).filter(|v| !v.starts_with("--")).cloned().unwrap_or_default());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_profiles_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_are_trimmed_upper_cased_and_checked() {
        assert_eq!(clean_name("  kyttu "), Ok("KYTTU".to_string()));
        assert_eq!(clean_name("player 2"), Ok("PLAYER 2".to_string()));
        assert_eq!(clean_name("a   b"), Ok("A B".to_string()));
        assert_eq!(clean_name("x-1_y"), Ok("X-1_Y".to_string()));
        assert_eq!(clean_name("A"), Ok("A".to_string()));
        assert_eq!(clean_name("ABCDEFGHIJKLMNOP"), Ok("ABCDEFGHIJKLMNOP".to_string()));
        assert_eq!(clean_name("ABCDEFGHIJKLMNOPQ"), Err(NameError::TooLong));
        assert_eq!(clean_name(""), Err(NameError::Empty));
        assert_eq!(clean_name("   "), Err(NameError::Empty));
        for bad in ["a/b", "..", "a\\b", "a.b", "é", "a:b", "a*b", "a\u{0}b", "~", "a+b"] {
            assert_eq!(clean_name(bad), Err(NameError::BadChar), "{bad:?}");
        }
        for reserved in ["con", "NUL", "com1", "LPT9", "aux", "prn"] {
            assert_eq!(clean_name(reserved), Err(NameError::Reserved), "{reserved}");
        }
        // only the device names themselves are kept back
        assert_eq!(clean_name("COM10"), Ok("COM10".to_string()));
        assert_eq!(clean_name("CONSOLE"), Ok("CONSOLE".to_string()));
    }

    #[test]
    fn a_name_maps_to_a_safe_folder_and_default_maps_to_the_root() {
        let root = Path::new("ROOT");
        assert_eq!(profile_dir(root, DEFAULT_NAME), PathBuf::from("ROOT"));
        assert_eq!(profile_dir(root, "KYTTU"), root.join("profiles").join("KYTTU"));
        assert_eq!(profile_dir(root, "PLAYER 2"), root.join("profiles").join("PLAYER+2"));
        assert_ne!(profile_dir(root, "A B"), profile_dir(root, "A_B"));
        assert_ne!(profile_dir(root, "A B"), profile_dir(root, "A-B"));
        assert_eq!(picker_dir(root), root.join("profiles").join(".picker"));
        for name in ["A", "PLAYER 2", "X-1_Y", "ABCDEFGHIJKLMNOP"] {
            let folder = folder_name(&clean_name(name).unwrap());
            assert!(folder.chars().all(|c| c.is_ascii_alphanumeric() || "-_+".contains(c)), "{folder}");
            assert!(!folder.starts_with('.'), "{folder} cannot be the trash or picker folder");
        }
    }

    #[test]
    fn the_index_round_trips() {
        let mut index = Index::default();
        let root = temp("round_trip");
        index.add(&root, "kyttu", 100).unwrap();
        index.add(&root, "player 2", 200).unwrap();
        index.touch("PLAYER 2", 300);
        let again = Index::parse(&index.to_text());
        assert_eq!(again, index);
        assert_eq!(again.last, "PLAYER 2");
        assert_eq!(again.get("PLAYER 2"), Some(&Profile { name: "PLAYER 2".into(), created: 200, last_used: 300 }));
        index.save(&root).unwrap();
        assert_eq!(Index::load(&root), index);
        assert!(root.join(INDEX_FILE).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_or_damaged_index_is_default_alone() {
        let root = temp("missing");
        assert_eq!(Index::load(&root), Index::default());
        assert_eq!(Index::parse("not json"), Index::default());
        assert_eq!(Index::parse("{}"), Index::default());
        assert_eq!(Index::default().profiles.len(), 1);
        assert_eq!(Index::default().last, DEFAULT_NAME);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_drops_bad_names_and_repeats_and_keeps_default_first() {
        let text = r#"{"last":"GONE","profiles":[
            {"name":"B","created":5,"last_used":6},
            {"name":"../evil","created":1},
            {"name":"b","created":9},
            {"name":"DEFAULT","created":2,"last_used":3},
            {"created":4}, 7
        ]}"#;
        let index = Index::parse(text);
        let names: Vec<&str> = index.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["DEFAULT", "B"]);
        assert_eq!(index.profiles[0], Profile { name: "DEFAULT".into(), created: 2, last_used: 3 });
        assert_eq!(index.profiles[1].created, 5);
        // a last profile that is not in the list falls back to DEFAULT
        assert_eq!(index.last, DEFAULT_NAME);
    }

    #[test]
    fn add_creates_the_folder_and_refuses_a_repeat_or_a_bad_name() {
        let root = temp("add");
        let mut index = Index::default();
        assert_eq!(index.add(&root, " two ", 10), Ok("TWO".to_string()));
        assert!(root.join("profiles").join("TWO").is_dir());
        assert_eq!(index.add(&root, "two", 11), Err(ProfileError::Exists));
        assert_eq!(index.add(&root, "default", 11), Err(ProfileError::Exists));
        assert_eq!(index.add(&root, "a/b", 11), Err(ProfileError::Name(NameError::BadChar)));
        assert_eq!(index.profiles.len(), 2);
        assert_eq!(index.get("TWO").unwrap().last_used, 0);
        // ensure: an old name is kept, a new one is added
        assert_eq!(index.ensure(&root, "TWO", 12), Ok("TWO".to_string()));
        assert_eq!(index.get("TWO").unwrap().created, 10);
        assert_eq!(index.ensure(&root, "three", 13), Ok("THREE".to_string()));
        assert_eq!(index.get("THREE").unwrap().created, 13);
        assert_eq!(index.ensure(&root, "DEFAULT", 14), Ok("DEFAULT".to_string()));
        assert_eq!(index.profiles.len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn touch_sets_last_used_and_the_last_profile() {
        let root = temp("touch");
        let mut index = Index::default();
        index.add(&root, "TWO", 10).unwrap();
        assert!(index.touch("TWO", 50));
        assert_eq!(index.last, "TWO");
        assert_eq!(index.get("TWO").unwrap().last_used, 50);
        assert!(index.touch(DEFAULT_NAME, 60));
        assert_eq!(index.last, DEFAULT_NAME);
        assert_eq!(index.get("TWO").unwrap().last_used, 50);
        assert!(!index.touch("NOBODY", 70));
        assert_eq!(index.last, DEFAULT_NAME);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_moves_the_folder_to_the_trash_and_refuses_default() {
        let root = temp("delete");
        std::fs::write(root.join("settings.json"), "{}").unwrap();
        let mut index = Index::default();
        index.add(&root, "TWO", 10).unwrap();
        index.touch("TWO", 20);
        let dir = profile_dir(&root, "TWO");
        std::fs::write(dir.join("checkpoint.json"), "save").unwrap();
        assert_eq!(index.remove(&root, DEFAULT_NAME, 30), Err(ProfileError::Default));
        assert_eq!(index.remove(&root, "NOBODY", 30), Err(ProfileError::Missing));
        let moved = index.remove(&root, "TWO", 30).unwrap().expect("the folder moved");
        assert_eq!(moved, root.join("profiles").join(".trash").join("TWO-30"));
        assert_eq!(std::fs::read_to_string(moved.join("checkpoint.json")).unwrap(), "save");
        assert!(!dir.exists());
        assert!(!index.has("TWO"));
        assert_eq!(index.last, DEFAULT_NAME);
        // the root (DEFAULT) files are untouched
        assert!(root.join("settings.json").is_file());
        // the same name deleted again in the same second gets its own trash folder
        index.add(&root, "TWO", 30).unwrap();
        let again = index.remove(&root, "TWO", 30).unwrap().unwrap();
        assert_eq!(again, root.join("profiles").join(".trash").join("TWO-30-2"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_moves_the_folder_and_keeps_the_times() {
        let root = temp("rename");
        let mut index = Index::default();
        index.add(&root, "TWO", 10).unwrap();
        index.add(&root, "THREE", 11).unwrap();
        index.touch("TWO", 20);
        std::fs::write(profile_dir(&root, "TWO").join("settings.json"), "mine").unwrap();
        assert_eq!(index.rename(&root, DEFAULT_NAME, "X"), Err(ProfileError::Default));
        assert_eq!(index.rename(&root, "TWO", "three"), Err(ProfileError::Exists));
        assert_eq!(index.rename(&root, "TWO", "default"), Err(ProfileError::Exists));
        assert_eq!(index.rename(&root, "TWO", "a/b"), Err(ProfileError::Name(NameError::BadChar)));
        assert_eq!(index.rename(&root, "NOBODY", "X"), Err(ProfileError::Missing));
        assert_eq!(index.rename(&root, "TWO", "second one"), Ok("SECOND ONE".to_string()));
        assert!(!index.has("TWO"));
        assert_eq!(index.get("SECOND ONE"), Some(&Profile { name: "SECOND ONE".into(), created: 10, last_used: 20 }));
        assert_eq!(index.last, "SECOND ONE");
        assert_eq!(std::fs::read_to_string(profile_dir(&root, "SECOND ONE").join("settings.json")).unwrap(), "mine");
        assert!(!profile_dir(&root, "TWO").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_profiles_never_share_a_file() {
        let root = Path::new("ROOT");
        let names = ["DEFAULT", "A", "A B", "A_B", "A-B", "B"];
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                let (da, db) = (profile_dir(root, a), profile_dir(root, b));
                assert_ne!(da.join("settings.json"), db.join("settings.json"), "{a} and {b}");
                // DEFAULT's own files sit beside `profiles/`, never inside another profile's folder
                assert_ne!(da.join(LOCK_FILE), db.join(LOCK_FILE));
            }
        }
    }

    #[test]
    fn the_lock_text_round_trips_and_bad_text_is_no_lock() {
        let lock = Lock { pid: 4242, started: 100, beat: 160 };
        assert_eq!(Lock::parse(&lock.to_text()), Some(lock));
        assert_eq!(Lock::parse(r#"{"pid":7,"started":5}"#), Some(Lock { pid: 7, started: 5, beat: 5 }));
        assert_eq!(Lock::parse(""), None);
        assert_eq!(Lock::parse("{}"), None);
        assert_eq!(Lock::parse(r#"{"pid":-1,"started":5}"#), None);
    }

    #[test]
    fn a_lock_is_stale_when_its_process_is_gone_or_it_is_too_old() {
        let lock = Lock { pid: 9, started: 100, beat: 200 };
        // fresh, process unknown or alive: held
        assert!(!lock.is_stale(210, 20, None));
        assert!(!lock.is_stale(220, 20, Some(true)));
        // the process is gone: stale at once
        assert!(lock.is_stale(201, 20, Some(false)));
        // older than the age: stale even when a process with that pid exists (the pid was reused)
        assert!(lock.is_stale(221, 20, Some(true)));
        assert!(lock.is_stale(221, 20, None));
        // a clock set back does not make it stale
        assert!(!lock.is_stale(50, 20, None));
    }

    #[test]
    fn the_lock_file_says_who_holds_the_folder() {
        let dir = temp("lock");
        let alive = |_: u32| Some(true);
        assert_eq!(lock_state_with(&dir, 1, 100, 20, alive), LockState::Free);
        write_lock(&dir, 1, 90, 100).unwrap();
        assert_eq!(lock_state_with(&dir, 1, 105, 20, alive), LockState::Ours);
        assert_eq!(
            lock_state_with(&dir, 2, 105, 20, alive),
            LockState::InUse(Lock { pid: 1, started: 90, beat: 100 })
        );
        // the heartbeat keeps it held
        write_lock(&dir, 1, 90, 130).unwrap();
        assert!(matches!(lock_state_with(&dir, 2, 140, 20, alive), LockState::InUse(_)));
        // no heartbeat for longer than the age, or a dead process: free
        assert_eq!(lock_state_with(&dir, 2, 151, 20, alive), LockState::Free);
        assert_eq!(lock_state_with(&dir, 2, 131, 20, |_| Some(false)), LockState::Free);
        // another instance cannot release it; the owner can
        release_lock(&dir, 2);
        assert!(dir.join(LOCK_FILE).is_file());
        release_lock(&dir, 1);
        assert!(!dir.join(LOCK_FILE).exists());
        // an unreadable lock is no lock
        std::fs::write(dir.join(LOCK_FILE), "garbage").unwrap();
        assert_eq!(lock_state_with(&dir, 2, 100, 20, alive), LockState::Free);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn this_process_is_alive_where_the_system_can_say() {
        assert_ne!(pid_alive(std::process::id()), Some(false));
    }

    #[test]
    fn the_profile_option_is_read_in_both_spellings() {
        assert_eq!(profile_arg(&strings(&["--profile", "two"])), Some("two".to_string()));
        assert_eq!(profile_arg(&strings(&["--offline", "--profile=two"])), Some("two".to_string()));
        assert_eq!(profile_arg(&strings(&["--profile", "--offline"])), Some(String::new()));
        assert_eq!(profile_arg(&strings(&["--profile"])), Some(String::new()));
        assert_eq!(profile_arg(&strings(&["--profiles", "--offline"])), None);
    }

    #[test]
    fn a_restart_drops_the_old_profile_options_and_keeps_the_rest() {
        let raw = strings(&["--offline", "--profile", "ONE", "--profiles", "--log-level", "debug", "--profile=X", "--profile-force"]);
        assert_eq!(
            restart_args(&raw, &strings(&["--profile", "TWO"])),
            strings(&["--offline", "--log-level", "debug", "--profile", "TWO"])
        );
        assert_eq!(restart_args(&strings(&["--profile", "--offline"]), &[]), strings(&["--offline"]));
        assert_eq!(restart_args(&[], &strings(&["--profiles"])), strings(&["--profiles"]));
    }
}
