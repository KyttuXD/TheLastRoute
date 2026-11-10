//! The in-game overlay, drawn after the 3D scene: the talk box and its choices, the toast, the crosshair and the
//! interaction prompt, the map-name banner, the player panel (name, role), the small fading hotbar, the stamina bar and a light vignette.
//! It also holds the pause menu and the loading and error screens. Everything is drawn with the shared look in ui.rs.
//!
//! The rules that are not pixels (which choices fit on screen, what the prompt says, how a banner fades, how the typed
//! text is cut into rows) are plain functions at the top of the file, and tested.

use crate::body::MAX_STAMINA;
use crate::input::Action;
use crate::model::{Era, ItemUse, Kind, PropAction, PropUse};
use crate::model::WeatherKind;
use crate::weather::Weather;
use crate::inventory::Stack;
use crate::play::{Game, Talk};
use crate::ui;
use macroquad::prelude::*;
use std::collections::{BTreeSet, HashMap};

/// The item icon textures, loaded once from the pack (main.rs), by item id. An item without one is drawn as a colour square.
pub type Icons = HashMap<String, Texture2D>;

// ---- rules that are not pixels ----

/// Fast start, slow finish: 0 at 0, 1 at 1 (clamped).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Seconds the map banner fades in, stays, and is gone by.
const BANNER_IN: f32 = 0.6;
const BANNER_HOLD: f32 = 3.4;
const BANNER_END: f32 = 4.4;

/// The longest NPC name a tag shows, in characters.
#[allow(dead_code)]
pub const TAG_MAX_CHARS: usize = 24;

/// The text of an NPC name tag: the trimmed name, cut with an ellipsis past `max_chars` characters; None for a blank
/// name (nothing is drawn). Not wired into the tag drawing yet because that is in main.rs.
#[allow(dead_code)]
pub fn clip_tag(name: &str, max_chars: usize) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || max_chars == 0 {
        return None;
    }
    if name.chars().count() <= max_chars {
        return Some(name.to_string());
    }
    let kept: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    Some(format!("{}…", kept.trim_end()))
}

/// The map banner's opacity `age` seconds after the map changed.
pub fn banner_alpha(age: f32) -> f32 {
    if age < 0.0 || age >= BANNER_END {
        0.0
    } else if age < BANNER_IN {
        age / BANNER_IN
    } else if age < BANNER_HOLD {
        1.0
    } else {
        1.0 - (age - BANNER_HOLD) / (BANNER_END - BANNER_HOLD)
    }
}

/// The part of `total` choices to draw when only `max_vis` fit: (first, one past the last). The window keeps `sel` inside
/// it, as near the middle as the list ends allow, so a long list scrolls instead of running off the screen.
pub fn choice_window(total: usize, sel: usize, max_vis: usize) -> (usize, usize) {
    let max_vis = max_vis.max(1);
    if total <= max_vis {
        return (0, total);
    }
    let sel = sel.min(total - 1);
    let start = sel.saturating_sub(max_vis / 2).min(total - max_vis);
    (start, start + max_vis)
}

/// The rows of a wrapped line that the typewriter has revealed after `shown` characters of a line `total` characters long.
/// Wrapping drops the spaces it breaks at, so each row costs its characters plus one.
pub fn reveal_rows(rows: &[String], shown: usize, total: usize) -> Vec<String> {
    if shown >= total {
        return rows.to_vec();
    }
    let mut left = shown;
    let mut out = Vec::new();
    for row in rows {
        if left == 0 {
            break;
        }
        let n = row.chars().count();
        out.push(row.chars().take(left).collect());
        left = left.saturating_sub(n + 1);
    }
    out
}

/// `s` cut short with "..." so that `measure` says it is no wider than `max_w`. Unchanged when it already fits.
pub fn ellipsize(s: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(s) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "...";
        if measure(&candidate) <= max_w {
            return candidate;
        }
    }
    "...".to_string()
}

/// What the facing tile offers: a line under the crosshair.
#[derive(Debug, PartialEq)]
pub struct Prompt {
    pub text: String,
    /// A shut door (drawn in the warning colour).
    pub locked: bool,
}

/// The prompt for the tile in front of the player. An NPC can be talked to; a door that still blocks says why.
pub fn prompt_for(kind: Option<&Kind>, flags: &BTreeSet<String>, inv: &[String]) -> Option<Prompt> {
    match kind? {
        Kind::Npc { name, .. } => Some(Prompt { text: format!("Talk to {name}"), locked: false }),
        k @ Kind::Door { locked, .. } if k.blocks(flags, inv) => Some(Prompt {
            text: if locked.is_empty() { "Locked".to_string() } else { format!("Locked: {locked}") },
            locked: true,
        }),
        _ => None,
    }
}

/// The prompt word of a usable prop: the legend's own `text`, else the word of its action.
pub fn use_word(u: &PropUse) -> String {
    if u.text.trim().is_empty() {
        action_word(u.action).to_string()
    } else {
        u.text.clone()
    }
}

/// The default prompt word of each prop action.
pub fn action_word(a: PropAction) -> &'static str {
    match a {
        PropAction::Sit => "Sit",
        PropAction::Lean => "Lean",
        PropAction::Work => "Work",
        PropAction::DrawWater => "Draw water",
        PropAction::Read => "Read",
        PropAction::Sleep => "Sleep",
        PropAction::WarmHands => "Warm hands",
        PropAction::Look => "Look",
    }
}

/// The prompt line under the crosshair. Seated, the only thing to do is stand up (any movement key does it). Else an NPC
/// close in front can be talked to (`talk_to`: its name), else the facing tile offers its door or usable prop.
pub fn prompt_line(talk_to: Option<&str>, seated: bool, kind: Option<&Kind>, flags: &BTreeSet<String>, inv: &[String]) -> Option<Prompt> {
    if seated {
        return Some(Prompt { text: "Stand up".to_string(), locked: false });
    }
    if let Some(name) = talk_to {
        return Some(Prompt { text: format!("Talk to {name}"), locked: false });
    }
    if let Some(Kind::Prop { use_: Some(u), .. }) = kind {
        return Some(Prompt { text: use_word(u), locked: false });
    }
    // An NPC tile alone says nothing: the NPC walks off its tile, so only one close in front (above) is talked to.
    if matches!(kind, Some(Kind::Npc { .. })) {
        return None;
    }
    prompt_for(kind, flags, inv)
}

/// The index of the nearest NPC (name, position in metres) within `range` metres of `from` and in front of a body facing
/// `fwd` (a unit vector on the ground): the one E would talk to. Behind or beside the body (more than about 70 degrees
/// off the facing) does not count. A body standing on an NPC (under 0.3 m) counts, so it is never unreachable.
#[cfg(test)]
pub fn nearest_in_front(from: (f32, f32), fwd: (f32, f32), npcs: &[(&str, (f32, f32))], range: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (k, (_, p)) in npcs.iter().enumerate() {
        let (dx, dz) = (p.0 - from.0, p.1 - from.1);
        let d = (dx * dx + dz * dz).sqrt();
        if d > range {
            continue;
        }
        if d > 0.3 && (dx * fwd.0 + dz * fwd.1) / d < 0.35 {
            continue;
        }
        if best.map_or(true, |(_, bd)| d < bd) {
            best = Some((k, d));
        }
    }
    best.map(|(k, _)| k)
}

/// The weather label under the map name ("Light rain", "Storm"); None when the weather is clear.
pub fn weather_label(kind: WeatherKind, intensity: f32) -> Option<String> {
    let strength = if intensity < 0.35 {
        "Light "
    } else if intensity > 0.75 {
        "Heavy "
    } else {
        ""
    };
    let word = match kind {
        WeatherKind::Clear => return None,
        WeatherKind::Rain => "rain",
        WeatherKind::Snow => "snow",
        WeatherKind::Fog => "fog",
        WeatherKind::Storm => return Some("Storm".to_string()),
        WeatherKind::Leaves => return Some("Windy".to_string()),
    };
    let mut s = format!("{strength}{word}");
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    Some(s)
}

/// The weather of the map the player stands on (None on indoor maps, which have none).
pub fn weather_of(g: &Game) -> Option<&Weather> {
    g.weather.get(&g.map)
}

/// The NPC E would talk to now: its name, when one stands close in front of the body.
pub fn talk_target(g: &Game) -> Option<String> {
    // The same choice E makes (nearest in front, in reach, not behind a wall), so the prompt never promises a talk E will
    // not do.
    g.npc_ahead_name()
}


/// Whether the toast now showing is a new one (so its slide-in starts again): a different text, or more time left than
/// the last frame had (the same text was shown again).
pub fn toast_restarted(prev: Option<(&str, f32)>, text: &str, left: f32) -> bool {
    match prev {
        None => true,
        Some((t, l)) => t != text || left > l + 0.001,
    }
}

/// How far a toast has slid in, 0 (off the top) to 1 (in place): it slides in over a quarter second and out over the
/// last third of one.
pub fn toast_visible(age: f32, left: f32) -> f32 {
    ease_out(age / 0.25).min(ease_out(left / 0.35))
}

/// The nameplate colour of a speaker: the same name always gets the same colour.
pub fn speaker_color(name: &str) -> Color {
    const COLORS: [Color; 6] = [
        Color::new(0.55, 0.75, 1.0, 1.0),
        Color::new(0.95, 0.62, 0.45, 1.0),
        Color::new(0.55, 0.90, 0.75, 1.0),
        Color::new(0.80, 0.62, 1.0, 1.0),
        Color::new(0.95, 0.80, 0.40, 1.0),
        Color::new(0.95, 0.55, 0.65, 1.0),
    ];
    let h = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[(h % COLORS.len() as u32) as usize]
}

/// The opacity of spinner tick `i` of `n` at time `t`: the lit tick runs round, the rest trail off behind it.
pub fn spinner_alpha(i: usize, n: usize, t: f32) -> f32 {
    let head = (t * 1.6).rem_euclid(1.0);
    let me = i as f32 / n as f32;
    let behind = (head - me).rem_euclid(1.0);
    (1.0 - behind).powi(2).max(0.12)
}

/// The size factor for the screen: 1.0 at 720 pixels tall.
/// The HUD scale: the screen height against 720p, times the player's UI scale setting (0.8 to 1.3).
pub fn scale_for(sh: f32) -> f32 {
    (sh / 720.0).clamp(0.75, 1.6) * crate::settings::get().ui_scale
}

/// The selection `delta` steps from `sel` in a list of `n` rows, wrapping round both ends (0 for an empty list).
pub fn wrap_sel(sel: usize, delta: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (sel.min(n - 1) as i32 + delta).rem_euclid(n as i32) as usize
}

/// `hp` as a fraction of `max`, clamped to 0..1 (0 when `max` is not positive).
pub fn hp_frac(hp: i32, max: i32) -> f32 {
    if max <= 0 {
        0.0
    } else {
        (hp as f32 / max as f32).clamp(0.0, 1.0)
    }
}

/// Which quest the tracker shows and which of its steps is next. `quests` holds, per quest in log order, whether it is
/// done and its steps as (text, done). The first quest that is not done wins; its next step is the first step not done
/// (None when every step is done but the quest is not).
pub fn tracker_pick(quests: &[(bool, &[(String, bool)])]) -> Option<(usize, Option<usize>)> {
    let q = quests.iter().position(|(done, _)| !*done)?;
    Some((q, quests[q].1.iter().position(|(_, d)| !*d)))
}

/// What an item's use does, in words.
pub fn describe_use(u: &ItemUse) -> String {
    match u {
        ItemUse::Heal(n) => format!("Heals {n} HP"),
        ItemUse::Damage(n) => format!("Deals {n} damage in a fight"),
    }
}

fn ui_fit(s: &str, max_w: f32, size: f32) -> String {
    ellipsize(s, max_w, &|x| ui::measure(x, size))
}

// ---- the hotbar and the inventory grid: rules that are not pixels ----

/// How many hotbar slots there are (the keys 1 to 9).
pub const HOTBAR_SLOTS: usize = 9;
/// Seconds without a change before the hotbar starts to fade.
pub const HOTBAR_IDLE: f32 = 4.0;
/// Seconds the fade takes.
pub const HOTBAR_FADE: f32 = 0.6;
/// The opacity the idle hotbar settles at.
pub const HOTBAR_DIM: f32 = 0.35;
/// Seconds the selected item's name stays above the bar.
pub const NAME_SHOW: f32 = 1.6;
/// Columns of the bag grid in the inventory screen (the same nine as the hotbar).
pub const BAG_COLS: usize = 9;

/// The hotbar's opacity after `idle` seconds with no change: full for 4 s, then easing to 35%.
pub fn hotbar_alpha(idle: f32) -> f32 {
    if idle <= HOTBAR_IDLE {
        return 1.0;
    }
    let t = ((idle - HOTBAR_IDLE) / HOTBAR_FADE).clamp(0.0, 1.0);
    1.0 - (1.0 - HOTBAR_DIM) * t
}

/// The opacity of the selected item's name `age` seconds after the selection (0 once it is gone).
pub fn name_alpha(age: f32) -> f32 {
    if !(0.0..NAME_SHOW).contains(&age) {
        return 0.0;
    }
    ((NAME_SHOW - age) / 0.4).clamp(0.0, 1.0)
}

/// The nine hotbar slots: squares of about 34 px at 720p with 4 px gaps, bottom centre.
pub fn hotbar_rects(sw: f32, sh: f32) -> [Rect; HOTBAR_SLOTS] {
    let k = scale_for(sh);
    let (size, gap) = (34.0 * k, 4.0 * k);
    let total = HOTBAR_SLOTS as f32 * size + (HOTBAR_SLOTS as f32 - 1.0) * gap;
    let x0 = ((sw - total) / 2.0).max(0.0);
    let y = sh - 22.0 - size;
    let mut out = [Rect::new(0.0, 0.0, size, size); HOTBAR_SLOTS];
    for (i, r) in out.iter_mut().enumerate() {
        r.x = x0 + i as f32 * (size + gap);
        r.y = y;
    }
    out
}

/// A stable colour (r, g, b in 0..1) for an item id, for items without an icon: FNV-1a of the id picks the hue.
pub fn item_color(id: &str) -> [f32; 3] {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    let hue = (h % 360) as f32 / 60.0;
    let (s, v) = (0.45_f32, 0.78_f32);
    let c = v * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// The first letter or digit of a name, upper case ('?' for none).
pub fn item_initial(name: &str) -> char {
    name.chars().find(|c| c.is_alphanumeric()).map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
}

/// What the number keys do right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigitRoute {
    /// Pick a dialogue choice.
    Choice,
    /// Select a hotbar slot.
    Slot,
    /// A full screen is open and reads them itself (inventory: move to slot).
    Panel,
    /// Nothing (a line is being read).
    None,
}

/// Routes the digits. `talk` is None without a dialogue, else whether it is showing choices.
pub fn digit_route(talk: Option<bool>, panel_open: bool) -> DigitRoute {
    if panel_open {
        return DigitRoute::Panel;
    }
    match talk {
        Some(true) => DigitRoute::Choice,
        Some(false) => DigitRoute::None,
        None => DigitRoute::Slot,
    }
}

/// The wheel as a selection step: scrolling up goes to the previous slot, down to the next (0 for no scroll).
pub fn wheel_step(dy: f32) -> i32 {
    if dy > 0.0 {
        -1
    } else if dy < 0.0 {
        1
    } else {
        0
    }
}

/// The digit key pressed this frame, as a slot index 0..9.
pub fn digit_pressed() -> Option<usize> {
    const DIGITS: [KeyCode; HOTBAR_SLOTS] = [
        KeyCode::Key1,
        KeyCode::Key2,
        KeyCode::Key3,
        KeyCode::Key4,
        KeyCode::Key5,
        KeyCode::Key6,
        KeyCode::Key7,
        KeyCode::Key8,
        KeyCode::Key9,
    ];
    DIGITS.iter().position(|k| is_key_pressed(*k))
}

/// What the hotbar last showed, so a change (pick-up, use, move, selection) can be noticed.
#[derive(Clone, Debug, PartialEq)]
pub struct HotbarState {
    idle: f32,
    name_age: f32,
    sel: usize,
    snap: Vec<Option<(String, u32)>>,
}

impl Default for HotbarState {
    fn default() -> HotbarState {
        HotbarState { idle: 0.0, name_age: 99.0, sel: 0, snap: Vec::new() }
    }
}

impl HotbarState {
    /// One frame: a changed snapshot or selection brings the bar back to full; a new selection shows the name.
    pub fn update(&mut self, snap: Vec<Option<(String, u32)>>, sel: usize, dt: f32) {
        if sel != self.sel {
            self.name_age = 0.0;
            self.idle = 0.0;
        } else if snap != self.snap {
            self.idle = 0.0;
        } else {
            self.idle += dt;
        }
        self.name_age += dt;
        self.sel = sel;
        self.snap = snap;
    }

    /// A key or the wheel was used: back to full, and show the name again.
    pub fn poke(&mut self) {
        self.idle = 0.0;
        self.name_age = 0.0;
    }

    pub fn alpha(&self) -> f32 {
        hotbar_alpha(self.idle)
    }
}

/// The hotbar's content as plain data.
pub fn hotbar_snapshot(slots: &[Option<Stack>]) -> Vec<Option<(String, u32)>> {
    slots.iter().map(|s| s.as_ref().map(|s| (s.item.clone(), s.count))).collect()
}

/// Where the cursor is in the inventory screen: a hotbar slot (the top row) or a place in the bag grid below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Slot(usize),
    Bag(usize),
}

/// Puts a cursor back on something that exists (the bag shrinks when an item is moved out of it).
pub fn cursor_fix(c: Cursor, bag_len: usize) -> Cursor {
    match c {
        Cursor::Slot(i) => Cursor::Slot(i.min(HOTBAR_SLOTS - 1)),
        Cursor::Bag(j) if j < bag_len => c,
        Cursor::Bag(j) if bag_len == 0 => Cursor::Slot(j % BAG_COLS),
        Cursor::Bag(_) => Cursor::Bag(bag_len - 1),
    }
}

/// Moves the cursor one step (`dx` columns, `dy` rows; only the sign counts). Left and right wrap within the row, down
/// from the hotbar enters the bag in the same column (or its last item), up from the bag's first row returns to the hotbar.
pub fn cursor_move(c: Cursor, dx: i32, dy: i32, bag_len: usize) -> Cursor {
    let c = cursor_fix(c, bag_len);
    match c {
        Cursor::Slot(i) => {
            if dx != 0 {
                Cursor::Slot(wrap_sel(i, dx.signum(), HOTBAR_SLOTS))
            } else if dy > 0 && bag_len > 0 {
                Cursor::Bag(i.min(bag_len - 1))
            } else {
                c
            }
        }
        Cursor::Bag(j) => {
            let (row, col) = (j / BAG_COLS, j % BAG_COLS);
            if dx != 0 {
                let row_len = (bag_len - row * BAG_COLS).min(BAG_COLS);
                Cursor::Bag(row * BAG_COLS + wrap_sel(col, dx.signum(), row_len))
            } else if dy < 0 {
                if row == 0 {
                    Cursor::Slot(col)
                } else {
                    Cursor::Bag(j - BAG_COLS)
                }
            } else if dy > 0 {
                let below = j + BAG_COLS;
                if below < bag_len {
                    Cursor::Bag(below)
                } else if (row + 1) * BAG_COLS < bag_len {
                    Cursor::Bag(bag_len - 1)
                } else {
                    c
                }
            } else {
                c
            }
        }
    }
}

/// The inventory screen's grid, inside the panel `r`: the hotbar row, the bag rows under it, and where the details go.
#[derive(Clone, Copy, Debug)]
pub struct InvLayout {
    pub cell: f32,
    pub gap: f32,
    pub x: f32,
    pub hot_y: f32,
    pub bag_y: f32,
    pub bag_rows: usize,
    pub detail_x: f32,
    pub detail_w: f32,
}

pub fn inv_layout(r: Rect, k: f32) -> InvLayout {
    let gap = 6.0 * k;
    let x = r.x + 24.0 * k;
    let avail = r.w * 0.62 - 24.0 * k;
    let cell = ((avail - 8.0 * gap) / BAG_COLS as f32).clamp(20.0, 56.0 * k);
    let hot_y = r.y + 94.0 * k;
    let bag_y = hot_y + cell + 40.0 * k;
    let room = r.y + r.h - 20.0 * k - bag_y;
    let bag_rows = (((room + gap) / (cell + gap)).floor().max(1.0)) as usize;
    let detail_x = x + BAG_COLS as f32 * cell + 8.0 * gap + 28.0 * k;
    InvLayout { cell, gap, x, hot_y, bag_y, bag_rows, detail_x, detail_w: (r.x + r.w - 24.0 * k - detail_x).max(0.0) }
}

impl InvLayout {
    pub fn slot_rect(&self, i: usize) -> Rect {
        Rect::new(self.x + i as f32 * (self.cell + self.gap), self.hot_y, self.cell, self.cell)
    }
    /// The rect of bag cell `j` when the grid is scrolled to start at row `first_row`.
    pub fn bag_rect(&self, j: usize, first_row: usize) -> Rect {
        let (row, col) = (j / BAG_COLS - first_row, j % BAG_COLS);
        Rect::new(self.x + col as f32 * (self.cell + self.gap), self.bag_y + row as f32 * (self.cell + self.gap), self.cell, self.cell)
    }
}

/// The stack under the cursor.
fn stack_at(g: &Game, c: Cursor) -> Option<&Stack> {
    match c {
        Cursor::Slot(i) => g.items.slots.get(i).and_then(|s| s.as_ref()),
        Cursor::Bag(j) => g.items.bag.get(j),
    }
}

fn item_name(g: &Game, id: &str) -> String {
    g.pack.items.get(id).map_or_else(|| id.to_string(), |i| i.name.clone())
}

/// One item slot: dark translucent fill, thin border, the icon or a hashed colour square with the first letter, a tiny
/// count in the corner when more than 1. `a` fades the whole slot.
fn draw_item_slot(r: Rect, stack: Option<&Stack>, name: &str, icons: &Icons, border: Color, line: f32, a: f32, k: f32) {
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.04, 0.05, 0.08, 0.62 * a));
    draw_rectangle_lines(r.x, r.y, r.w, r.h, line, ui::fade(border, a));
    let Some(s) = stack else { return };
    let pad = r.w * 0.14;
    let (ix, iy, iw) = (r.x + pad, r.y + pad, r.w - 2.0 * pad);
    match icons.get(&s.item) {
        Some(t) => draw_texture_ex(
            t,
            ix,
            iy,
            Color::new(1.0, 1.0, 1.0, a),
            DrawTextureParams { dest_size: Some(vec2(iw, iw)), ..Default::default() },
        ),
        None => {
            let c = item_color(&s.item);
            draw_rectangle(ix, iy, iw, iw, Color::new(c[0], c[1], c[2], a));
            let letter = item_initial(name).to_string();
            let size = (iw * 0.62).max(10.0);
            ui::text(&letter, ix + (iw - ui::measure(&letter, size)) / 2.0, iy + iw * 0.5 + size * 0.34, size, Color::new(0.05, 0.06, 0.09, a));
        }
    }
    if s.count > 1 {
        let size = (11.0 * k).max(9.0);
        let t = s.count.to_string();
        ui::text_shadow(&t, r.x + r.w - ui::measure(&t, size) - 2.0 * k, r.y + r.h - 2.0 * k, size, ui::fade(ui::TEXT, a));
    }
}

// ---- the overlay ----

/// The overlay's own clock: the seconds the map banner and the toast have been showing.
pub struct Hud {
    map: String,
    banner_age: f32,
    last_toast: Option<(String, f32)>,
    toast_age: f32,
    time: f32,
    hotbar: HotbarState,
}

impl Default for Hud {
    fn default() -> Hud {
        Hud::new()
    }
}

impl Hud {
    pub fn new() -> Hud {
        // The banner is over until the first update sees the map (which starts it).
        Hud { map: String::new(), banner_age: BANNER_END, last_toast: None, toast_age: 0.0, time: 0.0, hotbar: HotbarState::default() }
    }

    /// Moves the clocks on one frame and notices a new map or a new toast.
    pub fn update(&mut self, g: &Game, dt: f32) {
        self.time += dt;
        self.hotbar.update(hotbar_snapshot(&g.items.slots), g.items.selected, dt);
        if self.map != g.map {
            self.map = g.map.clone();
            self.banner_age = 0.0;
        } else {
            self.banner_age += dt;
        }
        match g.toast.as_ref() {
            Some((text, left)) => {
                let prev = self.last_toast.as_ref().map(|(t, l)| (t.as_str(), *l));
                if toast_restarted(prev, text, *left) {
                    self.toast_age = 0.0;
                } else {
                    self.toast_age += dt;
                }
                self.last_toast = Some((text.clone(), *left));
            }
            None => self.last_toast = None,
        }
    }

    /// A number key, the wheel or F was used: the hotbar comes back to full and shows the selected name.
    pub fn poke_hotbar(&mut self) {
        self.hotbar.poke();
    }

    /// The small hotbar, bottom centre. Main calls it only while the world is live (not paused, no screen open); it
    /// hides itself during a dialogue.
    pub fn draw_hotbar(&self, g: &Game, icons: &Icons) {
        if g.talk.is_some() {
            return;
        }
        let (sw, sh) = (screen_width(), screen_height());
        let k = scale_for(sh);
        let a = if crate::settings::get().hotbar_fade { self.hotbar.alpha() } else { 1.0 };
        let rects = hotbar_rects(sw, sh);
        for (i, base) in rects.iter().enumerate() {
            let selected = i == g.items.selected;
            let mut r = *base;
            if selected {
                r.y -= 2.0 * k;
            }
            let stack = g.items.slots[i].as_ref();
            let name = stack.map(|s| item_name(g, &s.item)).unwrap_or_default();
            draw_item_slot(r, stack, &name, icons, if selected { ui::ACCENT } else { ui::BORDER }, if selected { 2.0 } else { 1.0 }, a, k);
            let d = (i + 1).to_string();
            let size = (10.0 * k).max(8.0);
            ui::text(&d, base.x + (base.w - ui::measure(&d, size)) / 2.0, base.y - 4.0 * k, size, ui::fade(if selected { ui::ACCENT } else { ui::DIM }, a));
        }
        let na = name_alpha(self.hotbar.name_age);
        if na > 0.0 {
            if let Some(s) = g.items.slots.get(g.items.selected).and_then(|s| s.as_ref()) {
                let name = item_name(g, &s.item);
                let size = 15.0 * k;
                ui::text_shadow(&name, sw / 2.0 - ui::measure(&name, size) / 2.0, rects[0].y - 16.0 * k, size, ui::fade(ui::TEXT, na));
            }
        }
    }

    /// Draws the whole overlay over the 3D view (call with the default camera set).
    pub fn draw(&self, g: &Game) {
        let (sw, sh) = (screen_width(), screen_height());
        let k = scale_for(sh);
        ui::vignette(0.35);
        let era = g.pack.story(&g.story).map_or(Era::Sample, |s| s.era);
        let talking = g.talk.is_some();
        let fighting = g.battle.is_some();

        // map name, top left; story title, top right
        ui::text_shadow(&g.current_map().name, 18.0, 30.0 * k, 20.0 * k, ui::MUTED);
        // The weather, small, under the map name (outdoor maps only; clear weather says nothing).
        if g.current_map().outdoor {
            if let Some(label) = weather_of(g).and_then(|w| weather_label(w.kind, w.intensity)) {
                ui::text_shadow(&label, 18.0, 46.0 * k, 14.0 * k, ui::DIM);
            }
        }
        let story_title = g.pack.story(&g.story).map_or_else(|| g.story.clone(), |s| s.title.clone());
        let tw = ui::measure(&story_title, 16.0 * k);
        ui::text_shadow(&story_title, sw - tw - 18.0, 30.0 * k, 16.0 * k, ui::DIM);

        self.draw_vitals(g, k);
        if !talking {
            self.draw_tracker(g, sw, 44.0 * k, k);
            // In a fight the crosshair stays but the talk prompts go; the fight draws its own key row (battle_ui.rs).
            self.draw_crosshair_and_prompt(g, sw, sh, k, !fighting);
            self.draw_stamina(g, sw, sh, k);
        }
        if !talking && !fighting {
            self.draw_player_panel(g, sh, k, era);
        }
        self.draw_banner(g, sw, sh, k, era);
        if let Some(talk) = g.talk.as_ref() {
            self.draw_talk(g, talk, sw, sh, k);
        }
        self.draw_toast(sw, k);
    }

    /// Top left under the map name: the HP bar with its numbers.
    fn draw_vitals(&self, g: &Game, k: f32) {
        let p = &g.party;
        let (x, y) = (16.0, 44.0 * k);
        let bx = x;
        let bw = 210.0 * k;
        let low = hp_frac(p.hp, p.max_hp) < 0.3;
        let hp_bar = Rect::new(bx, y + 2.0 * k, bw, 18.0 * k);
        ui::bar(hp_bar, hp_frac(p.hp, p.max_hp), if low { ui::BAD } else { ui::GOOD });
        let label = format!("HP {}/{}", p.hp.max(0), p.max_hp);
        ui::text_shadow(&label, bx + 6.0 * k, hp_bar.y + 14.0 * k, 13.0 * k, ui::TEXT);
    }

    /// Top right under the story title: the first unfinished quest and its next step.
    fn draw_tracker(&self, g: &Game, sw: f32, top: f32, k: f32) {
        let log = g.quest_log();
        let view: Vec<(bool, &[(String, bool)])> = log.iter().map(|q| (q.done, q.steps.as_slice())).collect();
        let Some((qi, si)) = tracker_pick(&view) else { return };
        let q = &log[qi];
        let w = 280.0 * k;
        let x = sw - w - 16.0;
        let step = si.map(|i| q.steps[i].0.as_str()).unwrap_or("");
        let rows = ui::wrap(step, w - 36.0 * k, 14.0 * k);
        let h = 42.0 * k + rows.len() as f32 * 18.0 * k;
        let r = Rect::new(x, top, w, h);
        ui::panel(r, ui::fade(ui::PANEL, 0.8), Some(ui::ACCENT));
        ui::text("QUEST", r.x + 18.0 * k, r.y + 16.0 * k, 11.0 * k, ui::DIM);
        ui::text(&ui_fit(&q.title, w - 36.0 * k, 17.0 * k), r.x + 18.0 * k, r.y + 34.0 * k, 17.0 * k, ui::ACCENT);
        for (i, row) in rows.iter().enumerate() {
            ui::text(row, r.x + 18.0 * k, r.y + 54.0 * k + i as f32 * 18.0 * k, 14.0 * k, ui::MUTED);
        }
    }

    fn draw_crosshair_and_prompt(&self, g: &Game, sw: f32, sh: f32, k: f32, prompts: bool) {
        let (cx, cy) = (sw / 2.0, sh / 2.0);
        let (fx, fy) = g.facing_tile();
        let talk_to = if prompts && g.seated.is_none() { talk_target(g) } else { None };
        let prompt = if prompts {
            prompt_line(talk_to.as_deref(), g.seated.is_some(), g.kind_at(fx, fy), &g.flags, &g.inv)
        } else {
            None
        };
        let tint = match &prompt {
            Some(p) if p.locked => ui::BAD,
            Some(_) => ui::ACCENT,
            None => Color::new(1.0, 1.0, 1.0, 0.8),
        };
        let (gap, len) = (5.0 * k, 7.0 * k);
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let (x0, y0) = (cx + dx * gap, cy + dy * gap);
            let (x1, y1) = (cx + dx * (gap + len), cy + dy * (gap + len));
            draw_line(x0 + 1.0, y0 + 1.0, x1 + 1.0, y1 + 1.0, 2.0, ui::SHADOW);
            draw_line(x0, y0, x1, y1, 2.0, tint);
        }
        draw_circle(cx, cy, 1.5, tint);

        if let Some(p) = prompt {
            let size = 18.0 * k;
            let w = ui::measure(&p.text, size) + 28.0 * k;
            let (x, y) = (cx - w / 2.0, cy + 34.0 * k);
            draw_rectangle(x, y, w, 30.0 * k, Color::new(0.0, 0.0, 0.0, 0.55));
            draw_rectangle_lines(x, y, w, 30.0 * k, 1.0, ui::fade(tint, 0.7));
            ui::text(&p.text, x + 14.0 * k, y + 21.0 * k, size, if p.locked { ui::BAD } else { ui::TEXT });
        }
    }

    fn draw_player_panel(&self, g: &Game, sh: f32, k: f32, era: Era) {
        let has_role = !g.player.role.is_empty();
        let inner = 20.0 * k;
        let w = ui::measure(&g.player.name, 20.0 * k).max(ui::measure(&g.player.role, 14.0 * k));
        let w = (w + 2.0 * inner).clamp(190.0 * k, 240.0 * k);
        let h = 16.0 * k + 22.0 * k + if has_role { 18.0 * k } else { 0.0 } + 14.0 * k;
        let r = Rect::new(16.0, sh - 46.0 * k - h, w, h);
        ui::panel(r, ui::PANEL, Some(ui::era_color(era)));

        let x = r.x + inner;
        let mut y = r.y + 16.0 * k + 16.0 * k;
        ui::text(&ui_fit(&g.player.name, w - 2.0 * inner, 20.0 * k), x, y, 20.0 * k, ui::ACCENT);
        if has_role {
            y += 18.0 * k;
            ui::text(&ui_fit(&g.player.role, w - 2.0 * inner, 14.0 * k), x, y, 14.0 * k, ui::MUTED);
        }
    }

    fn draw_stamina(&self, g: &Game, sw: f32, sh: f32, k: f32) {
        let frac = g.body.stamina / MAX_STAMINA;
        if frac >= 0.995 {
            return;
        }
        let (w, h) = (220.0 * k, 8.0 * k);
        let r = Rect::new((sw - w) / 2.0, sh - 100.0 * k, w, h);
        let (label, color) = if g.body.exhausted {
            ("EXHAUSTED", ui::BAD)
        } else if frac < 0.3 {
            ("STAMINA", ui::ACCENT)
        } else {
            ("STAMINA", ui::GOOD)
        };
        ui::text_shadow(label, sw / 2.0 - ui::measure(label, 12.0 * k) / 2.0, r.y - 6.0 * k, 12.0 * k, ui::MUTED);
        ui::bar(r, frac, color);
    }

    fn draw_banner(&self, g: &Game, sw: f32, sh: f32, k: f32, era: Era) {
        let a = banner_alpha(self.banner_age);
        if a <= 0.0 {
            return;
        }
        let tint = ui::era_color(era);
        let name = g.current_map().name.to_uppercase();
        let size = 56.0 * k;
        let cx = sw / 2.0;
        let y = sh * 0.2 + (1.0 - ease_out(self.banner_age / BANNER_IN)) * 12.0;
        let tw = ui::measure_title(&name, size);
        ui::title(&name, cx - tw / 2.0 + 3.0, y + 3.0, size, ui::fade(ui::SHADOW, a));
        ui::title(&name, cx - tw / 2.0, y, size, ui::fade(ui::TEXT, a));
        let rule = (tw * 0.6).max(120.0 * k);
        draw_line(cx - rule / 2.0, y + 14.0 * k, cx + rule / 2.0, y + 14.0 * k, 2.0, ui::fade(tint, a));
        let sub = format!("{}  -  {}", g.pack.story(&g.story).map_or(g.story.as_str(), |s| s.title.as_str()), era.label());
        let sw2 = ui::measure(&sub, 16.0 * k);
        ui::text_shadow(&sub, cx - sw2 / 2.0, y + 38.0 * k, 16.0 * k, ui::fade(tint, a));
    }

    fn draw_toast(&self, sw: f32, k: f32) {
        let Some((text, left)) = self.last_toast.as_ref() else { return };
        let size = 20.0 * k;
        let (w, h) = (ui::measure(text, size) + 44.0 * k, 40.0 * k);
        let vis = toast_visible(self.toast_age, *left);
        let y = 18.0 * k - (1.0 - vis) * (h + 30.0 * k);
        let r = Rect::new((sw - w) / 2.0, y, w, h);
        ui::panel(r, ui::PANEL, Some(ui::ACCENT));
        ui::text(text, r.x + 22.0 * k, r.y + h * 0.5 + 7.0 * k, size, ui::TEXT);
    }

    fn draw_talk(&self, g: &Game, talk: &Talk, sw: f32, sh: f32, k: f32) {
        let bw = (sw - 48.0).min(1000.0 * k);
        let x = (sw - bw) / 2.0;
        let (size, lh, pad) = (22.0 * k, 30.0 * k, 24.0 * k);

        let line = g.current_line();
        let speaker = line.map(|l| l.speaker.as_str()).unwrap_or("");
        let narration = speaker.is_empty();
        let is_you = speaker == "You";
        let label = if narration { String::new() } else { g.speaker_label(speaker) };
        let plate = if is_you { ui::ACCENT } else { speaker_color(&label) };

        let text = line.map(|l| l.text.as_str()).unwrap_or("");
        let total = text.chars().count();
        let rows = ui::wrap(text, bw - 2.0 * pad - 12.0 * k, size);
        let typed = reveal_rows(&rows, (talk.shown.max(0.0) as usize).min(total), total);
        let top_pad = if narration { 18.0 * k } else { 34.0 * k };
        let h = top_pad + rows.len().max(2) as f32 * lh + 14.0 * k;
        let y = sh - 24.0 * k - h;
        let r = Rect::new(x, y, bw, h);
        ui::panel(r, ui::PANEL, Some(if narration { ui::DIM } else { plate }));

        if !narration {
            let pw = ui::measure_title(&label, 24.0 * k) + 30.0 * k;
            let ph = 30.0 * k;
            let (px, py) = (x + pad - 6.0 * k, y - 14.0 * k);
            draw_rectangle(px + 3.0, py + 4.0, pw, ph, ui::SHADOW);
            draw_rectangle(px, py, pw, ph, plate);
            ui::title(&label, px + 15.0 * k, py + ph - 7.0 * k, 24.0 * k, ui::BG_BOTTOM);
        }
        let body_color = if narration {
            ui::MUTED
        } else if is_you {
            ui::ACCENT
        } else {
            ui::TEXT
        };
        for (i, row) in typed.iter().enumerate() {
            ui::text(row, x + pad + 6.0 * k, y + top_pad + 20.0 * k + i as f32 * lh, size, body_color);
        }

        let done = talk.shown >= total as f32;
        if !talk.choosing && done {
            let pulse = 0.5 + 0.5 * (self.time * 6.0).sin();
            let (mx, my) = (x + bw - 30.0 * k, y + h - 20.0 * k + pulse * 3.0 * k);
            let s = 7.0 * k;
            draw_triangle(vec2(mx - s, my - s), vec2(mx + s, my - s), vec2(mx, my + s * 0.8), ui::fade(ui::ACCENT, 0.45 + 0.55 * pulse));
        }

        if talk.choosing {
            self.draw_choices(g, talk.sel, x, y, bw, k);
        }
    }

    /// The choices as numbered cards stacked above the talk box (whose top is `box_y`). Only as many as fit under the top
    /// of the screen are drawn, in a window that follows the selected one.
    fn draw_choices(&self, g: &Game, sel: usize, x: f32, box_y: f32, bw: f32, k: f32) {
        let choices = g.offered_choices();
        if choices.is_empty() {
            return;
        }
        let (card_h, gap) = (38.0 * k, 6.0 * k);
        let bottom = box_y - 16.0 * k;
        let avail = bottom - 64.0 * k;
        let max_vis = (((avail + gap) / (card_h + gap)).floor().max(1.0) as usize).min(choices.len());
        let (start, end) = choice_window(choices.len(), sel, max_vis);
        let n = end - start;
        let y0 = bottom - (n as f32 * card_h + (n as f32 - 1.0) * gap);
        for (slot, i) in (start..end).enumerate() {
            let selected = i == sel;
            let r = Rect::new(x, y0 + slot as f32 * (card_h + gap), bw, card_h);
            ui::panel(r, if selected { ui::PANEL_LIT } else { ui::PANEL }, Some(if selected { ui::ACCENT } else { ui::BORDER }));
            if selected {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, ui::fade(ui::ACCENT, 0.8));
            }
            let b = 26.0 * k;
            let (bx, by) = (r.x + 18.0 * k, r.y + (card_h - b) / 2.0);
            draw_rectangle(bx, by, b, b, if selected { ui::ACCENT } else { ui::BORDER });
            if i < 9 {
                let d = (i + 1).to_string();
                ui::text(&d, bx + (b - ui::measure(&d, 18.0 * k)) / 2.0, by + b - 6.0 * k, 18.0 * k, if selected { ui::BG_BOTTOM } else { ui::TEXT });
            }
            let text_x = bx + b + 16.0 * k;
            let room = r.x + r.w - text_x - 56.0 * k;
            ui::text(
                &ui_fit(&choices[i], room, 20.0 * k),
                text_x,
                r.y + card_h * 0.5 + 7.0 * k,
                20.0 * k,
                if selected { ui::ACCENT } else { ui::TEXT },
            );
            // more above / below the window
            let mark_x = r.x + r.w - 40.0 * k;
            let mid = r.y + card_h * 0.5;
            if slot == 0 && start > 0 {
                draw_triangle(vec2(mark_x, mid - 6.0 * k), vec2(mark_x - 6.0 * k, mid + 3.0 * k), vec2(mark_x + 6.0 * k, mid + 3.0 * k), ui::MUTED);
                ui::text(&format!("+{start}"), mark_x + 10.0 * k, mid + 5.0 * k, 13.0 * k, ui::MUTED);
            }
            if slot + 1 == n && end < choices.len() {
                draw_triangle(vec2(mark_x, mid + 6.0 * k), vec2(mark_x - 6.0 * k, mid - 3.0 * k), vec2(mark_x + 6.0 * k, mid - 3.0 * k), ui::MUTED);
                ui::text(&format!("+{}", choices.len() - end), mark_x + 10.0 * k, mid + 5.0 * k, 13.0 * k, ui::MUTED);
            }
        }
    }
}

// ---- journal and inventory screens ----

/// Which full screen is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Journal,
    Inventory,
}

/// The journal (J) and inventory (I or Tab) screens. While one is open the world stands still.
pub struct Panels {
    open: Option<Screen>,
    sel: usize,
    cur: Cursor,
}

impl Default for Panels {
    fn default() -> Panels {
        Panels::new()
    }
}

impl Panels {
    pub fn new() -> Panels {
        Panels { open: None, sel: 0, cur: Cursor::Slot(0) }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens a screen from its key. Only outside a talk; in a fight they open too (and hold the fight still).
    pub fn poll_open(&mut self, g: &Game) {
        if self.open.is_some() || g.talk.is_some() {
            return;
        }
        let opened = if crate::settings::get().bindings.pressed(Action::Journal) {
            Some(Screen::Journal)
        } else if crate::settings::get().bindings.pressed(Action::Inventory) || is_key_pressed(KeyCode::Tab) {
            Some(Screen::Inventory)
        } else {
            None
        };
        if opened.is_some() {
            self.open = opened;
            self.sel = 0;
            self.cur = Cursor::Slot(g.items.selected);
        }
    }

    fn rows_len(&self, g: &Game) -> usize {
        match self.open {
            Some(Screen::Journal) => g.quest_log().len(),
            Some(Screen::Inventory) => 0,
            None => 0,
        }
    }

    /// Reads the keys of the open screen. Journal: W/S select. Inventory: W/A/S/D or the arrows move the cursor, Enter
    /// uses the item under it, 1-9 moves it to that hotbar slot (swapping). Esc / J / I / Tab close.
    pub fn handle(&mut self, g: &mut Game) {
        let Some(screen) = self.open else { return };
        let b = crate::settings::get().bindings;
        if [KeyCode::Escape, KeyCode::Tab].iter().any(|k| is_key_pressed(*k)) || b.pressed(Action::Journal) || b.pressed(Action::Inventory) {
            self.open = None;
            return;
        }
        if screen == Screen::Journal {
            let n = self.rows_len(g);
            self.sel = self.sel.min(n.saturating_sub(1));
            if b.pressed(Action::Forward) || is_key_pressed(KeyCode::Up) {
                self.sel = wrap_sel(self.sel, -1, n);
            }
            if b.pressed(Action::Back) || is_key_pressed(KeyCode::Down) {
                self.sel = wrap_sel(self.sel, 1, n);
            }
            return;
        }
        let dx = (b.pressed(Action::Right) || is_key_pressed(KeyCode::Right)) as i32 - (b.pressed(Action::Left) || is_key_pressed(KeyCode::Left)) as i32;
        let dy = (b.pressed(Action::Back) || is_key_pressed(KeyCode::Down)) as i32 - (b.pressed(Action::Forward) || is_key_pressed(KeyCode::Up)) as i32;
        self.cur = cursor_move(self.cur, dx, 0, g.items.bag.len());
        self.cur = cursor_move(self.cur, 0, dy, g.items.bag.len());
        if let Some(n) = digit_pressed() {
            let moved = match self.cur {
                Cursor::Slot(i) => {
                    g.items.swap(i, n);
                    true
                }
                Cursor::Bag(j) => {
                    g.items.move_bag_to_slot(j, n);
                    true
                }
            };
            if moved {
                g.sync_inv();
                self.cur = Cursor::Slot(n);
            }
        }
        self.cur = cursor_fix(self.cur, g.items.bag.len());
        let confirm = is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) || b.pressed(Action::Interact);
        if confirm {
            let place = match self.cur {
                Cursor::Slot(i) => crate::inventory::Place::Slot(i),
                Cursor::Bag(j) => crate::inventory::Place::Bag(j),
            };
            if stack_at(g, self.cur).is_some() {
                if !g.use_item_at(place) {
                    g.toast("That cannot be used now");
                }
                self.cur = cursor_fix(self.cur, g.items.bag.len());
            }
        }
    }

    pub fn draw(&self, g: &Game, icons: &Icons) {
        let Some(screen) = self.open else { return };
        let (sw, sh) = (screen_width(), screen_height());
        let k = scale_for(sh);
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.66));
        ui::vignette(0.8);
        let w = (940.0 * k).min(sw - 32.0);
        let h = (sh - 130.0 * k).min(580.0 * k);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0 - 10.0 * k, w, h);
        ui::panel(r, ui::fade(ui::PANEL, 0.97), Some(ui::ACCENT));
        let heading = if screen == Screen::Journal { "JOURNAL" } else { "INVENTORY" };
        ui::title(heading, r.x + 28.0 * k, r.y + 46.0 * k, 38.0 * k, ui::ACCENT);
        let vitals = format!("HP {}/{}", g.party.hp.max(0), g.party.max_hp);
        ui::text(&vitals, r.x + r.w - 24.0 * k - ui::measure(&vitals, 16.0 * k), r.y + 40.0 * k, 16.0 * k, ui::MUTED);
        let split = r.x + 300.0 * k;
        if screen == Screen::Journal {
            draw_line(split, r.y + 64.0 * k, split, r.y + r.h - 20.0 * k, 1.0, ui::BORDER);
        }
        let list_top = r.y + 76.0 * k;
        let row_h = 34.0 * k;
        let max_rows = ((r.h - 100.0 * k) / row_h).floor().max(1.0) as usize;
        let (dx, dw) = (split + 24.0 * k, r.x + r.w - split - 48.0 * k);
        match screen {
            Screen::Journal => {
                let log = g.quest_log();
                if log.is_empty() {
                    ui::text("No quests yet.", r.x + 28.0 * k, list_top + 24.0 * k, 17.0 * k, ui::DIM);
                }
                let sel = self.sel.min(log.len().saturating_sub(1));
                let (a, b) = choice_window(log.len(), sel, max_rows);
                for (slot, i) in (a..b).enumerate() {
                    let row = Rect::new(r.x + 20.0 * k, list_top + slot as f32 * row_h, split - r.x - 36.0 * k, row_h - 4.0 * k);
                    let lit = i == sel;
                    draw_rectangle(row.x, row.y, row.w, row.h, if lit { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.25) });
                    let mark = if log[i].done { "[x] " } else { "[ ] " };
                    let t = ui_fit(&format!("{mark}{}", log[i].title), row.w - 16.0 * k, 16.0 * k);
                    let col = if lit { ui::ACCENT } else if log[i].done { ui::DIM } else { ui::TEXT };
                    ui::text(&t, row.x + 8.0 * k, row.y + row.h * 0.5 + 6.0 * k, 16.0 * k, col);
                }
                if let Some(q) = log.get(sel) {
                    ui::text(&ui_fit(&q.title, dw, 26.0 * k), dx, r.y + 100.0 * k, 26.0 * k, ui::TEXT);
                    let mut y = r.y + 130.0 * k;
                    for row in ui::wrap(&q.description, dw, 16.0 * k) {
                        ui::text(&row, dx, y, 16.0 * k, ui::MUTED);
                        y += 22.0 * k;
                    }
                    y += 14.0 * k;
                    for (text, done) in &q.steps {
                        draw_rectangle_lines(dx, y - 12.0 * k, 14.0 * k, 14.0 * k, 1.5, if *done { ui::GOOD } else { ui::MUTED });
                        if *done {
                            draw_rectangle(dx + 3.0 * k, y - 9.0 * k, 8.0 * k, 8.0 * k, ui::GOOD);
                        }
                        let col = if *done { ui::DIM } else { ui::TEXT };
                        let wrapped = ui::wrap(text, dw - 26.0 * k, 16.0 * k);
                        for (j, row) in wrapped.iter().enumerate() {
                            ui::text(row, dx + 24.0 * k, y + j as f32 * 21.0 * k, 16.0 * k, col);
                        }
                        y += 26.0 * k + wrapped.len().saturating_sub(1) as f32 * 21.0 * k;
                    }
                }
            }
            Screen::Inventory => {
                let lay = inv_layout(r, k);
                let cur = cursor_fix(self.cur, g.items.bag.len());
                let cursor_border = |lit: bool| if lit { ui::ACCENT } else { ui::BORDER };
                ui::text("HOTBAR", lay.x, lay.hot_y - 8.0 * k, 12.0 * k, ui::DIM);
                for i in 0..HOTBAR_SLOTS {
                    let rc = lay.slot_rect(i);
                    let stack = g.items.slots[i].as_ref();
                    let name = stack.map(|s| item_name(g, &s.item)).unwrap_or_default();
                    draw_item_slot(rc, stack, &name, icons, cursor_border(cur == Cursor::Slot(i)), if cur == Cursor::Slot(i) { 3.0 } else { 1.0 }, 1.0, k);
                    let d = (i + 1).to_string();
                    let col = if i == g.items.selected { ui::ACCENT } else { ui::DIM };
                    ui::text(&d, rc.x + 3.0 * k, rc.y + 11.0 * k, 10.0 * k, col);
                }
                ui::text("BAG", lay.x, lay.bag_y - 8.0 * k, 12.0 * k, ui::DIM);
                let bag = &g.items.bag;
                if bag.is_empty() {
                    ui::text("Nothing in the bag.", lay.x, lay.bag_y + 22.0 * k, 16.0 * k, ui::DIM);
                }
                let total_rows = bag.len().div_ceil(BAG_COLS);
                let cur_row = if let Cursor::Bag(j) = cur { j / BAG_COLS } else { 0 };
                let (first, last) = choice_window(total_rows, cur_row, lay.bag_rows);
                for j in (first * BAG_COLS)..(last * BAG_COLS).min(bag.len()) {
                    let rc = lay.bag_rect(j, first);
                    let name = item_name(g, &bag[j].item);
                    draw_item_slot(rc, Some(&bag[j]), &name, icons, cursor_border(cur == Cursor::Bag(j)), if cur == Cursor::Bag(j) { 3.0 } else { 1.0 }, 1.0, k);
                }
                if total_rows > lay.bag_rows {
                    let more = format!("rows {}-{} of {}", first + 1, last, total_rows);
                    ui::text(&more, lay.x + 40.0 * k, lay.bag_y - 8.0 * k, 12.0 * k, ui::DIM);
                }
                let (dx, dw) = (lay.detail_x, lay.detail_w);
                match stack_at(g, cur) {
                    Some(st) => {
                        let item = g.pack.items.get(&st.item);
                        let name = item_name(g, &st.item);
                        ui::text(&ui_fit(&name, dw, 26.0 * k), dx, r.y + 100.0 * k, 26.0 * k, ui::TEXT);
                        let mut y = r.y + 130.0 * k;
                        if st.count > 1 {
                            ui::text(&format!("x{}", st.count), dx, y, 16.0 * k, ui::ACCENT);
                            y += 24.0 * k;
                        }
                        if let Some(it) = item {
                            for row in ui::wrap(&it.description, dw, 16.0 * k) {
                                ui::text(&row, dx, y, 16.0 * k, ui::MUTED);
                                y += 22.0 * k;
                            }
                        }
                        y += 14.0 * k;
                        match item.and_then(|i| i.use_effect.as_ref()) {
                            Some(e) => {
                                ui::text(&describe_use(e), dx, y, 16.0 * k, ui::GOOD);
                            }
                            None => ui::text("Cannot be used.", dx, y, 15.0 * k, ui::DIM),
                        }
                    }
                    None => ui::text("Empty.", dx, r.y + 100.0 * k, 17.0 * k, ui::DIM),
                }
            }
        }
    }
}

// ---- the pause menu ----

/// What the player picked in the pause menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseChoice {
    Resume,
    Settings,
    Menu,
    Quit,
}

const PAUSE_ITEMS: [(&str, &str, PauseChoice); 4] = [
    ("Resume", "", PauseChoice::Resume),
    ("Settings", "", PauseChoice::Settings),
    ("Back to menu", "", PauseChoice::Menu),
    ("Quit", "", PauseChoice::Quit),
];

/// The four button rectangles of the pause panel, top to bottom, for a screen of this size.
pub fn pause_rects(sw: f32, sh: f32) -> [Rect; 4] {
    let k = scale_for(sh);
    let (w, h, gap) = (340.0 * k, 60.0 * k, 12.0 * k);
    let total = 4.0 * h + 3.0 * gap;
    let top = sh * 0.5 - total / 2.0 + 28.0 * k;
    let x = (sw - w) / 2.0;
    [0, 1, 2, 3].map(|i| Rect::new(x, top + i as f32 * (h + gap), w, h))
}

/// The pause menu: Resume, Settings, Back to menu, Quit.
pub struct Pause {
    sel: usize,
    frames: u32,
    last_mouse: (f32, f32),
}

impl Default for Pause {
    fn default() -> Pause {
        Pause::new()
    }
}

impl Pause {
    pub fn new() -> Pause {
        Pause { sel: 0, frames: 0, last_mouse: mouse_position() }
    }

    /// Moves the selection by `delta`, wrapping round the list.
    pub fn step(&mut self, delta: i32) {
        let n = PAUSE_ITEMS.len() as i32;
        self.sel = (self.sel as i32 + delta).rem_euclid(n) as usize;
    }

    pub fn choice(&self) -> PauseChoice {
        PAUSE_ITEMS[self.sel].2
    }

    /// One frame: reads the keys and the mouse, draws the panel over the (already drawn) scene, and returns what was
    /// picked. The keys are ignored on the first frame, so the Escape that opened the menu does not close it.
    pub fn frame(&mut self, t: f32) -> Option<PauseChoice> {
        let (sw, sh) = (screen_width(), screen_height());
        let rects = pause_rects(sw, sh);
        let live = self.frames > 0;
        self.frames += 1;

        let mut picked = None;
        if live {
            if is_key_pressed(KeyCode::Escape) {
                return Some(PauseChoice::Resume);
            }
            let b = crate::settings::get().bindings;
            if is_key_pressed(KeyCode::Up) || b.pressed(Action::Forward) {
                self.step(-1);
            }
            if is_key_pressed(KeyCode::Down) || b.pressed(Action::Back) {
                self.step(1);
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) || b.pressed(Action::Interact) {
                picked = Some(self.choice());
            }
            let m = mouse_position();
            let over = rects.iter().position(|r| r.contains(vec2(m.0, m.1)));
            if m != self.last_mouse {
                if let Some(i) = over {
                    self.sel = i;
                }
            }
            if is_mouse_button_pressed(MouseButton::Left) {
                if let Some(i) = over {
                    self.sel = i;
                    picked = Some(self.choice());
                }
            }
            self.last_mouse = m;
        }
        self.draw(sw, sh, &rects, t);
        picked
    }

    fn draw(&self, sw: f32, sh: f32, rects: &[Rect; 4], t: f32) {
        let k = scale_for(sh);
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.62));
        ui::vignette(0.8);
        let (first, last) = (rects[0], rects[3]);
        let frame = Rect::new(first.x - 32.0 * k, first.y - 110.0 * k, first.w + 64.0 * k, last.y + last.h - first.y + 150.0 * k);
        ui::panel(frame, ui::fade(ui::PANEL, 0.97), None);
        ui::title_centered("PAUSED", sw / 2.0, frame.y + 62.0 * k, 52.0 * k, ui::ACCENT);
        draw_line(sw / 2.0 - 40.0 * k, frame.y + 78.0 * k, sw / 2.0 + 40.0 * k, frame.y + 78.0 * k, 2.0, ui::fade(ui::ACCENT, 0.6 + 0.3 * (t * 2.0).sin()));
        for (i, (label, sub, _)) in PAUSE_ITEMS.iter().enumerate() {
            let lit = i == self.sel;
            ui::button(rects[i], label, sub, lit, true, if lit { 1.0 } else { 0.0 }, ui::ACCENT);
        }
    }
}

// ---- loading and error screens (the browser build) ----

fn spinner(cx: f32, cy: f32, r: f32, t: f32, color: Color) {
    const N: usize = 12;
    for i in 0..N {
        let ang = i as f32 / N as f32 * std::f32::consts::TAU;
        let (s, c) = ang.sin_cos();
        let a = spinner_alpha(i, N, t);
        draw_line(cx + s * r * 0.55, cy - c * r * 0.55, cx + s * r, cy - c * r, 3.0, ui::fade(color, a));
    }
}

/// The loading screen: the backdrop, the title and a progress panel with a spinner and the status line.
#[allow(dead_code)]
pub fn draw_loading(status: &str, t: f32) {
    let (sw, sh) = (screen_width(), screen_height());
    let k = scale_for(sh);
    ui::backdrop(t, ui::ACCENT);
    let cx = sw / 2.0;
    ui::title_centered("KYTTU - LRS", cx, sh * 0.3, 84.0 * k, ui::ACCENT);
    let w = (560.0 * k).min(sw - 40.0);
    let r = Rect::new(cx - w / 2.0, sh * 0.42, w, 120.0 * k);
    ui::panel(r, ui::PANEL, Some(ui::ACCENT));
    spinner(r.x + 52.0 * k, r.y + 50.0 * k, 18.0 * k, t, ui::ACCENT);
    let tx = r.x + 92.0 * k;
    ui::text("Loading the game files", tx, r.y + 46.0 * k, 22.0 * k, ui::TEXT);
    ui::text(&ui_fit(status, r.x + r.w - tx - 20.0 * k, 15.0 * k), tx, r.y + 70.0 * k, 15.0 * k, ui::MUTED);
    // an unending bar: a lit stretch runs along the track
    let track = Rect::new(r.x + 24.0 * k, r.y + r.h - 24.0 * k, r.w - 48.0 * k, 6.0 * k);
    draw_rectangle(track.x, track.y, track.w, track.h, Color::new(0.0, 0.0, 0.0, 0.55));
    let seg = track.w * 0.28;
    let pos = (t * 0.5).rem_euclid(1.0) * (track.w + seg) - seg;
    let (x0, x1) = (pos.max(0.0), (pos + seg).min(track.w));
    if x1 > x0 {
        draw_rectangle(track.x + x0, track.y, x1 - x0, track.h, ui::ACCENT);
    }
}

/// The error screen: the heading, then the lines in a panel (long lines wrap, indented lines stay indented).
#[allow(dead_code)]
pub fn draw_error(lines: &[String], t: f32) {
    let (sw, sh) = (screen_width(), screen_height());
    let k = scale_for(sh);
    ui::backdrop(t, ui::BAD);
    let cx = sw / 2.0;
    ui::title_centered("KYTTU - LRS COULD NOT START", cx, sh * 0.2, 54.0 * k, ui::BAD);
    let w = (900.0 * k).min(sw - 40.0);
    let r = Rect::new(cx - w / 2.0, sh * 0.26, w, sh * 0.62);
    ui::panel(r, ui::PANEL, Some(ui::BAD));
    let (size, lh) = (17.0 * k, 23.0 * k);
    let max_rows = ((r.h - 40.0 * k) / lh).floor().max(1.0) as usize;
    let mut y = r.y + 34.0 * k;
    let mut rows = 0;
    for (i, line) in lines.iter().enumerate() {
        let indent = if line.starts_with(' ') { 20.0 * k } else { 0.0 };
        let color = if i == 0 { ui::BAD } else { ui::TEXT };
        for row in ui::wrap(line, r.w - 52.0 * k - indent, size).iter() {
            if rows >= max_rows {
                ui::text("...", r.x + 26.0 * k, y, size, ui::MUTED);
                return;
            }
            ui::text(row, r.x + 26.0 * k + indent, y, size, color);
            y += lh;
            rows += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Cond;

    fn owned(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_long_npc_name_is_clipped_with_an_ellipsis_and_an_empty_one_draws_nothing() {
        assert_eq!(clip_tag("Felix", TAG_MAX_CHARS).as_deref(), Some("Felix"));
        assert_eq!(clip_tag("  Felix  ", TAG_MAX_CHARS).as_deref(), Some("Felix"));
        assert_eq!(clip_tag("", TAG_MAX_CHARS), None);
        assert_eq!(clip_tag("   ", TAG_MAX_CHARS), None);
        let long = clip_tag("Sir Bartholomew Featherstonehaugh the Third", 12).unwrap();
        assert!(long.ends_with('…') && long.chars().count() <= 12, "{long}");
        assert_eq!(clip_tag("abcdef", 6).as_deref(), Some("abcdef"));
    }

    #[test]
    fn banner_fades_in_holds_and_fades_out() {
        assert_eq!(banner_alpha(-1.0), 0.0);
        assert_eq!(banner_alpha(0.0), 0.0);
        assert!(banner_alpha(BANNER_IN / 2.0) > 0.4 && banner_alpha(BANNER_IN / 2.0) < 0.6);
        assert_eq!(banner_alpha(1.5), 1.0);
        assert!(banner_alpha((BANNER_HOLD + BANNER_END) / 2.0) < 0.6);
        assert_eq!(banner_alpha(BANNER_END), 0.0);
        assert_eq!(banner_alpha(99.0), 0.0);
    }

    #[test]
    fn ease_out_runs_from_zero_to_one_and_clamps() {
        assert_eq!(ease_out(-3.0), 0.0);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(7.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "fast at the start");
    }

    #[test]
    fn choice_window_always_keeps_the_selection_and_never_exceeds_the_room() {
        for total in 1..=14usize {
            for max_vis in 1..=6usize {
                for sel in 0..total {
                    let (a, b) = choice_window(total, sel, max_vis);
                    assert!(a <= sel && sel < b, "total {total} max {max_vis} sel {sel}: {a}..{b}");
                    assert!(b <= total);
                    assert!(b - a <= max_vis);
                    assert_eq!(b - a, total.min(max_vis));
                }
            }
        }
        assert_eq!(choice_window(0, 0, 3), (0, 0));
        assert_eq!(choice_window(20, 0, 5), (0, 5), "the top of the list");
        assert_eq!(choice_window(20, 19, 5), (15, 20), "the end of the list");
        assert_eq!(choice_window(20, 10, 5), (8, 13), "the middle");
        assert_eq!(choice_window(3, 1, 0), (1, 2), "no room still shows the selected one");
    }

    #[test]
    fn reveal_rows_follow_the_typed_count() {
        let rows = owned(&["hello there", "general kenobi"]);
        assert!(reveal_rows(&rows, 0, 26).is_empty());
        assert_eq!(reveal_rows(&rows, 5, 26), owned(&["hello"]));
        assert_eq!(reveal_rows(&rows, 11, 26), owned(&["hello there"]));
        assert_eq!(reveal_rows(&rows, 12, 26), owned(&["hello there"]), "the break space costs one");
        assert_eq!(reveal_rows(&rows, 15, 26), owned(&["hello there", "gen"]));
        assert_eq!(reveal_rows(&rows, 26, 26), rows);
        assert_eq!(reveal_rows(&rows, 100, 26), rows);
    }

    #[test]
    fn ellipsize_cuts_to_the_width_and_leaves_short_text_alone() {
        let measure = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(ellipsize("short", 100.0, &measure), "short");
        let cut = ellipsize("a very long choice text", 100.0, &measure);
        assert!(cut.ends_with("..."), "{cut}");
        assert!(measure(&cut) <= 100.0, "{cut}");
        assert_eq!(ellipsize("anything", 5.0, &measure), "...", "nothing fits but the dots");
    }

    #[test]
    fn prompt_names_an_npc_and_a_shut_door_only() {
        let npc = Kind::Npc {
            name: "Mara".into(),
            scene: "s".into(),
            color: [0, 0, 0],
            body: Default::default(),
            skin: None,
            routine: None,
        };
        let door = Kind::Door { open_if: Cond::Has("key".into()), locked: "It is locked.".into(), height: 1.0, color: [0, 0, 0] };
        let flags = BTreeSet::new();
        let none: Vec<String> = Vec::new();
        let with_key = vec!["key".to_string()];

        assert_eq!(prompt_for(Some(&npc), &flags, &none), Some(Prompt { text: "Talk to Mara".into(), locked: false }));
        assert_eq!(prompt_for(Some(&door), &flags, &none), Some(Prompt { text: "Locked: It is locked.".into(), locked: true }));
        assert_eq!(prompt_for(Some(&door), &flags, &with_key), None, "an open door says nothing");
        assert_eq!(prompt_for(Some(&Kind::Floor), &flags, &none), None);
        assert_eq!(prompt_for(None, &flags, &none), None);
        let silent = Kind::Door { open_if: Cond::Has("key".into()), locked: String::new(), height: 1.0, color: [0, 0, 0] };
        assert_eq!(prompt_for(Some(&silent), &flags, &none).unwrap().text, "Locked");
    }

    fn bench(text: &str, action: PropAction) -> Kind {
        Kind::Prop {
            prop: "bench".into(),
            yaw: 0.0,
            color: [0, 0, 0],
            use_: Some(PropUse { action, text: text.into(), scene: None, seat_height: 0.45 }),
        }
    }

    #[test]
    fn a_usable_prop_prompts_with_its_word_or_its_actions_default() {
        let (flags, none) = (BTreeSet::new(), Vec::<String>::new());
        let p = prompt_line(None, false, Some(&bench("Rest", PropAction::Sit)), &flags, &none).unwrap();
        assert_eq!(p.text, "Rest");
        let p = prompt_line(None, false, Some(&bench("", PropAction::DrawWater)), &flags, &none).unwrap();
        assert_eq!(p.text, "Draw water");
        assert_eq!(action_word(PropAction::WarmHands), "Warm hands");
        let plain = Kind::Prop { prop: "crate".into(), yaw: 0.0, color: [0, 0, 0], use_: None };
        assert_eq!(prompt_line(None, false, Some(&plain), &flags, &none), None);
    }

    #[test]
    fn seated_says_stand_up_and_a_close_npc_beats_the_tile() {
        let (flags, none) = (BTreeSet::new(), Vec::<String>::new());
        let b = bench("Sit", PropAction::Sit);
        assert_eq!(prompt_line(Some("Mara"), true, Some(&b), &flags, &none).unwrap().text, "Stand up");
        assert_eq!(prompt_line(Some("Mara"), false, Some(&b), &flags, &none).unwrap().text, "Talk to Mara");
        let npc_tile = Kind::Npc { name: "Mara".into(), scene: "s".into(), color: [0, 0, 0], body: Default::default(), skin: None, routine: None };
        assert_eq!(prompt_line(None, false, Some(&npc_tile), &flags, &none), None, "an empty NPC tile is not talkable");
    }

    #[test]
    fn the_talk_target_is_the_nearest_npc_in_front_within_two_metres() {
        let npcs = [("Far", (0.0, -1.9)), ("Near", (0.0, -1.0)), ("Behind", (0.0, 0.5)), ("Out", (0.0, -3.0))];
        // Facing -z (yaw 0).
        assert_eq!(nearest_in_front((0.0, 0.0), (0.0, -1.0), &npcs, 2.0), Some(1));
        assert_eq!(nearest_in_front((0.0, 0.0), (0.0, 1.0), &npcs, 2.0), Some(2), "facing the other way finds the one behind");
        assert_eq!(nearest_in_front((0.0, 0.0), (1.0, 0.0), &npcs, 2.0), None, "side on: nobody");
        assert_eq!(nearest_in_front((0.0, 0.0), (0.0, -1.0), &[], 2.0), None);
        assert_eq!(nearest_in_front((0.0, 0.0), (1.0, 0.0), &[("Here", (0.1, 0.0))], 2.0), Some(0));
    }

    #[test]
    fn the_weather_label_names_the_kind_and_its_strength() {
        assert_eq!(weather_label(WeatherKind::Clear, 0.9), None);
        assert_eq!(weather_label(WeatherKind::Rain, 0.2).as_deref(), Some("Light rain"));
        assert_eq!(weather_label(WeatherKind::Rain, 0.6).as_deref(), Some("Rain"));
        assert_eq!(weather_label(WeatherKind::Snow, 0.9).as_deref(), Some("Heavy snow"));
        assert_eq!(weather_label(WeatherKind::Storm, 0.5).as_deref(), Some("Storm"));
    }

    #[test]
    fn a_toast_restarts_on_new_text_or_more_time() {
        assert!(toast_restarted(None, "a", 2.0));
        assert!(!toast_restarted(Some(("a", 2.0)), "a", 1.9));
        assert!(toast_restarted(Some(("a", 1.0)), "b", 0.9), "different text");
        assert!(toast_restarted(Some(("a", 0.5)), "a", 2.5), "the same text shown again");
    }

    #[test]
    fn a_toast_slides_in_and_out() {
        assert_eq!(toast_visible(0.0, 2.5), 0.0);
        assert_eq!(toast_visible(1.0, 2.0), 1.0);
        assert!(toast_visible(1.0, 0.05) < 0.5, "nearly gone");
        assert_eq!(toast_visible(1.0, 0.0), 0.0);
    }

    #[test]
    fn a_speaker_always_gets_the_same_colour() {
        assert_eq!(speaker_color("Mara"), speaker_color("Mara"));
        let seen: BTreeSet<u32> = ["Mara", "Odd", "Kess", "Ren", "Sol", "Wick", "Ivo", "Tam"]
            .iter()
            .map(|n| (speaker_color(n).r * 1000.0) as u32)
            .collect();
        assert!(seen.len() > 1, "the palette is used, not one colour");
    }

    #[test]
    fn the_spinner_head_is_brightest_and_every_tick_stays_visible() {
        for step in 0..20 {
            let t = step as f32 * 0.137;
            for i in 0..12 {
                let a = spinner_alpha(i, 12, t);
                assert!((0.12..=1.0).contains(&a), "tick {i} at {t}: {a}");
            }
        }
        // at t = 0 the head is tick 0
        assert_eq!(spinner_alpha(0, 12, 0.0), 1.0);
        assert!(spinner_alpha(1, 12, 0.0) < spinner_alpha(0, 12, 0.0));
    }

    #[test]
    fn the_pause_selection_wraps_both_ways() {
        let mut p = Pause { sel: 0, frames: 0, last_mouse: (0.0, 0.0) };
        assert_eq!(p.choice(), PauseChoice::Resume);
        p.step(-1);
        assert_eq!(p.choice(), PauseChoice::Quit, "up from the top goes to the bottom");
        p.step(1);
        assert_eq!(p.choice(), PauseChoice::Resume);
        p.step(1);
        assert_eq!(p.choice(), PauseChoice::Settings);
        p.step(1);
        assert_eq!(p.choice(), PauseChoice::Menu);
        p.step(5); // four entries now: 2 + 5 wraps to the last
        assert_eq!(p.choice(), PauseChoice::Quit);
    }

    #[test]
    fn pause_buttons_are_stacked_inside_the_screen() {
        for (sw, sh) in [(1280.0, 720.0), (800.0, 600.0), (1920.0, 1080.0)] {
            let rects = pause_rects(sw, sh);
            for r in &rects {
                assert!(r.x >= 0.0 && r.x + r.w <= sw && r.y >= 0.0 && r.y + r.h <= sh, "{r:?} on {sw}x{sh}");
            }
            for pair in rects.windows(2) {
                assert!(pair[0].y + pair[0].h < pair[1].y, "the buttons do not overlap");
            }
        }
    }

    #[test]
    fn wrap_sel_wraps_and_handles_empty_lists() {
        assert_eq!(wrap_sel(0, -1, 4), 3);
        assert_eq!(wrap_sel(3, 1, 4), 0);
        assert_eq!(wrap_sel(1, 1, 4), 2);
        assert_eq!(wrap_sel(9, 1, 4), 0, "a stale selection is clamped first");
        assert_eq!(wrap_sel(0, 1, 0), 0);
        assert_eq!(wrap_sel(0, -1, 1), 0);
    }

    #[test]
    fn bar_fractions_clamp() {
        assert_eq!(hp_frac(15, 30), 0.5);
        assert_eq!(hp_frac(-4, 30), 0.0);
        assert_eq!(hp_frac(99, 30), 1.0);
        assert_eq!(hp_frac(5, 0), 0.0);
    }

    #[test]
    fn the_tracker_picks_the_first_unfinished_quest_and_its_next_step() {
        let steps = |v: &[bool]| v.iter().enumerate().map(|(i, d)| (format!("s{i}"), *d)).collect::<Vec<_>>();
        let (a, b, c) = (steps(&[true, true]), steps(&[true, false, false]), steps(&[false]));
        assert_eq!(tracker_pick(&[]), None);
        assert_eq!(tracker_pick(&[(true, a.as_slice())]), None, "all done shows nothing");
        assert_eq!(tracker_pick(&[(true, a.as_slice()), (false, b.as_slice()), (false, c.as_slice())]), Some((1, Some(1))));
        assert_eq!(tracker_pick(&[(false, a.as_slice())]), Some((0, None)), "no open step left");
    }

    #[test]
    fn use_descriptions_name_the_effect() {
        assert_eq!(describe_use(&ItemUse::Heal(12)), "Heals 12 HP");
        assert!(describe_use(&ItemUse::Damage(7)).contains('7'));
    }

    #[test]
    fn the_hotbar_is_full_for_four_seconds_then_fades_to_a_third() {
        assert_eq!(hotbar_alpha(0.0), 1.0);
        assert_eq!(hotbar_alpha(HOTBAR_IDLE), 1.0);
        let mid = hotbar_alpha(HOTBAR_IDLE + HOTBAR_FADE / 2.0);
        assert!(mid < 1.0 && mid > HOTBAR_DIM, "fading: {mid}");
        assert!((hotbar_alpha(HOTBAR_IDLE + HOTBAR_FADE) - HOTBAR_DIM).abs() < 1e-6);
        assert!((hotbar_alpha(60.0) - 0.35).abs() < 1e-6);
    }

    #[test]
    fn a_change_or_a_poke_brings_the_hotbar_back_and_idling_fades_it() {
        let snap = |n: u32| vec![Some(("tonic".to_string(), n)), None];
        let mut st = HotbarState::default();
        st.update(snap(1), 0, 0.016);
        for _ in 0..100 {
            st.update(snap(1), 0, 0.1);
        }
        assert!(st.alpha() < 0.5, "10 s idle dims it");
        st.update(snap(2), 0, 0.016);
        assert_eq!(st.alpha(), 1.0, "a count change wakes it");
        for _ in 0..100 {
            st.update(snap(2), 0, 0.1);
        }
        st.update(snap(2), 1, 0.016);
        assert_eq!(st.alpha(), 1.0, "a new selection wakes it");
        assert!(name_alpha(st.name_age) > 0.9, "and shows the name");
        for _ in 0..100 {
            st.update(snap(2), 1, 0.1);
        }
        st.poke();
        assert_eq!(st.alpha(), 1.0);
        assert_eq!(name_alpha(NAME_SHOW + 0.1), 0.0);
    }

    #[test]
    fn the_hotbar_fits_the_screen_at_small_and_large_sizes() {
        for (sw, sh) in [(800.0_f32, 600.0_f32), (1280.0, 720.0), (1920.0, 1080.0)] {
            let rects = hotbar_rects(sw, sh);
            for (i, r) in rects.iter().enumerate() {
                assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= sw && r.y + r.h <= sh, "slot {i} on {sw}x{sh}");
                if i > 0 {
                    assert!(r.x >= rects[i - 1].x + rects[i - 1].w, "no overlap");
                }
            }
            let mid = (rects[0].x + rects[8].x + rects[8].w) / 2.0;
            assert!((mid - sw / 2.0).abs() < 0.5, "centred");
            assert!(rects[0].w <= 52.0, "small");
            assert!(rects[0].y + rects[0].h < sh - 18.0, "near the bottom edge");
        }
        assert!((hotbar_rects(1280.0, 720.0)[0].w - 34.0).abs() < 0.01, "34 px at 720p");
    }

    #[test]
    fn the_item_colour_is_stable_and_different_per_id() {
        assert_eq!(item_color("tonic"), item_color("tonic"));
        assert_ne!(item_color("tonic"), item_color("key"));
        for id in ["", "a", "tonic", "old_key_7"] {
            assert!(item_color(id).iter().all(|c| (0.0..=1.0).contains(c)), "{id}");
        }
        assert_eq!(item_initial("  tonic"), 'T');
        assert_eq!(item_initial("!!"), '?');
    }

    #[test]
    fn digits_pick_choices_while_choosing_and_slots_otherwise() {
        assert_eq!(digit_route(Some(true), false), DigitRoute::Choice);
        assert_eq!(digit_route(Some(false), false), DigitRoute::None);
        assert_eq!(digit_route(None, false), DigitRoute::Slot);
        assert_eq!(digit_route(None, true), DigitRoute::Panel);
        assert_eq!(digit_route(Some(true), true), DigitRoute::Panel);
    }

    #[test]
    fn the_wheel_steps_one_slot_by_sign() {
        assert_eq!(wheel_step(1.0), -1);
        assert_eq!(wheel_step(-120.0), 1);
        assert_eq!(wheel_step(0.0), 0);
    }

    #[test]
    fn the_inventory_cursor_moves_over_the_hotbar_and_the_bag() {
        use Cursor::*;
        assert_eq!(cursor_move(Slot(8), 1, 0, 0), Slot(0), "wraps");
        assert_eq!(cursor_move(Slot(0), -1, 0, 0), Slot(8));
        assert_eq!(cursor_move(Slot(3), 0, 1, 0), Slot(3), "no bag, nowhere to go");
        assert_eq!(cursor_move(Slot(3), 0, 1, 12), Bag(3));
        assert_eq!(cursor_move(Slot(7), 0, 1, 3), Bag(2), "short bag: its last item");
        assert_eq!(cursor_move(Bag(3), 0, -1, 12), Slot(3));
        assert_eq!(cursor_move(Bag(10), 0, -1, 12), Bag(1));
        assert_eq!(cursor_move(Bag(1), 0, 1, 12), Bag(10));
        assert_eq!(cursor_move(Bag(5), 0, 1, 12), Bag(11), "last row is short: lands on the last item");
        assert_eq!(cursor_move(Bag(11), 0, 1, 12), Bag(11), "bottom stays");
        assert_eq!(cursor_move(Bag(11), 1, 0, 12), Bag(9), "wraps inside the short row");
        assert_eq!(cursor_move(Bag(9), -1, 0, 12), Bag(11));
        assert_eq!(cursor_fix(Bag(5), 0), Slot(5), "an emptied bag sends the cursor up");
        assert_eq!(cursor_fix(Bag(9), 4), Bag(3));
    }

    #[test]
    fn the_inventory_grid_fits_its_panel() {
        for (sw, sh) in [(800.0_f32, 600.0_f32), (1920.0, 1080.0)] {
            let k = scale_for(sh);
            let w = (940.0 * k).min(sw - 32.0);
            let h = (sh - 130.0 * k).min(580.0 * k);
            let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0 - 10.0 * k, w, h);
            let lay = inv_layout(r, k);
            assert!(lay.bag_rows >= 1 && lay.detail_w > 100.0, "{sw}x{sh}: rows {} detail {}", lay.bag_rows, lay.detail_w);
            for i in 0..HOTBAR_SLOTS {
                let c = lay.slot_rect(i);
                assert!(c.x >= r.x && c.x + c.w <= lay.detail_x && c.y + c.h <= r.y + r.h, "slot {i}");
            }
            let last = lay.bag_rect((lay.bag_rows - 1) * BAG_COLS + 8, 0);
            assert!(last.y + last.h <= r.y + r.h && last.x + last.w <= lay.detail_x, "bag fits");
        }
    }
}
