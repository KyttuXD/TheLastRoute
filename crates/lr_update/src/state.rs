//! What the launcher remembers between runs, in `state.json` beside the `builds/` folder:
//!
//! * `current`: the build to start; `previous`: the build to roll back to (kept on disk).
//! * `bad`: builds that crashed at start repeatedly; they are never installed again (a higher build is).
//! * `floor`: the highest build ever installed. Anything lower is refused (a replayed old pointer cannot downgrade).
//! * `failures`: how many launches of `current` in a row ended badly (reset by a good launch or a new current).
//! * `launch`: the record written just before the game is started.
//!
//! The file is replaced by writing a temp file and renaming it over the old one (atomic on every system; Rust's rename
//! replaces an existing file on Windows too). A damaged or missing file is rebuilt from the `builds/` folder.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    pub current: Option<u64>,
    pub previous: Option<u64>,
    pub bad: Vec<u64>,
    pub floor: u64,
    pub failures: u32,
    pub launch: Option<Launch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub build: u64,
    pub started: u64,
    /// "pending", "ok" or "failed".
    pub result: String,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl State {
    pub fn is_bad(&self, build: u64) -> bool {
        self.bad.contains(&build)
    }

    /// Records `build` as the one to run; the old current becomes the rollback target.
    pub fn adopt(&mut self, build: u64) {
        if self.current != Some(build) {
            if let Some(old) = self.current {
                if old != build && !self.is_bad(old) {
                    self.previous = Some(old);
                }
            }
            self.current = Some(build);
            self.failures = 0;
        }
        if self.previous == Some(build) {
            self.previous = None;
        }
        self.floor = self.floor.max(build);
    }

    /// Marks the current build bad and moves to the rollback target. Returns the build rolled back to, if any.
    pub fn roll_back(&mut self) -> Option<u64> {
        if let Some(bad) = self.current.take() {
            if !self.bad.contains(&bad) {
                self.bad.push(bad);
            }
        }
        self.failures = 0;
        match self.previous.take() {
            Some(p) if !self.is_bad(p) => {
                self.current = Some(p);
                Some(p)
            }
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "schema": "lr.native.state/1",
            "current": self.current,
            "previous": self.previous,
            "bad": self.bad,
            "floor": self.floor,
            "failures": self.failures,
            "launch": self.launch.as_ref().map(|l| json!({"build": l.build, "started": l.started, "result": l.result})),
        })
    }

    pub fn from_json(text: &str) -> Option<State> {
        let v: Value = serde_json::from_str(text).ok()?;
        let num = |k: &str| v.get(k).and_then(Value::as_u64);
        let launch = v.get("launch").filter(|l| l.is_object()).and_then(|l| {
            Some(Launch { build: l.get("build")?.as_u64()?, started: l.get("started")?.as_u64()?, result: l.get("result")?.as_str()?.to_string() })
        });
        Some(State {
            current: num("current"),
            previous: num("previous"),
            bad: v.get("bad").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default(),
            floor: num("floor").unwrap_or(0),
            failures: num("failures").unwrap_or(0).min(1000) as u32,
            launch,
        })
    }
}

pub fn builds_dir(dir: &Path) -> PathBuf {
    dir.join("builds")
}

pub fn build_dir(dir: &Path, build: u64) -> PathBuf {
    builds_dir(dir).join(build.to_string())
}

pub fn tmp_dir(dir: &Path) -> PathBuf {
    dir.join("tmp")
}

pub const MANIFEST_COPY: &str = ".manifest.json";
pub const SIGNATURE_COPY: &str = ".manifest.json.sig";

/// Build numbers present in `builds/` with a manifest copy, highest first.
pub fn builds_on_disk(dir: &Path) -> Vec<u64> {
    let mut found: Vec<u64> = std::fs::read_dir(builds_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n: u64 = e.file_name().to_str()?.parse().ok()?;
            e.path().join(MANIFEST_COPY).is_file().then_some(n)
        })
        .collect();
    found.sort_unstable_by(|a, b| b.cmp(a));
    found
}

/// Reads the state. A missing file means a first run (rebuilt from `builds/` when builds exist); a damaged one is
/// rebuilt the same way, and the second element says so.
pub fn load(dir: &Path) -> (State, bool) {
    let text = std::fs::read_to_string(dir.join("state.json"));
    match text {
        Ok(text) => match State::from_json(&text) {
            Some(state) => (state, false),
            None => (rebuild(dir), true),
        },
        Err(_) => (rebuild(dir), false),
    }
}

fn rebuild(dir: &Path) -> State {
    let found = builds_on_disk(dir);
    let mut state = State::default();
    if let Some(&top) = found.first() {
        state.current = Some(top);
        state.floor = top;
        state.previous = found.get(1).copied();
    }
    state
}

/// Writes the state atomically.
pub fn save(dir: &Path, state: &State) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // The temporary name carries the process id: two launchers saving at the same moment never write the same file.
    let tmp = dir.join(format!("state.json.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(&state.to_json()).unwrap_or_default();
    std::fs::write(&tmp, text)?;
    let renamed = std::fs::rename(&tmp, dir.join("state.json"));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    renamed
}

/// Removes everything except the builds named in `keep`, and the leftovers of interrupted downloads in `tmp/`.
/// Errors are ignored (Windows may still hold a file of a build that is running); the next start tries again.
pub fn collect_garbage(dir: &Path, keep: &[u64]) {
    if let Ok(entries) = std::fs::read_dir(builds_dir(dir)) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let keep_it = name.parse::<u64>().is_ok_and(|n| keep.contains(&n));
            if !keep_it {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    // tmp/ holds another launcher's download while it installs: it is cleared only when nobody holds the install lock
    // (the installer itself calls this with its own lock held, and has already removed its own leftovers).
    if let Some(_lock) = InstallLock::acquire(dir, 1800) {
        remove_tmp(dir);
    }
}

pub fn remove_tmp(dir: &Path) {
    let _ = std::fs::remove_dir_all(tmp_dir(dir));
}

/// A lock file that keeps two launchers from installing at once. Stale after `stale_secs` (a launcher that died).
pub struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    pub fn acquire(dir: &Path, stale_secs: u64) -> Option<InstallLock> {
        let path = dir.join("install.lock");
        let _ = std::fs::create_dir_all(dir);
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = write!(f, "{} {}", std::process::id(), now_unix());
                    return Some(InstallLock { path });
                }
                Err(_) => {
                    let age = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if age > stale_secs {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    return None;
                }
            }
        }
        None
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lr-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn adopt_keeps_the_old_build_as_previous_and_resets_failures() {
        let mut s = State::default();
        s.adopt(46);
        s.failures = 1;
        s.adopt(47);
        assert_eq!((s.current, s.previous, s.failures, s.floor), (Some(47), Some(46), 0, 47));
        s.adopt(47);
        assert_eq!(s.previous, Some(46), "adopting the same build again changes nothing");
    }

    #[test]
    fn roll_back_marks_bad_and_returns_the_previous_build() {
        let mut s = State::default();
        s.adopt(46);
        s.adopt(47);
        assert_eq!(s.roll_back(), Some(46));
        assert_eq!((s.current, s.previous, s.bad.clone()), (Some(46), None, vec![47]));
        assert_eq!(s.roll_back(), None, "nothing left to go back to");
        assert_eq!((s.current, s.bad), (None, vec![47, 46]));
    }

    #[test]
    fn json_round_trips() {
        let mut s = State::default();
        s.adopt(3);
        s.adopt(4);
        s.bad.push(2);
        s.launch = Some(Launch { build: 4, started: 99, result: "pending".into() });
        assert_eq!(State::from_json(&s.to_json().to_string()), Some(s));
        assert_eq!(State::from_json("{ not json"), None);
    }

    #[test]
    fn save_replaces_and_load_reads_back() {
        let dir = temp("save");
        let mut s = State::default();
        s.adopt(5);
        save(&dir, &s).unwrap();
        s.adopt(6);
        save(&dir, &s).unwrap();
        let leftovers = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).count();
        assert_eq!(leftovers, 0, "no temporary file is left behind");
        assert_eq!(load(&dir), (s, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_state_is_rebuilt_from_the_builds_on_disk() {
        let dir = temp("rebuild");
        for n in [8u64, 9] {
            let b = build_dir(&dir, n);
            std::fs::create_dir_all(&b).unwrap();
            std::fs::write(b.join(MANIFEST_COPY), "{}").unwrap();
        }
        std::fs::create_dir_all(build_dir(&dir, 10)).unwrap(); // no manifest copy: not a finished install
        std::fs::write(dir.join("state.json"), "garbage").unwrap();
        let (s, damaged) = load(&dir);
        assert!(damaged);
        assert_eq!((s.current, s.previous, s.floor), (Some(9), Some(8), 9));
        std::fs::remove_file(dir.join("state.json")).unwrap();
        let (s, damaged) = load(&dir);
        assert!(!damaged && s.current == Some(9));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_collection_keeps_the_named_builds_and_drops_tmp() {
        let dir = temp("gc");
        for n in [1u64, 2, 3] {
            std::fs::create_dir_all(build_dir(&dir, n)).unwrap();
        }
        std::fs::create_dir_all(tmp_dir(&dir).join("4.partial")).unwrap();
        collect_garbage(&dir, &[2, 3]);
        assert!(!build_dir(&dir, 1).exists() && build_dir(&dir, 2).exists() && build_dir(&dir, 3).exists());
        assert!(!tmp_dir(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_collection_leaves_tmp_alone_while_another_launcher_installs() {
        let dir = temp("gc-locked");
        std::fs::create_dir_all(tmp_dir(&dir).join("4.partial")).unwrap();
        let lock = InstallLock::acquire(&dir, 600).expect("lock");
        collect_garbage(&dir, &[]);
        assert!(tmp_dir(&dir).join("4.partial").exists(), "another launcher's download is not deleted");
        drop(lock);
        collect_garbage(&dir, &[]);
        assert!(!tmp_dir(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_install_lock_is_exclusive_and_released() {
        let dir = temp("lock");
        let a = InstallLock::acquire(&dir, 600).expect("first lock");
        assert!(InstallLock::acquire(&dir, 600).is_none(), "second lock while held");
        drop(a);
        assert!(InstallLock::acquire(&dir, 600).is_some(), "free again after drop");
        // a stale lock (age over the limit) is taken over
        drop(InstallLock::acquire(&dir, 600).unwrap());
        std::fs::write(dir.join("install.lock"), "999 1").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::options().write(true).open(dir.join("install.lock")).unwrap().set_modified(old).unwrap();
        assert!(InstallLock::acquire(&dir, 600).is_some(), "a lock older than the limit is taken over");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
