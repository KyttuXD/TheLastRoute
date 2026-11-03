//! The settings overlay (settings_overlay.tscn + settings_panel.gd), laid out as the web's four
//! tabs (games/last-route/index.html, the controls overlay) with SKIN, VOICE and MODS as their own
//! tabs after GAMEPLAY: GAMEPLAY, SKIN, VOICE, MODS, AUDIO, VIDEO and CONTROLS.
//! A 440x254 panel centred on the canvas with SETTINGS, the rebind hint, the tab bar and ESC CLOSE.
//! Rows sit at the scene file's offsets; the tab bar and sliders are the project theme's fixed purple.
//! The web's click-to-move and touch rows have no desktop meaning and are left out. SKIN and MODS
//! show a note instead of their lists during a run.

use super::menus::Ui;
use super::style::*;
use crate::app::assets::Assets;
use crate::app::settings::Settings;
use crate::content::Content;
use macroquad::prelude::*;

/// The web overlay's tabs, in its order (index.html `data-settings-tab`).
const TABS: [&str; 7] = [
    "GAMEPLAY", "SKIN", "VOICE", "MODS", "AUDIO", "VIDEO", "CONTROLS",
];

/// The saved keys (app/settings.rs) each tab shows, in TABS order. Every saved key is in exactly
/// one tab unless it is in OFF_PANEL_KEYS. The draw code below must match this table.
#[allow(dead_code)]
const TAB_KEYS: [&[&str]; 7] = [
    &["pause_on_focus_loss", "teammate_waypoints"],
    &[],
    &[],
    &[],
    &[
        "master_volume",
        "gameplay_volume",
        "ui_volume",
        "voice_volume",
        "voices",
        "subtitles",
    ],
    &["fullscreen", "ads", "vsync", "quality", "fps_limit", "fov"],
    &["sensitivity"],
];

/// Saved keys with no row in this panel: `username` is edited on the account screens, and
/// `scheme` comes from LR_SCHEME or the saved file.
#[allow(dead_code)]
const OFF_PANEL_KEYS: [&str; 2] = ["username", "scheme"];

/// MOVEMENT / ACTIONS / SYSTEM (ControlsRegistry.LAST_ROUTE_TABS) with this build's keys.
const SECTIONS: [(&str, &[(&str, &str)]); 3] = [
    (
        "MOVEMENT",
        &[
            ("MOVE UP", "W"),
            ("MOVE DOWN", "S"),
            ("MOVE LEFT", "A"),
            ("MOVE RIGHT", "D"),
            ("LOOK UP", "I / UP"),
            ("LOOK DOWN", "K / DOWN"),
            ("LOOK LEFT", "J / LEFT"),
            ("LOOK RIGHT", "L / RIGHT"),
            ("SPRINT", "SHIFT"),
            ("CROUCH", "C"),
            ("JUMP / HOP", "SPACE"),
        ],
    ),
    (
        "ACTIONS",
        &[
            ("INTERACT", "E / MOUSE 2"),
            ("FLASHLIGHT", "F"),
            ("TASER", "G"),
            ("USE HELD TOOL", "MOUSE 1"),
            ("INVENTORY", "1-5 / WHEEL"),
            ("DROP HELD ITEM", "Q"),
            ("TOGGLE MAP", "M"),
            ("HIDE HUD", "F1"),
        ],
    ),
    ("SYSTEM", &[("PAUSE", "ESCAPE"), ("FULLSCREEN", "F11")]),
];

const SKIN_TEXT: &str =
    "SKINS ARE STORED ON YOUR ACCOUNT. THIS OFFLINE BUILD PLAYS THE DEFAULT RUNNER.";
const VOICE_TEXT: &str = "VOICE CHAT RUNS IN ONLINE ROOMS. THIS OFFLINE BUILD HAS NO ROOM SERVER.";
const MODS_FOOTNOTE: &str =
    "PUT PACK FOLDERS (pack.json + hunters/ ...) IN \"mods\" BESIDE THE GAME, THEN RESTART.";

/// A ref_theme HSlider: inset track 4 high, accent fill up to the grabber centre, and the
/// default theme's translucent white grabber (14 units). Returns the new value while dragged.
fn slider(ui: &mut Ui, id: u32, r: Rect, value: f32, min: f32, max: f32) -> Option<f32> {
    let track = Rect::new(r.x, r.y + (r.h - 4.0) * 0.5, r.w, 4.0);
    sbox(track, REF_INSET, REF_PANEL_EDGE, 1.0, 0.0);
    let f = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let g = 14.0;
    let cx = r.x + g * 0.5 + f * (r.w - g);
    fill(Rect::new(r.x, track.y, cx - r.x, 4.0), REF_ACCENT);
    let hot = hovered(Rect::new(r.x, r.y - 4.0, r.w, r.h + 8.0));
    circle(
        vec2(cx, r.y + r.h * 0.5),
        7.0,
        rgba(
            1.0,
            1.0,
            1.0,
            if hot || ui.drag == Some(id) {
                0.85
            } else {
                0.75
            },
        ),
    );
    if hot && is_mouse_button_pressed(MouseButton::Left) {
        ui.drag = Some(id);
    }
    if ui.drag == Some(id) {
        if !is_mouse_button_down(MouseButton::Left) {
            ui.drag = None;
            return None;
        }
        let m = mouse_u().x;
        let nf = ((m - r.x - g * 0.5) / (r.w - g)).clamp(0.0, 1.0);
        return Some(min + nf * (max - min));
    }
    None
}

/// The settings' tab and row labels: 0.55 of the old size (the web's .72rem labels, menu_size).
fn setting_label_u() -> f32 {
    menu_size(8.0, 0.55)
}

/// The settings slider's track: the web's `clamp(90px, 18vw, 170px)`, which is 170px at 1280x720
/// (64 units before the menu scale).
fn setting_slider_u() -> f32 {
    menu_size(64.0, 1.0)
}

/// An HBox row: label (BODY 8), slider (the web's 170px track), value label; separation 8.
fn slider_row(
    a: &Assets,
    ui: &mut Ui,
    id: u32,
    x: f32,
    y: f32,
    caption: &str,
    value: f32,
    min: f32,
    max: f32,
    shown: &str,
) -> Option<f32> {
    let p = pal();
    let label_u = setting_label_u();
    let track = setting_slider_u();
    label_vc(a, F::Body, caption, x, y, 16.0, label_u, p.body);
    let sx = x + tw(a, F::Body, caption, label_u) + 8.0;
    let out = slider(ui, id, Rect::new(sx, y + 2.0, track, 12.0), value, min, max);
    label_vc(a, F::Body, shown, sx + track + 8.0, y, 16.0, label_u, p.body);
    out
}

/// A TabBar in the project theme, centred in `r`; returns a clicked index.
pub fn tab_bar(a: &Assets, r: Rect, tabs: &[&str], current: usize, focused: bool) -> Option<usize> {
    let widths: Vec<f32> = tabs
        .iter()
        .map(|t| tw(a, F::Body, t, setting_label_u()) + 20.0)
        .collect();
    let total: f32 = widths.iter().sum();
    let mut x = r.x + (r.w - total) * 0.5;
    let mut hit = None;
    for (i, t) in tabs.iter().enumerate() {
        let tr = Rect::new(x, r.y, widths[i], r.h);
        let hover = hovered(tr);
        let (face, edge, ink) = if i == current {
            // Last Route: the web's selected tab (#271927 face, amber edge, #fff8f1 text)
            (
                lr(hex(0x271927), REF_ACCENT),
                lr(hex(0xf2bb62), REF_ACCENT_EDGE),
                lr(hex(0xfff8f1), REF_ON_AMBER),
            )
        } else if hover {
            (
                lr(hex(0x271927), rgba(0.181, 0.149, 0.188, 1.0)),
                REF_ACCENT_EDGE,
                REF_INK,
            )
        } else {
            (REF_PANEL, REF_PANEL_EDGE, REF_BODY)
        };
        sbox(tr, face, edge, 1.0, 0.0);
        if i == current && focused {
            border(tr, 1.0, REF_TEAL);
        }
        label_vc(a, F::Body, t, tr.x + 10.0, tr.y, tr.h, setting_label_u(), ink);
        if hover && is_mouse_button_pressed(MouseButton::Left) {
            crate::audio::sound::menu_blip();
            hit = Some(i);
        }
        x += widths[i];
    }
    hit
}

/// One line of the GAMEPLAY tab's scrolling text: its size and colour, its indent, and how far
/// the next line sits below it.
struct TextLine {
    text: String,
    size: f32,
    col: Color,
    dx: f32,
    step: f32,
}

fn text_line(text: String, size: f32, col: Color, dx: f32) -> TextLine {
    TextLine {
        text,
        size,
        col,
        dx,
        step: lh(F::Body, size),
    }
}

fn heading(out: &mut Vec<TextLine>, title: &str) {
    out.push(TextLine {
        text: track(title),
        size: 8.0,
        col: pal().amber,
        dx: 0.0,
        step: lh(F::Body, 8.0) + 4.0,
    });
}

fn gap(out: &mut Vec<TextLine>, px: f32) {
    out.push(TextLine {
        text: String::new(),
        size: 7.0,
        col: pal().muted,
        dx: 0.0,
        step: px,
    });
}

/// The text of a section tab: SKIN (1), VOICE (2) or MODS (3). SKIN and MODS show a note during a run.
fn section_text(a: &Assets, content: &Content, in_run: bool, tab: usize) -> Vec<TextLine> {
    let p = pal();
    let mut out = Vec::new();
    match tab {
        1 => {
            if in_run {
                out.push(text_line("SKINS CHANGE FROM THE TITLE SCREEN.".into(), 7.0, p.muted, 0.0));
            } else {
                for l in wrap(a, F::Body, SKIN_TEXT, 400.0, 7.0) {
                    out.push(text_line(l, 7.0, p.muted, 0.0));
                }
            }
        }
        2 => {
            for l in wrap(a, F::Body, VOICE_TEXT, 400.0, 7.0) {
                out.push(text_line(l, 7.0, p.muted, 0.0));
            }
        }
        _ => {
            if in_run {
                out.push(text_line("MODS CHANGE FROM THE TITLE SCREEN.".into(), 7.0, p.muted, 0.0));
                return out;
            }
            heading(&mut out, "INSTALLED MODS");
            if content.packs.is_empty() {
                out.push(text_line("NO MODS INSTALLED".into(), 7.0, p.muted, 0.0));
            }
            for pk in &content.packs {
                out.push(text_line(
                    format!("{} {} // {}", pk.name.to_uppercase(), pk.version, pk.author),
                    7.0,
                    p.ink,
                    0.0,
                ));
                let adds = clip(
                    a,
                    F::Body,
                    &format!("ADDS: {}", pk.provides.join(", ")),
                    400.0,
                    6.0,
                );
                out.push(text_line(adds, 6.0, p.body, 6.0));
                gap(&mut out, 4.0);
            }
            for w in content.warnings.iter().take(4) {
                out.push(text_line(clip(a, F::Body, w, 400.0, 6.0), 6.0, p.alert, 0.0));
            }
            for l in wrap(a, F::Body, MODS_FOOTNOTE, 400.0, 6.0) {
                out.push(text_line(l, 6.0, p.muted, 0.0));
            }
        }
    }
    out
}

/// Draws a column of text lines inside `area`; the mouse wheel scrolls it.
fn scroll_text(a: &Assets, ui: &mut Ui, x: f32, area: Rect, lines: &[TextLine]) {
    let total: f32 = lines.iter().map(|l| l.step).sum();
    if hovered(area) {
        ui.scroll -= mouse_wheel().1.signum() * 20.0;
    }
    ui.scroll = ui.scroll.clamp(0.0, (total - area.h).max(0.0));
    clip_begin(area);
    let mut y = area.y - ui.scroll;
    for l in lines {
        if !l.text.is_empty() {
            label(a, F::Body, &l.text, x + l.dx, y, l.size, l.col);
        }
        y += l.step;
    }
    clip_end();
}

/// Draws the overlay; returns true when it closes. `in_run` shows a note for SKIN and MODS.
pub fn draw_settings(
    a: &Assets,
    ui: &mut Ui,
    st: &mut Settings,
    content: &Content,
    in_run: bool,
) -> bool {
    let _scope = MenuScope::enter();
    let p = pal();
    let panel = centered(440.0, 254.0);
    // opaque: the title screen behind the overlay must not show through the panel
    fill(panel, Color::new(p.panel.r, p.panel.g, p.panel.b, 1.0));
    let (ox, oy) = (panel.x, panel.y);
    let tw15 = tw(a, F::Title, "SETTINGS", 15.0);
    label(
        a,
        F::Title,
        "SETTINGS",
        ox + (440.0 - tw15) * 0.5,
        oy + 6.0,
        15.0,
        p.ink,
    );
    label_c(
        a,
        F::Body,
        "PRESS REBIND, THEN A KEY  //  ESC CANCELS A REBIND IN PROGRESS",
        ox + 220.0,
        oy + 25.0,
        7.0,
        p.muted,
    );
    let mut pos = ui.settings_tab.min(TABS.len() - 1);
    if let Some(i) = tab_bar(
        a,
        Rect::new(ox + 20.0, oy + 37.0, 400.0, 22.0),
        &TABS,
        pos,
        true,
    ) {
        pos = i;
        ui.scroll = 0.0;
    }
    if is_key_pressed(KeyCode::Tab) {
        let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        pos = if back {
            (pos + TABS.len() - 1) % TABS.len()
        } else {
            (pos + 1) % TABS.len()
        };
        ui.scroll = 0.0;
    }
    ui.settings_tab = pos;
    let x = ox + 20.0;
    let mut changed = false;
    match ui.settings_tab {
        0 => {
            // GAMEPLAY: the two focus and waypoint toggles stay put; the SKIN, VOICE and MODS
            // text scrolls below them.
            if state_toggle(
                a,
                Rect::new(x, oy + 66.0, 400.0, 16.0),
                "PAUSE WHEN WINDOW LOSES FOCUS",
                st.pause_on_focus_loss,
                false,
                plain(),
            ) {
                st.pause_on_focus_loss = !st.pause_on_focus_loss;
                changed = true;
            }
            if state_toggle(
                a,
                Rect::new(x, oy + 86.0, 400.0, 14.0),
                "TEAMMATE WAYPOINTS",
                st.teammate_waypoints,
                false,
                plain(),
            ) {
                st.teammate_waypoints = !st.teammate_waypoints;
                changed = true;
            }
        }
        1 | 2 | 3 => {
            // SKIN, VOICE and MODS each scroll their text below the tab bar
            let lines = section_text(a, content, in_run, ui.settings_tab);
            scroll_text(a, ui, x, Rect::new(x, oy + 66.0, 400.0, 160.0), &lines);
        }
        4 => {
            let pct = |v: f32| format!("{}%", (v * 100.0).round() as i32);
            if let Some(v) = slider_row(
                a,
                ui,
                1,
                x,
                oy + 70.0,
                "MASTER VOLUME",
                st.master_volume * 100.0,
                0.0,
                100.0,
                &pct(st.master_volume),
            ) {
                st.master_volume = v.round() / 100.0;
                changed = true;
            }
            if let Some(v) = slider_row(
                a,
                ui,
                2,
                x,
                oy + 94.0,
                "GAMEPLAY / HUNTERS",
                st.gameplay_volume * 100.0,
                0.0,
                100.0,
                &pct(st.gameplay_volume),
            ) {
                st.gameplay_volume = v.round() / 100.0;
                changed = true;
            }
            if let Some(v) = slider_row(
                a,
                ui,
                3,
                x,
                oy + 118.0,
                "MENU / UI",
                st.ui_volume * 100.0,
                0.0,
                100.0,
                &pct(st.ui_volume),
            ) {
                st.ui_volume = v.round() / 100.0;
                changed = true;
            }
            // SPOKEN CHAT: label 96, the slider fills, readout 34 (separation 4)
            let y = oy + 142.0;
            label_vc(a, F::Body, "SPOKEN CHAT", x, y, 16.0, setting_label_u(), p.body);
            if let Some(v) = slider(
                ui,
                4,
                Rect::new(x + 100.0, y + 2.0, 262.0, 12.0),
                st.voice_volume,
                0.0,
                100.0,
            ) {
                st.voice_volume = (v / 5.0).round() * 5.0;
                changed = true;
            }
            label_vc(
                a,
                F::Body,
                &format!("{}%", st.voice_volume as i32),
                x + 366.0,
                y,
                16.0,
                setting_label_u(),
                p.body,
            );
            let bw = (400.0 - 4.0) * 0.5;
            if state_toggle(
                a,
                Rect::new(x, oy + 166.0, bw, 16.0),
                "VOICES",
                st.voices,
                false,
                plain(),
            ) {
                st.voices = !st.voices;
                changed = true;
            }
            if state_toggle(
                a,
                Rect::new(x + bw + 4.0, oy + 166.0, bw, 16.0),
                "SUBTITLES",
                st.subtitles,
                false,
                plain(),
            ) {
                st.subtitles = !st.subtitles;
                changed = true;
            }
        }
        5 => {
            // FULLSCREEN toggle, the F11 hint and the ADS cycle button share one row
            let y = oy + 66.0;
            if state_toggle(
                a,
                Rect::new(x, y, 110.0, 16.0),
                "FULLSCREEN",
                st.fullscreen,
                false,
                plain(),
            ) {
                st.fullscreen = !st.fullscreen;
                set_fullscreen(st.fullscreen);
                changed = true;
            }
            label_vc(a, F::Body, "F11 ANYWHERE", x + 118.0, y, 16.0, 7.0, p.muted);
            let ax = x + 118.0 + tw(a, F::Body, "F11 ANYWHERE", 7.0) + 8.0;
            let ads = format!("ADS: {}", st.ads.to_uppercase());
            if button(
                a,
                Rect::new(ax, y, x + 400.0 - ax, 16.0),
                &ads,
                8.0,
                Kind::Secondary,
                plain(),
            ) {
                st.ads = match st.ads.as_str() {
                    "off" => "gentle",
                    "gentle" => "annoying",
                    _ => "off",
                }
                .into();
                changed = true;
            }
            let y = oy + 86.0;
            if state_toggle(
                a,
                Rect::new(x, y, 110.0, 16.0),
                "VSYNC",
                st.vsync,
                false,
                plain(),
            ) {
                st.vsync = !st.vsync;
                changed = true;
            }
            let q = format!(
                "QUALITY: {}",
                if st.quality == "AUTO" {
                    "AUTO (HIGH)".to_string()
                } else {
                    st.quality.clone()
                }
            );
            if button(
                a,
                Rect::new(x + 118.0, y, 282.0, 16.0),
                &q,
                8.0,
                Kind::Secondary,
                plain(),
            ) {
                st.quality = match st.quality.as_str() {
                    "AUTO" => "LOW",
                    "LOW" => "MEDIUM",
                    "MEDIUM" => "HIGH",
                    _ => "AUTO",
                }
                .into();
                changed = true;
            }
            let fps = if st.fps_limit < 15.0 {
                "UNLIMITED".to_string()
            } else {
                format!("{}", st.fps_limit as i32)
            };
            if let Some(v) = slider_row(
                a,
                ui,
                5,
                x,
                oy + 106.0,
                "FPS LIMIT",
                st.fps_limit,
                0.0,
                240.0,
                &fps,
            ) {
                st.fps_limit = (v / 30.0).round() * 30.0;
                changed = true;
            }
            if let Some(v) = slider_row(
                a,
                ui,
                6,
                x,
                oy + 126.0,
                "FIELD OF VIEW",
                st.fov,
                60.0,
                105.0,
                &format!("{}°", st.fov.round() as i32),
            ) {
                st.fov = v.round();
                changed = true;
            }
            let hint = match st.quality.as_str() {
                "LOW" => "LOW: short view, fewer props, no shadows",
                "MEDIUM" => "MEDIUM: longer view, more props, no flashlight shadows",
                "HIGH" => "HIGH: full view, props, flashlight shadows",
                _ => "AUTO: picks for this device, eases off if frames drop",
            };
            label_vc(a, F::Body, hint, x, oy + 144.0, 12.0, 7.0, p.muted);
        }
        _ => {
            // CONTROLS: the mouse sensitivity row, then ControlsScroll (20,88)-(420,228) with the
            // section tabs and the binding rows
            let sens = st.sensitivity_percent().round().max(1.0);
            if let Some(v) = slider_row(
                a,
                ui,
                7,
                x,
                oy + 66.0,
                "MOUSE SENSITIVITY",
                st.sensitivity_percent(),
                0.0,
                100.0,
                &format!("{}", sens as i32),
            ) {
                st.sensitivity = 0.0005 + v / 100.0 * (0.012 - 0.0005);
                changed = true;
            }
            let area = Rect::new(x, oy + 88.0, 400.0, 140.0);
            let tab_h = F::Body.height() * 8.0 + 8.0;
            let names: Vec<&str> = SECTIONS.iter().map(|s| s.0).collect();
            if hovered(area) {
                ui.scroll -= mouse_wheel().1.signum() * 20.0;
            }
            let rows = SECTIONS[ui.controls_tab.min(2)].1;
            let row_h = button_min(a, "X", 8.0, false).y;
            let total = tab_h + 3.0 + rows.len() as f32 * (row_h + 3.0);
            ui.scroll = ui.scroll.clamp(0.0, (total - area.h).max(0.0));
            clip_begin(area);
            let mut y = area.y - ui.scroll;
            if let Some(i) = tab_bar(
                a,
                Rect::new(x, y, 400.0, tab_h),
                &names,
                ui.controls_tab,
                false,
            ) {
                ui.controls_tab = i;
                ui.scroll = 0.0;
            }
            y += tab_h + 3.0;
            for (name, key) in rows {
                label_vc(a, F::Body, name, x, y, row_h, setting_label_u(), p.body);
                label_vc(
                    a,
                    F::Body,
                    key,
                    x + 118.0 + (92.0 - tw(a, F::Body, key, setting_label_u())) * 0.5,
                    y,
                    row_h,
                    setting_label_u(),
                    p.amber,
                );
                let mut bx = x + 118.0 + 92.0 + 6.0;
                for (t, w) in [("CHANGE", 60.0f32), ("+", 28.0), ("RESET", 48.0)] {
                    let w = w.max(button_min(a, t, 8.0, false).x);
                    button(
                        a,
                        Rect::new(bx, y, w, row_h),
                        t,
                        8.0,
                        Kind::Secondary,
                        disabled(),
                    );
                    bx += w + 6.0;
                }
                y += row_h + 3.0;
            }
            clip_end();
        }
    }
    if changed {
        st.save();
    }
    let close = esc_close(a, Rect::new(ox + 193.0, oy + 234.0, 54.0, 16.0));
    close || is_key_pressed(KeyCode::Escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saved keys the settings file writes, sorted.
    fn saved_keys() -> Vec<String> {
        let v = Settings::default().to_json();
        let mut keys: Vec<String> = v
            .as_object()
            .expect("settings are a JSON object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn tabs_match_the_web_overlay_in_order() {
        assert_eq!(
            TABS,
            ["GAMEPLAY", "SKIN", "VOICE", "MODS", "AUDIO", "VIDEO", "CONTROLS"]
        );
        assert_eq!(TABS.len(), TAB_KEYS.len());
    }

    #[test]
    fn every_saved_setting_is_in_exactly_one_tab() {
        for key in saved_keys() {
            let shown = TAB_KEYS
                .iter()
                .filter(|keys| keys.contains(&key.as_str()))
                .count();
            let expected = if OFF_PANEL_KEYS.contains(&key.as_str()) {
                0
            } else {
                1
            };
            assert_eq!(shown, expected, "{key} should appear in {expected} tab(s)");
        }
    }

    #[test]
    fn tabs_only_name_saved_settings() {
        let saved = saved_keys();
        for keys in TAB_KEYS.iter() {
            for key in keys.iter() {
                assert!(saved.iter().any(|k| k == key), "{key} is not a saved setting");
            }
        }
        for key in OFF_PANEL_KEYS {
            assert!(saved.iter().any(|k| k == key), "{key} is not a saved setting");
        }
    }
}
