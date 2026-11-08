//! Night Watch UI kit: palette and 480x270 canvas scale (`theme`), text helpers (`text`), immediate-mode widgets (`widgets`).
#![allow(dead_code)]

pub mod text;
pub mod theme;
pub mod widgets;

pub use text::{fit, fit_with, glyph, line_height, measure, text, text_center, text_right, wrap, wrap_with};
pub use theme::*;
pub use widgets::{apply_text_char, button_colors, slider_value_at, ButtonStyle, Ui, UiResult};
