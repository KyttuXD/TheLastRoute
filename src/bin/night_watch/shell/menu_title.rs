//! The title screen: logo, PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT.
#![allow(dead_code)]

use super::*;

/// The game title is "KYTTU - Night Watch"; the logo draws this prefix, then NIGHT, then WATCH in red.
const GAME_PREFIX: &str = "KYTTU -";

enum TitleGo {
    Go(Screen),
    Quit,
}

struct TitleItem {
    label: &'static str,
    sub: String,
    disabled: bool,
    go: TitleGo,
}

impl Shell {
    pub(super) fn title(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let custom_ok = c.save.custom_unlocked("classic");
        let items = vec![
            TitleItem { label: "PLAY", sub: String::new(), disabled: false, go: TitleGo::Go(Screen::Board) },
            TitleItem {
                label: "CUSTOM NIGHT",
                sub: if custom_ok { String::new() } else { "BEAT NIGHT 5 TO UNLOCK".to_string() },
                disabled: !custom_ok,
                go: TitleGo::Go(Screen::Custom),
            },
            TitleItem { label: "ACCOUNT", sub: Shell::account_label(), disabled: false, go: TitleGo::Go(Screen::Account) },
            TitleItem { label: "SETTINGS", sub: String::new(), disabled: false, go: TitleGo::Go(Screen::Settings) },
            TitleItem { label: "CREDITS", sub: String::new(), disabled: false, go: TitleGo::Go(Screen::Credits) },
            TitleItem { label: "QUIT", sub: String::new(), disabled: false, go: TitleGo::Quit },
        ];
        let n = items.len();
        self.title_index = self.title_index.min(n - 1);
        let mut act = ShellAction::None;

        if c.keys.down {
            self.title_index = wrap_index(self.title_index, 1, n);
            c.cue("move");
        }
        if c.keys.up {
            self.title_index = wrap_index(self.title_index, -1, n);
            c.cue("move");
        }
        if c.keys.enter {
            act = self.title_activate(c, &items[self.title_index]);
        }

        // logo: "KYTTU - Night Watch" as KYTTU - NIGHT in ink, WATCH in red, 6 px apart, centred
        let wk = measure(GAME_PREFIX, 22.0);
        let wn = measure("NIGHT", 22.0);
        let ww = measure("WATCH", 22.0);
        let x0 = 240.0 - (wk + 6.0 + wn + 6.0 + ww) * 0.5;
        text(GAME_PREFIX, x0, 22.0, 22.0, INK);
        text("NIGHT", x0 + wk + 6.0, 22.0, 22.0, INK);
        text("WATCH", x0 + wk + 6.0 + wn + 6.0, 22.0, 22.0, RED_TEXT);

        let moved = c.input.mouse_delta.0 != 0.0 || c.input.mouse_delta.1 != 0.0;
        let mut y = 64.0;
        for (i, it) in items.iter().enumerate() {
            let r = Rect::new(150.0, y, 180.0, 15.0);
            let res = c.ui.region(r, !it.disabled);
            if res.hovered && moved {
                self.title_index = i;
            }
            if (res.clicked || res.denied) && matches!(act, ShellAction::None) {
                self.title_index = i;
                act = self.title_activate(c, it);
            }
            let sel = i == self.title_index;
            if sel {
                fill_rect(r, FOCUS_WASH);
            }
            let label = if sel { format!(">> {}", it.label) } else { it.label.to_string() };
            text_center(&label, 240.0, y + 3.5, 8.0, if it.disabled { DIM } else { INK });
            y += 16.0;
            if !it.sub.is_empty() {
                text_center(&it.sub, 240.0, y, 5.0, BODY);
                y += 8.0;
            }
        }
        act
    }

    fn title_activate(&mut self, c: &mut Ctx<'_>, it: &TitleItem) -> ShellAction {
        if it.disabled {
            self.toast = "Custom Night is locked. Beat Night 5.".to_string();
            c.cue("deny");
            return ShellAction::None;
        }
        match it.go {
            TitleGo::Quit => ShellAction::Quit,
            TitleGo::Go(s) => {
                c.cue("pick");
                self.go(s);
                ShellAction::None
            }
        }
    }
}
