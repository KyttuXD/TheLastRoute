//! The story server's room API as the start menu uses it: who the player is (a guest name or an account), the room the
//! player is in, and the pack fingerprint that must match before a player joins a room.
//!
//! The server (src/bin/lr_story_server) is the authority: it reads the packs itself, decides which are multiplayer, and
//! checks every call. This file builds the requests and reads the replies. Nothing here decides a rule.

use crate::assets_vfs;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The player's identity on the server: a guest name with the lease token that proves it, or a signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Guest { name: String, token: String },
    Account { username: String, session: String },
}

impl Who {
    /// The name the room shows for this player.
    pub fn name(&self) -> &str {
        match self {
            Who::Guest { name, .. } => name,
            Who::Account { username, .. } => username,
        }
    }

    /// The name with what kind of identity it is, for the menu.
    pub fn label(&self) -> String {
        match self {
            Who::Guest { name, .. } => format!("{name} (guest)"),
            Who::Account { username, .. } => format!("{username} (account)"),
        }
    }

    /// The identity fields a create or join call carries (`guestName` and `guestToken`, or `sessionToken`).
    pub fn fields(&self) -> Value {
        match self {
            Who::Guest { name, token } => json!({ "guestName": name, "guestToken": token }),
            Who::Account { session, .. } => json!({ "sessionToken": session }),
        }
    }
}

/// One player in a room, as the server shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerView {
    pub name: String,
    pub ready: bool,
    pub host: bool,
    pub connected: bool,
}

/// A room, as the server's room view gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomView {
    pub code: String,
    /// The story (pack) id the room plays.
    pub pack: String,
    /// The fingerprint of that pack on the server (`lr_story pack` prints the same one).
    pub fingerprint: String,
    pub host: String,
    pub max_players: usize,
    pub players: Vec<PlayerView>,
    /// True once every player is ready.
    pub started: bool,
}

impl RoomView {
    /// The room view object itself. Replies carry it under `room` (create, join, ready, pass-host, and GET /api/rooms/<code>).
    pub fn from_json(v: &Value) -> Option<RoomView> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
        let players = v
            .get("players")?
            .as_array()?
            .iter()
            .map(|p| PlayerView {
                name: p.get("name").and_then(Value::as_str).unwrap_or("?").to_string(),
                ready: p.get("ready") == Some(&Value::Bool(true)),
                host: p.get("host") == Some(&Value::Bool(true)),
                connected: p.get("connected") != Some(&Value::Bool(false)),
            })
            .collect();
        Some(RoomView {
            code: text("code")?,
            pack: text("pack")?,
            fingerprint: text("packFingerprint").unwrap_or_default(),
            host: text("host").unwrap_or_default(),
            max_players: v.get("maxPlayers").and_then(Value::as_u64).unwrap_or(6) as usize,
            players,
            started: v.get("started") == Some(&Value::Bool(true)),
        })
    }

    /// The player entry for `name`, if that player is in the room.
    pub fn player(&self, name: &str) -> Option<&PlayerView> {
        self.players.iter().find(|p| p.name == name)
    }
}

/// A room the player is in: its code, the player's token (which only that player holds) and the room as last seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub code: String,
    pub token: String,
    pub room: RoomView,
}

/// The seat a create or join reply gives: `{ok, code, playerToken, room}`.
pub fn seat_from(reply: &Value) -> Option<Seat> {
    let room = RoomView::from_json(reply.get("room")?)?;
    Some(Seat {
        code: reply.get("code").and_then(Value::as_str)?.to_string(),
        token: reply.get("playerToken").and_then(Value::as_str)?.to_string(),
        room,
    })
}

/// The fingerprint of a pack folder: a sha256 over every file's relative path, a NUL, its bytes and a NUL, files
/// sorted by path. The same algorithm as `lr_story pack` and the story server. Reads through the asset store, so it
/// works in the browser too.
pub fn fingerprint(dir: &Path) -> Result<String, String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for (rel, path) in &files {
        let data = assets_vfs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update(&data);
        hasher.update([0u8]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let entries = assets_vfs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for path in entries {
        if assets_vfs::is_file(&path) {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push((parts.join("/"), path));
        } else {
            collect(root, &path, out)?;
        }
    }
    Ok(())
}

/// The longest room code the menu accepts.
pub const MAX_CODE_LEN: usize = 8;

/// A typed or pasted room code made safe to send: trimmed, only letters and digits, uppercased, at most `MAX_CODE_LEN`.
pub fn clean_room_code(raw: &str) -> String {
    raw.trim().chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).take(MAX_CODE_LEN).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_code_is_trimmed_uppercased_and_clamped() {
        assert_eq!(clean_room_code("  ab12cd \n"), "AB12CD");
        assert_eq!(clean_room_code("ab-12 cd"), "AB12CD");
        assert_eq!(clean_room_code("abcdefghijkl"), "ABCDEFGH");
        assert_eq!(clean_room_code("   "), "");
    }

    #[test]
    fn a_room_view_reads_its_players_and_flags() {
        let view = json!({
            "code": "ABCDE", "pack": "simulora", "packFingerprint": "ff00", "host": "Ada", "maxPlayers": 6, "started": false,
            "players": [
                { "name": "Ada", "ready": true, "host": true, "connected": true },
                { "name": "Bo", "ready": false, "host": false, "connected": false }
            ]
        });
        let room = RoomView::from_json(&view).unwrap();
        assert_eq!(room.code, "ABCDE");
        assert_eq!(room.fingerprint, "ff00");
        assert_eq!(room.players.len(), 2);
        assert!(room.player("Ada").unwrap().ready && room.player("Ada").unwrap().host);
        assert!(!room.player("Bo").unwrap().connected);
        assert!(room.player("Cy").is_none());
        assert!(RoomView::from_json(&json!({ "code": "ABCDE" })).is_none(), "a view needs its players and pack");
    }

    #[test]
    fn a_seat_comes_from_a_create_or_join_reply() {
        let reply = json!({ "ok": true, "code": "KQ7M2", "playerToken": "tok", "room": {
            "code": "KQ7M2", "pack": "lastroute", "packFingerprint": "aa", "host": "Dia", "players": [] } });
        let seat = seat_from(&reply).unwrap();
        assert_eq!((seat.code.as_str(), seat.token.as_str(), seat.room.pack.as_str()), ("KQ7M2", "tok", "lastroute"));
    }

    #[test]
    fn identity_fields_match_what_the_server_reads() {
        let guest = Who::Guest { name: "W-7948".into(), token: "t".into() };
        assert_eq!(guest.fields(), json!({ "guestName": "W-7948", "guestToken": "t" }));
        let account = Who::Account { username: "ADA".into(), session: "s".into() };
        assert_eq!(account.fields(), json!({ "sessionToken": "s" }));
        assert_eq!(account.name(), "ADA");
        assert_eq!(guest.label(), "W-7948 (guest)");
    }

    #[test]
    fn a_guest_call_always_carries_its_lease_token_and_an_account_call_never_does() {
        let guest = Who::Guest { name: "W-1".into(), token: "lease".into() };
        let f = guest.fields();
        assert_eq!(f.get("guestToken"), Some(&json!("lease")), "the server requires the guest lease token");
        assert!(f.get("sessionToken").is_none());
        let account = Who::Account { username: "ADA".into(), session: "s".into() };
        let f = account.fields();
        assert!(f.get("guestToken").is_none() && f.get("guestName").is_none(), "an account sends only its session");
    }

    #[test]
    fn a_room_view_fills_defaults_and_a_seat_needs_its_token() {
        let room = RoomView::from_json(&json!({ "code": "C", "pack": "p", "players": [{ "name": "A" }] })).unwrap();
        assert_eq!(room.max_players, 6, "no maxPlayers means six");
        assert!(room.fingerprint.is_empty() && !room.started);
        let p = room.player("A").unwrap();
        assert!(p.connected && !p.ready && !p.host, "a missing connected flag reads as connected");
        let no_token = json!({ "code": "C", "room": { "code": "C", "pack": "p", "players": [] } });
        assert!(seat_from(&no_token).is_none(), "a reply without playerToken gives no seat");
    }

    /// The same bytes give the same fingerprint, a changed file changes it, and it is the sha256 `lr_story pack` uses.
    #[test]
    fn the_fingerprint_follows_the_pack_files() {
        let dir = std::env::temp_dir().join(format!("lr_story_fp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(dir.join("pack.json"), b"{}").unwrap();
        std::fs::write(dir.join("scenes").join("a.json"), b"x").unwrap();
        let first = fingerprint(&dir).unwrap();
        assert_eq!(first, fingerprint(&dir).unwrap());
        std::fs::write(dir.join("scenes").join("a.json"), b"y").unwrap();
        assert_ne!(first, fingerprint(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
