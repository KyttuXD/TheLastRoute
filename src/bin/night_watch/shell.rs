//! The Night Watch app shell: the hub menu with the nine night types and QUIT
//! (Godot `night_watch_menu.gd`, website `index.html` hub). Selection logic is plain state so it
//! can be tested without a window; `draw` and `poll_input` are the macroquad edge.

use macroquad::prelude::*;

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.72);

/// Seconds of wall time per in-game hour on the drifting title clock.
const CLOCK_SECONDS_PER_HOUR: f32 = 12.0;

/// The night types with a playable driver (a screen, controls and a save). The others are ticked with no input.
/// `main.rs` starts each of these with its own driver instead of `modes::start`.
pub const PLAYABLE: [&str; 9] = ["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra", "mixed"];

pub struct HubEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub tag: &'static str,
}

/// The nine night types in hub order, as in the Godot mode table. An id is the key `modes::start` opens.
pub const HUB_ENTRIES: [HubEntry; 9] = [
    HubEntry { id: "classic", label: "1. CLASSIC", tag: "DOORS \u{b7} LIGHTS \u{b7} CAMERAS" },
    HubEntry { id: "mask", label: "2. MASKED HOURS", tag: "MASK \u{b7} MUSIC BOX \u{b7} VENTS" },
    HubEntry { id: "lure", label: "3. DEAD AIR", tag: "VENTS \u{b7} REBOOTS \u{b7} CLIMATE" },
    HubEntry { id: "bedroom", label: "4. BEDTIME", tag: "LISTEN \u{b7} NO CAMERAS" },
    HubEntry { id: "story", label: "5. THE FACILITY", tag: "ROOMS \u{b7} TASKS \u{b7} NOISE" },
    HubEntry { id: "descent", label: "6. THE DESCENT", tag: "WIRES \u{b7} SIGNALS \u{b7} PIPES" },
    HubEntry { id: "dayshift", label: "7. SALVAGE RUN", tag: "MANAGE \u{b7} SALVAGE" },
    HubEntry { id: "extra", label: "8. OPEN AIR", tag: "THREE WATCHES \u{b7} OUTDOORS" },
    HubEntry { id: "mixed", label: "9. MIXED SHIFT", tag: "ROSTER \u{b7} DIRECTOR \u{b7} CHAOS" },
];

/// Rows on the hub: the night types, then QUIT.
pub const ROW_COUNT: usize = HUB_ENTRIES.len() + 1;
const QUIT_ROW: usize = HUB_ENTRIES.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellInput {
    Up,
    Down,
    Confirm,
    Back,
    Hover(usize),
    Pick(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellAction {
    None,
    Quit,
    Start(&'static str),
}

pub struct Shell {
    selected: usize,
    toast: Option<String>,
    clock: f32,
}

impl Shell {
    pub fn new() -> Self {
        Self { selected: 0, toast: None, clock: 0.0 }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn toast(&self) -> Option<&str> {
        self.toast.as_deref()
    }

    pub fn set_toast(&mut self, text: impl Into<String>) {
        self.toast = Some(text.into());
    }

    pub fn tick(&mut self, dt: f32) {
        self.clock += dt;
    }

    /// The title clock label, drifting 12 AM to 6 AM like the Godot title.
    pub fn clock_label(&self) -> String {
        match (self.clock / CLOCK_SECONDS_PER_HOUR) as usize % 7 {
            0 => "12 AM".to_owned(),
            h => format!("{h} AM"),
        }
    }

    pub fn handle(&mut self, input: ShellInput) -> ShellAction {
        match input {
            ShellInput::Up => {
                self.selected = (self.selected + ROW_COUNT - 1) % ROW_COUNT;
                ShellAction::None
            }
            ShellInput::Down => {
                self.selected = (self.selected + 1) % ROW_COUNT;
                ShellAction::None
            }
            ShellInput::Back => ShellAction::Quit,
            ShellInput::Hover(row) => {
                if row < ROW_COUNT {
                    self.selected = row;
                }
                ShellAction::None
            }
            ShellInput::Pick(row) => {
                if row < ROW_COUNT {
                    self.selected = row;
                    return self.confirm();
                }
                ShellAction::None
            }
            ShellInput::Confirm => self.confirm(),
        }
    }

    fn confirm(&self) -> ShellAction {
        if self.selected == QUIT_ROW {
            return ShellAction::Quit;
        }
        ShellAction::Start(HUB_ENTRIES[self.selected].id)
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

const ROW_HEIGHT: f32 = 44.0;
const ROW_WIDTH: f32 = 520.0;

fn row_rect(row: usize) -> Rect {
    let top = (screen_height() - ROW_HEIGHT * ROW_COUNT as f32) * 0.5 + 60.0;
    Rect::new((screen_width() - ROW_WIDTH) * 0.5, top + ROW_HEIGHT * row as f32, ROW_WIDTH, ROW_HEIGHT - 4.0)
}

/// Reads this frame's keyboard and mouse into at most one shell input.
pub fn poll_input(shell: &Shell) -> Option<ShellInput> {
    if is_key_pressed(KeyCode::Escape) {
        return Some(ShellInput::Back);
    }
    if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W) {
        return Some(ShellInput::Up);
    }
    if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S) {
        return Some(ShellInput::Down);
    }
    if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
        return Some(ShellInput::Confirm);
    }
    let (mx, my) = mouse_position();
    let hovered = (0..ROW_COUNT).find(|&row| row_rect(row).contains(vec2(mx, my)));
    if is_mouse_button_pressed(MouseButton::Left) {
        return hovered.map(ShellInput::Pick);
    }
    hovered
        .filter(|&row| row != shell.selected() && mouse_delta_position() != Vec2::ZERO)
        .map(ShellInput::Hover)
}

pub fn draw(shell: &Shell) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let sw = screen_width();

    let clock = shell.clock_label();
    let clock_w = measure_text(&clock, None, 24, 1.0).width;
    draw_text(&clock, sw - clock_w - 24.0, 36.0, 24.0, INK);

    let night_w = measure_text("NIGHT", None, 64, 1.0).width;
    let watch_w = measure_text("WATCH", None, 64, 1.0).width;
    let gap = 18.0;
    let logo_x = (sw - (night_w + gap + watch_w)) * 0.5;
    let logo_y = row_rect(0).y - 56.0;
    draw_text("NIGHT", logo_x, logo_y, 64.0, INK);
    draw_text("WATCH", logo_x + night_w + gap, logo_y, 64.0, RED_TEXT);

    for row in 0..ROW_COUNT {
        let rect = row_rect(row);
        let active = row == shell.selected();
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
        if active {
            draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, AMBER);
        }
        let color = if active { AMBER } else { INK };
        if row == QUIT_ROW {
            draw_text("QUIT", rect.x + 16.0, rect.y + 28.0, 28.0, color);
        } else {
            let entry = &HUB_ENTRIES[row];
            draw_text(entry.label, rect.x + 16.0, rect.y + 28.0, 28.0, color);
            let tag_w = measure_text(entry.tag, None, 18, 1.0).width;
            draw_text(entry.tag, rect.x + rect.w - tag_w - 16.0, rect.y + 27.0, 18.0, DIM);
        }
    }

    if let Some(toast) = shell.toast() {
        let w = measure_text(toast, None, 24, 1.0).width;
        draw_text(toast, (sw - w) * 0.5, screen_height() - 48.0, 24.0, AMBER);
    }
    let hint = "UP / DOWN  ENTER START  ESC QUIT";
    let hint_w = measure_text(hint, None, 18, 1.0).width;
    draw_text(hint, (sw - hint_w) * 0.5, screen_height() - 20.0, 18.0, DIM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_wraps_both_ways() {
        let mut shell = Shell::new();
        shell.handle(ShellInput::Up);
        assert_eq!(shell.selected(), QUIT_ROW);
        shell.handle(ShellInput::Down);
        assert_eq!(shell.selected(), 0);
    }

    #[test]
    fn confirm_starts_the_selected_night() {
        let mut shell = Shell::new();
        shell.handle(ShellInput::Down);
        assert_eq!(shell.handle(ShellInput::Confirm), ShellAction::Start("mask"));
    }

    #[test]
    fn quit_row_and_back_quit() {
        let mut shell = Shell::new();
        assert_eq!(shell.handle(ShellInput::Pick(QUIT_ROW)), ShellAction::Quit);
        assert_eq!(shell.handle(ShellInput::Back), ShellAction::Quit);
    }

    #[test]
    fn every_playable_night_is_on_the_hub() {
        for id in PLAYABLE {
            assert!(HUB_ENTRIES.iter().any(|entry| entry.id == id), "{id} is not on the hub");
        }
    }

    #[test]
    fn clock_drifts_from_midnight_to_six() {
        let mut shell = Shell::new();
        assert_eq!(shell.clock_label(), "12 AM");
        shell.tick(CLOCK_SECONDS_PER_HOUR * 6.0);
        assert_eq!(shell.clock_label(), "6 AM");
    }
}
