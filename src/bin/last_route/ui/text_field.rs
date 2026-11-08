//! Single-line text fields in the menus: what each kind of field takes, and the keyboard for it, including the clipboard
//! shortcuts (Ctrl or Cmd with V paste, C copy, X cut, A select all; Shift+Insert also pastes).
//!
//! The fields have no caret: text is added and removed at the end. Select all therefore marks the whole field, and the next
//! typed character, paste, Backspace or Delete replaces or clears it. The rules (which characters, how many, what may be
//! copied) are plain functions so they are tested without a window; [`edit`] reads the keyboard for one frame and hands it
//! to [`apply`].
//!
//! Fields served: the JOIN ROOM code, the ACCOUNT sign-in form's username and password, the ACCOUNT SETTINGS username and
//! the admin menu's FIELDS boxes. The SKIN tab's link box has a caret of its own (`skin_tab.rs`): it takes its typing and
//! its paste through [`Kind::Link`] and shares the clipboard and the shortcut keys with the fields here.

use super::clipboard;
use macroquad::input::{get_char_pressed, is_key_down, is_key_pressed, KeyCode};

/// What a field accepts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// An account name: A-Z 0-9 _, shown upper case, at most [`USERNAME_MAX`].
    Username,
    /// A password: any printable character (spaces inside are fine), at most [`PASSWORD_MAX`]. Never copied.
    Password,
    /// A room code: letters and digits shown upper case, [`CODE_MAX`] of them. A paste replaces the whole code.
    Code,
    /// Any printable text, at most [`FREE_MAX`] (the admin menu's boxes). No select all.
    Free,
    /// A link (the SKIN tab's URL box): printable ASCII without spaces, at most [`LINK_MAX`]. A paste drops every space and
    /// line break, so a link copied with a trailing newline lands whole. No select all.
    Link,
}

pub const USERNAME_MAX: usize = 24;
pub const PASSWORD_MAX: usize = 200;
pub const CODE_MAX: usize = crate::net::api::CODE_LENGTH;
pub const FREE_MAX: usize = 120;
/// The longest link the SKIN box holds (data/skins/rules.json `max_length`, the cap the box uses at runtime).
pub const LINK_MAX: usize = 512;

impl Kind {
    /// The most characters the field holds.
    pub fn max(self) -> usize {
        match self {
            Kind::Username => USERNAME_MAX,
            Kind::Password => PASSWORD_MAX,
            Kind::Code => CODE_MAX,
            Kind::Free => FREE_MAX,
            Kind::Link => LINK_MAX,
        }
    }

    /// A typed character as the field keeps it, or None when the field does not take it.
    pub fn typed(self, ch: char) -> Option<char> {
        match self {
            Kind::Username => (ch.is_ascii_alphanumeric() || ch == '_').then(|| ch.to_ascii_uppercase()),
            Kind::Code => ch.is_ascii_alphanumeric().then(|| ch.to_ascii_uppercase()),
            Kind::Password | Kind::Free => (!ch.is_control()).then_some(ch),
            Kind::Link => ch.is_ascii_graphic().then_some(ch),
        }
    }

    /// What Ctrl/Cmd+C or +X puts on the clipboard: the field's text, or None for an empty field and for a password,
    /// which never leaves the game through the clipboard.
    pub fn copy_text(self, text: &str) -> Option<String> {
        if self == Kind::Password || text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    /// A paste replaces the field instead of adding to it (a room code is one unit).
    fn paste_replaces(self) -> bool {
        self == Kind::Code
    }

    fn can_select_all(self) -> bool {
        !matches!(self, Kind::Free | Kind::Link)
    }
}

/// What a paste of `raw` puts in a field of this kind: at most `room` characters, filtered like typed text.
///
/// * Username: every whitespace and line break goes, then the typed filter (A-Z 0-9 _, upper case).
/// * Link: the same, with the printable ASCII filter, so a link copied with a trailing newline lands whole.
/// * Code: see [`code_from_paste`].
/// * Password and free text: the text is trimmed, only its first line is used (no line break ever lands in the field),
///   control characters go, spaces inside stay.
pub fn pasted(kind: Kind, raw: &str, room: usize) -> String {
    match kind {
        Kind::Username | Kind::Link => raw.chars().filter(|c| !c.is_whitespace()).filter_map(|c| kind.typed(c)).take(room).collect(),
        Kind::Code => code_from_paste(raw).chars().take(room).collect(),
        Kind::Password | Kind::Free => {
            // invisible marks (zero-width spaces and joiners-of-nothing, BOM, bidi overrides) that a copy from a web page or
            // a chat carries along are not part of any password the player typed, and would make a right password wrong
            let clean: String = raw.chars().filter(|&c| !invisible_mark(c)).collect();
            let line = clean.trim().split(['\r', '\n', '\u{2028}', '\u{2029}']).next().unwrap_or("").trim_end();
            line.chars().filter_map(|c| kind.typed(c)).take(room).collect()
        }
    }
}

/// A character that shows nothing: zero-width space, soft hyphen, word joiner, BOM, and the left-to-right / right-to-left
/// marks, embeddings and overrides (a pasted right-to-left override could reverse how a text reads). The joiner and
/// non-joiner (U+200C, U+200D) stay: emoji are built with them.
pub fn invisible_mark(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{061C}' | '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}' | '\u{FEFF}'
    )
}

/// The room code in pasted text. A paste of one word (the code, with or without spaces or a line break around it) is
/// that word typed in: letters and digits, upper case. A paste of several words (a message that carries the code) uses
/// the first word that is a code of the room alphabet (`api::valid_room_code`, as COPY CODE writes it), else the first
/// that the join dialog accepts (`world_sync::parse_join_code`: a reserved code too), with punctuation around the word
/// ("code: ABC12.") ignored. When no word is a code, the text is filtered like typing it in.
pub fn code_from_paste(raw: &str) -> String {
    let words: Vec<&str> = raw.split_whitespace().collect();
    if words.len() > 1 {
        let clean = |w: &str| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_uppercase();
        let found = words
            .iter()
            .map(|w| clean(w))
            .find(|w| crate::net::api::valid_room_code(w))
            .or_else(|| words.iter().map(|w| clean(w)).find(|w| crate::net::world_sync::parse_join_code(w).is_some()));
        if let Some(code) = found {
            return code;
        }
    }
    raw.chars().filter_map(|c| Kind::Code.typed(c)).collect()
}

/// A clipboard shortcut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shortcut {
    Paste,
    Copy,
    Cut,
    SelectAll,
}

/// The keys of the shortcuts that went down this frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct Pressed {
    pub v: bool,
    pub c: bool,
    pub x: bool,
    pub a: bool,
    pub insert: bool,
}

/// Whether the "command" modifier is held: Cmd (Super) on macOS, Ctrl elsewhere. Ctrl with Alt is AltGr on many
/// keyboards (Windows reports it as both keys), which types characters such as @ and must not count.
pub fn command_held(ctrl: bool, alt: bool, sup: bool) -> bool {
    sup || (ctrl && !alt)
}

/// The shortcut the keys of this frame make, if any.
pub fn classify(command: bool, shift: bool, k: Pressed) -> Option<Shortcut> {
    if command {
        if k.v {
            return Some(Shortcut::Paste);
        }
        if k.c {
            return Some(Shortcut::Copy);
        }
        if k.x {
            return Some(Shortcut::Cut);
        }
        if k.a {
            return Some(Shortcut::SelectAll);
        }
    }
    (shift && !command && k.insert).then_some(Shortcut::Paste)
}

/// Whether Cmd/Ctrl is held right now.
pub fn command_down() -> bool {
    let any = |a: KeyCode, b: KeyCode| is_key_down(a) || is_key_down(b);
    command_held(
        any(KeyCode::LeftControl, KeyCode::RightControl),
        any(KeyCode::LeftAlt, KeyCode::RightAlt),
        any(KeyCode::LeftSuper, KeyCode::RightSuper),
    )
}

/// The clipboard shortcut pressed this frame, if any.
pub fn shortcut_pressed() -> Option<Shortcut> {
    let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    classify(
        command_down(),
        shift,
        Pressed {
            v: is_key_pressed(KeyCode::V),
            c: is_key_pressed(KeyCode::C),
            x: is_key_pressed(KeyCode::X),
            a: is_key_pressed(KeyCode::A),
            insert: is_key_pressed(KeyCode::Insert),
        },
    )
}

/// One frame of keyboard for a field.
#[derive(Default, Debug)]
pub struct Input {
    /// The characters typed this frame, in order.
    pub chars: Vec<char>,
    /// A command key is held: the typed characters are shortcut leftovers and are ignored.
    pub command: bool,
    pub shortcut: Option<Shortcut>,
    pub backspace: bool,
    pub delete: bool,
    /// The text the paste delivered this frame.
    pub paste: Option<String>,
}

/// What a frame of input asks of the clipboard.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// Text to put on the system clipboard (a copy, or a cut that also emptied the field).
    pub copy: Option<String>,
    /// The field's text changed.
    pub changed: bool,
}

/// Applies one frame of `input` to `field`. `selected` is the select-all mark.
pub fn apply(kind: Kind, field: &mut String, selected: &mut bool, input: &Input) -> Outcome {
    let before = field.clone();
    let max = kind.max();
    if !input.command {
        for &ch in &input.chars {
            let Some(c) = kind.typed(ch) else { continue };
            if *selected {
                field.clear();
                *selected = false;
            }
            if field.chars().count() < max {
                field.push(c);
            }
        }
    }
    if input.backspace || input.delete {
        if *selected {
            field.clear();
            *selected = false;
        } else if input.backspace {
            field.pop();
        }
    }
    let mut out = Outcome::default();
    match input.shortcut {
        Some(Shortcut::SelectAll) if kind.can_select_all() => *selected = !field.is_empty(),
        Some(Shortcut::Copy) => out.copy = kind.copy_text(field),
        Some(Shortcut::Cut) => {
            // a password stays as it is: cutting it copies nothing, so it must not delete it either
            if let Some(text) = kind.copy_text(field) {
                out.copy = Some(text);
                field.clear();
                *selected = false;
            }
        }
        _ => {}
    }
    if let Some(raw) = &input.paste {
        let replace = kind.paste_replaces() || *selected;
        let room = if replace { max } else { max.saturating_sub(field.chars().count()) };
        let add = pasted(kind, raw, room);
        if !add.is_empty() {
            if replace {
                field.clear();
            }
            field.push_str(&add);
            *selected = false;
        }
    }
    if field.is_empty() {
        *selected = false;
    }
    out.changed = *field != before;
    out
}

/// Reads this frame's keyboard for the focused field and applies it; copies and cuts go to the system clipboard. Call it
/// once per frame, for the focused field only (it takes the frame's typed characters). Returns whether the text changed.
pub fn edit(kind: Kind, field: &mut String, selected: &mut bool) -> bool {
    let command = command_down();
    let mut chars = vec![];
    while let Some(ch) = get_char_pressed() {
        chars.push(ch);
    }
    let shortcut = shortcut_pressed();
    let input = Input {
        chars,
        command,
        shortcut,
        backspace: is_key_pressed(KeyCode::Backspace),
        delete: is_key_pressed(KeyCode::Delete),
        paste: clipboard::poll_paste(shortcut == Some(Shortcut::Paste)),
    };
    let out = apply(kind, field, selected, &input);
    if let Some(text) = &out.copy {
        clipboard::copy(text);
    }
    // the page answers the browser's own copy/cut event from this, so it must be fresh every frame
    clipboard::offer(&kind.copy_text(field).unwrap_or_default());
    out.changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Input {
        Input { chars: s.chars().collect(), ..Input::default() }
    }
    fn with(shortcut: Shortcut) -> Input {
        Input { command: true, shortcut: Some(shortcut), ..Input::default() }
    }
    fn paste(s: &str) -> Input {
        Input { paste: Some(s.to_string()), ..Input::default() }
    }

    #[test]
    fn typed_characters_follow_each_kind() {
        assert_eq!(Kind::Username.typed('a'), Some('A'));
        assert_eq!(Kind::Username.typed('_'), Some('_'));
        assert_eq!(Kind::Username.typed('-'), None);
        assert_eq!(Kind::Username.typed(' '), None);
        assert_eq!(Kind::Code.typed('z'), Some('Z'));
        assert_eq!(Kind::Code.typed('_'), None);
        assert_eq!(Kind::Password.typed('a'), Some('a'), "a password keeps its case");
        assert_eq!(Kind::Password.typed(' '), Some(' '));
        assert_eq!(Kind::Password.typed('\u{16}'), None, "the control character Ctrl+V types is not text");
        assert_eq!(Kind::Free.typed('\n'), None);
    }

    #[test]
    fn a_pasted_username_is_filtered_like_typing() {
        assert_eq!(pasted(Kind::Username, "  ayttu_01\r\n", 24), "AYTTU_01");
        assert_eq!(pasted(Kind::Username, "ay ttu\tx-y", 24), "AYTTUXY", "whitespace goes, other symbols go");
        assert_eq!(pasted(Kind::Username, "naïve", 24), "NAVE");
        assert_eq!(pasted(Kind::Username, &"a".repeat(40), 24).len(), 24, "capped at the field length");
        assert_eq!(pasted(Kind::Username, "abcdef", 3), "ABC", "the room left caps it");
        assert_eq!(pasted(Kind::Username, "", 24), "");
    }

    #[test]
    fn a_pasted_link_lands_whole() {
        assert_eq!(pasted(Kind::Link, "https://example.com/a.png\r\n", 512), "https://example.com/a.png", "a trailing newline goes");
        assert_eq!(pasted(Kind::Link, "  https://example.com/a.png  \n", 512), "https://example.com/a.png");
        assert_eq!(pasted(Kind::Link, "https://exa mple.com", 512), "https://example.com", "spaces inside go");
        assert_eq!(pasted(Kind::Link, "https://example.com/long", 10), "https://ex", "the room left caps it");
        assert_eq!(pasted(Kind::Link, "naïve\u{202e}", 512), "nave", "non-ASCII and control characters go");
        assert_eq!(Kind::Link.typed(' '), None, "a typed space is refused");
        assert_eq!(Kind::Link.typed('/'), Some('/'), "a link keeps its case and symbols");
        assert_eq!(Kind::Link.copy_text("https://a.b/c.png").as_deref(), Some("https://a.b/c.png"));
        assert_eq!(Kind::Link.copy_text(""), None);
    }

    #[test]
    fn a_pasted_password_is_one_printable_line() {
        assert_eq!(pasted(Kind::Password, "Tmp-Pass_9!\n", 200), "Tmp-Pass_9!");
        assert_eq!(pasted(Kind::Password, "  Tmp-Pass_9!  \r\n", 200), "Tmp-Pass_9!", "surrounding whitespace goes");
        assert_eq!(pasted(Kind::Password, "two words\nsecond line", 200), "two words", "spaces inside stay, the second line does not");
        assert_eq!(pasted(Kind::Password, "a\u{0}b\u{7f}c", 200), "abc", "control characters go");
        assert_eq!(pasted(Kind::Password, "Ünï€", 200), "Ünï€", "non-ASCII printable characters stay");
        assert_eq!(pasted(Kind::Password, &"p".repeat(300), 200).len(), 200);
        assert_eq!(pasted(Kind::Password, "\u{FEFF}Tmp\u{200B}Pass\u{202E}9\u{200F}", 200), "TmpPass9", "invisible and bidi marks go");
        assert_eq!(pasted(Kind::Password, "ab\u{2028}cd", 200), "ab", "a Unicode line separator ends the line too");
        assert_eq!(pasted(Kind::Password, "\u{FEFF}  pw  ", 200), "pw", "a BOM in front does not stop the trim");
    }

    #[test]
    fn a_pasted_code_is_a_code() {
        assert_eq!(pasted(Kind::Code, "abc12", 5), "ABC12");
        assert_eq!(pasted(Kind::Code, "  abc12 \r\n", 5), "ABC12", "surrounding whitespace goes");
        assert_eq!(pasted(Kind::Code, "ABC 12", 5), "ABC12", "spaces inside a lone paste are typed away");
        assert_eq!(pasted(Kind::Code, "ABC12345", 5), "ABC12", "capped at five");
        assert_eq!(pasted(Kind::Code, "AB-C12", 5), "ABC12");
    }

    #[test]
    fn a_message_that_carries_the_code_gives_the_code() {
        assert_eq!(code_from_paste("Join my room: ABC12"), "ABC12");
        assert_eq!(code_from_paste("code ABC12. see you"), "ABC12", "punctuation around the word is ignored");
        assert_eq!(code_from_paste("(abc12)\nthanks"), "ABC12");
        assert_eq!(code_from_paste("HELLO ABC23"), "ABC23", "a code of the room alphabet (no O, I, 0 or 1) wins over a reserved-looking word");
        assert_eq!(code_from_paste("HELLO ABC12"), "HELLO", "with no alphabet code, the first word the join dialog accepts");
        assert_eq!(code_from_paste("let me in"), "LETMEIN", "no code in it: filtered like typing");
        // a reserved code (the join dialog accepts I, O, 0 and 1 for an owner's code) is used when nothing better is there
        assert_eq!(code_from_paste("room O1O1O please"), "O1O1O");
    }

    #[test]
    fn copy_and_cut_never_give_away_a_password() {
        assert_eq!(Kind::Password.copy_text("secret"), None);
        assert_eq!(Kind::Username.copy_text("AYTTU"), Some("AYTTU".to_string()));
        assert_eq!(Kind::Code.copy_text("ABC12"), Some("ABC12".to_string()));
        assert_eq!(Kind::Code.copy_text(""), None, "an empty field copies nothing");

        let (mut pass, mut sel) = ("secret".to_string(), true);
        assert_eq!(apply(Kind::Password, &mut pass, &mut sel, &with(Shortcut::Copy)).copy, None);
        let out = apply(Kind::Password, &mut pass, &mut sel, &with(Shortcut::Cut));
        assert_eq!(out.copy, None, "cut copies nothing from a password");
        assert_eq!(pass, "secret", "and does not delete it either");
        assert!(!out.changed);
    }

    #[test]
    fn copy_takes_the_text_and_cut_empties_the_field() {
        let (mut name, mut sel) = ("AYTTU".to_string(), false);
        let out = apply(Kind::Username, &mut name, &mut sel, &with(Shortcut::Copy));
        assert_eq!((out.copy.as_deref(), name.as_str(), out.changed), (Some("AYTTU"), "AYTTU", false));
        let out = apply(Kind::Username, &mut name, &mut sel, &with(Shortcut::Cut));
        assert_eq!((out.copy.as_deref(), name.as_str(), out.changed), (Some("AYTTU"), "", true));
    }

    #[test]
    fn select_all_then_typing_or_pasting_replaces() {
        let (mut name, mut sel) = ("OLD".to_string(), false);
        apply(Kind::Username, &mut name, &mut sel, &with(Shortcut::SelectAll));
        assert!(sel);
        apply(Kind::Username, &mut name, &mut sel, &typed("n"));
        assert_eq!((name.as_str(), sel), ("N", false), "a typed character replaces the selection");

        let (mut name, mut sel) = ("OLD".to_string(), true);
        apply(Kind::Username, &mut name, &mut sel, &paste("fresh"));
        assert_eq!((name.as_str(), sel), ("FRESH", false), "a paste replaces the selection");

        let (mut name, mut sel) = ("OLD".to_string(), true);
        apply(Kind::Username, &mut name, &mut sel, &Input { backspace: true, ..Input::default() });
        assert_eq!((name.as_str(), sel), ("", false), "Backspace clears the selection");

        let (mut name, mut sel) = ("OLD".to_string(), true);
        apply(Kind::Username, &mut name, &mut sel, &typed("!"));
        assert_eq!((name.as_str(), sel), ("OLD", true), "a character the field refuses keeps the selection");

        let (mut empty, mut sel) = (String::new(), false);
        apply(Kind::Username, &mut empty, &mut sel, &with(Shortcut::SelectAll));
        assert!(!sel, "there is nothing to select in an empty field");

        let (mut free, mut sel) = ("abc".to_string(), false);
        apply(Kind::Free, &mut free, &mut sel, &with(Shortcut::SelectAll));
        assert!(!sel, "free text boxes have no select all");
    }

    #[test]
    fn a_paste_adds_up_to_the_cap_or_replaces() {
        let (mut name, mut sel) = ("ABCDEFGHIJKLMNOPQRSTUV".to_string(), false);
        let out = apply(Kind::Username, &mut name, &mut sel, &paste("xyz123"));
        assert_eq!(name, "ABCDEFGHIJKLMNOPQRSTUVXY", "only the room left is added");
        assert!(out.changed);

        let (mut code, mut sel) = ("AB".to_string(), false);
        apply(Kind::Code, &mut code, &mut sel, &paste("  zz9yy \n"));
        assert_eq!(code, "ZZ9YY", "a pasted code replaces what was typed");

        let (mut code, mut sel) = ("AB".to_string(), false);
        let out = apply(Kind::Code, &mut code, &mut sel, &paste("\n \r\n"));
        assert_eq!((code.as_str(), out.changed), ("AB", false), "a paste with nothing usable changes nothing");

        let (mut pass, mut sel) = ("abc".to_string(), false);
        apply(Kind::Password, &mut pass, &mut sel, &paste("def\nghi"));
        assert_eq!(pass, "abcdef", "a password paste adds its first line");
    }

    #[test]
    fn typing_respects_the_cap_and_the_command_key() {
        let (mut code, mut sel) = (String::new(), false);
        apply(Kind::Code, &mut code, &mut sel, &typed("ab-c1d2e"));
        assert_eq!(code, "ABC1D", "five characters, symbols skipped, upper case");

        // Ctrl+V: the control character and any leftover letter are not typed
        let (mut name, mut sel) = (String::new(), false);
        let input = Input { chars: vec!['\u{16}', 'v'], command: true, ..Input::default() };
        apply(Kind::Username, &mut name, &mut sel, &input);
        assert_eq!(name, "", "a letter that arrives with the command key down is not typed");

        let (mut pass, mut sel) = ("ab".to_string(), false);
        apply(Kind::Password, &mut pass, &mut sel, &Input { backspace: true, ..Input::default() });
        assert_eq!(pass, "a");
    }

    #[test]
    fn shortcuts_need_the_command_key() {
        let v = Pressed { v: true, ..Pressed::default() };
        assert_eq!(classify(true, false, v), Some(Shortcut::Paste));
        assert_eq!(classify(false, false, v), None, "a bare V is a letter");
        assert_eq!(classify(true, false, Pressed { c: true, ..Pressed::default() }), Some(Shortcut::Copy));
        assert_eq!(classify(true, false, Pressed { x: true, ..Pressed::default() }), Some(Shortcut::Cut));
        assert_eq!(classify(true, false, Pressed { a: true, ..Pressed::default() }), Some(Shortcut::SelectAll));
        let insert = Pressed { insert: true, ..Pressed::default() };
        assert_eq!(classify(false, true, insert), Some(Shortcut::Paste), "Shift+Insert pastes");
        assert_eq!(classify(false, false, insert), None);
        assert_eq!(classify(true, false, Pressed::default()), None);
    }

    #[test]
    fn altgr_is_not_the_command_key() {
        assert!(command_held(true, false, false), "Ctrl");
        assert!(command_held(false, false, true), "Cmd");
        assert!(!command_held(true, true, false), "Ctrl+Alt is AltGr: it types characters");
        assert!(command_held(true, true, true), "Cmd counts even with Alt");
        assert!(!command_held(false, false, false));
        assert!(!command_held(false, true, false));
    }

    #[test]
    fn the_input_for_a_frame_with_nothing_typed_changes_nothing() {
        let (mut name, mut sel) = ("AYTTU".to_string(), true);
        let out = apply(Kind::Username, &mut name, &mut sel, &Input::default());
        assert_eq!((name.as_str(), sel, out), ("AYTTU", true, Outcome::default()));
    }
}
