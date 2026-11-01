//! The settings overlay (settings_overlay.tscn + settings_panel.gd): a 440x254 panel centred on
//! the canvas with SETTINGS, the rebind hint, the category tab bar (AUDIO / DISPLAY / CONTROLS /
//! SKIN / VOICE / MODS; a running route hides SKIN and MODS) and ESC CLOSE. Rows sit at the
//! scene file's offsets; the tab bar and sliders are the project theme's fixed purple.

use super::menus::Ui;
use super::style::*;
use crate::app::assets::Assets;
use crate::app::settings::Settings;
use crate::content::Content;
use macroquad::prelude::*;

const TITLES: [&str; 6] = ["AUDIO", "DISPLAY", "CONTROLS", "SKIN", "VOICE", "MODS"];

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

/// An HBox row: label (BODY 8), slider 180x12, value label; separation 8.
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
    label_vc(a, F::Body, caption, x, y, 16.0, 8.0, p.body);
    let sx = x + tw(a, F::Body, caption, 8.0) + 8.0;
    let out = slider(ui, id, Rect::new(sx, y + 2.0, 180.0, 12.0), value, min, max);
    label_vc(a, F::Body, shown, sx + 188.0, y, 16.0, 8.0, p.body);
    out
}

/// A TabBar in the project theme, centred in `r`; returns a clicked index.
pub fn tab_bar(a: &Assets, r: Rect, tabs: &[&str], current: usize, focused: bool) -> Option<usize> {
    let widths: Vec<f32> = tabs.iter().map(|t| tw(a, F::Body, t, 8.0) + 20.0).collect();
    let total: f32 = widths.iter().sum();
    let mut x = r.x + (r.w - total) * 0.5;
    let mut hit = None;
    for (i, t) in tabs.iter().enumerate() {
        let tr = Rect::new(x, r.y, widths[i], r.h);
        let hover = hovered(tr);
        let (face, edge, ink) = if i == current {
            (REF_ACCENT, REF_ACCENT_EDGE, REF_ON_AMBER)
        } else if hover {
            (rgba(0.181, 0.149, 0.188, 1.0), REF_ACCENT_EDGE, REF_INK)
        } else {
            (REF_PANEL, REF_PANEL_EDGE, REF_BODY)
        };
        sbox(tr, face, edge, 1.0, 0.0);
        if i == current && focused {
            border(tr, 1.0, REF_TEAL);
        }
        label_vc(a, F::Body, t, tr.x + 10.0, tr.y, tr.h, 8.0, ink);
        if hover && is_mouse_button_pressed(MouseButton::Left) {
            crate::audio::sound::menu_blip();
            hit = Some(i);
        }
        x += widths[i];
    }
    hit
}

/// Draws the overlay; returns true when it closes. `in_run` hides SKIN and MODS.
pub fn draw_settings(
    a: &Assets,
    ui: &mut Ui,
    st: &mut Settings,
    content: &Content,
    in_run: bool,
) -> bool {
    let p = pal();
    let panel = centered(440.0, 254.0);
    fill(panel, p.panel);
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
    let cats: Vec<usize> = (0..6)
        .filter(|c| !(in_run && (*c == 3 || *c == 5)))
        .collect();
    let names: Vec<&str> = cats.iter().map(|&c| TITLES[c]).collect();
    let mut pos = cats.iter().position(|&c| c == ui.settings_tab).unwrap_or(0);
    if let Some(i) = tab_bar(
        a,
        Rect::new(ox + 20.0, oy + 37.0, 400.0, 22.0),
        &names,
        pos,
        true,
    ) {
        pos = i;
        ui.scroll = 0.0;
    }
    if is_key_pressed(KeyCode::Tab) {
        let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        pos = if back {
            (pos + cats.len() - 1) % cats.len()
        } else {
            (pos + 1) % cats.len()
        };
        ui.scroll = 0.0;
    }
    ui.settings_tab = cats[pos];
    let x = ox + 20.0;
    let mut changed = false;
    match ui.settings_tab {
        0 => {
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
            label_vc(a, F::Body, "SPOKEN CHAT", x, y, 16.0, 8.0, p.body);
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
                8.0,
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
        1 => {
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
            let sens = st.sensitivity_percent().round().max(1.0);
            if let Some(v) = slider_row(
                a,
                ui,
                7,
                x,
                oy + 186.0,
                "MOUSE SENSITIVITY",
                st.sensitivity_percent(),
                0.0,
                100.0,
                &format!("{}", sens as i32),
            ) {
                st.sensitivity = 0.0005 + v / 100.0 * (0.012 - 0.0005);
                changed = true;
            }
            if state_toggle(
                a,
                Rect::new(x, oy + 208.0, 400.0, 16.0),
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
                Rect::new(x, oy + 226.0, 400.0, 14.0),
                "TEAMMATE WAYPOINTS",
                st.teammate_waypoints,
                false,
                plain(),
            ) {
                st.teammate_waypoints = !st.teammate_waypoints;
                changed = true;
            }
        }
        2 => {
            // ControlsScroll (20,62)-(420,228): the section tabs, then the binding rows
            let area = Rect::new(x, oy + 62.0, 400.0, 166.0);
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
                label_vc(a, F::Body, name, x, y, row_h, 8.0, p.body);
                label_vc(
                    a,
                    F::Body,
                    key,
                    x + 118.0 + (92.0 - tw(a, F::Body, key, 8.0)) * 0.5,
                    y,
                    row_h,
                    8.0,
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
        3 => {
            label(a, F::Body, &track("SKIN"), x, oy + 66.0, 8.0, p.amber);
            for (i, l) in wrap(
                a,
                F::Body,
                "SKINS ARE STORED ON YOUR ACCOUNT. THIS OFFLINE BUILD PLAYS THE DEFAULT RUNNER.",
                400.0,
                7.0,
            )
            .iter()
            .enumerate()
            {
                label(
                    a,
                    F::Body,
                    l,
                    x,
                    oy + 80.0 + i as f32 * lh(F::Body, 7.0),
                    7.0,
                    p.muted,
                );
            }
        }
        4 => {
            label(a, F::Body, &track("VOICE"), x, oy + 66.0, 8.0, p.amber);
            for (i, l) in wrap(
                a,
                F::Body,
                "VOICE CHAT RUNS IN ONLINE ROOMS. THIS OFFLINE BUILD HAS NO ROOM SERVER.",
                400.0,
                7.0,
            )
            .iter()
            .enumerate()
            {
                label(
                    a,
                    F::Body,
                    l,
                    x,
                    oy + 80.0 + i as f32 * lh(F::Body, 7.0),
                    7.0,
                    p.muted,
                );
            }
        }
        _ => {
            let mut y = oy + 66.0;
            label(a, F::Body, &track("INSTALLED MODS"), x, y, 8.0, p.amber);
            y += lh(F::Body, 8.0) + 4.0;
            if content.packs.is_empty() {
                label(a, F::Body, "NO MODS INSTALLED", x, y, 7.0, p.muted);
                y += lh(F::Body, 7.0) + 4.0;
            }
            for pk in &content.packs {
                label(
                    a,
                    F::Body,
                    &format!("{} {} // {}", pk.name.to_uppercase(), pk.version, pk.author),
                    x,
                    y,
                    7.0,
                    p.ink,
                );
                y += lh(F::Body, 7.0);
                label(
                    a,
                    F::Body,
                    &clip(
                        a,
                        F::Body,
                        &format!("ADDS: {}", pk.provides.join(", ")),
                        400.0,
                        6.0,
                    ),
                    x + 6.0,
                    y,
                    6.0,
                    p.body,
                );
                y += lh(F::Body, 6.0) + 4.0;
            }
            for w in content.warnings.iter().take(4) {
                label(
                    a,
                    F::Body,
                    &clip(a, F::Body, w, 400.0, 6.0),
                    x,
                    y,
                    6.0,
                    p.alert,
                );
                y += lh(F::Body, 6.0);
            }
            for l in wrap(a, F::Body, "PUT PACK FOLDERS (pack.json + hunters/ ...) IN \"mods\" BESIDE THE GAME, THEN RESTART.", 400.0, 6.0) {
                label(a, F::Body, &l, x, y + 4.0, 6.0, p.muted);
                y += lh(F::Body, 6.0);
            }
        }
    }
    if changed {
        st.save();
    }
    let close = esc_close(a, Rect::new(ox + 193.0, oy + 234.0, 54.0, 16.0));
    close || is_key_pressed(KeyCode::Escape)
}
