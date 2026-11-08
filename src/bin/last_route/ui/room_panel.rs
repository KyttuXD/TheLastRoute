//! The room panel's logic: who is in the room, what the host sets, how a code is joined, and what a mod pack mismatch
//! says. No drawing here; `ui/menus.rs` draws the rows this builds (hook list in the task notes).
//!
//! * Player list (TODO 198, 842, 856): [`roster_rows`] builds one row per runner from the session's peer roster
//!   (`NetSession::roster`), the local runner first and always present, so a room whose roster is not in yet still shows
//!   the local row. An ADMIN tag comes from a name test the caller passes (the listing flag of the room directory).
//! * Staging (TODO 841, 855): [`RoomPanel`]. The host edits the lobby settings and the route choice; each edit queues a
//!   typed [`LobbyEvent`] message for the guests. A guest's panel is read-only: it applies what the host sent and refuses
//!   local edits.
//! * Join by code (TODO 854): [`JoinState`] checks the typed code, hands a valid one to the room-link join, and turns the
//!   directory's answer into one line.
//! * Mod packs (TODO 870): [`pack_verdict`] turns a [`PackCheck`] into a warning or a refusal, by the data switch.
//! * REFRESH (list of open rooms): [`Refresh`] spaces the presses.
//!
//! Every player-facing line is in `data/net/rejoin.json` "strings" and upper case. Every limit is data.

use crate::net::protocol::{LobbyEvent, Message, PackCheck, RoomSettings};
use crate::net::rejoin::{line, rules};
use crate::net::world_sync::RosterEntry;
use crate::net::NetError;
use serde_json::{json, Value};

/// One line of the player list.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelRow {
    /// The runner's session id; None for the local runner before it has one.
    pub peer: Option<u32>,
    pub name: String,
    pub host: bool,
    pub you: bool,
    pub admin: bool,
    pub ping: Option<u32>,
    pub status: String,
}

/// The rows of the player list: the local runner first, then every runner the roster lists, each once. `local_peer` is
/// this runner's session id when it has one (a guest); a roster entry with that id, or with this runner's name, is the
/// local row and is not listed twice. `is_admin` is the directory's admin flag for a name.
pub fn roster_rows(local_name: &str, local_is_host: bool, local_peer: Option<u32>, roster: &[RosterEntry], is_admin: &dyn Fn(&str) -> bool) -> Vec<PanelRow> {
    let status = |host: bool| if host { line("status_host_staging", "HOST // STAGING", &[]) } else { line("status_staging", "STAGING", &[]) };
    let local = local_name.trim().to_uppercase();
    let mut rows = vec![PanelRow {
        peer: local_peer,
        name: local.clone(),
        host: local_is_host,
        you: true,
        admin: is_admin(local.as_str()),
        ping: None,
        status: status(local_is_host),
    }];
    let mut others: Vec<&RosterEntry> = roster.iter().filter(|e| !e.name.trim().is_empty()).collect();
    // the host's row comes before the guests'
    others.sort_by_key(|e| (!e.host, e.peer));
    for e in others {
        let name = e.name.trim().to_uppercase();
        let is_local = local_peer == Some(e.peer) || name == local;
        let listed = rows.iter().any(|r| r.peer == Some(e.peer) || r.name == name);
        if is_local || listed {
            // the roster knows this runner's ping and role better than the fallback row did
            if is_local {
                if let Some(r) = rows.first_mut() {
                    r.ping = r.ping.or(e.ping);
                    r.peer = r.peer.or(Some(e.peer));
                }
            }
            continue;
        }
        rows.push(PanelRow { peer: Some(e.peer), admin: is_admin(name.as_str()), name, host: e.host, you: false, ping: e.ping, status: status(e.host) });
    }
    rows
}

/// The text of a row: `NAME // HOST // YOU // ADMIN // 42MS` with the tags it has.
pub fn row_text(row: &PanelRow) -> String {
    let mut parts = vec![row.name.clone()];
    if row.host {
        parts.push(line("tag_host", "HOST", &[]));
    }
    if row.you {
        parts.push(line("tag_you", "YOU", &[]));
    }
    if row.admin {
        parts.push(line("tag_admin", "ADMIN", &[]));
    }
    if let Some(ms) = row.ping {
        parts.push(format!("{ms}MS"));
    }
    parts.join(" // ")
}

/// What applying a host message did to a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    /// The lobby settings were replaced (a guest's read-only copy).
    Settings,
    /// The host started the route with these settings.
    Start(RoomSettings),
    /// The host ended the route.
    End(String),
    /// The room closed.
    Closed(String),
    /// Nothing for the panel (a host does not take settings from the wire; another kind of message).
    Ignored,
}

/// The key of the route choice in the settings' extra fields.
pub const ROUTE_KEY: &str = "route";

/// The staging panel of a room.
#[derive(Clone, Debug)]
pub struct RoomPanel {
    host: bool,
    settings: RoomSettings,
    out: Vec<Message>,
}

impl RoomPanel {
    pub fn new(host: bool, code: &str) -> RoomPanel {
        RoomPanel { host, settings: RoomSettings { code: code.trim().to_uppercase(), ..RoomSettings::default() }, out: Vec::new() }
    }

    /// Guests may not change anything: the settings are the host's.
    pub fn read_only(&self) -> bool {
        !self.host
    }

    pub fn settings(&self) -> &RoomSettings {
        &self.settings
    }

    /// The route the host chose; empty before a choice.
    pub fn route(&self) -> &str {
        self.settings.rest.get(ROUTE_KEY).and_then(Value::as_str).unwrap_or("")
    }

    /// The label under a read-only setting.
    pub fn read_only_note(&self) -> Option<String> {
        self.read_only().then(|| line("settings_read_only", "SET BY THE HOST", &[]))
    }

    /// Host: changes the settings with `edit` and queues them for the guests. False (and nothing changes) on a guest.
    pub fn edit_settings(&mut self, edit: impl FnOnce(&mut RoomSettings)) -> bool {
        if !self.host {
            return false;
        }
        edit(&mut self.settings);
        self.out.push(LobbyEvent::Settings(self.settings.clone()).message());
        true
    }

    /// Host: the route choice, sent to the guests with the settings. False on a guest or an empty choice.
    pub fn choose_route(&mut self, route: &str) -> bool {
        let route = route.trim();
        if route.is_empty() {
            return false;
        }
        self.edit_settings(|s| {
            s.rest.insert(ROUTE_KEY.into(), json!(route));
        })
    }

    /// Host: starts the route; the guests get the settings with the start.
    pub fn start(&mut self) -> bool {
        if !self.host {
            return false;
        }
        self.out.push(LobbyEvent::Start(self.settings.clone()).message());
        true
    }

    /// Host: ends the route (the room stays open).
    pub fn end(&mut self, reason: &str) -> bool {
        if !self.host {
            return false;
        }
        self.out.push(LobbyEvent::End { reason: reason.to_string() }.message());
        true
    }

    /// The messages the host owes its guests since the last call (broadcast them).
    pub fn take_outgoing(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.out)
    }

    /// A guest takes what the host sent. A host never takes settings from the wire.
    pub fn apply(&mut self, event: &LobbyEvent) -> Applied {
        if self.host {
            return Applied::Ignored;
        }
        match event {
            LobbyEvent::Settings(s) => {
                self.settings = s.clone();
                Applied::Settings
            }
            LobbyEvent::Start(s) => {
                self.settings = s.clone();
                Applied::Start(s.clone())
            }
            LobbyEvent::End { reason } => Applied::End(reason.clone()),
            LobbyEvent::Closed { reason } => Applied::Closed(reason.clone()),
            LobbyEvent::Daytime(_) | LobbyEvent::Packs(_) => Applied::Ignored,
        }
    }
}

/// Why a typed room code is not joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeError {
    Empty,
    WrongLength,
    BadChars,
}

/// The code to join from what the player typed: trimmed, upper case, five letters or digits.
pub fn check_join_code(typed: &str) -> Result<String, CodeError> {
    let code = typed.trim().to_uppercase();
    if code.is_empty() {
        return Err(CodeError::Empty);
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CodeError::BadChars);
    }
    if code.chars().count() != 5 {
        return Err(CodeError::WrongLength);
    }
    Ok(code)
}

/// Where a JOIN ROOM attempt is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinPhase {
    Idle,
    /// The directory is being asked for the room.
    Checking(String),
    /// The room was found and the link is opening (room_link join).
    Joining(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinState {
    pub phase: JoinPhase,
}

impl Default for JoinState {
    fn default() -> JoinState {
        JoinState { phase: JoinPhase::Idle }
    }
}

impl JoinState {
    /// The player pressed JOIN with `typed`. A valid code is returned for the caller to look up and join; a bad one
    /// puts the error on screen and returns None. Pressing again while a join is under way returns None.
    pub fn submit(&mut self, typed: &str) -> Option<String> {
        if matches!(self.phase, JoinPhase::Checking(_) | JoinPhase::Joining(_)) {
            return None;
        }
        match check_join_code(typed) {
            Ok(code) => {
                self.phase = JoinPhase::Checking(code.clone());
                Some(code)
            }
            Err(e) => {
                let text = match e {
                    CodeError::Empty => line("join_empty", "TYPE A ROOM CODE", &[]),
                    CodeError::WrongLength => line("join_short", "A ROOM CODE HAS 5 LETTERS OR DIGITS", &[]),
                    CodeError::BadChars => line("join_bad_chars", "A ROOM CODE HAS ONLY LETTERS AND DIGITS", &[]),
                };
                self.phase = JoinPhase::Failed(text);
                None
            }
        }
    }

    /// The directory has the room: the link opens.
    pub fn found(&mut self) {
        if let JoinPhase::Checking(code) = &self.phase {
            self.phase = JoinPhase::Joining(code.clone());
        }
    }

    /// The lookup or the link failed.
    pub fn failed(&mut self, error: &NetError) {
        self.phase = JoinPhase::Failed(join_error_text(error));
    }

    /// The link is up: back to idle.
    pub fn joined(&mut self) {
        self.phase = JoinPhase::Idle;
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, JoinPhase::Checking(_) | JoinPhase::Joining(_))
    }

    /// The feedback line; empty when idle.
    pub fn text(&self) -> String {
        match &self.phase {
            JoinPhase::Idle => String::new(),
            JoinPhase::Checking(code) => line("join_checking", "LOOKING FOR ROOM {code}...", &[("code", code)]),
            JoinPhase::Joining(code) => line("join_joining", "JOINING ROOM {code}...", &[("code", code)]),
            JoinPhase::Failed(text) => text.clone(),
        }
    }
}

/// One line for a failed join: not found, full, another build, or a plain failure.
pub fn join_error_text(error: &NetError) -> String {
    let said = error.message_or("").to_uppercase();
    if said.contains("BUILD MISMATCH") {
        return line("join_mismatch", "BUILD MISMATCH", &[]);
    }
    match error.status() {
        Some(404) => line("join_not_found", "NO ROOM WITH THAT CODE", &[]),
        Some(403) | Some(409) | Some(423) if said.contains("FULL") || error.status() == Some(409) => line("join_full", "THAT ROOM IS FULL", &[]),
        _ if said.contains("FULL") => line("join_full", "THAT ROOM IS FULL", &[]),
        _ if said.contains("NOT FOUND") || said.contains("NO ROOM") => line("join_not_found", "NO ROOM WITH THAT CODE", &[]),
        _ => line("join_failed", "COULD NOT JOIN THAT ROOM", &[]),
    }
}

/// What to do about the room's mod packs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackVerdict {
    Fine,
    /// Show this line and play on.
    Warn(String),
    /// Show this line and leave the room.
    Refuse(String),
}

/// The data switch `data/net/anticheat.json` "packs.refuse_on_mismatch" (off when the file is missing).
pub fn refuse_on_mismatch() -> bool {
    static REFUSE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REFUSE.get_or_init(|| {
        crate::content::embedded::data_under("net/anticheat.json")
            .next()
            .and_then(|(_, t)| serde_json::from_str::<Value>(t).ok())
            .and_then(|v| v.get("packs")?.get("refuse_on_mismatch")?.as_bool())
            .unwrap_or(false)
    })
}

/// A pack check turned into what the guest sees. A match, or a host that announced nothing, is fine.
pub fn pack_verdict(check: &PackCheck, refuse: bool) -> PackVerdict {
    let names = check.names();
    if names.is_empty() {
        return PackVerdict::Fine;
    }
    let list = names.join(", ").to_uppercase();
    if refuse {
        PackVerdict::Refuse(line("pack_refused", "MOD PACKS DO NOT MATCH THE HOST // {names}", &[("names", &list)]))
    } else {
        PackVerdict::Warn(line("pack_mismatch", "YOUR MOD PACKS DO NOT MATCH THE HOST: {names}", &[("names", &list)]))
    }
}

/// REFRESH of the open-room list: presses are spaced `refresh_min_seconds` apart and ignored while a request is out.
#[derive(Clone, Debug, Default)]
pub struct Refresh {
    pending: bool,
    last: Option<f64>,
}

impl Refresh {
    /// True when the press starts a request (the caller then calls the list api and, when it answers, [`Refresh::done`]).
    pub fn press(&mut self, now: f64) -> bool {
        if self.pending || self.last.map_or(false, |t| now - t < rules().refresh_min_seconds) {
            return false;
        }
        self.pending = true;
        self.last = Some(now);
        true
    }

    pub fn done(&mut self) {
        self.pending = false;
    }

    /// The button label.
    pub fn label(&self) -> String {
        if self.pending {
            line("refreshing", "REFRESHING...", &[])
        } else {
            line("refresh", "REFRESH", &[])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{pack_digest, PackAnnounce};

    fn entry(peer: u32, name: &str, host: bool, ping: Option<u32>) -> RosterEntry {
        RosterEntry { peer, name: name.into(), skin: String::new(), host, depth: None, sending: false, ping, state: None }
    }
    fn nobody(_: &str) -> bool {
        false
    }

    #[test]
    fn rows_come_from_the_peer_roster_with_the_local_row_first() {
        let roster = vec![entry(3, "bo", false, Some(40)), entry(1, "ana", true, Some(25)), entry(2, "cy", false, None)];
        let rows = roster_rows("kyttu", false, Some(9), &roster, &nobody);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["KYTTU", "ANA", "CY", "BO"], "local first, then the host, then the guests by session");
        assert!(rows[0].you && !rows[0].host);
        assert!(rows[1].host && !rows[1].you);
        assert_eq!(rows[3].ping, Some(40));
    }

    #[test]
    fn with_no_roster_the_local_row_is_the_fallback() {
        let rows = roster_rows("  kyttu ", true, None, &[], &nobody);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].name.as_str(), rows[0].host, rows[0].you), ("KYTTU", true, true));
        assert!(rows[0].status.contains("HOST"));
    }

    #[test]
    fn the_local_runner_is_not_listed_twice() {
        let roster = vec![entry(9, "Kyttu", false, Some(31)), entry(5, "ana", false, None), entry(5, "ana", false, None)];
        let rows = roster_rows("kyttu", false, Some(9), &roster, &nobody);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].ping, Some(31), "the local row takes its ping from the roster");
    }

    #[test]
    fn the_admin_tag_follows_the_listing_flag() {
        let rows = roster_rows("kyttu", false, None, &[entry(1, "ana", true, None)], &|n: &str| n == "ANA");
        assert!(!rows[0].admin && rows[1].admin);
        assert!(row_text(&rows[1]).contains("ADMIN") && row_text(&rows[1]).contains("HOST"));
        assert!(row_text(&rows[0]).contains("YOU"));
    }

    #[test]
    fn the_host_edits_and_the_guest_sees_it_read_only() {
        let mut host = RoomPanel::new(true, "abcde");
        let mut guest = RoomPanel::new(false, "abcde");
        assert!(!host.read_only() && guest.read_only() && guest.read_only_note().is_some());

        assert!(host.choose_route("INFINITE"));
        assert!(host.edit_settings(|s| s.scale = 2));
        assert!(host.start());
        let sent = host.take_outgoing();
        assert_eq!(sent.len(), 3);
        assert!(host.take_outgoing().is_empty());

        let mut started = None;
        for m in sent {
            let wire = Message::decode(&m.encode()).unwrap();
            if let Applied::Start(s) = guest.apply(&LobbyEvent::from_message(&wire).unwrap()) {
                started = Some(s);
            }
        }
        assert_eq!(guest.route(), "INFINITE");
        assert_eq!(guest.settings().scale, 2);
        assert_eq!(started.map(|s| s.code), Some("ABCDE".to_string()));
    }

    #[test]
    fn a_guest_cannot_edit_and_a_host_ignores_the_wire() {
        let mut guest = RoomPanel::new(false, "ABCDE");
        assert!(!guest.edit_settings(|s| s.scale = 9));
        assert!(!guest.choose_route("X") && !guest.start() && !guest.end("NO"));
        assert_eq!(guest.settings().scale, 0);
        assert!(guest.take_outgoing().is_empty());

        let mut host = RoomPanel::new(true, "ABCDE");
        let forged = LobbyEvent::Settings(RoomSettings { code: "ZZZZZ".into(), scale: 7, ..RoomSettings::default() });
        assert_eq!(host.apply(&forged), Applied::Ignored);
        assert_eq!(host.settings().code, "ABCDE");
    }

    #[test]
    fn a_guest_hears_the_end_and_the_close() {
        let mut guest = RoomPanel::new(false, "ABCDE");
        assert_eq!(guest.apply(&LobbyEvent::End { reason: "OVER".into() }), Applied::End("OVER".into()));
        assert_eq!(guest.apply(&LobbyEvent::Closed { reason: "GONE".into() }), Applied::Closed("GONE".into()));
        assert_eq!(guest.apply(&LobbyEvent::Daytime(true)), Applied::Ignored);
    }

    #[test]
    fn join_codes_are_checked_before_anything_is_sent() {
        assert_eq!(check_join_code(" ab3de "), Ok("AB3DE".to_string()));
        assert_eq!(check_join_code(""), Err(CodeError::Empty));
        assert_eq!(check_join_code("ABCD"), Err(CodeError::WrongLength));
        assert_eq!(check_join_code("ABCDEF"), Err(CodeError::WrongLength));
        assert_eq!(check_join_code("AB-DE"), Err(CodeError::BadChars));

        let mut j = JoinState::default();
        assert_eq!(j.submit("ab"), None);
        assert!(!j.busy() && j.text().contains("5 LETTERS"));
        assert_eq!(j.submit("abcde"), Some("ABCDE".to_string()));
        assert!(j.busy() && j.text().contains("ABCDE"));
        assert_eq!(j.submit("fghij"), None, "a join under way is not doubled");
        j.found();
        assert!(matches!(j.phase, JoinPhase::Joining(_)));
        j.joined();
        assert_eq!(j.text(), "");
    }

    #[test]
    fn a_refused_join_reads_as_one_line() {
        let http = |status: u16, error: &str| NetError::Http { status, code: String::new(), error: error.into(), body: Value::Null };
        let mut j = JoinState::default();
        j.submit("abcde");
        j.failed(&http(404, "ROOM NOT FOUND"));
        assert_eq!(j.text(), "NO ROOM WITH THAT CODE");
        assert!(!j.busy(), "a failed join can be tried again");
        assert!(join_error_text(&http(409, "")).contains("FULL"));
        assert!(join_error_text(&http(400, "BUILD MISMATCH // other build")).contains("BUILD MISMATCH"));
        assert!(join_error_text(&NetError::Timeout).contains("COULD NOT JOIN"));
    }

    #[test]
    fn a_pack_mismatch_warns_or_refuses_by_the_data_switch() {
        let host = PackAnnounce::new(&[("neon_halls".to_string(), "aa11".to_string())]);
        let none: Vec<(String, String)> = Vec::new();
        let check = crate::net::protocol::check_packs(&none, &host);
        match pack_verdict(&check, false) {
            PackVerdict::Warn(t) => assert!(t.contains("NEON_HALLS"), "{t}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(pack_verdict(&check, true), PackVerdict::Refuse(_)));
        let same = [("neon_halls".to_string(), "aa11".to_string())];
        assert_eq!(pack_verdict(&crate::net::protocol::check_packs(&same, &host), true), PackVerdict::Fine);
        assert_eq!(pack_digest(&same), host.hash);
        assert!(!refuse_on_mismatch(), "the shipped data only warns");
    }

    #[test]
    fn refresh_presses_are_spaced() {
        let mut r = Refresh::default();
        assert!(r.press(100.0));
        assert!(!r.press(100.1), "a request is out");
        r.done();
        assert!(!r.press(100.1), "too soon");
        assert!(r.press(100.0 + rules().refresh_min_seconds + 0.1));
        assert_eq!(r.label(), "REFRESHING...");
    }
}
