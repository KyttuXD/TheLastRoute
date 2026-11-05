//! The settings overlay (settings_overlay.tscn + settings_panel.gd), laid out as the web's four
//! tabs (games/last-route/index.html, the controls overlay) with SKIN, VOICE and MODS as their own
//! tabs after GAMEPLAY: GAMEPLAY, SKIN, VOICE, MODS, AUDIO, VIDEO and CONTROLS.
//! A 440x254 panel centred on the canvas with SETTINGS, the rebind hint, the tab bar and ESC CLOSE.
//! Rows sit at the scene file's offsets; the tab bar and sliders are the project theme's fixed purple.
//! The web's click-to-move and touch rows have no desktop meaning and are left out. SKIN and MODS
//! show a note instead of their lists during a run.

use super::menus::{wheel_notch, Ui};
use super::style::*;
use crate::app::assets::Assets;
use crate::app::settings::Settings;
use crate::config::{FOV_MAX_DEG, FOV_MIN_DEG};
use crate::content::Content;
use crate::net::world_sync::SKIN_DEFAULT;
use crate::shared::skins;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::sync::mpsc::{Receiver, TryRecvError};

/// The web overlay's tabs, in its order (index.html `data-settings-tab`).
const TABS: [&str; 7] = [
    "GAMEPLAY", "AUDIO", "VIDEO", "CONTROLS", "SKIN", "VOICE", "MODS",
];

/// The saved keys (app/settings.rs) each tab shows, in TABS order. Every saved key is in exactly
/// one tab unless it is in OFF_PANEL_KEYS. The draw code below must match this table.
#[allow(dead_code)]
const TAB_KEYS: [&[&str]; 7] = [
    &["pause_on_focus_loss", "teammate_waypoints", "reduced_motion"],
    &[
        "master_volume",
        "gameplay_volume",
        "ui_volume",
        "voice_volume",
        "voices",
        "subtitles",
        "speech_enabled",
    ],
    &["fullscreen", "ads", "vsync", "quality", "fps_limit", "fov", "graphics_api"],
    &["sensitivity"],
    &[],
    &["arms"],
    &[],
];

/// Saved keys with no row in this panel: `username` is edited on the account screens, `scheme` comes
/// from LR_SCHEME or the saved file, and `skin` is chosen from the title screen.
#[allow(dead_code)]
const OFF_PANEL_KEYS: [&str; 3] = ["username", "scheme", "skin"];

/// The GRAPHICS API row: the saved choice and the API this run opened with (they differ until a restart, or when
/// AUTO or a safeguard picked).
fn graphics_api_label(setting: &str, metal_now: bool, marker_forced: bool) -> String {
    if marker_forced && !metal_now {
        return format!("GRAPHICS API: {setting} (OPENGL: LAST METAL START DID NOT FINISH)");
    }
    format!("GRAPHICS API: {setting} (NOW {})", if metal_now { "METAL" } else { "OPENGL" })
}

/// The two lines under the VSYNC and FPS LIMIT rows. VSYNC ON waits for the screen's refresh, so the frame rate
/// stops there and FPS LIMIT cannot raise it; VSYNC OFF lets FPS LIMIT (or UNLIMITED) decide. Plain statements of
/// how the two rows work, with no measured numbers.
const VSYNC_NOTES: [&str; 2] = [
    "VSYNC ON: LOCKED TO YOUR SCREEN'S REFRESH (60 OR 75 FPS)",
    "VSYNC OFF: THE FPS LIMIT DECIDES; UNLIMITED = AS FAST AS IT RUNS",
];

/// The two lines under the GRAPHICS API row (macOS). The numbers are the stress-fly measurements on an Apple M4,
/// 75 Hz display, vsync off (docs/GRAPHICS_API.md); change them there and here together.
const METAL_NOTES: [&str; 2] = [
    "NEXT START. METAL IS EXPERIMENTAL: FULLSCREEN ABOUT 340 FPS VS",
    "OPENGL 190-260, BUT IN A WINDOW METAL STOPS AT THE REFRESH RATE",
];

/// A one-line note in the VIDEO tab, shrunk to fit the 400-wide column.
fn video_note(a: &Assets, text: &str, x: f32, top: f32, col: Color) {
    let size = 7.0_f32.min(7.0 * 400.0 / tw(a, F::Body, text, 7.0).max(1.0));
    label_vc(a, F::Body, text, x, top, 12.0, size, col);
}

/// MOVEMENT / ACTIONS / SYSTEM (ControlsRegistry.LAST_ROUTE_TABS) with Godot/web defaults
/// (project.godot [input] + LAST_ROUTE_TABS). Labels only — rebinding stays disabled.
const SECTIONS: [(&str, &[(&str, &str)]); 3] = [
    (
        "MOVEMENT",
        &[
            ("MOVE UP", "W / L-STICK"),
            ("MOVE DOWN", "S / L-STICK"),
            ("MOVE LEFT", "A / L-STICK"),
            ("MOVE RIGHT", "D / L-STICK"),
            ("LOOK UP", "I / UP / R-STICK"),
            ("LOOK DOWN", "K / DOWN / R-STICK"),
            ("LOOK LEFT", "J / LEFT / R-STICK"),
            ("LOOK RIGHT", "L / RIGHT / R-STICK"),
            ("SPRINT", "SHIFT / PAD L3"),
            ("CROUCH", "C / PAD B"),
            ("JUMP / HOP", "SPACE / PAD A"),
        ],
    ),
    (
        "ACTIONS",
        &[
            ("INTERACT", "E / MOUSE 2 / PAD X"),
            ("FLASHLIGHT", "F / PAD RB"),
            ("TASER", "G / PAD LB"),
            ("USE HELD TOOL", "MOUSE 1 / RT"),
            ("INVENTORY", "TAB / PAD Y"),
            ("HOTBAR", "1-5 / WHEEL"),
            ("DROP HELD ITEM", "Q / DPAD DOWN"),
            ("CHAT", "T / /"),
            ("PUSH TO TALK", "X / R-STICK"),
            ("QUICK CHAT", "` / DPAD UP"),
            ("TOGGLE MAP", "M / PAD BACK"),
            ("PLAYER LIST", "O"),
            ("HIDE HUD", "F1"),
        ],
    ),
    (
        "SYSTEM",
        &[("PAUSE", "ESCAPE / PAD START"), ("FULLSCREEN", "F11")],
    ),
];

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
        4 => {
            out.push(text_line("SKINS CHANGE FROM THE TITLE SCREEN.".into(), 7.0, p.muted, 0.0));
        }
        5 => {
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
        ui.scroll -= wheel_notch(mouse_wheel().1, 20.0);
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

// ------------------------------------------------------------------------------------------- SKIN tab

/// Which SKIN text box has the keyboard.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Url,
    Minecraft,
}

/// The arm styles ARMS cycles through (skin_panel.gd `_cycle_arms`).
const ARMS: [&str; 3] = ["auto", "classic", "slim"];

/// The SKIN tab's state between frames. It lives here rather than in Settings: a picked skin waits for APPLY, and a
/// download runs on a thread that reports back through `fetch`.
struct SkinUi {
    /// A validated 64x64 PNG waiting for APPLY. None keeps APPLY greyed.
    pick: Option<Vec<u8>>,
    /// The preview texture. Rebuilt (set to None) when the pick or the chosen skin changes; inside None means no picture.
    shown: Option<Option<Texture2D>>,
    status: String,
    url: String,
    minecraft: String,
    field: Option<Field>,
    /// A download in progress; its result arrives here.
    fetch: Option<Receiver<Result<Vec<u8>, String>>>,
    rules: Option<skins::SkinRules>,
    catalog: Option<Vec<skins::CatalogSkin>>,
    catalog_index: usize,
    folder_cursor: usize,
    /// OTHERS MAY COPY: on by default (Godot settings.gd `skin_copyable`). Not saved by this build yet.
    copyable: bool,
}

impl Default for SkinUi {
    fn default() -> Self {
        SkinUi {
            pick: None,
            shown: None,
            status: String::new(),
            url: String::new(),
            minecraft: String::new(),
            field: None,
            fetch: None,
            rules: None,
            catalog: None,
            catalog_index: 0,
            folder_cursor: 0,
            copyable: true,
        }
    }
}

thread_local! {
    static SKIN: RefCell<SkinUi> = RefCell::new(SkinUi::default());
}

/// A button that is greyed (and does nothing) when `on` is false.
fn gate(on: bool) -> BtnState {
    if on {
        plain()
    } else {
        disabled()
    }
}

/// APPLY is greyed until a skin is picked (skin_panel.gd `_refresh`).
fn apply_enabled(s: &SkinUi) -> bool {
    s.pick.is_some()
}

/// DEFAULT restores the built-in skin and drops any picked one (skin_panel.gd `_reset`).
fn reset_to_default(st: &mut Settings, s: &mut SkinUi) {
    st.skin = SKIN_DEFAULT.to_string();
    s.pick = None;
    s.shown = None;
    s.status = "Back to the default skin.".to_string();
}

/// What ARMS shows: the setting, and for auto the arm style the skin in the preview resolves to.
fn arms_label(arms: &str, slim: bool) -> String {
    if arms == "auto" {
        format!("ARMS: AUTO ({})", if slim { "SLIM" } else { "CLASSIC" })
    } else {
        format!("ARMS: {}", arms.to_uppercase())
    }
}

/// True when the preview should wear slim arms. HOOK: the integrator points this at ArmsSetting::resolve (task 6).
fn preview_slim(arms: &str, png: Option<&[u8]>) -> bool {
    crate::app::settings::ArmsSetting::resolve(arms, png)
}

/// TEMPLATE: saves the skin template to Downloads and reports where, or why not.
fn save_template_status(s: &mut SkinUi) {
    s.status = match crate::shared::skin_template::save_template_to_downloads() {
        Ok(path) => format!("Saved template to {}", path.display()),
        Err(e) => format!("Could not save the template: {e}"),
    };
}

/// The ARMS setting after `cur`: auto, classic, slim, then auto again.
fn next_arms(cur: &str) -> &'static str {
    let i = ARMS.iter().position(|a| *a == cur).unwrap_or(0);
    ARMS[(i + 1) % ARMS.len()]
}

/// Puts a validated skin into the preview, waiting for APPLY (skin_panel.gd `_preview`).
fn set_pick(s: &mut SkinUi, bytes: Vec<u8>) {
    s.pick = Some(bytes);
    s.shown = None;
    s.status = "Preview only. Press APPLY to wear it.".to_string();
}

/// A refused source: nothing is picked and the reason is shown (skin_panel.gd "Not applied: ...").
fn refuse(s: &mut SkinUi, why: &str) {
    s.pick = None;
    s.shown = None;
    s.status = format!("Not applied: {why}.");
}

/// A PNG as a preview texture, drawn with nearest-neighbour filtering so the pixels stay sharp.
fn texture_of(png: &[u8]) -> Texture2D {
    let tex = Texture2D::from_file_with_format(png, Some(ImageFormat::Png));
    tex.set_filter(FilterMode::Nearest);
    tex
}

/// Starts a download on a thread. `poll_fetch` takes the result on a later frame, so the UI never waits for the network.
fn start_fetch(s: &mut SkinUi, job: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static) {
    if s.fetch.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    s.fetch = Some(rx);
    s.status = "Downloading...".to_string();
}

/// Takes a finished download, when there is one, into the preview or the refusal.
fn poll_fetch(s: &mut SkinUi) {
    let Some(rx) = s.fetch.as_ref() else { return };
    let done = match rx.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("the download stopped".to_string()),
    };
    s.fetch = None;
    match done {
        Ok(bytes) => set_pick(s, bytes),
        Err(why) => refuse(s, &why),
    }
}

/// PICK A PNG. This build has no file dialog, so it offers the 64x64 PNGs in the skins folder one at a time.
fn pick_from_folder(s: &mut SkinUi) {
    let ids: Vec<String> = skins::list_skins(skins::skin_dir().as_deref())
        .into_iter()
        .skip(1)
        .collect();
    if ids.is_empty() {
        s.status = "No 64x64 PNG is in the skins folder yet.".to_string();
        return;
    }
    let id = ids[s.folder_cursor % ids.len()].clone();
    s.folder_cursor += 1;
    match skins::skin_png(&id) {
        Some((_, bytes)) => set_pick(s, bytes),
        None => refuse(s, "that file can no longer be read"),
    }
}

/// FROM URL: the link is checked by the rules and downloaded on a thread, then validated like a picked file.
fn from_url(s: &mut SkinUi) {
    let Some(rules) = s.rules.clone() else {
        refuse(s, "the skin rules are missing from this build");
        return;
    };
    let url = s.url.trim().to_string();
    start_fetch(s, move || skins::fetch_url_skin(&url, &rules));
}

/// FROM MC NAME: the name's skin through Mojang's APIs, on a thread (see `skins::fetch_minecraft_skin`).
fn from_minecraft(s: &mut SkinUi) {
    let Some(rules) = s.rules.clone() else {
        refuse(s, "the skin rules are missing from this build");
        return;
    };
    let name = s.minecraft.trim().to_string();
    start_fetch(s, move || skins::fetch_minecraft_skin(&name, &rules));
}

/// FROM CATALOG: the chosen catalogue skin goes to the preview, then APPLY like any other source.
fn from_catalog(s: &mut SkinUi) {
    let Some(entry) = s
        .catalog
        .as_ref()
        .and_then(|list| list.get(s.catalog_index))
        .cloned()
    else {
        return;
    };
    match skins::catalog_png(&entry) {
        None => refuse(s, &format!("the picture for {} is not in this build", entry.name)),
        Some(png) => match skins::skin_from_png(png) {
            Ok((_, bytes)) => set_pick(s, bytes),
            Err(why) => refuse(s, &why),
        },
    }
}

/// APPLY: the picked skin is saved as the user's own file and becomes the chosen skin.
fn apply_pick(s: &mut SkinUi, st: &mut Settings, changed: &mut bool) {
    let Some(bytes) = s.pick.take() else { return };
    match skins::save_custom_skin(&bytes) {
        Ok(id) => {
            st.skin = id;
            *changed = true;
            s.shown = None;
            s.status = "Skin applied.".to_string();
        }
        Err(e) => {
            s.pick = Some(bytes);
            s.status = format!("Not applied: the skin could not be saved ({e}).");
        }
    }
}

/// Keys for the focused SKIN text box: printable characters (the name box takes only the rules' name characters) and
/// Backspace. Returns true when Enter was pressed.
fn type_into(s: &mut SkinUi) -> bool {
    let Some(field) = s.field else {
        while get_char_pressed().is_some() {}
        return false;
    };
    let Some(rules) = s.rules.as_ref() else {
        return false;
    };
    let (text, cap) = match field {
        Field::Url => (&mut s.url, rules.max_url_length),
        Field::Minecraft => (&mut s.minecraft, rules.name_max),
    };
    while let Some(ch) = get_char_pressed() {
        let allowed = match field {
            Field::Url => ch.is_ascii_graphic(),
            Field::Minecraft => rules.name_chars.contains(ch),
        };
        if allowed && text.len() < cap {
            text.push(ch);
        }
    }
    if is_key_pressed(KeyCode::Backspace) {
        text.pop();
    }
    is_key_pressed(KeyCode::Enter)
}

/// The skin's front (head and hat, body, arms, legs) at 4 screen pixels to a skin pixel, centred in `area`. This is
/// the 2D stand-in for the 3D mannequin preview.
fn draw_front(tex: &Texture2D, area: Rect) {
    let k = 4.0;
    let cx = area.x + area.w * 0.5;
    let top = area.y + (area.h - 32.0 * k) * 0.5;
    let part = |src: (f32, f32), size: (f32, f32), x: f32, y: f32| {
        draw_texture_ex(
            tex,
            x,
            y,
            WHITE,
            DrawTextureParams {
                source: Some(Rect::new(src.0, src.1, size.0, size.1)),
                dest_size: Some(vec2(size.0 * k, size.1 * k)),
                ..Default::default()
            },
        );
    };
    part((4.0, 20.0), (4.0, 12.0), cx - 8.0 * k, top + 20.0 * k);
    part((20.0, 52.0), (4.0, 12.0), cx, top + 20.0 * k);
    part((44.0, 20.0), (4.0, 12.0), cx + 4.0 * k, top + 8.0 * k);
    part((36.0, 52.0), (4.0, 12.0), cx - 8.0 * k, top + 8.0 * k);
    part((20.0, 20.0), (8.0, 12.0), cx - 4.0 * k, top + 8.0 * k);
    part((8.0, 8.0), (8.0, 8.0), cx - 4.0 * k, top);
    part((40.0, 8.0), (8.0, 8.0), cx - 4.0 * k, top);
}

thread_local! {
    /// The rotating 3D mannequin in the preview box (ui/skin_preview.rs).
    static SKIN_PREVIEW: RefCell<crate::ui::skin_preview::SkinPreview> = RefCell::new(crate::ui::skin_preview::SkinPreview::new());
}

/// The preview box: the chosen skin (or the pick waiting for APPLY), else the word DEFAULT.
fn draw_preview(a: &Assets, s: &mut SkinUi, st: &Settings, area: Rect) {
    if s.shown.is_none() {
        let png = match &s.pick {
            Some(bytes) => Some(bytes.clone()),
            None if st.skin == SKIN_DEFAULT => None,
            None => skins::skin_png(&st.skin).map(|(_, bytes)| bytes),
        };
        s.shown = Some(png.map(|bytes| texture_of(&bytes)));
    }
    let png: Option<Vec<u8>> = match &s.pick {
        Some(bytes) => Some(bytes.clone()),
        None if st.skin == SKIN_DEFAULT => None,
        None => skins::skin_png(&st.skin).map(|(_, bytes)| bytes),
    };
    let slim = preview_slim(&st.arms, png.as_deref());
    if a.npc_body.is_some() {
        let tex = s.shown.as_ref().and_then(Option::as_ref);
        SKIN_PREVIEW.with(|p| p.borrow_mut().draw(a, area, tex, slim, get_frame_time()));
        return;
    }
    // Fallback: the flat front, when the 3D preview target cannot be made.
    match s.shown.as_ref().and_then(Option::as_ref) {
        Some(tex) => draw_front(tex, area),
        None => label_c(
            a,
            F::Body,
            "DEFAULT",
            area.x + area.w * 0.5,
            area.y + area.h * 0.5 - 4.0,
            7.0,
            pal().muted,
        ),
    }
}

/// The SKIN tab, laid out as the Godot skin panel (game/menus/skin_panel.gd): the preview on the left; on the right
/// PICK A PNG and APPLY, DEFAULT and TEMPLATE, the FROM URL and FROM MC NAME boxes, the copy row, the catalogue row,
/// OTHERS MAY COPY and ARMS, then the status line and "Your skin".
fn skin_tab(a: &Assets, st: &mut Settings, ox: f32, oy: f32, changed: &mut bool) {
    let p = pal();
    SKIN.with(|cell| {
        let mut s = cell.borrow_mut();
        if s.rules.is_none() {
            s.rules = skins::rules();
        }
        if s.catalog.is_none() {
            s.catalog = Some(skins::catalog());
        }
        poll_fetch(&mut s);

        let top = oy + 66.0;
        let cx = ox + 180.0;
        let bw = 158.0;
        let bx = cx + bw + 2.0;
        let gw = 80.0;
        let half = (240.0 - 2.0) * 0.5;
        let url_box = Rect::new(cx, top + 36.0, bw, 16.0);
        let mc_box = Rect::new(cx, top + 54.0, bw, 16.0);

        if is_mouse_button_pressed(MouseButton::Left) {
            if hovered(url_box) {
                s.field = Some(Field::Url);
            } else if hovered(mc_box) {
                s.field = Some(Field::Minecraft);
            }
        }
        if type_into(&mut s) {
            match s.field {
                Some(Field::Url) => from_url(&mut s),
                Some(Field::Minecraft) => from_minecraft(&mut s),
                None => {}
            }
        }
        let busy = s.fetch.is_some();

        draw_preview(a, &mut s, st, Rect::new(ox + 20.0, top, 150.0, 150.0));

        // Rows 1-2: PICK A PNG | APPLY, DEFAULT | TEMPLATE
        if button(a, Rect::new(cx, top, half, 16.0), "PICK A PNG", 7.0, Kind::Secondary, gate(!busy)) {
            pick_from_folder(&mut s);
        }
        if button(
            a,
            Rect::new(cx + half + 2.0, top, half, 16.0),
            "APPLY",
            7.0,
            Kind::Secondary,
            gate(!busy && apply_enabled(&s)),
        ) {
            apply_pick(&mut s, st, changed);
        }
        let default_on = !busy && (st.skin != SKIN_DEFAULT || s.pick.is_some());
        if button(a, Rect::new(cx, top + 18.0, half, 16.0), "DEFAULT", 7.0, Kind::Secondary, gate(default_on)) {
            reset_to_default(st, &mut s);
            *changed = true;
        }
        if button(
            a,
            Rect::new(cx + half + 2.0, top + 18.0, half, 16.0),
            "TEMPLATE",
            7.0,
            Kind::Secondary,
            gate(!busy),
        ) {
            save_template_status(&mut s);
        }

        // Row 3: the https link box and FROM URL
        line_edit(
            a,
            url_box,
            &s.url,
            "https:// link to a 64x64 PNG skin",
            7.0,
            s.field == Some(Field::Url),
            false,
            get_time() as f32,
        );
        if button(a, Rect::new(bx, top + 36.0, gw, 16.0), "FROM URL", 7.0, Kind::Secondary, gate(!busy)) {
            from_url(&mut s);
        }

        // Row 4: the Minecraft name box and FROM MC NAME
        line_edit(
            a,
            mc_box,
            &s.minecraft,
            "Minecraft username",
            7.0,
            s.field == Some(Field::Minecraft),
            false,
            get_time() as f32,
        );
        if button(a, Rect::new(bx, top + 54.0, gw, 16.0), "FROM MC NAME", 7.0, Kind::Secondary, gate(!busy)) {
            from_minecraft(&mut s);
        }

        // Row 5: the co-op player list (no one is listed in this build) and COPY SKIN, greyed
        let copy_row = top + 72.0;
        sbox(Rect::new(cx, copy_row, bw, 16.0), p.inset, p.field_edge, 1.0, p.corner);
        label_vc(a, F::Body, "(no one to copy)", cx + 6.0, copy_row, 16.0, 7.0, p.muted);
        button(a, Rect::new(bx, copy_row, gw, 16.0), "COPY SKIN", 7.0, Kind::Secondary, disabled());

        // Row 6: the catalogue (cycles through the entries) and FROM CATALOG
        let catalog_row = top + 90.0;
        let (list_len, catalog_label) = match s.catalog.as_ref() {
            Some(list) if !list.is_empty() => {
                let i = s.catalog_index % list.len();
                (list.len(), list[i].name.clone())
            }
            _ => (0, "(no catalog skins)".to_string()),
        };
        if button(
            a,
            Rect::new(cx, catalog_row, bw, 16.0),
            &catalog_label,
            7.0,
            Kind::Secondary,
            gate(list_len > 0),
        ) {
            s.catalog_index = (s.catalog_index + 1) % list_len.max(1);
        }
        if button(
            a,
            Rect::new(bx, catalog_row, gw, 16.0),
            "FROM CATALOG",
            7.0,
            Kind::Secondary,
            gate(!busy && list_len > 0),
        ) {
            from_catalog(&mut s);
        }

        // Row 7: OTHERS MAY COPY and ARMS
        let options_row = top + 108.0;
        if state_toggle(
            a,
            Rect::new(cx, options_row, bw, 16.0),
            "OTHERS MAY COPY",
            s.copyable,
            false,
            plain(),
        ) {
            s.copyable = !s.copyable;
        }
        let shown_png: Option<Vec<u8>> = match &s.pick {
            Some(bytes) => Some(bytes.clone()),
            None if st.skin == SKIN_DEFAULT => None,
            None => skins::skin_png(&st.skin).map(|(_, bytes)| bytes),
        };
        let label_text = arms_label(&st.arms, preview_slim(&st.arms, shown_png.as_deref()));
        if button(a, Rect::new(bx, options_row, gw, 16.0), &label_text, 7.0, Kind::Secondary, plain()) {
            st.arms = next_arms(&st.arms).to_string();
            *changed = true;
        }

        // Status, then the skin in use
        for (i, line) in wrap(a, F::Body, &s.status, 240.0, 6.0).iter().enumerate() {
            label(a, F::Body, line, cx, top + 126.0 + i as f32 * 8.0, 6.0, p.body);
        }
        let mine = if st.skin == SKIN_DEFAULT { "default" } else { "custom" };
        label(a, F::Body, &format!("Your skin: {mine}"), cx, top + 150.0, 7.0, p.muted);
    });
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
            if state_toggle(
                a,
                Rect::new(x, oy + 106.0, 400.0, 14.0),
                "REDUCED MOTION",
                st.reduced_motion,
                false,
                plain(),
            ) {
                st.reduced_motion = !st.reduced_motion;
                super::style::set_reduced_motion(st.reduced_motion);
                changed = true;
            }
        }
        4 if !in_run => {
            // SKIN: the Godot skin panel (game/menus/skin_panel.gd), see skin_tab
            skin_tab(a, st, ox, oy, &mut changed);
        }
        4 | 5 | 6 => {
            // SKIN (during a run), VOICE and MODS each scroll their text below the tab bar
            let lines = section_text(a, content, in_run, ui.settings_tab);
            scroll_text(a, ui, x, Rect::new(x, oy + 66.0, 400.0, 160.0), &lines);
        }
        1 => {
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
            if state_toggle(
                a,
                Rect::new(x, oy + 190.0, 400.0, 16.0),
                "SPEAK NEARBY CHAT",
                st.speech_enabled,
                false,
                plain(),
            ) {
                st.speech_enabled = !st.speech_enabled;
                changed = true;
            }
        }
        2 => {
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
                1000.0,
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
                FOV_MIN_DEG,
                FOV_MAX_DEG,
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
            video_note(a, VSYNC_NOTES[0], x, oy + 156.0, p.muted);
            video_note(a, VSYNC_NOTES[1], x, oy + 168.0, p.muted);
            // Metal or OpenGL: macOS only, read when the game starts (app/gfx.rs)
            if cfg!(target_os = "macos") {
                let label = graphics_api_label(
                    &st.graphics_api,
                    crate::app::platform::is_metal(),
                    crate::app::gfx::chosen().marker_forced,
                );
                if button(a, Rect::new(x, oy + 184.0, 400.0, 16.0), &label, 8.0, Kind::Secondary, plain()) {
                    let next = crate::app::gfx::Choice::parse(&st.graphics_api)
                        .unwrap_or(crate::app::gfx::Choice::Auto)
                        .next();
                    st.graphics_api = next.id().into();
                    changed = true;
                }
                video_note(a, METAL_NOTES[0], x, oy + 202.0, p.muted);
                video_note(a, METAL_NOTES[1], x, oy + 214.0, p.muted);
            }
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
                ui.scroll -= wheel_notch(mouse_wheel().1, 20.0);
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
    fn graphics_api_row_names_the_running_api() {
        assert_eq!(graphics_api_label("AUTO", true, false), "GRAPHICS API: AUTO (NOW METAL)");
        assert_eq!(graphics_api_label("OPENGL", false, false), "GRAPHICS API: OPENGL (NOW OPENGL)");
        assert_eq!(graphics_api_label("METAL", false, false), "GRAPHICS API: METAL (NOW OPENGL)");
    }

    #[test]
    fn the_label_says_when_a_crashed_metal_start_forced_opengl() {
        let l = graphics_api_label("METAL", false, true);
        assert!(l.contains("DID NOT FINISH"), "{l}");
        // a run that is on Metal needs no warning, whatever the marker said
        assert_eq!(graphics_api_label("METAL", true, true), "GRAPHICS API: METAL (NOW METAL)");
    }

    #[test]
    fn the_video_tab_notes_explain_vsync_and_say_metal_is_experimental() {
        assert!(VSYNC_NOTES[0].starts_with("VSYNC ON") && VSYNC_NOTES[1].starts_with("VSYNC OFF"));
        assert!(VSYNC_NOTES[1].contains("FPS LIMIT"));
        assert!(METAL_NOTES.iter().any(|l| l.contains("EXPERIMENTAL")));
        // the notes are drawn in capitals only
        for l in VSYNC_NOTES.iter().chain(METAL_NOTES.iter()) {
            assert_eq!(l.to_uppercase(), *l);
        }
    }

    #[test]
    fn tabs_match_the_web_overlay_in_order() {
        assert_eq!(
            TABS,
            ["GAMEPLAY", "AUDIO", "VIDEO", "CONTROLS", "SKIN", "VOICE", "MODS"]
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
    fn default_bindings_match_godot_defaults() {
        let actions = SECTIONS.iter().find(|(n, _)| *n == "ACTIONS").unwrap().1;
        let get = |name: &str| {
            actions
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, b)| *b)
                .unwrap_or("")
        };
        assert!(get("INVENTORY").contains("TAB"), "inventory default is Tab: {}", get("INVENTORY"));
        assert!(get("INVENTORY").contains("PAD Y"), "inventory gamepad Y: {}", get("INVENTORY"));
        assert!(get("CHAT").contains('T'), "chat default is T: {}", get("CHAT"));
        assert!(get("PLAYER LIST").contains('O'), "player list default is O: {}", get("PLAYER LIST"));
        let movement = SECTIONS.iter().find(|(n, _)| *n == "MOVEMENT").unwrap().1;
        let jump = movement.iter().find(|(n, _)| *n == "JUMP / HOP").unwrap().1;
        assert!(jump.contains("PAD A"), "jump gamepad: {jump}");
    }

    #[test]
    fn apply_is_greyed_until_a_skin_is_picked() {
        let mut s = SkinUi::default();
        assert!(!apply_enabled(&s), "nothing picked: APPLY is off");
        set_pick(&mut s, vec![1, 2, 3]);
        assert!(apply_enabled(&s), "a picked skin turns APPLY on");
    }

    #[test]
    fn default_restores_the_default_skin() {
        let mut st = Settings::default();
        st.skin = "CUSTOM".to_string();
        let mut s = SkinUi::default();
        set_pick(&mut s, vec![9]);
        reset_to_default(&mut st, &mut s);
        assert_eq!(st.skin, SKIN_DEFAULT);
        assert!(!apply_enabled(&s), "DEFAULT drops the picked skin too");
    }

    #[test]
    fn arms_cycle_auto_classic_slim() {
        assert_eq!(next_arms("auto"), "classic");
        assert_eq!(next_arms("classic"), "slim");
        assert_eq!(next_arms("slim"), "auto");
    }

    #[test]
    fn skin_tab_defaults_match_godot() {
        let s = SkinUi::default();
        assert!(s.copyable, "OTHERS MAY COPY is on by default (settings.gd skin_copyable)");
        assert_eq!(Settings::default().arms, "auto");
    }

    #[test]
    fn arms_persists_through_the_settings_struct() {
        let mut st = Settings::default();
        let mut seen = vec![st.arms.clone()];
        for _ in 0..3 {
            st.arms = next_arms(&st.arms).to_string();
            seen.push(st.arms.clone());
        }
        assert_eq!(seen, ["auto", "classic", "slim", "auto"]);
    }

    #[test]
    fn arms_label_shows_the_effective_style_for_auto() {
        assert_eq!(arms_label("auto", true), "ARMS: AUTO (SLIM)");
        assert_eq!(arms_label("auto", false), "ARMS: AUTO (CLASSIC)");
        assert_eq!(arms_label("slim", false), "ARMS: SLIM");
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
