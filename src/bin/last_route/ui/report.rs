//! The end of a run, as the original plays it: the death moment (death_moment.gd: a dark
//! subtitle box low on the screen, the catcher's name in spaced capitals and her line typed one
//! word every 250 ms, from death_lines.json) and then the run card (run_report.gd: the signal
//! backdrop with its two extra frames, GAME OVER, the reason and the stats grid).

use super::style::*;
use crate::app::assets::Assets;
use crate::content::embedded;
use crate::game::Game;
use macroquad::prelude::*;
use serde_json::Value;

const WORD_SECONDS: f32 = 0.25;
const INPUT_GUARD_SECONDS: f32 = 0.4;
const AUTO_DISMISS_SECONDS: f32 = 7.0;

/// FNV-1a, 32 bit, over the (ASCII) text, as death_lines.gd hashes the pick key.
fn fnv(text: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for c in text.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

fn line_text(v: &Value) -> Option<(String, bool)> {
    match v {
        Value::String(s) => Some((s.clone(), false)),
        Value::Object(o) => o.get("text").and_then(|t| t.as_str()).map(|t| {
            (
                t.to_string(),
                o.get("style").and_then(|s| s.as_str()) == Some("caption"),
            )
        }),
        _ => None,
    }
}

/// DeathLines.pick: the cause's lines and the hunter's own (plus her lore line), one chosen
/// by a hash of seed|depth|hunter|cause. Void hunters are voiceless (None).
pub fn pick_line(
    hunter_id: &str,
    cause: &str,
    seed_text: &str,
    depth: i32,
    category: &str,
) -> Option<(String, bool)> {
    pick_line_with(hunter_id, cause, seed_text, depth, category, cfg!(debug_assertions))
}

/// `pick_line` with the placeholder rule explicit: lines starting with [PLACEHOLDER] render only in debug builds (DeathLines
/// `allow_placeholders`); a release build falls back to the generic copy until real ones are written.
pub fn pick_line_with(
    hunter_id: &str,
    cause: &str,
    seed_text: &str,
    depth: i32,
    category: &str,
    allow_placeholders: bool,
) -> Option<(String, bool)> {
    if category == "void" {
        return None;
    }
    let data: Value = serde_json::from_slice(embedded::asset("ui/death_lines.json")?).ok()?;
    let id = hunter_id.to_lowercase().replace('_', "");
    let key = match cause {
        "deal" => "deal",
        "cold" => "cold",
        _ if id == "nyxie" => {
            if cause == "lamp" {
                "darkness"
            } else {
                "nyxie"
            }
        }
        _ => "",
    };
    let mut pool: Vec<(String, bool)> = vec![];
    if !key.is_empty() {
        if let Some(list) = data["causes"][key].as_array() {
            pool.extend(list.iter().filter_map(line_text));
        }
    }
    let hunter = &data["hunters"][id.as_str()];
    let caption_hunter = hunter["style"].as_str() == Some("caption");
    if let Some(list) = hunter["lines"].as_array() {
        pool.extend(
            list.iter()
                .filter_map(line_text)
                .map(|(t, c)| (t, c || caption_hunter)),
        );
    }
    if let Some(l) = line_text(&hunter["loreLine"]) {
        pool.push((l.0, l.1 || caption_hunter));
    }
    pool.retain(|(t, _)| {
        !t.trim().is_empty() && (allow_placeholders || !t.trim().to_uppercase().starts_with("[PLACEHOLDER]"))
    });
    if pool.is_empty() {
        return None;
    }
    let i = fnv(&format!("{seed_text}|{depth}|{id}|{key}")) as usize % pool.len();
    let (t, c) = pool[i].clone();
    Some((t.replace("{depth}", &depth.to_string()), c))
}

/// The death moment's state while it is on screen.
pub struct DeathMoment {
    pub name: String,
    pub words: Vec<String>,
    pub caption: bool,
    pub shown: usize,
    pub timer: f32,
    pub age: f32,
    pub full: bool,
    pub done: bool,
}

impl DeathMoment {
    pub fn new(name: &str, line: &str, caption: bool) -> DeathMoment {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let mut m = DeathMoment {
            name: name.to_uppercase(),
            shown: 0,
            timer: 0.0,
            age: 0.0,
            full: false,
            done: false,
            caption,
            words,
        };
        if caption {
            m.shown = m.words.len();
            m.full = true;
        }
        m
    }

    /// Advances typing and input; any key or click finishes the line, then dismisses it.
    pub fn update(&mut self, dt: f32) {
        self.age += dt;
        self.timer += dt;
        if !self.full {
            self.shown = self
                .words
                .len()
                .min((self.timer / WORD_SECONDS) as usize + 1);
            if self.shown >= self.words.len() {
                self.full = true;
                self.timer = 0.0;
            }
        } else if self.timer >= AUTO_DISMISS_SECONDS {
            self.done = true;
        }
        let pressed = get_last_key_pressed().is_some()
            || is_mouse_button_pressed(MouseButton::Left)
            || is_mouse_button_pressed(MouseButton::Right);
        if pressed && self.age >= INPUT_GUARD_SECONDS {
            if is_key_pressed(KeyCode::Escape) || self.full {
                self.done = true;
            } else {
                self.shown = self.words.len();
                self.full = true;
                self.timer = 0.0;
            }
        }
    }

    pub fn draw(&self, a: &Assets) {
        let p = pal();
        let v = view();
        let fade = (self.age / 0.25).min(1.0);
        fill(
            Rect::new(0.0, 0.0, v.x, v.y),
            lr(
                rgba(0.0078, 0.0039, 0.0118, 0.55 * fade),
                rgba(0.02, 0.01, 0.03, 0.55 * fade),
            ),
        );
        // SubtitleBox: anchored bottom centre, -170..170 x -86..-22, pad 9, column separation 4
        let text = self.words[..self.shown].join(" ");
        let lines = wrap(a, F::Body, &text, 322.0, 9.0);
        let f = if self.caption { F::Italic } else { F::Body };
        let content_h = lh(F::Body, 6.0)
            + 4.0
            + lines.len().max(1) as f32 * lh(F::Body, 9.0)
            + 4.0
            + lh(F::Body, 5.0);
        let h = 64.0f32.max(content_h + 18.0);
        let r = Rect::new(v.x * 0.5 - 170.0, v.y - 22.0 - h, 340.0, h);
        let a_ = |c: Color| alpha(c, c.a * fade);
        sbox(
            r,
            a_(lr(hexa(0x000000f0), rgba(0.04, 0.02, 0.06, 0.94))),
            a_(lr(hex(0x5a5446), p.panel_edge)),
            1.0,
            p.corner,
        );
        let x = r.x + 9.0;
        let mut y = r.y + 9.0;
        label(a, F::Body, &track(&self.name), x, y, 6.0, a_(p.amber));
        y += lh(F::Body, 6.0) + 4.0;
        for l in &lines {
            label(a, f, l, x, y, 9.0, a_(p.ink));
            y += lh(F::Body, 9.0);
        }
        let hy = r.y + r.h - 9.0 - lh(F::Body, 5.0);
        label_r(
            a,
            F::Body,
            "CLICK OR PRESS A KEY",
            r.x + r.w - 9.0,
            hy,
            5.0,
            a_(p.dim),
        );
    }
}

/// What the card says and offers.
pub struct Card {
    pub kicker: String,
    pub title: String,
    pub reason: String,
    pub primary: String,
    pub secondary: String,
    pub stats: Vec<(&'static str, String)>,
    /// The SPECTATE button, when another runner is in the room to watch (game/spectate.rs).
    pub spectate: Option<String>,
}

pub fn card_for(g: &Game, seed_text: &str, can_respawn: bool, spectate_ok: bool) -> Option<Card> {
    let r = g.report.as_ref()?;
    let name = r.hunter_name.to_uppercase();
    let reason = match r.cause {
        "cold" => "FROZEN ON THE ROUTE".to_string(),
        _ => format!("CAUGHT BY {name}"),
    };
    let infinite = g.is_infinite();
    let depth = r.depth.max(1);
    Some(Card {
        kicker: if infinite {
            "RUNNER SIGNAL // LOST".into()
        } else {
            "ROUTE SIGNAL // ARCHIVED".into()
        },
        title: "GAME OVER".into(),
        reason: if infinite {
            reason
        } else {
            format!("CAPTURED BY {name}")
        },
        primary: if can_respawn {
            "RESPAWN AT CHECKPOINT".into()
        } else {
            "RESTART ROUTE".into()
        },
        secondary: "MAIN MENU".into(),
        spectate: (spectate_ok && !g.spectating()).then(|| crate::game::spectate::labels().button.clone()),
        stats: vec![
            ("DEPTH REACHED", depth.to_string()),
            ("SCORE", r.score.max(0).to_string()),
            (
                "FLOOR RELAYS",
                format!("{} / {}", r.relays.max(0), g.fac.relays.len()),
            ),
            ("COINS", r.coins.max(0).to_string()),
            ("DIFFICULTY", r.difficulty.name().to_string()),
            ("ROUTE SEED", seed_text.to_string()),
        ],
    })
}

/// The run card's two buttons (for hit-testing), laid out like draw_card.
pub fn draw_card(a: &Assets, c: &Card, focus: usize) -> Option<usize> {
    let p = pal();
    backdrop(0.72);
    report_frames();
    let l6 = lh(F::Body, 6.0);
    let l7 = lh(F::Body, 7.0);
    let row_h = l6.max(l7);
    let grid_h = c.stats.len() as f32 * row_h + (c.stats.len() as f32 - 1.0) * 2.0;
    let bh = button_min(a, "X", 8.0, false).y.max(18.0);
    let content_h = l7 + 5.0 + lh(F::Title, 22.0) + 5.0 + l7 + 5.0 + grid_h + 12.0 + 5.0 + bh;
    let h = 192.0f32.max(content_h + 26.0);
    let panel = centered(340.0, h);
    sbox(panel, p.panel, lr(p.panel_edge, hex(0x563041)), 1.0, p.corner);
    let x = panel.x + 13.0;
    let w = panel.w - 26.0;
    let mut y = panel.y + 13.0;
    label(a, F::Body, &c.kicker, x, y, 7.0, p.amber);
    y += l7 + 5.0;
    label(a, F::Title, &c.title, x, y, 22.0, p.ink);
    y += lh(F::Title, 22.0) + 5.0;
    label(a, F::Body, &c.reason, x, y, 7.0, p.alert);
    y += l7 + 5.0;
    panel_box(Rect::new(x, y, w, grid_h + 12.0), p.inset, p.panel_edge);
    let key_w = c
        .stats
        .iter()
        .map(|(k, _)| tw(a, F::Body, k, 6.0))
        .fold(0.0, f32::max);
    let mut gy = y + 6.0;
    for (k, val) in &c.stats {
        label_vc(a, F::Body, k, x + 6.0, gy, row_h, 6.0, p.muted);
        let vx = x + 6.0 + key_w + 12.0;
        label_vc(
            a,
            F::Body,
            &ellipsize(a, F::Body, val, x + w - 6.0 - vx, 7.0),
            vx,
            gy,
            row_h,
            7.0,
            p.ink,
        );
        gy += row_h + 2.0;
    }
    y += grid_h + 12.0 + 5.0;
    let w1 = button_min(a, &c.primary, 8.0, false).x.max(92.0);
    let w2 = button_min(a, &c.secondary, 7.0, false).x.max(76.0);
    let w3 = c.spectate.as_ref().map_or(0.0, |s| button_min(a, s, 7.0, false).x.max(76.0));
    let mut hit = None;
    if button(
        a,
        Rect::new(x, y, w1, bh),
        &c.primary,
        8.0,
        Kind::Primary,
        state(focus == 0),
    ) {
        hit = Some(0);
    }
    if button(
        a,
        Rect::new(x + w1 + 6.0, y, w2, bh),
        &c.secondary,
        7.0,
        Kind::Secondary,
        state(focus == 1),
    ) {
        hit = Some(1);
    }
    if let Some(sl) = &c.spectate {
        if button(
            a,
            Rect::new(x + w1 + w2 + 12.0, y, w3, bh),
            sl,
            7.0,
            Kind::Secondary,
            state(focus == 2),
        ) {
            hit = Some(2);
        }
    }
    hit
}

/// The WATCHING line of a spectator, at the top of the screen, and the hint under it.
pub fn draw_watching(a: &Assets, target: Option<&str>) {
    let p = pal();
    let v = view();
    let labels = crate::game::spectate::labels();
    let line = match target {
        Some(name) => labels.watching_line(&name.to_uppercase()),
        None => labels.no_one.clone(),
    };
    let y = 14.0;
    label_r(a, F::Body, &line, v.x - 14.0, y, 7.0, p.amber);
    label_r(a, F::Body, &labels.hint, v.x - 14.0, y + lh(F::Body, 7.0) + 2.0, 5.0, p.dim);
}

/// The follow-camera labels under the watching line (data/ui/spectate_camera.json), drawn while the follow camera is on.
pub fn draw_follow_hint(a: &Assets) {
    let p = pal();
    let v = view();
    let cfg = crate::game::spectate::camera_cfg();
    let y = 14.0 + (lh(F::Body, 7.0) + 2.0) + (lh(F::Body, 5.0) + 4.0);
    label_r(a, F::Body, &cfg.follow_label, v.x - 14.0, y, 5.0, p.amber);
    label_r(a, F::Body, &cfg.cycle_hint, v.x - 14.0, y + lh(F::Body, 5.0) + 2.0, 5.0, p.dim);
}

/// The SPECTATE prompt of a downed runner, low on the screen. True when it is pressed this frame.
pub fn draw_downed_prompt(a: &Assets) -> bool {
    let v = view();
    let label = &crate::game::spectate::labels().downed_prompt;
    let w = button_min(a, label, 7.0, false).x.max(150.0);
    let bh = button_min(a, label, 7.0, false).y.max(18.0);
    button(
        a,
        Rect::new(v.x * 0.5 - w * 0.5, v.y - bh - 30.0, w, bh),
        label,
        7.0,
        Kind::Secondary,
        state(false),
    )
}

#[cfg(test)]
mod secret_line_tests {
    use super::*;

    #[test]
    fn the_wolf_spiders_have_caption_placeholder_lines_that_only_a_debug_build_shows() {
        for id in ["white_wolf_spider"] {
            assert_eq!(pick_line_with(id, "", "ROOM-X", 3, "secret", false), None, "{id}: a release build shows the generic line");
            let (text, caption) = pick_line_with(id, "", "ROOM-X", 3, "secret", true).unwrap_or_else(|| panic!("{id}: no placeholder line"));
            assert!(text.starts_with("[PLACEHOLDER]") && caption, "{id}: {text}");
        }
    }

    #[test]
    fn an_ordinary_hunter_is_unchanged_by_the_placeholder_rule() {
        let release = pick_line_with("aki", "", "ROOM-X", 3, "az", false);
        assert!(release.is_some() && release == pick_line_with("aki", "", "ROOM-X", 3, "az", true));
    }
}
