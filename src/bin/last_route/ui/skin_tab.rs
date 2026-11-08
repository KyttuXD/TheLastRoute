//! The SKIN tab of the settings overlay, laid out as the Godot skin panel (game/menus/skin_panel.gd): the rotating
//! preview on the left; on the right PICK A PNG and APPLY, DEFAULT, the FROM URL box, the co-op COPY SKIN row, the
//! catalogue row, OTHERS MAY COPY and ARMS, then the status and "Your skin". Every rectangle is in menu design units
//! (the preview box included: see `skin_preview::screen_rect`).
//!
//! PICK A PNG opens the system's own file chooser (shared/file_pick.rs): the browser's file window on the web, the
//! system open dialog on Windows and macOS. The tab is reached from the title screen's settings and, in a run, from the
//! pause menu's SKIN entry; an applied skin changes `Settings`, and the game loop wears it from there (the hands, the
//! upload and the room).

use super::style::*;
use super::text_field::{self, pasted};
use crate::app::assets::Assets;
use crate::app::settings::Settings;
use crate::net::world_sync::SKIN_DEFAULT;
use crate::shared::file_pick::{self, Outcome};
use crate::shared::skins::{self, ValidSkin};
use macroquad::prelude::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The accept string PICK A PNG gives the file chooser (an HTML `accept` value: the page uses it as it is, the native
/// dialog filters by the extension).
const PNG_KIND: &str = "image/png,.png";

/// The captions of the tab's buttons, row by row (the boxes and the pair-buttons beside them are described in `draw`).
const PICK_LABEL: &str = "PICK A PNG";
const APPLY_LABEL: &str = "APPLY";
const DEFAULT_LABEL: &str = "DEFAULT";
const URL_LABEL: &str = "FROM URL";
const COPY_LABEL: &str = "COPY SKIN";
const CATALOG_LABEL: &str = "FROM CATALOG";
const SHARE_LABEL: &str = "OTHERS MAY COPY";
#[cfg_attr(not(test), allow(dead_code))]
const ROW_LABELS: [&[&str]; ROWS] = [
    &[PICK_LABEL, APPLY_LABEL],
    &[DEFAULT_LABEL],
    &[URL_LABEL],
    &[COPY_LABEL],
    &[CATALOG_LABEL],
    &[SHARE_LABEL],
];

/// The arm styles ARMS cycles through (skin_panel.gd `_cycle_arms`).
const ARMS: [&str; 3] = ["auto", "classic", "slim"];

// ------------------------------------------------------------------------------------------- text editing

/// A one-line text box's text and caret (a char index). The SKIN boxes only take ASCII, but the code counts chars anyway.
#[derive(Default, Clone, Debug, PartialEq)]
struct TextBox {
    text: String,
    caret: usize,
}

impl TextBox {
    fn len(&self) -> usize {
        self.text.chars().count()
    }
    fn byte_at(&self, char_index: usize) -> usize {
        self.text.char_indices().nth(char_index).map_or(self.text.len(), |(b, _)| b)
    }
    /// Inserts `ch` at the caret unless the box already holds `cap` characters.
    fn insert(&mut self, ch: char, cap: usize) -> bool {
        if self.len() >= cap {
            return false;
        }
        let at = self.byte_at(self.caret);
        self.text.insert(at, ch);
        self.caret += 1;
        true
    }
    fn backspace(&mut self) {
        if self.caret > 0 {
            let at = self.byte_at(self.caret - 1);
            self.text.remove(at);
            self.caret -= 1;
        }
    }
    fn delete(&mut self) {
        if self.caret < self.len() {
            let at = self.byte_at(self.caret);
            self.text.remove(at);
        }
    }
    fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }
    fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.len());
    }
    fn home(&mut self) {
        self.caret = 0;
    }
    fn end(&mut self) {
        self.caret = self.len();
    }
    fn set_end(&mut self) {
        self.end();
    }
}

/// Key repeat for a held key: fires when pressed, then after `delay` seconds again `per_sec` times a second.
#[derive(Default, Clone, Copy, Debug)]
struct Repeat {
    since: Option<f64>,
    last: f64,
}

impl Repeat {
    fn fire(&mut self, down: bool, now: f64, delay: f64, per_sec: f64) -> bool {
        if !down {
            self.since = None;
            return false;
        }
        match self.since {
            None => {
                self.since = Some(now);
                self.last = now;
                true
            }
            Some(t0) => {
                if now - t0 >= delay && now - self.last >= 1.0 / per_sec {
                    self.last = now;
                    true
                } else {
                    false
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------- text fitting

/// `word` cut into pieces that each fit `width` (a word that fits is one piece).
fn split_long(word: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    if measure(word) <= width {
        return vec![word.to_string()];
    }
    let mut pieces = Vec::new();
    let mut cur = String::new();
    for c in word.chars() {
        let mut test = cur.clone();
        test.push(c);
        if measure(&test) > width && !cur.is_empty() {
            pieces.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
}

/// `text` wrapped to `width` units in at most `max_lines` lines: words wrap, a word wider than a line (a path, a link) is
/// broken between characters, and what does not fit ends the last line with an ellipsis.
fn wrap_to(text: &str, width: f32, max_lines: usize, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        for piece in split_long(word, width, measure) {
            let test = if cur.is_empty() { piece.clone() } else { format!("{cur} {piece}") };
            if measure(&test) <= width {
                cur = test;
            } else {
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                cur = piece;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && measure(&format!("{last}…")) > width {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
}

/// `text` shortened from the left to fit `width`, with an ellipsis in front: the end of a path is the part that matters.
fn fit_tail(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let tail: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if measure(&tail) <= width {
            return tail;
        }
    }
    "…".to_string()
}

// ------------------------------------------------------------------------------------------- layout

/// The SKIN tab's grid, from the panel's top-left corner. The right column is 240 units wide: a 158 box and an 80 button
/// with a 2 unit gap (the Godot panel's HBoxes). Row `n` (0 = PICK A PNG | APPLY) is 16 units tall every 18 units.
#[derive(Clone, Copy, Debug)]
struct Grid {
    top: f32,
    cx: f32,
    bw: f32,
    bx: f32,
    gw: f32,
    full: f32,
}

fn grid(ox: f32, oy: f32) -> Grid {
    let (cx, bw, gw) = (ox + 180.0, 158.0, 80.0);
    Grid { top: oy + 66.0, cx, bw, bx: cx + bw + 2.0, gw, full: bw + 2.0 + gw }
}

impl Grid {
    /// The preview box: 150 x 150 units at the left of the panel, in design units like every other rectangle here.
    fn preview(&self, ox: f32) -> Rect {
        Rect::new(ox + 20.0, self.top, 150.0, 150.0)
    }
    /// Row `n` (0 = PICK A PNG) of 16 unit boxes every 18 units.
    fn row(&self, n: usize) -> f32 {
        self.top + 18.0 * n as f32
    }
    /// DEFAULT, alone on the second row and as wide as the column.
    fn default_button(&self) -> Rect {
        Rect::new(self.cx, self.row(1), self.full, 16.0)
    }
    /// OTHERS MAY COPY (a short toggle) and ARMS (the rest of the row, wide enough for "ARMS: AUTO (CLASSIC)").
    fn options(&self) -> (Rect, Rect) {
        let toggle_w = 112.0;
        let y = self.row(ROWS - 1);
        (Rect::new(self.cx, y, toggle_w, 16.0), Rect::new(self.cx + toggle_w + 2.0, y, self.full - toggle_w - 2.0, 16.0))
    }
    /// Where the status text starts: just under the last row (three status lines fit before "Your skin").
    fn status_top(&self) -> f32 {
        self.row(ROWS - 1) + 16.0 + 4.0
    }
    /// The line of "Your skin", under the status.
    fn mine_top(&self) -> f32 {
        self.status_top() + 28.0
    }
}

/// How many rows of controls the right column has (see `ROW_LABELS`).
const ROWS: usize = 6;

/// The status text's size in units and the line step.
const STATUS_SIZE: f32 = 6.0;
const STATUS_STEP: f32 = 8.0;

// ------------------------------------------------------------------------------------------- state

/// A validated skin waiting for APPLY.
#[derive(Clone, Debug, PartialEq)]
struct Picked {
    png: Vec<u8>,
    slim: bool,
    name: String,
}

/// PICK A PNG waiting on the file chooser, with the status line it replaced (a chooser that is closed puts it back).
struct Picking {
    req: file_pick::Request,
    status: String,
    detail: String,
}

/// A co-op runner whose skin can be copied (filled by the app from the room: `set_copy_peers`).
#[derive(Clone, Debug, PartialEq)]
pub struct CopyPeer {
    pub name: String,
    pub hash: String,
    pub slim: bool,
}

/// The SKIN tab's state between frames. It lives here rather than in Settings: a picked skin waits for APPLY, and a
/// download runs on a thread that reports back through `fetch`.
struct SkinUi {
    pick: Option<Picked>,
    /// The preview texture. Rebuilt (set to None) when the pick or the chosen skin changes; inside None means no picture.
    shown: Option<Option<Texture2D>>,
    /// The arm layout read for the skin in the preview (the pick's, else the applied skin's).
    shown_slim: bool,
    /// What `shown` was built for: the applied skin id and the save generation.
    shown_key: Option<(String, u64)>,
    status: String,
    /// A muted second line under the status (a folder path).
    detail: String,
    url: TextBox,
    /// The link box has the keyboard.
    url_focused: bool,
    repeats: [Repeat; 4],
    /// A download in progress; its result arrives here.
    fetch: Option<Receiver<Result<ValidSkin, String>>>,
    fetch_label: String,
    rules: Option<skins::SkinRules>,
    catalog: Option<Vec<skins::CatalogSkin>>,
    catalog_index: usize,
    /// The file chooser PICK A PNG opened, until it answers.
    picking: Option<Picking>,
    copy_index: usize,
    /// The file name of the skin applied this session (for "Your skin").
    applied_name: String,
}

impl Default for SkinUi {
    fn default() -> Self {
        SkinUi {
            pick: None,
            shown: None,
            shown_slim: false,
            shown_key: None,
            status: String::new(),
            detail: String::new(),
            url: TextBox::default(),
            url_focused: false,
            repeats: [Repeat::default(); 4],
            fetch: None,
            fetch_label: String::new(),
            rules: None,
            catalog: None,
            catalog_index: 0,
            picking: None,
            copy_index: 0,
            applied_name: String::new(),
        }
    }
}

thread_local! {
    static SKIN: RefCell<SkinUi> = RefCell::new(SkinUi::default());
    static COPY_PEERS: RefCell<Vec<CopyPeer>> = const { RefCell::new(Vec::new()) };
    /// Where the worn skin stands on its way to other runners (the app sets it each frame; empty says nothing).
    static SHARE_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
    /// The rotating 3D mannequin in the preview box (ui/skin_preview.rs).
    static SKIN_PREVIEW: RefCell<crate::ui::skin_preview::SkinPreview> = RefCell::new(crate::ui::skin_preview::SkinPreview::new());
}

/// Set when Escape was used inside the tab (leaving a text box): the panel then stays open.
static ESCAPE_TAKEN: AtomicBool = AtomicBool::new(false);

/// True when the SKIN tab used this frame's Escape press, so the settings panel must not close on it. Reading resets it.
pub fn take_escape() -> bool {
    ESCAPE_TAKEN.swap(false, Ordering::SeqCst)
}

/// The co-op runners whose skins the COPY SKIN row offers (the app sets it each frame from the open room).
pub fn set_copy_peers(peers: Vec<CopyPeer>) {
    COPY_PEERS.with(|p| *p.borrow_mut() = peers);
}

/// What the SKIN tab says about sharing the worn skin with other runners (see `skins::ShareState::text`).
pub fn set_share_text(text: String) {
    SHARE_TEXT.with(|t| *t.borrow_mut() = text);
}

/// The tab is not showing: a focused text box is dropped and a file chooser still open is let go of, so a hidden tab never
/// holds the keyboard or takes a file later.
pub fn blur() {
    SKIN.with(|c| {
        if let Ok(mut s) = c.try_borrow_mut() {
            s.url_focused = false;
            if let Some(p) = s.picking.take() {
                file_pick::release(p.req);
            }
        }
    });
}

/// A button that is greyed (and does nothing) when `on` is false.
fn gate(on: bool) -> BtnState {
    if on {
        plain()
    } else {
        disabled()
    }
}

/// APPLY is greyed until a skin is picked (skin_panel.gd `_refresh`).
fn apply_enabled(s: &SkinUi) -> bool {
    s.pick.is_some()
}

/// DEFAULT restores the built-in skin and drops any picked one (skin_panel.gd `_reset`).
fn reset_to_default(st: &mut Settings, s: &mut SkinUi) {
    st.skin = SKIN_DEFAULT.to_string();
    st.skin_slim = false;
    s.pick = None;
    s.shown = None;
    s.applied_name.clear();
    s.detail.clear();
    s.status = "Back to the default skin.".to_string();
}

/// What ARMS shows: the setting, and for auto the arm style the skin in the preview resolves to.
fn arms_label(arms: &str, slim: bool) -> String {
    if arms == "auto" {
        format!("ARMS: AUTO ({})", if slim { "SLIM" } else { "CLASSIC" })
    } else {
        format!("ARMS: {}", arms.to_uppercase())
    }
}

/// True when the preview should wear slim arms: the ARMS setting applied to the layout read for the skin in the preview.
fn preview_slim(arms: &str, detected_slim: bool) -> bool {
    crate::app::settings::ArmsSetting::resolve(arms, detected_slim)
}

/// The ARMS setting after `cur`: auto, classic, slim, then auto again.
fn next_arms(cur: &str) -> &'static str {
    let i = ARMS.iter().position(|a| *a == cur).unwrap_or(0);
    ARMS[(i + 1) % ARMS.len()]
}

/// The arm style a validated skin is previewed and applied with, as the status says it.
fn arms_word(slim: bool) -> &'static str {
    if slim {
        "slim"
    } else {
        "classic"
    }
}

/// Puts a validated skin into the preview, waiting for APPLY (skin_panel.gd `_preview`). `name` is the file or source.
fn set_pick(s: &mut SkinUi, valid: ValidSkin, name: &str) {
    s.status = format!("Preview only ({name}). Press APPLY to wear it ({} arms).", arms_word(valid.slim));
    s.detail.clear();
    s.pick = Some(Picked { png: valid.png, slim: valid.slim, name: name.to_string() });
    s.shown = None;
}

/// A refused source: nothing is picked and the reason is shown (skin_panel.gd "Not applied: ...").
fn refuse(s: &mut SkinUi, why: &str) {
    s.pick = None;
    s.shown = None;
    s.detail.clear();
    s.status = format!("Not applied: {why}.");
}

/// A refused file, named: "Not applied: night.png: the file is too big."
fn refuse_file(s: &mut SkinUi, name: &str, why: &str) {
    refuse(s, &format!("{name}: {why}"));
}

/// A PNG as a preview texture, drawn with nearest-neighbour filtering so the pixels stay sharp. None when it does not
/// decode (the preview then shows the default skin).
fn texture_of(png: &[u8]) -> Option<Texture2D> {
    crate::shared::assets::try_tex(png)
}

/// Starts a download on a thread. `poll_fetch` takes the result on a later frame, so the UI never waits for the network.
fn start_fetch(s: &mut SkinUi, label: &str, job: impl FnOnce() -> Result<ValidSkin, String> + Send + 'static) {
    if s.fetch.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    crate::shared::task::detach(move || {

        let _ = tx.send(job());
    });
    s.fetch = Some(rx);
    s.fetch_label = label.to_string();
    s.detail.clear();
    s.status = "Downloading...".to_string();
}

/// Takes a finished download, when there is one, into the preview or the refusal.
fn poll_fetch(s: &mut SkinUi) {
    let Some(rx) = s.fetch.as_ref() else { return };
    let done = match rx.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("the download stopped".to_string()),
    };
    s.fetch = None;
    let label = std::mem::take(&mut s.fetch_label);
    match done {
        Ok(valid) => set_pick(s, valid, &label),
        Err(why) => refuse(s, &why),
    }
}

/// Takes the PNG files dropped on the window into the preview. A file over the size cap is refused by its size on disk
/// before it is read, so a huge file is never loaded.
fn take_dropped(s: &mut SkinUi) {
    for file in get_dropped_files() {
        let name = file
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "the dropped file".to_string());
        if !name.to_ascii_lowercase().ends_with(".png") {
            refuse_file(s, &name, "it is not a PNG file");
            continue;
        }
        let cap = skins::max_bytes();
        let bytes = match (&file.path, file.bytes) {
            (Some(path), _) => skins::read_capped(path, cap).map_err(|e| e.to_string()),
            (None, Some(b)) if b.len() <= cap => Ok(b),
            (None, _) => Err("the file is too big".to_string()),
        };
        take_file(s, &name, bytes);
    }
}

/// A file from the chooser or from a drop: it must be a PNG by its name, and its bytes go through the same check as
/// ever (`skins::validate_skin`: size cap, signature, chunks, 64x64, opaque base layer). The preview waits for APPLY.
fn take_file(s: &mut SkinUi, name: &str, bytes: Result<Vec<u8>, String>) {
    if !name.to_ascii_lowercase().ends_with(".png") {
        refuse_file(s, name, "it is not a PNG file");
        return;
    }
    match bytes.and_then(|b| skins::validate_skin(&b, skins::max_bytes())) {
        Ok(valid) => set_pick(s, valid, name),
        Err(why) => refuse_file(s, name, &why),
    }
}

/// PICK A PNG: opens the file chooser (the browser's file window, the system open dialog) and waits for its answer. A
/// chooser still open from an earlier press is let go of first, so a second press asks the browser again.
fn start_pick(s: &mut SkinUi) {
    if let Some(old) = s.picking.take() {
        file_pick::release(old.req);
    }
    s.url_focused = false;
    match file_pick::open(PNG_KIND, false, false) {
        Ok(req) => {
            s.picking = Some(Picking { req, status: std::mem::take(&mut s.status), detail: std::mem::take(&mut s.detail) });
            s.status = "Choose a PNG in the file window.".to_string();
        }
        Err(why) => {
            s.detail.clear();
            s.status = why;
        }
    }
}

/// Takes the chooser's answer, when there is one, each frame.
fn poll_pick(s: &mut SkinUi) {
    let Some(req) = s.picking.as_ref().map(|p| p.req) else { return };
    let outcome = file_pick::poll(req, skins::max_bytes());
    take_picked(s, outcome);
}

/// What the chooser's answer does. Pending waits. A closed chooser changes nothing: the status line it replaced comes
/// back. A chosen file is checked like a dropped one.
fn take_picked(s: &mut SkinUi, outcome: Outcome) {
    if outcome == Outcome::Pending {
        return;
    }
    let before = s.picking.take();
    match outcome {
        Outcome::Pending => {}
        Outcome::Cancelled => {
            if let Some(p) = before {
                s.status = p.status;
                s.detail = p.detail;
            }
        }
        Outcome::Picked(files) => match files.into_iter().next() {
            Some(file) => take_file(s, &file.name, file.bytes),
            None => {
                if let Some(p) = before {
                    s.status = p.status;
                    s.detail = p.detail;
                }
            }
        },
    }
}

/// FROM URL: the link is checked by the rules and downloaded on a thread, then validated like a picked file.
fn from_url(s: &mut SkinUi) {
    let Some(rules) = s.rules.clone() else {
        refuse(s, "the skin rules are missing from this build");
        return;
    };
    let url = s.url.text.trim().to_string();
    start_fetch(s, "link", move || skins::fetch_url_skin(&url, &rules));
}

/// FROM CATALOG: the chosen catalogue skin goes to the preview, then APPLY like any other source.
fn from_catalog(s: &mut SkinUi) {
    let Some(entry) = s.catalog.as_ref().and_then(|list| list.get(s.catalog_index)).cloned() else {
        return;
    };
    match skins::catalog_png(&entry) {
        None => refuse(s, &format!("the picture for {} is not in this build", entry.name)),
        Some(png) => match skins::validate_skin(&png, skins::max_bytes()) {
            Ok(valid) => set_pick(s, valid, &entry.name),
            Err(why) => refuse(s, &why),
        },
    }
}

/// COPY SKIN: the chosen co-op runner's skin (already downloaded for drawing) goes to the preview. The arm style comes
/// from the owner (the pixels were normalised), as in Godot.
fn copy_peer(s: &mut SkinUi, peer: &CopyPeer) {
    match skins::read_skin(&peer.hash).ok_or_else(|| "that skin is not downloaded yet".to_string()).and_then(|b| skins::validate_skin(&b, skins::max_bytes())) {
        Ok(mut valid) => {
            valid.slim = peer.slim;
            set_pick(s, valid, &peer.name);
        }
        Err(why) => refuse(s, &why),
    }
}

/// APPLY: the picked skin is saved as the user's own file and becomes the chosen skin.
fn apply_pick(s: &mut SkinUi, st: &mut Settings, changed: &mut bool) {
    let Some(picked) = s.pick.take() else { return };
    match skins::save_custom_skin(&picked.png) {
        Ok(id) => {
            st.skin = id;
            st.skin_slim = picked.slim;
            *changed = true;
            s.shown = None;
            s.detail.clear();
            s.applied_name = picked.name;
            s.status = "Skin applied.".to_string();
        }
        Err(e) => {
            s.pick = Some(picked);
            s.detail.clear();
            s.status = format!("Not applied: the skin could not be saved ({e}).");
        }
    }
}

/// The skin in use as "Your skin" says it.
fn mine_text(st: &Settings, s: &SkinUi) -> String {
    if st.skin == SKIN_DEFAULT {
        "Your skin: default".to_string()
    } else if s.applied_name.is_empty() {
        "Your skin: custom".to_string()
    } else {
        format!("Your skin: custom ({})", s.applied_name)
    }
}

/// One frame of typing and paste for the link box. Only a focused box takes either: a paste made while the box has no
/// keyboard is dropped. Typed characters are skipped while Cmd/Ctrl is held (a shortcut's leftover letter is not text),
/// and both go through the shared rules for [`Kind::Link`] (printable ASCII, no spaces or line breaks, at most `cap`).
fn link_keys(tb: &mut TextBox, focused: bool, cap: usize, command: bool, typed: &[char], clip: Option<&str>) {
    if !focused {
        return;
    }
    if !command {
        for &ch in typed {
            if let Some(c) = text_field::Kind::Link.typed(ch) {
                tb.insert(c, cap);
            }
        }
    }
    if let Some(clip) = clip {
        for ch in pasted(text_field::Kind::Link, clip, cap.saturating_sub(tb.len())).chars() {
            tb.insert(ch, cap);
        }
    }
}

/// Keys for the focused link box: printable ASCII characters, paste (Ctrl or Cmd + V, Shift+Insert), copy and cut (Ctrl or
/// Cmd + C / X: the whole link), Backspace and Delete with key repeat, Left, Right, Home and End. Returns true when Enter
/// was pressed.
fn edit_keys(s: &mut SkinUi, now: f64) -> bool {
    use super::text_field::{shortcut_pressed, Shortcut};
    let command = super::text_field::command_down();
    let shortcut = shortcut_pressed();
    let mut typed = vec![];
    while let Some(ch) = get_char_pressed() {
        typed.push(ch);
    }
    // taken every frame, focused or not: the web page keeps a paste until it is read, and a box without the keyboard must
    // not get that paste later. The browser answers the shortcut a moment after the key, so this is a read per frame.
    let clip = super::clipboard::poll_paste(shortcut == Some(Shortcut::Paste));
    let Some(rules) = s.rules.as_ref() else { return false };
    let cap = rules.max_url_length;
    let (delay, per_sec) = (rules.repeat_delay_secs, rules.repeat_per_sec);
    if !s.url_focused {
        return false;
    }
    let tb = &mut s.url;
    link_keys(tb, true, cap, command, &typed, clip.as_deref());
    match shortcut {
        Some(Shortcut::Copy) if !tb.text.is_empty() => super::clipboard::copy(&tb.text),
        Some(Shortcut::Cut) if !tb.text.is_empty() => {
            super::clipboard::copy(&tb.text);
            *tb = TextBox::default();
        }
        _ => {}
    }
    // the page answers the browser's own copy and cut events from this
    super::clipboard::offer(&tb.text);
    let keys = [KeyCode::Backspace, KeyCode::Delete, KeyCode::Left, KeyCode::Right];
    for (i, key) in keys.iter().enumerate() {
        if s.repeats[i].fire(is_key_down(*key), now, delay, per_sec) {
            match i {
                0 => tb.backspace(),
                1 => tb.delete(),
                2 => tb.left(),
                _ => tb.right(),
            }
        }
    }
    if is_key_pressed(KeyCode::Home) {
        tb.home();
    }
    if is_key_pressed(KeyCode::End) {
        tb.end();
    }
    is_key_pressed(KeyCode::Enter)
}

// ------------------------------------------------------------------------------------------- drawing

/// The skin's front (head and hat, body, arms, legs) at 4 design units to a skin pixel, centred in `area` (units). This
/// is the 2D stand-in for the 3D mannequin preview; it is drawn in window pixels, so both the box and the scale go
/// through the menu unit.
fn draw_front(tex: &Texture2D, area: Rect) {
    let area = crate::ui::skin_preview::screen_rect(area, unit());
    let k = 4.0 * unit();
    let cx = area.x + area.w * 0.5;
    let top = area.y + (area.h - 32.0 * k) * 0.5;
    let part = |src: (f32, f32), size: (f32, f32), x: f32, y: f32| {
        draw_texture_ex(
            tex,
            x,
            y,
            WHITE,
            DrawTextureParams {
                source: Some(Rect::new(src.0, src.1, size.0, size.1)),
                dest_size: Some(vec2(size.0 * k, size.1 * k)),
                ..Default::default()
            },
        );
    };
    part((4.0, 20.0), (4.0, 12.0), cx - 8.0 * k, top + 20.0 * k);
    part((20.0, 52.0), (4.0, 12.0), cx, top + 20.0 * k);
    part((44.0, 20.0), (4.0, 12.0), cx + 4.0 * k, top + 8.0 * k);
    part((36.0, 52.0), (4.0, 12.0), cx - 8.0 * k, top + 8.0 * k);
    part((20.0, 20.0), (8.0, 12.0), cx - 4.0 * k, top + 8.0 * k);
    part((8.0, 8.0), (8.0, 8.0), cx - 4.0 * k, top);
    part((40.0, 8.0), (8.0, 8.0), cx - 4.0 * k, top);
}

/// Rebuilds the cached preview picture and arm layout when the pick, the applied skin or the saved file changed.
fn refresh_shown(s: &mut SkinUi, st: &Settings) {
    let key = (st.skin.clone(), skins::generation());
    if s.shown.is_some() && s.shown_key.as_ref() == Some(&key) {
        return;
    }
    let (png, slim) = match &s.pick {
        Some(p) => (Some(p.png.clone()), p.slim),
        None if st.skin == SKIN_DEFAULT => (None, false),
        None => (skins::skin_png(&st.skin).map(|(_, bytes)| bytes), st.skin_slim),
    };
    s.shown = Some(png.as_deref().and_then(texture_of));
    s.shown_slim = slim;
    s.shown_key = Some(key);
}

/// The preview box: the chosen skin (or the pick waiting for APPLY), else the default mannequin.
fn draw_preview(a: &Assets, s: &mut SkinUi, st: &Settings, area: Rect) {
    refresh_shown(s, st);
    let p = pal();
    sbox(area, p.inset, p.field_edge, 1.0, p.corner);
    let slim = preview_slim(&st.arms, s.shown_slim);
    let tex = s.shown.as_ref().and_then(Option::as_ref);
    if a.npc_body.is_some() {
        SKIN_PREVIEW.with(|pv| pv.borrow_mut().draw(a, area, tex, slim, get_frame_time()));
        return;
    }
    // Fallback: the flat front, when the 3D preview target cannot be made.
    match tex {
        Some(tex) => draw_front(tex, area),
        None => label_c(a, F::Body, "DEFAULT", area.x + area.w * 0.5, area.y + area.h * 0.5 - 4.0, 7.0, p.muted),
    }
}

/// A button whose label is cut to fit (so a long name never spills over its edges).
fn fit_button(a: &Assets, r: Rect, text: &str, kind: Kind, st: BtnState) -> bool {
    let shown = ellipsize(a, F::Body, text, r.w - 8.0, 7.0);
    button(a, r, &shown, 7.0, kind, st)
}

/// Draws the SKIN tab at the panel corner (`ox`, `oy`) and handles its input.
pub fn draw(a: &Assets, st: &mut Settings, ox: f32, oy: f32, changed: &mut bool) {
    let p = pal();
    SKIN.with(|cell| {
        let mut s = cell.borrow_mut();
        if s.rules.is_none() {
            s.rules = skins::rules();
        }
        if s.catalog.is_none() {
            s.catalog = Some(skins::catalog());
        }
        poll_fetch(&mut s);
        poll_pick(&mut s);
        take_dropped(&mut s);

        let g = grid(ox, oy);
        let url_box = Rect::new(g.cx, g.row(2), g.bw, 16.0);

        if is_mouse_button_pressed(MouseButton::Left) {
            if hovered(url_box) {
                s.url_focused = true;
                s.url.set_end();
            } else {
                s.url_focused = false;
            }
        }
        if edit_keys(&mut s, get_time()) && s.url_focused {
            from_url(&mut s);
        }
        if is_key_pressed(KeyCode::Escape) && s.url_focused {
            s.url_focused = false;
            ESCAPE_TAKEN.store(true, Ordering::SeqCst);
        }
        let busy = s.fetch.is_some();

        draw_preview(a, &mut s, st, g.preview(ox));

        // Row 0: PICK A PNG | APPLY. PICK A PNG stays live while a chooser is open: pressing it again asks again.
        let half = (g.full - 2.0) * 0.5;
        if fit_button(a, Rect::new(g.cx, g.row(0), half, 16.0), PICK_LABEL, Kind::Secondary, gate(!busy)) {
            start_pick(&mut s);
        }
        if fit_button(
            a,
            Rect::new(g.cx + half + 2.0, g.row(0), half, 16.0),
            APPLY_LABEL,
            Kind::Secondary,
            gate(!busy && apply_enabled(&s)),
        ) {
            apply_pick(&mut s, st, changed);
        }

        // Row 1: DEFAULT
        let default_on = !busy && (st.skin != SKIN_DEFAULT || s.pick.is_some());
        if fit_button(a, g.default_button(), DEFAULT_LABEL, Kind::Secondary, gate(default_on)) {
            reset_to_default(st, &mut s);
            *changed = true;
        }

        // Row 2: the https link box and FROM URL
        let t = get_time() as f32;
        let url_caret = s.url.caret;
        line_edit_caret(a, url_box, &s.url.text, Some(url_caret), "https:// link to a 64x64 PNG skin", 7.0, s.url_focused, false, t);
        if fit_button(a, Rect::new(g.bx, g.row(2), g.gw, 16.0), URL_LABEL, Kind::Secondary, gate(!busy)) {
            from_url(&mut s);
        }

        // Row 3: the co-op players whose skin can be copied (click to choose the next) and COPY SKIN
        let peers = COPY_PEERS.with(|c| c.borrow().clone());
        let copy_box = Rect::new(g.cx, g.row(3), g.bw, 16.0);
        if peers.is_empty() {
            sbox(copy_box, p.inset, p.field_edge, 1.0, p.corner);
            label_vc(a, F::Body, "(no one to copy)", g.cx + 6.0, copy_box.y, 16.0, 7.0, p.muted);
            button(a, Rect::new(g.bx, g.row(3), g.gw, 16.0), COPY_LABEL, 7.0, Kind::Secondary, disabled());
        } else {
            let i = s.copy_index % peers.len();
            let text = format!("{}/{} {}", i + 1, peers.len(), peers[i].name);
            if fit_button(a, copy_box, &text, Kind::Secondary, gate(peers.len() > 1)) {
                s.copy_index = (i + 1) % peers.len();
            }
            if fit_button(a, Rect::new(g.bx, g.row(3), g.gw, 16.0), COPY_LABEL, Kind::Secondary, gate(!busy)) {
                copy_peer(&mut s, &peers[i]);
            }
        }

        // Row 4: the catalogue (click to choose the next) and FROM CATALOG
        let (list_len, catalog_label) = match s.catalog.as_ref() {
            Some(list) if !list.is_empty() => {
                let i = s.catalog_index % list.len();
                (list.len(), format!("{}/{} {}", i + 1, list.len(), list[i].name.to_uppercase()))
            }
            _ => (0, "(NO CATALOG SKINS)".to_string()),
        };
        if fit_button(a, Rect::new(g.cx, g.row(4), g.bw, 16.0), &catalog_label, Kind::Secondary, gate(list_len > 1)) {
            s.catalog_index = (s.catalog_index + 1) % list_len.max(1);
        }
        if fit_button(a, Rect::new(g.bx, g.row(4), g.gw, 16.0), CATALOG_LABEL, Kind::Secondary, gate(!busy && list_len > 0)) {
            from_catalog(&mut s);
        }

        // Row 5: OTHERS MAY COPY and ARMS
        let (toggle_r, arms_r) = g.options();
        if state_toggle(a, toggle_r, SHARE_LABEL, st.skin_copy, false, plain()) {
            st.skin_copy = !st.skin_copy;
            *changed = true;
        }
        let label_text = arms_label(&st.arms, preview_slim(&st.arms, s.shown_slim));
        if fit_button(a, arms_r, &label_text, Kind::Secondary, plain()) {
            st.arms = next_arms(&st.arms).to_string();
            *changed = true;
        }

        // Status (up to three lines, a muted detail line takes the third), then the skin in use
        let measure = |t: &str| tw(a, F::Body, t, STATUS_SIZE);
        let max_lines = if s.detail.is_empty() { 3 } else { 2 };
        let mut y = g.status_top();
        for line in wrap_to(&s.status, g.full, max_lines, &measure) {
            label(a, F::Body, &line, g.cx, y, STATUS_SIZE, p.body);
            y += STATUS_STEP;
        }
        if !s.detail.is_empty() {
            label(a, F::Body, &fit_tail(&s.detail, g.full, &measure), g.cx, y, STATUS_SIZE, p.muted);
        }
        let share = SHARE_TEXT.with(|t| t.borrow().clone());
        let mine = if share.is_empty() { mine_text(st, &s) } else { format!("{} - {share}", mine_text(st, &s)) };
        label(a, F::Body, &ellipsize(a, F::Body, &mine, g.full, 7.0), g.cx, g.mine_top(), 7.0, p.muted);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Courier: every glyph is 0.6 em wide (style::tw for the system font).
    fn mono(size: f32) -> impl Fn(&str) -> f32 {
        move |t: &str| t.chars().count() as f32 * 0.6 * size
    }

    fn valid(slim: bool) -> ValidSkin {
        ValidSkin { png: vec![1, 2, 3], hash: "h".into(), slim, rgba: Vec::new(), legacy: false }
    }

    #[test]
    fn apply_is_greyed_until_a_skin_is_picked() {
        let mut s = SkinUi::default();
        assert!(!apply_enabled(&s), "nothing picked: APPLY is off");
        set_pick(&mut s, valid(false), "night.png");
        assert!(apply_enabled(&s), "a picked skin turns APPLY on");
        assert!(s.status.contains("night.png"), "the status names the previewed file: {}", s.status);
        assert!(s.status.contains("classic arms"));
        set_pick(&mut s, valid(true), "alex.png");
        assert!(s.status.contains("slim arms"));
    }

    #[test]
    fn default_restores_the_default_skin() {
        let mut st = Settings::default();
        st.skin = "CUSTOM".to_string();
        st.skin_slim = true;
        let mut s = SkinUi::default();
        s.applied_name = "x.png".into();
        set_pick(&mut s, valid(false), "x.png");
        reset_to_default(&mut st, &mut s);
        assert_eq!(st.skin, SKIN_DEFAULT);
        assert!(!st.skin_slim, "the default skin has classic arms");
        assert!(!apply_enabled(&s), "DEFAULT drops the picked skin too");
        assert_eq!(mine_text(&st, &s), "Your skin: default");
    }

    #[test]
    fn a_refused_source_shows_the_real_reason_and_names_the_file() {
        let mut s = SkinUi::default();
        set_pick(&mut s, valid(false), "a.png");
        refuse_file(&mut s, "noisy.png", "the file is too big");
        assert_eq!(s.status, "Not applied: noisy.png: the file is too big.");
        assert!(!apply_enabled(&s), "a refused file clears the pick");
    }

    #[test]
    fn your_skin_names_the_applied_file() {
        let mut st = Settings::default();
        let mut s = SkinUi::default();
        assert_eq!(mine_text(&st, &s), "Your skin: default");
        st.skin = "CUSTOM".into();
        assert_eq!(mine_text(&st, &s), "Your skin: custom", "after a restart the file name is not known");
        s.applied_name = "night.png".into();
        assert_eq!(mine_text(&st, &s), "Your skin: custom (night.png)");
    }

    #[test]
    fn arms_cycle_auto_classic_slim() {
        assert_eq!(next_arms("auto"), "classic");
        assert_eq!(next_arms("classic"), "slim");
        assert_eq!(next_arms("slim"), "auto");
    }

    #[test]
    fn skin_tab_defaults_match_godot() {
        assert!(Settings::default().skin_copy, "OTHERS MAY COPY is on by default (settings.gd skin_copyable)");
        assert_eq!(Settings::default().arms, "auto");
    }

    #[test]
    fn arms_persists_through_the_settings_struct() {
        let mut st = Settings::default();
        let mut seen = vec![st.arms.clone()];
        for _ in 0..3 {
            st.arms = next_arms(&st.arms).to_string();
            seen.push(st.arms.clone());
        }
        assert_eq!(seen, ["auto", "classic", "slim", "auto"]);
    }

    #[test]
    fn arms_label_shows_the_effective_style_for_auto() {
        assert_eq!(arms_label("auto", true), "ARMS: AUTO (SLIM)");
        assert_eq!(arms_label("auto", false), "ARMS: AUTO (CLASSIC)");
        assert_eq!(arms_label("slim", false), "ARMS: SLIM");
    }

    #[test]
    fn auto_arms_follow_the_skins_layout_and_the_others_force_it() {
        assert!(preview_slim("auto", true), "an Alex skin under AUTO previews slim");
        assert!(!preview_slim("auto", false));
        assert!(!preview_slim("classic", true));
        assert!(preview_slim("slim", false));
    }

    #[test]
    fn the_status_and_arms_label_fit_their_boxes_at_every_window_size() {
        for (w, h) in [(1280.0f32, 720.0f32), (1920.0, 1080.0), (844.0, 390.0)] {
            // the grid is in design units and does not depend on the window: the same boxes at every size
            let u = unit_for(w, h, true);
            let view = (w / u, h / u);
            let (ox, oy) = ((view.0 - 440.0) * 0.5, (view.1 - 254.0) * 0.5);
            let g = grid(ox, oy);
            let (toggle, arms) = g.options();
            assert!(toggle.right() < arms.x, "{w}x{h}: the toggle and ARMS do not touch");
            assert!((arms.right() - (g.bx + g.gw)).abs() < 1e-3, "{w}x{h}: ARMS ends where the right column ends");
            for a in ARMS {
                for slim in [false, true] {
                    let label = arms_label(a, slim);
                    assert!(mono(7.0)(&label) + 8.0 <= arms.w, "{w}x{h}: {label} fits {} units", arms.w);
                }
            }
            let toggle_text = mono(7.0)(SHARE_LABEL) + 10.0 + 20.0;
            assert!(toggle_text <= toggle.w, "{w}x{h}: the toggle's caption and chip fit");
            let default = g.default_button();
            assert!(mono(7.0)(DEFAULT_LABEL) + 8.0 <= default.w, "{w}x{h}: DEFAULT fits {} units", default.w);
            assert!((default.right() - (g.bx + g.gw)).abs() < 1e-3, "{w}x{h}: DEFAULT is as wide as the column");
            // the status block ends above ESC CLOSE (oy + 234): three lines, then "Your skin"
            assert!(g.status_top() + 3.0 * STATUS_STEP <= g.mine_top(), "three status lines stay clear of \"Your skin\"");
            assert!(g.mine_top() + 8.0 <= oy + 234.0, "{w}x{h}: \"Your skin\" sits above the close button");
            assert!(g.status_top() > arms.bottom(), "{w}x{h}: the status starts under the last row");
            let pv = g.preview(ox);
            assert!(pv.right() < g.cx, "{w}x{h}: the preview ends before the buttons start");
        }
    }

    #[test]
    fn a_long_unbroken_path_wraps_inside_the_status_box() {
        let path = "Not applied: a-very-long-skin-name.png in /Users/someone/Library/Application Support/KYTTU/TheLastRoute/skins/a/very/long/folder/name";
        let m = mono(6.0);
        let lines = wrap_to(path, 240.0, 3, &m);
        assert!(lines.len() <= 3);
        for l in &lines {
            assert!(m(l) <= 240.0 + 1e-3, "{l:?} is {} units", m(l));
        }
        let long_word = "x".repeat(200);
        let lines = wrap_to(&long_word, 240.0, 3, &m);
        assert_eq!(lines.len(), 3, "an unbroken word is cut between characters and capped at the line limit");
        assert!(lines.last().unwrap().ends_with('…'), "what does not fit ends in an ellipsis");
        assert!(lines.iter().all(|l| m(l) <= 240.0 + 1e-3));
        assert_eq!(wrap_to("short", 240.0, 3, &m), vec!["short".to_string()]);
        assert!(wrap_to("", 240.0, 3, &m).is_empty());
    }

    #[test]
    fn a_folder_path_is_shortened_from_the_left() {
        let m = mono(6.0);
        let shown = fit_tail("/Users/someone/Library/Application Support/KYTTU/TheLastRoute/skins", 100.0, &m);
        assert!(m(&shown) <= 100.0 && shown.starts_with('…') && shown.ends_with("skins"), "{shown}");
        assert_eq!(fit_tail("/a/b", 100.0, &m), "/a/b");
    }

    #[test]
    fn a_long_link_scrolls_to_keep_the_caret_inside_its_box() {
        let m = mono(7.0);
        let avail = 158.0 - 12.0;
        let link = "https://example.com/a/really/long/path/to/a/skin/that/is/much/wider/than/the/box.png";
        let end = link.chars().count();
        let (start, stop) = visible_window(link, end, avail, &|t| m(t));
        assert!(start > 0, "the text scrolls left so the end is visible");
        assert_eq!(stop, end);
        let shown: String = link.chars().skip(start).take(stop - start).collect();
        assert!(m(&shown) <= avail, "{} units fit {avail}", m(&shown));
        let (s0, e0) = visible_window(link, 0, avail, &|t| m(t));
        assert_eq!(s0, 0, "the caret at the start shows the start");
        assert!(e0 < end && m(&link.chars().take(e0).collect::<String>()) <= avail);
        assert_eq!(visible_window("short", 5, avail, &|t| m(t)), (0, 5), "short text is shown whole");
    }

    #[test]
    fn text_editing_moves_the_caret_inserts_and_deletes_there() {
        let mut t = TextBox::default();
        for c in "abcd".chars() {
            assert!(t.insert(c, 4));
        }
        assert!(!t.insert('e', 4), "the cap holds");
        t.left();
        t.left();
        t.insert('X', 5);
        assert_eq!(t.text, "abXcd");
        t.backspace();
        assert_eq!(t.text, "abcd");
        t.delete();
        assert_eq!(t.text, "abd");
        t.home();
        t.backspace();
        assert_eq!(t.text, "abd", "backspace at the start does nothing");
        t.end();
        t.right();
        assert_eq!(t.caret, 3);
        t.delete();
        assert_eq!(t.text, "abd");
    }

    #[test]
    fn a_pasted_skin_url_with_a_trailing_newline_lands_whole() {
        let mut t = TextBox::default();
        link_keys(&mut t, true, 512, true, &[], Some("https://example.com/skins/night.png\r\n"));
        assert_eq!(t.text, "https://example.com/skins/night.png", "the newline goes, the link lands whole");
        assert_eq!(t.caret, t.len(), "the caret is at the end after a paste");
    }

    #[test]
    fn a_paste_into_an_unfocused_link_box_is_ignored() {
        let mut t = TextBox::default();
        link_keys(&mut t, false, 512, true, &[], Some("https://example.com/a.png"));
        assert_eq!(t, TextBox::default(), "a paste while the box has no keyboard does nothing");
        link_keys(&mut t, false, 512, false, &['x'], None);
        assert_eq!(t, TextBox::default(), "typing does nothing either");
    }

    #[test]
    fn a_paste_is_capped_by_the_room_left() {
        let mut t = TextBox { text: "https://".to_string(), caret: 8 };
        link_keys(&mut t, true, 12, true, &[], Some("example.com/a.png"));
        assert_eq!(t.text, "https://exam", "only the room left (4 of 12) is filled");
    }

    #[test]
    fn typed_keys_skip_the_command_leftovers() {
        let mut t = TextBox::default();
        link_keys(&mut t, true, 512, true, &['v', '\u{16}'], None);
        assert_eq!(t.text, "", "a letter that comes with Ctrl or Cmd is not typed");
        link_keys(&mut t, true, 512, false, &['a', ' ', 'b'], None);
        assert_eq!(t.text, "ab", "a space is refused, the letters are typed");
    }

    #[test]
    fn a_held_key_repeats_after_the_delay() {
        let mut r = Repeat::default();
        let (delay, per_sec) = (0.4, 30.0);
        assert!(r.fire(true, 0.0, delay, per_sec), "fires on the press");
        assert!(!r.fire(true, 0.1, delay, per_sec), "not before the delay");
        assert!(!r.fire(true, 0.39, delay, per_sec));
        assert!(r.fire(true, 0.41, delay, per_sec), "starts repeating after the delay");
        assert!(!r.fire(true, 0.42, delay, per_sec), "at the repeat rate, not every frame");
        assert!(r.fire(true, 0.45, delay, per_sec));
        assert!(!r.fire(false, 0.5, delay, per_sec), "released");
        assert!(r.fire(true, 0.6, delay, per_sec), "a new press fires again");
        let rules = skins::rules().expect("rules");
        assert_eq!((rules.repeat_delay_secs, rules.repeat_per_sec), (0.4, 30.0), "the numbers come from data/skins/rules.json");
    }

    /// A skin file as the chooser hands it over.
    fn png_bytes(slim: bool) -> Vec<u8> {
        let mut px = vec![0u8; 64 * 64 * 4];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[40, 90, 160, 255]);
        }
        if slim {
            // a slim (Alex) skin has the arms' fourth texture column clear
            let layout = skins::layout().expect("the layout loads");
            for r in &layout.slim_probe_rects {
                for y in r[1]..r[1] + r[3] {
                    for x in r[0]..r[0] + r[2] {
                        px[((y * 64 + x) * 4 + 3) as usize] = 0;
                    }
                }
            }
        }
        skins::encode_png(&px, 64, 64)
    }

    fn picked(name: &str, bytes: Result<Vec<u8>, String>) -> Outcome {
        Outcome::Picked(vec![file_pick::PickedFile { name: name.to_string(), bytes }])
    }

    /// A tab that is waiting on a chooser that replaced the status line given (and a detail line).
    fn waiting(status: &str) -> SkinUi {
        let mut s = SkinUi::default();
        s.picking = Some(Picking { req: file_pick::test_request(1), status: status.to_string(), detail: "some folder".to_string() });
        s.status = "Choose a PNG in the file window.".to_string();
        s
    }

    #[test]
    fn a_chosen_png_goes_to_the_preview_and_waits_for_apply() {
        let mut s = waiting("Skin applied.");
        take_picked(&mut s, picked("night.png", Ok(png_bytes(false))));
        assert!(s.picking.is_none(), "the chooser has answered");
        assert!(apply_enabled(&s), "a valid skin turns APPLY on");
        assert!(s.status.contains("night.png") && s.status.contains("classic arms"), "{}", s.status);
        let mut s = waiting("");
        take_picked(&mut s, picked("alex.PNG", Ok(png_bytes(true))));
        assert!(s.status.contains("slim arms"), "the arm layout is read from the pixels: {}", s.status);
    }

    #[test]
    fn a_closed_chooser_changes_nothing() {
        let mut s = waiting("Skin applied.");
        s.pick = None;
        take_picked(&mut s, Outcome::Cancelled);
        assert!(s.picking.is_none());
        assert_eq!(s.status, "Skin applied.", "the status line the chooser replaced comes back");
        assert_eq!(s.detail, "some folder");
        assert!(!apply_enabled(&s), "no skin was picked");
        // a pick made earlier stays when a later chooser is closed
        let mut s = SkinUi::default();
        set_pick(&mut s, valid(false), "first.png");
        s.picking = Some(Picking { req: file_pick::test_request(2), status: s.status.clone(), detail: String::new() });
        take_picked(&mut s, Outcome::Cancelled);
        assert!(apply_enabled(&s) && s.pick.as_ref().unwrap().name == "first.png");
    }

    #[test]
    fn a_waiting_chooser_is_left_alone() {
        let mut s = waiting("x");
        take_picked(&mut s, Outcome::Pending);
        assert!(s.picking.is_some(), "still waiting");
        assert_eq!(s.status, "Choose a PNG in the file window.");
    }

    #[test]
    fn a_bad_pick_gives_its_real_reason_and_names_the_file() {
        let mut s = waiting("");
        take_picked(&mut s, picked("notes.txt", Ok(b"hello".to_vec())));
        assert_eq!(s.status, "Not applied: notes.txt: it is not a PNG file.");
        let mut s = waiting("");
        take_picked(&mut s, picked("big.png", Err("the file is too big".to_string())));
        assert_eq!(s.status, "Not applied: big.png: the file is too big.");
        assert!(!apply_enabled(&s));
        let mut s = waiting("");
        let mut broken = png_bytes(false);
        broken[40] ^= 1;
        take_picked(&mut s, picked("broken.png", Ok(broken)));
        assert!(s.status.starts_with("Not applied: broken.png: the PNG could not be read"), "{}", s.status);
        let mut s = waiting("");
        take_picked(&mut s, picked("small.png", Ok(skins::encode_png(&vec![255u8; 32 * 32 * 4], 32, 32))));
        assert!(s.status.starts_with("Not applied: small.png:"), "a picture that is not 64x64 is refused: {}", s.status);
        // a chooser that answered with no file at all changes nothing
        let mut s = waiting("kept");
        take_picked(&mut s, Outcome::Picked(Vec::new()));
        assert_eq!(s.status, "kept");
    }

    #[test]
    fn the_rows_have_no_gap_after_the_removals() {
        let flat: Vec<&str> = ROW_LABELS.iter().flat_map(|r| r.iter().copied()).collect();
        assert!(flat.iter().all(|l| !l.contains("TEMPLATE") && !l.contains("MC NAME")), "the removed buttons are gone: {flat:?}");
        assert_eq!(flat, ["PICK A PNG", "APPLY", "DEFAULT", "FROM URL", "COPY SKIN", "FROM CATALOG", "OTHERS MAY COPY"]);
        assert!(ROW_LABELS.iter().all(|r| !r.is_empty()), "every row has a control");
        let g = grid(0.0, 0.0);
        for n in 1..ROWS {
            assert!((g.row(n) - g.row(n - 1) - 18.0).abs() < 1e-3, "row {n} follows row {} with the same step", n - 1);
        }
        let (toggle, arms) = g.options();
        assert_eq!((toggle.y, arms.y), (g.row(ROWS - 1), g.row(ROWS - 1)), "the options are the last row");
        assert!(g.status_top() - (g.row(ROWS - 1) + 16.0) <= 4.0, "the status follows the last row without a blank gap");
        assert_eq!(g.default_button().y, g.row(1), "DEFAULT sits right under PICK A PNG");
        // every caption fits its button
        let m = mono(7.0);
        let half = (g.full - 2.0) * 0.5;
        assert!(m(PICK_LABEL) + 8.0 <= half && m(APPLY_LABEL) + 8.0 <= half);
        assert!(m(URL_LABEL) + 8.0 <= g.gw && m(COPY_LABEL) + 8.0 <= g.gw && m(CATALOG_LABEL) + 8.0 <= g.gw);
    }

    #[test]
    fn the_chooser_is_asked_for_a_png() {
        assert_eq!(file_pick::extensions_of(PNG_KIND), ["png"]);
    }

    #[test]
    fn a_dropped_or_chosen_file_needs_a_png_name() {
        let mut s = SkinUi::default();
        take_file(&mut s, "skin.png", Ok(png_bytes(false)));
        assert!(apply_enabled(&s));
        take_file(&mut s, "skin.jpg", Ok(png_bytes(false)));
        assert!(!apply_enabled(&s), "a refused file clears the pick");
        assert_eq!(s.status, "Not applied: skin.jpg: it is not a PNG file.");
    }

    #[test]
    fn escape_is_taken_once_and_resets() {
        ESCAPE_TAKEN.store(true, Ordering::SeqCst);
        assert!(take_escape());
        assert!(!take_escape(), "reading it resets it, so the next Escape closes the panel");
    }
}
