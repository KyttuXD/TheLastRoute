//! The cloud copy of the one-run autosave, from `game/autoload/run_saves.gd` (`cloud_sync`, `_on_cloud_response`,
//! `adopt_cloud`, `pending_sync`, `_mark_unsynced`).
//!
//! * A local save marks the cloud as owed: `<save file>.unsynced` holds `push` (upload) or `clear` (the run ended).
//! * `cloud_sync()`: if something is owed it is sent (`PUT {data, updatedAt}`; `data: null` clears); otherwise the
//!   account's copy is pulled (`GET`) and adopted only when it is newer (`saved_at`) and no clear is owed.
//! * A 2xx answer settles the debt; `409 STALE` on a push drops our claim and pulls instead (another machine holds a
//!   newer run); anything else (offline, 401, 404, 413, 429, 5xx) stays owed for the next save or title visit.
//!
//! The data is whatever JSON object the game stores; it must carry `saved_at` (unix seconds) like the original.

use super::api::{self, CloudPut, Json};
use super::{BackendConfig, NetError, Pending};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owed {
    None,
    Push,
    Clear,
}

/// What one sync did.
#[derive(Clone, Debug, PartialEq)]
pub enum CloudOutcome {
    Pushed { updated_at: i64 },
    Cleared,
    /// The account's run is newer than ours: replace the local run with it (`adopt_cloud`).
    Adopt(Json),
    /// Nothing to do (in sync, or the cloud copy is not newer).
    UpToDate,
    /// The request failed; the debt stays owed.
    Deferred(String),
}

/// One save file and its cloud slot.
#[derive(Clone, Debug)]
pub struct CloudSaves {
    pub save_file: PathBuf,
    /// `api::cloud_path(world_revision)`.
    pub cloud_path: String,
}

pub fn saved_at(record: &Json) -> i64 {
    record.get("saved_at").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}

impl CloudSaves {
    pub fn new(save_file: &Path, world_revision: i64) -> CloudSaves {
        CloudSaves { save_file: save_file.to_path_buf(), cloud_path: api::cloud_path(world_revision) }
    }

    fn marker(&self) -> PathBuf {
        let mut name = self.save_file.as_os_str().to_owned();
        name.push(".unsynced");
        PathBuf::from(name)
    }

    /// `pending_sync()`.
    pub fn pending(&self) -> Owed {
        match std::fs::read_to_string(self.marker()) {
            Err(_) => Owed::None,
            Ok(text) => match text.trim() {
                "clear" => Owed::Clear,
                _ => Owed::Push,
            },
        }
    }

    /// `_mark_unsynced(owed)`: call with `Push` after every local save and `Clear` when the run ended.
    pub fn mark(&self, owed: Owed) {
        if !crate::shared::paths::WRITES_TO_DISK {
            return;
        }
        let marker = self.marker();
        match owed {
            Owed::None => {
                let _ = std::fs::remove_file(marker);
            }
            Owed::Push | Owed::Clear => {
                if let Some(parent) = marker.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(marker, if owed == Owed::Push { "push" } else { "clear" });
            }
        }
    }

    /// `cloud_sync()` + `_on_cloud_response` as one blocking call. `local` is the current local record (None when
    /// there is no run). The caller writes an `Adopt` record itself (the original's `_write_record`).
    pub fn sync_blocking(&self, cfg: &BackendConfig, token: &str, local: Option<&Json>) -> CloudOutcome {
        if token.is_empty() || cfg.offline {
            return CloudOutcome::Deferred("NOT SIGNED IN".into());
        }
        let now = super::unix_now();
        let owed = self.pending();
        let mut pull = owed == Owed::None;
        match owed {
            Owed::Clear => match api::cloud_put(cfg, &self.cloud_path, token, None, now) {
                Ok(CloudPut::Stored { .. }) => {
                    self.mark(Owed::None);
                    return CloudOutcome::Cleared;
                }
                Ok(CloudPut::Stale { .. }) => return CloudOutcome::Deferred("STALE".into()),
                Err(e) => return CloudOutcome::Deferred(e.to_string()),
            },
            Owed::Push => match local {
                None => self.mark(Owed::None),
                Some(record) => match api::cloud_put(cfg, &self.cloud_path, token, Some(record), saved_at(record)) {
                    Ok(CloudPut::Stored { updated_at }) => {
                        self.mark(Owed::None);
                        return CloudOutcome::Pushed { updated_at };
                    }
                    Ok(CloudPut::Stale { .. }) => {
                        // the account holds a newer run (another machine): drop our claim and let the pull decide
                        self.mark(Owed::None);
                        pull = true;
                    }
                    Err(e) => return CloudOutcome::Deferred(e.to_string()),
                },
            },
            Owed::None => {}
        }
        if !pull {
            return CloudOutcome::UpToDate;
        }
        match api::cloud_get(cfg, &self.cloud_path, token) {
            Ok(record) => match record.data {
                Some(data) if self.should_adopt(&data, local) => CloudOutcome::Adopt(data),
                _ => CloudOutcome::UpToDate,
            },
            Err(e) => CloudOutcome::Deferred(e.to_string()),
        }
    }

    /// `adopt_cloud`: only a newer record, and never while a clear is owed.
    pub fn should_adopt(&self, cloud: &Json, local: Option<&Json>) -> bool {
        if self.pending() == Owed::Clear {
            return false;
        }
        match local {
            Some(local) => saved_at(local) < saved_at(cloud),
            None => true,
        }
    }

    /// Non-blocking [`CloudSaves::sync_blocking`] for the frame loop.
    pub fn sync(&self, cfg: &BackendConfig, token: &str, local: Option<Json>) -> Pending<CloudOutcome> {
        let (this, cfg, token) = (self.clone(), cfg.clone(), token.to_string());
        Pending::spawn(move || -> Result<CloudOutcome, NetError> { Ok(this.sync_blocking(&cfg, &token, local.as_ref())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owed_marker_round_trip_and_adoption_rule() {
        let dir = std::env::temp_dir().join(format!("lr_net_saves_{}", crate::shared::task::process_id()));
        let _ = std::fs::create_dir_all(&dir);
        let saves = CloudSaves::new(&dir.join("run.json"), 1);
        assert_eq!(saves.cloud_path, "/api/v1/saves/last-route-rust/run-world-1");
        assert_eq!(saves.pending(), Owed::None);
        saves.mark(Owed::Push);
        assert_eq!(saves.pending(), Owed::Push);
        assert!(dir.join("run.json.unsynced").exists());
        let rec = |t: i64| serde_json::json!({ "saved_at": t }).as_object().unwrap().clone();
        assert!(saves.should_adopt(&rec(10), Some(&rec(5))));
        assert!(!saves.should_adopt(&rec(5), Some(&rec(5))));
        saves.mark(Owed::Clear);
        assert!(!saves.should_adopt(&rec(10), None), "never while a clear is owed");
        saves.mark(Owed::None);
        assert_eq!(saves.pending(), Owed::None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
