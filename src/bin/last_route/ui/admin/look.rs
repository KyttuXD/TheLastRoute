//! The admin screens' look, after game/config/palette.gd and game/ui_common/ref_theme.tres: the
//! Last Route palette tokens, Courier text measured in canvas units (the original lays the admin
//! menu out on the 480x270 design canvas that expands with the window), StyleBoxFlat-like boxes,
//! the button / toggle-row / dropdown / tab faces and Godot's default PopupMenu used by every
//! OptionButton.

use crate::app::assets::Assets;
use macroquad::prelude::*;

// -- Palette (game/config/palette.gd, Last Route scheme) ------------------------------------
pub const PANEL: Color = Color::new(0.0902, 0.0549, 0.0980, 1.0); // 170e19
pub const PANEL_EDGE: Color = Color::new(0.1804, 0.1216, 0.2000, 1.0); // 2e1f33
pub const INSET: Color = Color::new(0.1176, 0.0745, 0.1333, 1.0); // 1e1322
pub const INK: Color = Color::new(0.9490, 0.9294, 0.8824, 1.0); // f2ede1
pub const BODY: Color = Color::new(0.7373, 0.7020, 0.6588, 1.0); // bcb3a8
pub const MUTED: Color = Color::new(0.5490, 0.5059, 0.4941, 1.0); // 8c817e
pub const AMBER: Color = Color::new(0.7882, 0.6627, 0.8471, 1.0); // c9a9d8
pub const ON_AMBER: Color = Color::new(0.1020, 0.0588, 0.0784, 1.0); // 1a0f14
pub const ACCENT: Color = Color::new(0.6510, 0.4980, 0.6902, 1.0); // a67fb0
pub const ACCENT_EDGE: Color = Color::new(0.7686, 0.6353, 0.8157, 1.0); // c4a2d0
pub const ACCENT_DEEP: Color = Color::new(0.3529, 0.2471, 0.3882, 1.0); // 5a3f63
pub const TEAL: Color = Color::new(0.2157, 0.7255, 0.6902, 1.0); // 37b9b0
pub const ALERT: Color = Color::new(0.8863, 0.3176, 0.3647, 1.0); // e2515d
pub const STABLE: Color = Color::new(0.4353, 0.7490, 0.5412, 1.0); // 6fbf8a
pub const FIELD_EDGE: Color = Color::new(0.2941, 0.2235, 0.2824, 1.0); // 4b3948
pub const KEYCAP_EDGE: Color = Color::new(0.4157, 0.3216, 0.3922, 1.0); // 6a5264
/// ref_theme.tres TabBar tab_hovered fill.
pub const TAB_HOVER: Color = Color::new(0.1812, 0.1494, 0.1882, 1.0);

pub fn lightened(c: Color, a: f32) -> Color {
    Color::new(
        c.r + (1.0 - c.r) * a,
        c.g + (1.0 - c.g) * a,
        c.b + (1.0 - c.b) * a,
        c.a,
    )
}

pub fn darkened(c: Color, a: f32) -> Color {
    Color::new(c.r * (1.0 - a), c.g * (1.0 - a), c.b * (1.0 - a), c.a)
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, a)
}

/// Pixels per canvas unit: the window's expand-stretch factor on the 480x270 design canvas.
pub fn px_per_unit() -> f32 {
    crate::ui::style::unit().max(0.1)
}

/// Line box height of the Courier face at a font size (rounded ascent + descent, as Godot's
/// label metrics come out: 6 -> 7, 7 -> 9, 8 -> 10, 10 -> 12).
pub fn line_h(size: f32) -> f32 {
    (size * 0.8325).ceil() + (size * 0.3003).ceil()
}

fn ascent(size: f32) -> f32 {
    (size * 0.8325).ceil()
}

/// One frame of drawing context: the assets, the unit scale (px per canvas unit, zoom
/// included), the mouse, and whether a click is still unclaimed this frame.
pub struct Ui<'a> {
    pub a: &'a Assets,
    pub u: f32,
    pub mouse: Vec2,
    pub click: bool,
    pub down: bool,
    /// Widgets outside this rect (the scrolling page) neither draw nor take the mouse.
    pub clip: Option<Rect>,
}

impl<'a> Ui<'a> {
    pub fn new(a: &'a Assets, u: f32, click: bool) -> Ui<'a> {
        Ui {
            a,
            u,
            mouse: Vec2::from(mouse_position()),
            click,
            down: is_mouse_button_down(MouseButton::Left),
            clip: None,
        }
    }

    fn font_params(&self, size: f32, col: Color) -> TextParams<'a> {
        let px = (size * self.u).max(1.0);
        let fs = px.round().max(1.0);
        TextParams {
            font: self.a.font.as_ref(),
            font_size: fs as u16,
            font_scale: px / fs,
            color: col,
            ..Default::default()
        }
    }

    /// Text width in pixels at a font size in canvas units.
    pub fn tw(&self, s: &str, size: f32) -> f32 {
        let px = (size * self.u).max(1.0);
        let fs = px.round().max(1.0);
        measure_text(s, self.a.font.as_ref(), fs as u16, px / fs).width
    }

    /// Text with its line box's top at `top` (pixels).
    pub fn text(&self, s: &str, x: f32, top: f32, size: f32, col: Color) {
        draw_text_ex(
            s,
            x,
            top + ascent(size) * self.u,
            self.font_params(size, col),
        );
    }

    /// Text vertically centred in a box.
    pub fn text_mid(&self, s: &str, x: f32, r: Rect, size: f32, col: Color) {
        let top = r.y + (r.h - line_h(size) * self.u) / 2.0;
        self.text(s, x, top, size, col);
    }

    /// Text centred both ways in a box.
    pub fn text_center(&self, s: &str, r: Rect, size: f32, col: Color) {
        let w = self.tw(s, size);
        self.text_mid(s, r.x + (r.w - w) / 2.0, r, size, col);
    }

    /// Palette.track(): letters spaced with a thin space (0.2 em).
    pub fn tracked_w(&self, s: &str, size: f32) -> f32 {
        let n = s.chars().count();
        if n == 0 {
            return 0.0;
        }
        s.chars()
            .map(|c| self.tw(&c.to_string(), size))
            .sum::<f32>()
            + (n - 1) as f32 * 0.2 * size * self.u
    }

    pub fn tracked(&self, s: &str, x: f32, top: f32, size: f32, col: Color) {
        let mut cx = x;
        for c in s.chars() {
            let t = c.to_string();
            self.text(&t, cx, top, size, col);
            cx += self.tw(&t, size) + 0.2 * size * self.u;
        }
    }

    /// A StyleBoxFlat: fill and a 1-unit (EDGE_WIDTH) border drawn inside the rect.
    pub fn boxed(&self, r: Rect, fill: Color, edge: Color) {
        draw_rectangle(r.x, r.y, r.w, r.h, fill);
        self.edge(r, edge);
    }

    pub fn edge(&self, r: Rect, edge: Color) {
        let t = self.u.max(1.0);
        draw_rectangle(r.x, r.y, r.w, t, edge);
        draw_rectangle(r.x, r.y + r.h - t, r.w, t, edge);
        draw_rectangle(r.x, r.y, t, r.h, edge);
        draw_rectangle(r.x + r.w - t, r.y, t, r.h, edge);
    }

    pub fn visible(&self, r: Rect) -> bool {
        self.clip.map(|c| c.overlaps(&r)).unwrap_or(true)
    }

    pub fn hover(&self, r: Rect) -> bool {
        r.contains(self.mouse) && self.clip.map(|c| c.contains(self.mouse)).unwrap_or(true)
    }

    /// True once per click inside `r`; the click is then claimed.
    pub fn clicked(&mut self, r: Rect) -> bool {
        if self.click && self.hover(r) {
            self.click = false;
            return true;
        }
        false
    }

    /// Palette.style_button: a primary purple slab or a panel-coloured secondary; hover lifts,
    /// the held press deepens, keyboard focus swaps the edge for TEAL.
    pub fn button_face(&self, r: Rect, label: &str, size: f32, primary: bool, focus: bool) {
        let hover = self.hover(r);
        let pressed = hover && self.down;
        let (fill, edge) = if primary {
            (ACCENT, ACCENT_EDGE)
        } else {
            (PANEL, PANEL_EDGE)
        };
        let (f, e, col) = if pressed {
            (
                if primary {
                    ACCENT_DEEP
                } else {
                    darkened(fill, 0.2)
                },
                edge,
                INK,
            )
        } else if hover {
            (
                lightened(fill, 0.12),
                lightened(edge, 0.2),
                if primary { ON_AMBER } else { INK },
            )
        } else if focus {
            (fill, TEAL, if primary { ON_AMBER } else { INK })
        } else {
            (fill, edge, if primary { ON_AMBER } else { BODY })
        };
        self.boxed(r, f, e);
        if !label.is_empty() {
            self.text_center(label, r, size, col);
        }
    }

    /// A clickable button (face + click).
    pub fn button(&mut self, r: Rect, label: &str, size: f32, primary: bool, focus: bool) -> bool {
        if !self.visible(r) {
            return false;
        }
        self.button_face(r, label, size, primary, focus);
        let pressed = self.clicked(r);
        if pressed {
            crate::audio::sound::menu_blip();
        }
        pressed
    }

    /// A button for a command this build does not run yet: muted text on the flat face. It still takes
    /// the click, so pressing it reports why nothing happened instead of staying silent.
    pub fn button_muted(&mut self, r: Rect, label: &str, size: f32) -> bool {
        if !self.visible(r) {
            return false;
        }
        let hover = self.hover(r);
        self.boxed(r, PANEL, if hover { lightened(PANEL_EDGE, 0.2) } else { PANEL_EDGE });
        self.text_center(label, r, size, MUTED);
        self.clicked(r)
    }

    /// Palette.style_state_toggle on a CheckBox (compact): caption on the left, a boxed ON/OFF
    /// chip on the right; a pressed (ON) row takes the button's darker pressed fill.
    pub fn toggle_row(&mut self, r: Rect, caption: &str, on: bool, focus: bool) -> bool {
        if !self.visible(r) {
            return false;
        }
        let hover = self.hover(r);
        let fill = if on || (hover && self.down) {
            darkened(PANEL, 0.2)
        } else if hover {
            lightened(PANEL, 0.12)
        } else {
            PANEL
        };
        let edge = if hover {
            lightened(PANEL_EDGE, 0.2)
        } else if focus {
            TEAL
        } else {
            PANEL_EDGE
        };
        self.boxed(r, fill, edge);
        let u = self.u;
        self.text_mid(caption, r.x + 5.0 * u, r, 6.0, BODY);
        let label = if on { "ON" } else { "OFF" };
        let cw = (self.tw(label, 6.0) / u + 6.0).max(20.0) * u;
        let ch = (line_h(6.0) + 2.0) * u;
        let chip = Rect::new(r.x + r.w - 5.0 * u - cw, r.y + (r.h - ch) / 2.0, cw, ch);
        let col = if on { STABLE } else { MUTED };
        self.boxed(chip, INSET, if on { STABLE } else { KEYCAP_EDGE });
        self.text_center(label, chip, 6.0, col);
        self.clicked(r)
    }

    /// An OptionButton face (compact style_button) with Godot's down chevron at the right.
    /// `pad` is the text inset (5 for the admin dropdowns, 8 for theme-styled ones).
    pub fn dropdown_face(&self, r: Rect, text: &str, focus: bool, pad: f32) {
        if !self.visible(r) {
            return;
        }
        let hover = self.hover(r);
        let (f, e, col) = if hover && self.down {
            (darkened(PANEL, 0.2), PANEL_EDGE, INK)
        } else if hover {
            (lightened(PANEL, 0.12), lightened(PANEL_EDGE, 0.2), INK)
        } else if focus {
            (PANEL, TEAL, INK)
        } else {
            (PANEL, PANEL_EDGE, BODY)
        };
        self.boxed(r, f, e);
        let u = self.u;
        self.text_mid(text, r.x + pad * u, r, 8.0, col);
        self.chevron(
            vec2(r.x + r.w - 9.75 * u, r.y + r.h / 2.0),
            4.6 * u,
            Color::new(0.75, 0.75, 0.76, 1.0),
            false,
        );
    }

    /// A thick chevron: pointing down (`up` false) or up.
    pub fn chevron(&self, c: Vec2, half: f32, col: Color, up: bool) {
        let h = half * 0.55 * if up { -1.0 } else { 1.0 };
        let t = (half * 0.36).max(1.5);
        draw_line(c.x - half, c.y - h, c.x, c.y + h, t, col);
        draw_line(c.x, c.y + h, c.x + half, c.y - h, t, col);
        draw_circle(c.x, c.y + h, t * 0.5, col);
    }

    /// A sideways chevron (the TabBar scroll arrows): `right` points right.
    pub fn chevron_side(&self, c: Vec2, half: f32, col: Color, right: bool) {
        let w = half * 0.55 * if right { 1.0 } else { -1.0 };
        let t = (half * 0.36).max(1.5);
        draw_line(c.x - w, c.y - half, c.x + w, c.y, t, col);
        draw_line(c.x + w, c.y, c.x - w, c.y + half, t, col);
        draw_circle(c.x + w, c.y, t * 0.5, col);
    }

    /// A filled triangle glyph (the NPC count stepper's arrows).
    pub fn triangle(&self, c: Vec2, s: f32, col: Color, right: bool) {
        let d = if right { 1.0 } else { -1.0 };
        draw_triangle(
            vec2(c.x + d * s * 0.5, c.y),
            vec2(c.x - d * s * 0.5, c.y - s * 0.55),
            vec2(c.x - d * s * 0.5, c.y + s * 0.55),
            col,
        );
    }

    /// Palette.style_line_edit: INSET with a FIELD_EDGE hairline (TEAL when focused), a TEAL
    /// caret, MUTED placeholder.
    pub fn line_edit(
        &mut self,
        r: Rect,
        text: &str,
        placeholder: &str,
        focus: bool,
        t: f32,
    ) -> bool {
        if !self.visible(r) {
            return false;
        }
        self.boxed(r, INSET, if focus { TEAL } else { FIELD_EDGE });
        let u = self.u;
        let x = r.x + 6.0 * u;
        if text.is_empty() {
            self.text_mid(placeholder, x, r, 8.0, MUTED);
        } else {
            self.text_mid(text, x, r, 8.0, INK);
        }
        if focus && (t * 1.6).fract() < 0.6 {
            let cx = x + if text.is_empty() {
                0.0
            } else {
                self.tw(text, 8.0)
            };
            let h = line_h(8.0) * u;
            draw_rectangle(cx, r.y + (r.h - h) / 2.0, (u * 0.5).max(1.0), h, TEAL);
        }
        self.clicked(r)
    }
}

// -- Godot's default PopupMenu (the OptionButton list) -------------------------------------

/// Grey popup with radio items, 20 units a row, as Godot's built-in theme draws it.
pub struct PopupLook;

impl PopupLook {
    pub const ROW: f32 = 20.0;
    pub const PAD: f32 = 3.0;
    pub const BG: Color = Color::new(0.259, 0.259, 0.259, 1.0);
    pub const EDGE: Color = Color::new(0.18, 0.18, 0.18, 1.0);
    pub const HILITE: Color = Color::new(0.40, 0.40, 0.40, 1.0);
    pub const TEXT: Color = Color::new(0.875, 0.875, 0.875, 1.0);
}

pub fn rounded(r: Rect, rad: f32, col: Color) {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    draw_rectangle(r.x + rad, r.y, r.w - 2.0 * rad, r.h, col);
    draw_rectangle(r.x, r.y + rad, rad, r.h - 2.0 * rad, col);
    draw_rectangle(r.x + r.w - rad, r.y + rad, rad, r.h - 2.0 * rad, col);
    for (cx, cy) in [
        (r.x + rad, r.y + rad),
        (r.x + r.w - rad, r.y + rad),
        (r.x + rad, r.y + r.h - rad),
        (r.x + r.w - rad, r.y + r.h - rad),
    ] {
        draw_circle(cx, cy, rad, col);
    }
}

/// Godot's TabBar scroll buttons (the default theme's round increment / decrement icons).
pub fn scroll_arrow(ui: &Ui, c: Vec2, r: f32, right: bool, enabled: bool) {
    let hover = enabled && c.distance(ui.mouse) <= r;
    let disc = if !enabled {
        Color::new(0.36, 0.36, 0.37, 1.0)
    } else if hover {
        Color::new(0.92, 0.92, 0.92, 1.0)
    } else {
        Color::new(0.80, 0.80, 0.81, 1.0)
    };
    draw_circle(c.x, c.y, r, disc);
    let ink = if enabled {
        Color::new(0.17, 0.17, 0.18, 1.0)
    } else {
        Color::new(0.16, 0.16, 0.17, 1.0)
    };
    ui.chevron_side(
        vec2(c.x + if right { r * 0.08 } else { -r * 0.08 }, c.y),
        r * 0.5,
        ink,
        right,
    );
}
