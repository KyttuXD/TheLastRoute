//! The call box on screen: the caller's name, the line typed so far and the blips that go with each letter.
//! `CallView::update` plays the blips a `CallScript` owes this frame; `draw` paints the box. The wrap is pure.

use super::blip::BlipPlayer;
use super::framework::CallScript;
use macroquad::prelude::*;

/// Breaks `text` into lines of at most `max_width` by the `width` measure, at spaces (a long word gets its own line).
pub fn wrap(text: &str, max_width: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if width(&format!("{line} {word}")) <= max_width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The part of the wrapped `lines` that is revealed: the first `chars` characters of the full text, counting the
/// space each wrap swallowed, so words stay where they will end up while they are typed.
pub fn reveal(lines: &[String], chars: usize) -> Vec<String> {
    let mut left = chars;
    let mut out = Vec::new();
    for line in lines {
        let n = line.chars().count();
        out.push(line.chars().take(left).collect());
        if left <= n {
            break;
        }
        left -= n + 1;
    }
    out
}

pub struct CallView {
    pub player: BlipPlayer,
}

impl CallView {
    pub fn new() -> Self {
        Self { player: BlipPlayer::new() }
    }

    /// Plays this frame's blips, one per newly revealed letter, in the speaker's voice. Call after `script.update`.
    pub fn update(&mut self, script: &CallScript) {
        if script.open && !script.blip_letters.is_empty() {
            self.player.play_all(script.speaker(), &script.blip_letters);
        }
    }

    /// Draws the box inside `area` while the call is open: the name tag on top, the typed text below.
    pub fn draw(&self, script: &CallScript, area: Rect) {
        if !script.open {
            return;
        }
        let pad = 12.0;
        let size = 22.0;
        draw_rectangle(area.x, area.y, area.w, area.h, Color::new(0.04, 0.05, 0.08, 0.88));
        draw_rectangle_lines(area.x, area.y, area.w, area.h, 2.0, Color::new(0.45, 0.8, 0.95, 1.0));
        draw_text(script.speaker_name(), area.x + pad, area.y + pad + 14.0, size, Color::new(0.45, 0.8, 0.95, 1.0));
        // While it rings the text is the "Incoming call" line, which is not typed.
        let full = if script.line < 0 || script.text == crate::data::CALL_INCOMING_TEXT { script.text.as_str() } else { script.full_text() };
        let shown = script.text.chars().count();
        let wrapped = wrap(full, area.w - 2.0 * pad, |s| measure_text(s, None, size as u16, 1.0).width);
        for (i, line) in reveal(&wrapped, shown).iter().enumerate() {
            draw_text(line, area.x + pad, area.y + pad + 14.0 + (i as f32 + 1.4) * (size + 4.0), size, WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ten(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wrap_breaks_at_spaces_within_the_width() {
        assert_eq!(wrap("one two three", 70.0, ten), vec!["one two", "three"]);
        assert_eq!(wrap("short", 500.0, ten), vec!["short"]);
        assert_eq!(wrap("unbreakable word", 50.0, ten), vec!["unbreakable", "word"]);
        assert_eq!(wrap("", 50.0, ten), vec![""]);
    }

    #[test]
    fn reveal_follows_the_typed_character_count_across_lines() {
        let lines = wrap("one two three", 70.0, ten);
        assert_eq!(reveal(&lines, 0), vec![""]);
        assert_eq!(reveal(&lines, 5), vec!["one t"]);
        assert_eq!(reveal(&lines, 7), vec!["one two"]);
        assert_eq!(reveal(&lines, 8), vec!["one two", ""]);
        assert_eq!(reveal(&lines, 10), vec!["one two", "th"]);
        assert_eq!(reveal(&lines, 13), vec!["one two", "three"]);
        assert_eq!(reveal(&lines, 99), vec!["one two", "three"]);
    }
}
