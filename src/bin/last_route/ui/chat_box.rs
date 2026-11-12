//! The co-op chat box: the CHAT binding (data/controls.json `chat`, T by default, rebindable in CONTROLS) opens a one-line
//! input at the bottom left, Enter
//! sends it to the room, Escape closes it. While it is open [`ChatBox::typing`] is true and `app` takes no movement or
//! action keys. The feed above it shows the sender's name in their colour, fading in the last seconds of a line's
//! life, and every recent line while the box is open.
//!
//! The editing rules are plain methods on [`ChatBox`] (tested without a window); [`ChatBox::update`] reads one frame of
//! keyboard and hands it to them. The line the player sends is cut to the data's `input_max` (80 at most, the length
//! the host keeps) on a character boundary.

use super::style::*;
use crate::app::assets::Assets;
use crate::net::chat::{ChatFeed, ChatLine, ChatRules};
use super::text_field::{HoldTimer, KeyRepeat};
use macroquad::input::{get_char_pressed, is_key_down, is_key_pressed, KeyCode};
use macroquad::prelude::*;

const SAFE: f32 = 8.0;

/// The line being typed; None while the box is closed.
#[derive(Default, Debug, PartialEq)]
pub struct ChatBox {
    text: Option<String>,
    /// The line goes to NEARBY (Tab flips it) instead of TEAM.
    nearby: bool,
    /// How long Backspace has been held (it repeats, data/ui/key_repeat.json).
    backspace_hold: HoldTimer,
}

/// `text` cut to at most `max` characters, never inside a multi-byte character.
pub fn cut_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The key code for the data's open key (a letter A to Z); T when the letter is not one.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn open_key_code(letter: &str) -> KeyCode {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::A, KeyCode::B, KeyCode::C, KeyCode::D, KeyCode::E, KeyCode::F, KeyCode::G, KeyCode::H, KeyCode::I,
        KeyCode::J, KeyCode::K, KeyCode::L, KeyCode::M, KeyCode::N, KeyCode::O, KeyCode::P, KeyCode::Q, KeyCode::R,
        KeyCode::S, KeyCode::T, KeyCode::U, KeyCode::V, KeyCode::W, KeyCode::X, KeyCode::Y, KeyCode::Z,
    ];
    let c = letter.chars().next().map(|c| c.to_ascii_uppercase()).filter(char::is_ascii_uppercase).unwrap_or('T');
    LETTERS[(c as u8 - b'A') as usize]
}

/// The colour of a sender's name: the data's palette, picked by peer id.
pub fn name_color(peer_id: i64, palette: &[[f32; 3]]) -> Color {
    if palette.is_empty() {
        return rgba(0.95, 0.95, 0.9, 1.0);
    }
    let c = palette[peer_id.rem_euclid(palette.len() as i64) as usize];
    rgba(c[0], c[1], c[2], 1.0)
}

impl ChatBox {
    /// The box is open: the game takes no movement or action keys (`app` reads this).
    /// The channel the typed line is sent on: "team" or "nearby" (Tab while typing flips it).
    pub fn channel(&self) -> &'static str {
        if self.nearby {
            "nearby"
        } else {
            "team"
        }
    }

    /// The word the open box shows for its channel.
    pub fn channel_label(&self) -> &'static str {
        if self.nearby {
            "NEARBY"
        } else {
            "TEAM"
        }
    }

    pub fn toggle_channel(&mut self) {
        self.nearby = !self.nearby;
    }

    pub fn typing(&self) -> bool {
        self.text.is_some()
    }

    /// The line so far (None when closed).
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn open(&mut self) {
        self.text = Some(String::new());
    }

    pub fn close(&mut self) {
        self.text = None;
    }

    /// Adds one typed character; control characters and anything past the data's `input_max` are ignored.
    pub fn insert_char(&mut self, c: char) -> bool {
        let max = ChatRules::load().input_max;
        match self.text.as_mut() {
            Some(t) if !c.is_control() && t.chars().count() < max => {
                t.push(c);
                true
            }
            _ => false,
        }
    }

    /// Adds pasted or queued text the same way (each character through [`ChatBox::insert_char`]).
    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(t) = self.text.as_mut() {
            t.pop();
        }
    }

    /// Enter: closes the box and clears it; the trimmed line to send, None when it was empty or the box was closed.
    pub fn submit(&mut self) -> Option<String> {
        let t = self.text.take()?;
        let sent = cut_chars(t.trim(), ChatRules::load().input_max);
        (!sent.is_empty()).then_some(sent)
    }

    /// One frame of keyboard. `allowed` is false while something else owns the keyboard (a menu, the map, the admin
    /// overlay); the box then closes. The box opens on the player's CHAT binding (`bind`, settings CONTROLS). Returns the
    /// line to send when Enter was pressed on a non-empty line.
    pub fn update(&mut self, allowed: bool, bind: &crate::app::bindings::Bindings) -> Option<String> {
        if !allowed {
            self.close();
            return None;
        }
        if !self.typing() {
            if bind.pressed("chat") {
                // the key press itself arrives as a character: drop it
                while get_char_pressed().is_some() {}
                self.open();
            }
            return None;
        }
        while let Some(c) = get_char_pressed() {
            self.insert_char(c);
        }
        let cfg = KeyRepeat::load();
        for _ in 0..self.backspace_hold.step(is_key_pressed(KeyCode::Backspace), is_key_down(KeyCode::Backspace), get_frame_time(), &cfg) {
            self.backspace();
        }
        if is_key_pressed(KeyCode::Tab) {
            self.toggle_channel();
        }
        if is_key_pressed(KeyCode::Escape) {
            self.close();
            return None;
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
            return self.submit();
        }
        None
    }
}

fn shadowed(a: &Assets, text: &str, x: f32, top: f32, size: f32, col: Color) {
    label(a, F::Body, text, x + 1.0, top + 1.0, size, rgba(0.0, 0.0, 0.0, col.a));
    label(a, F::Body, text, x, top, size, col);
}

/// One feed line: the name in the sender's colour, then the text; `alpha` scales both.
fn draw_line(a: &Assets, line: &ChatLine, x: f32, top: f32, size: f32, alpha: f32, palette: &[[f32; 3]]) {
    let body = rgba(0.95, 0.95, 0.9, 0.95 * alpha);
    if line.system || line.display_name.is_empty() {
        shadowed(a, &line.text, x, top, size, body);
        return;
    }
    let mut name_col = name_color(line.peer_id, palette);
    name_col.a = alpha;
    let name = if line.channel == "nearby" { format!("(NEARBY) {}:", line.display_name) } else { format!("{}:", line.display_name) };
    shadowed(a, &name, x, top, size, name_col);
    shadowed(a, &line.text, x + tw(a, F::Body, &name, size) + size * 0.6, top, size, body);
}

/// The chat feed and, when `chat` is open, every recent line plus the input row, bottom-left (`app` calls this once per
/// frame after the HUD).
pub fn draw_chat(a: &Assets, feed: &ChatFeed, chat: &ChatBox) {
    let rules = ChatRules::load();
    let v = view();
    let size = (7.0f32).max((12.0 / unit()).ceil().min(9.0));
    let step = size + 3.0;
    let lines: Vec<(&ChatLine, f32)> = if chat.typing() {
        feed.recent().into_iter().map(|l| (l, 1.0)).collect()
    } else {
        feed.faded()
    };
    let bottom = v.y - SAFE - if chat.typing() { 40.0 + step } else { 40.0 };
    let mut top = bottom - step * lines.len() as f32;
    for (line, alpha) in lines {
        draw_line(a, line, SAFE, top, size, alpha.clamp(0.0, 1.0), &rules.name_colors);
        top += step;
    }
    if let Some(text) = chat.text() {
        let row = Rect::new(SAFE - 2.0, v.y - SAFE - 40.0 - 2.0, (v.x * 0.5).max(120.0), size + 4.0);
        fill(row, rgba(0.0, 0.0, 0.0, 0.6));
        let caret = if (get_time() * 2.0) as i64 % 2 == 0 { "_" } else { " " };
        shadowed(a, &format!("[{}] > {text}{caret}", chat.channel_label()), SAFE, v.y - SAFE - 40.0, size, rgba(1.0, 1.0, 1.0, 1.0));
    }
}

/// The speech bubbles: each recent speaker's line, cut short, above its nametag in the world (call after the nametag
/// pass, on the default camera). A speaker with no marker on the drawn level gets none.
pub fn draw_bubbles(g: &crate::game::Game, a: &Assets, rt: &RenderTarget, feed: &ChatFeed, marks: &[super::markers::Marker]) {
    use macroquad::camera::Camera as _;
    let bubbles = feed.bubbles();
    if bubbles.is_empty() {
        return;
    }
    let cam = crate::render::scene::camera(g, rt);
    let vp = cam.matrix();
    let screen = (screen_width(), screen_height());
    let r = super::markers::rules();
    let rules = ChatRules::load();
    let u = unit();
    let size = r.font_size;
    for b in bubbles {
        let Some(m) = marks.iter().find(|m| m.id == super::markers::MarkerId::Peer(b.peer_id.max(0) as u32)) else { continue };
        let probe = super::markers::Marker { name: "-".into(), ..m.clone() };
        let Some(tag) = super::markers::tag_for(&probe, g.fac.level, cam.position, vp, screen, r) else { continue };
        let text = b.text.to_uppercase();
        let w = tw(a, F::Body, &text, size);
        let x = tag.at.0 / u - w * 0.5;
        let top = tag.at.1 / u - lh(F::Body, size) * 2.6;
        let col = name_color(b.peer_id, &rules.name_colors);
        fill(Rect::new(x - 2.0, top - 1.0, w + 4.0, lh(F::Body, size) + 2.0), rgba(0.0, 0.0, 0.0, 0.55 * b.alpha * tag.alpha));
        shadowed(a, &text, x, top, size, Color::new(col.r, col.g, col.b, b.alpha * tag.alpha));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> ChatBox {
        let mut b = ChatBox::default();
        b.open();
        b.insert_str(s);
        b
    }

    #[test]
    fn the_channel_starts_on_team_and_tab_flips_it_to_nearby() {
        let mut b = typed("x");
        assert_eq!((b.channel(), b.channel_label()), ("team", "TEAM"));
        b.toggle_channel();
        assert_eq!((b.channel(), b.channel_label()), ("nearby", "NEARBY"));
        b.toggle_channel();
        assert_eq!(b.channel(), "team");
    }

    #[test]
    fn typing_edits_the_line_and_ignores_control_characters() {
        let mut b = ChatBox::default();
        assert!(!b.typing());
        assert!(!b.insert_char('a'), "a closed box takes nothing");
        b.open();
        assert!(b.typing());
        b.insert_str("hi\u{8}\n there");
        assert_eq!(b.text(), Some("hi there"));
        b.backspace();
        b.backspace();
        assert_eq!(b.text(), Some("hi the"));
        for _ in 0..20 {
            b.backspace();
        }
        assert_eq!(b.text(), Some(""), "backspace on an empty line is harmless");
    }

    #[test]
    fn the_line_stops_at_the_data_limit_and_is_cut_on_a_character_boundary() {
        let max = ChatRules::load().input_max;
        let b = typed(&"\u{e9}".repeat(max + 30));
        assert_eq!(b.text().unwrap().chars().count(), max, "typing stops at the limit");
        // two-byte letters: a byte cut would panic or split one
        let cut = cut_chars("\u{e9}\u{e9}\u{e9}\u{e9}", 3);
        assert_eq!(cut, "\u{e9}\u{e9}\u{e9}");
        assert_eq!(cut_chars("abc", 10), "abc");
        assert_eq!(cut_chars("\u{1f600}x\u{1f600}", 2), "\u{1f600}x");
        let mut b = typed(&format!("  {}  ", "w".repeat(max + 10)));
        let sent = b.submit().unwrap();
        assert!(sent.chars().count() <= max && !sent.starts_with(' '));
    }

    #[test]
    fn send_clears_and_closes_the_box() {
        let mut b = typed("  run!  ");
        assert_eq!(b.submit().as_deref(), Some("run!"));
        assert!(!b.typing(), "the box closes");
        assert_eq!(b.text(), None, "and holds nothing");
        assert_eq!(b.submit(), None, "a second Enter sends nothing");
        let mut empty = typed("   ");
        assert_eq!(empty.submit(), None, "a blank line is not sent");
        assert!(!empty.typing());
    }

    #[test]
    fn escape_closes_without_sending_and_a_reopened_box_starts_empty() {
        let mut b = typed("half a thought");
        b.close();
        assert!(!b.typing());
        b.open();
        assert_eq!(b.text(), Some(""));
        // not allowed (a menu took the keyboard): update closes it
        let mut b = typed("x");
        assert_eq!(b.update(false, &crate::app::bindings::Bindings::default()), None);
        assert!(!b.typing());
    }

    #[test]
    fn the_open_key_follows_the_data_and_names_pick_a_colour_by_peer() {
        assert_eq!(open_key_code("T"), KeyCode::T);
        assert_eq!(open_key_code("y"), KeyCode::Y);
        assert_eq!(open_key_code("?"), KeyCode::T, "a bad key falls back to T");
        assert_eq!(ChatRules::load().open_key, "T", "Standard value");
        let palette = ChatRules::load().name_colors.clone();
        assert!(!palette.is_empty());
        let (a, b) = (name_color(2, &palette), name_color(2 + palette.len() as i64, &palette));
        assert_eq!((a.r, a.g, a.b), (b.r, b.g, b.b), "the palette wraps");
        assert_ne!((name_color(2, &palette).r, name_color(2, &palette).g), (name_color(3, &palette).r, name_color(3, &palette).g));
        let _ = name_color(-5, &palette);
        let _ = name_color(1, &[]);
    }

    #[test]
    fn the_box_opens_on_the_chat_binding_and_a_rebind_moves_it() {
        let b = crate::app::bindings::Bindings::default();
        assert!(b.keys("chat").iter().any(|k| k == "T"), "CHAT starts on T (data/controls.json)");
        let moved = crate::app::bindings::Bindings::from_json(&serde_json::json!({ "chat": ["Y"] }));
        assert_eq!(moved.keys("chat"), vec!["Y".to_string()], "the rebound key is the one update reads");
    }
}
