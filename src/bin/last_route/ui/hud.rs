//! The in-run HUD (hud.tscn + hud.gd _layout_hud), in design units: FPS top-left, the 88x72
//! minimap top-right with the content-sized objective panel under it, the toast line at the top
//! centre, the prompt under the crosshair, the stamina bar over the five-slot hotbar, and the
//! escape struggle box. Text keeps at least 12 window pixels (hud.gd MIN_TEXT_PIXELS).

use super::map::draw_map;
use super::style::*;
use crate::app::assets::Assets;
use crate::config::*;
use crate::game::{Focus, Game, Item};
use macroquad::prelude::*;

/// Records what the HUD drew this frame (the self-test reads it).
#[derive(Default, Clone)]
pub struct HudDrawn {
    pub depth: bool,
    pub relays: bool,
    pub stamina: bool,
    pub hotbar: bool,
    pub minimap: bool,
}

const SAFE: f32 = 8.0;
const GAP: f32 = 4.0;

/// hud.tscn's StyleBoxFlat_hud (Last Route) or the scheme's frame (other schemes).
fn hud_box(r: Rect) {
    if is_last_route() {
        let p = pal();
        sbox(r, p.panel, p.panel_edge, 1.0, 2.0);
    } else {
        let p = pal();
        sbox(r, alpha(p.panel, 0.9), p.panel_edge, 1.0, p.corner);
    }
}

fn meter(r: Rect, f: f32, col: Color) {
    let (track, edge) = if is_last_route() {
        (hex(0x09060a), hexa(0xffffff57))
    } else {
        (rgba(0.106, 0.063, 0.086, 1.0), rgba(0.168, 0.102, 0.133, 0.9))
    };
    sbox(r, track, edge, 1.0, 0.0);
    if f > 0.0 {
        fill(Rect::new(r.x, r.y, r.w * f.clamp(0.0, 1.0), r.h), col);
    }
}

fn shadowed(a: &Assets, text: &str, x: f32, top: f32, size: f32, col: Color) {
    label(
        a,
        F::Body,
        text,
        x + 1.0,
        top + 1.0,
        size,
        rgba(0.0, 0.0, 0.0, col.a),
    );
    label(a, F::Body, text, x, top, size, col);
}

pub fn draw_hud(g: &Game, a: &Assets, fps: i32, _s: f32) -> HudDrawn {
    let mut drawn = HudDrawn::default();
    let p = pal();
    let v = view();
    let floor_units = (12.0 / unit()).ceil().min(9.0);
    let fs = |base: f32| base.max(floor_units);

    shadowed(
        a,
        &format!("FPS {fps}"),
        SAFE,
        SAFE,
        fs(7.0),
        lr(rgba(0.725, 1.0, 0.690, 0.95), rgba(0.729, 0.698, 0.659, 0.95)),
    );

    // minimap, then the objective panel under it, both right-aligned
    let right = v.x - SAFE;
    let mm = Rect::new(right - 88.0, SAFE, 88.0, 72.0);
    hud_box(mm);
    clip_begin(Rect::new(mm.x + 1.0, mm.y + 1.0, mm.w - 2.0, mm.h - 2.0));
    draw_map(g, mm, false);
    clip_end();
    drawn.minimap = true;

    let depth = format!("DEPTH {:02}", g.depth);
    let (relays, danger) = match &g.inf {
        Some(_) => (
            format!(
                "RELAYS {} // CHUNKS {}",
                g.relays_collected,
                g.chunks_visited()
            ),
            Some((g.danger() * 100.0).round().clamp(0.0, 100.0) as i32),
        ),
        None => (
            format!(
                "{}/{} SIGNAL RELAYS",
                g.relays_collected,
                g.fac.relays.len()
            ),
            None,
        ),
    };
    let (d7, d6) = (fs(7.0), fs(6.0));
    let danger_text = danger.map(|d| format!("DANGER {d}%"));
    let mut ow = tw(a, F::Body, &depth, d7).max(tw(a, F::Body, &relays, d6));
    if let Some(t) = &danger_text {
        ow = ow.max(tw(a, F::Body, t, d6) + 3.0 + 72.0);
    }
    let mut oh = lh(F::Body, d7) + 1.0 + lh(F::Body, d6);
    if danger.is_some() {
        oh += 1.0 + lh(F::Body, d6);
    }
    let obj = Rect::new(right - ow, SAFE + 72.0 + GAP, ow, oh);
    hud_box(obj);
    let mut y = obj.y;
    label(
        a,
        F::Body,
        &depth,
        obj.x,
        y,
        d7,
        lr(p.amber, rgba(0.788, 0.663, 0.847, 1.0)),
    );
    y += lh(F::Body, d7) + 1.0;
    label(
        a,
        F::Body,
        &relays,
        obj.x,
        y,
        d6,
        lr(p.ink, rgba(0.949, 0.929, 0.882, 1.0)),
    );
    y += lh(F::Body, d6) + 1.0;
    if let (Some(d), Some(t)) = (danger, &danger_text) {
        let col = if d < 40 {
            meter_safe()
        } else if d < 75 {
            METER_WARN
        } else {
            METER_LOW
        };
        label(a, F::Body, t, obj.x, y, d6, col);
        let bx = obj.x + tw(a, F::Body, t, d6) + 3.0;
        let bh = lh(F::Body, d6);
        meter(
            Rect::new(bx, y + (bh - 4.0) * 0.5, obj.x + obj.w - bx, 4.0),
            d as f32 / 100.0,
            col,
        );
    }
    drawn.depth = true;
    drawn.relays = true;

    // the toast line (MessageLabel): BODY with a dark outline and shadow, top centre at 49
    let toast = if g.toast_t > 0.0 {
        Some((&g.toast, g.toast_t))
    } else if g.message_t > 0.0 {
        Some((&g.message, g.message_t))
    } else {
        None
    };
    if let Some((t, life)) = toast {
        let al = (life / 0.5).min(1.0);
        let size = fs(10.0);
        let text = ellipsize(
            a,
            F::Body,
            t,
            340.0f32.min(v.x - 2.0 * (SAFE + 88.0 + GAP)),
            size,
        );
        let x = v.x * 0.5 - tw(a, F::Body, &text, size) * 0.5;
        label(
            a,
            F::Body,
            &text,
            x + 1.0,
            49.0 + 1.0,
            size,
            rgba(0.0, 0.0, 0.0, al),
        );
        label_outline(
            a,
            F::Body,
            &text,
            x,
            49.0,
            size,
            alpha(p.body, al),
            rgba(0.0, 0.0, 0.0, 0.85 * al),
            2.0,
        );
    }

    let c = (v * 0.5).round();
    if g.p.hidden.is_none() && g.escape.is_none() {
        // the crosshair: four 3-unit arms around a 2-unit gap, a dark pass one unit down-right
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let d = vec2(dx, dy);
            line(
                c + d * 2.0 + vec2(1.0, 1.0),
                c + d * 5.0 + vec2(1.0, 1.0),
                1.0,
                rgba(0.0, 0.0, 0.0, 0.55),
            );
            line(c + d * 2.0, c + d * 5.0, 1.0, p.ink);
        }
        fill(Rect::new(c.x - 0.5, c.y - 0.5, 1.0, 1.0), p.ink);
        if let Some(pr) = g.focus_prompt() {
            let size = fs(10.0);
            let x = c.x - tw(a, F::Body, &pr, size) * 0.5;
            shadowed(a, &pr, x, c.y + 16.0, size, p.ink);
        }
        if g.focus == Some(Focus::MusicBox) {
            let pct = (g.music_box_left / 45.0 * 100.0).clamp(0.0, 100.0);
            let col = if pct <= 0.0 {
                METER_LOW
            } else if g.music_box_left <= 15.0 {
                METER_WARN
            } else {
                meter_safe()
            };
            let t = format!("MUSIC BOX {pct:.0}%");
            shadowed(
                a,
                &t,
                c.x - tw(a, F::Body, &t, 7.0) * 0.5,
                c.y + 30.0,
                7.0,
                col,
            );
            if g.wind_hold > 0.0 {
                meter(
                    Rect::new(c.x - 40.0, c.y + 40.0, 80.0, 3.0),
                    (g.wind_hold / 1.5).min(1.0),
                    p.teal,
                );
            }
        }
    } else if g.p.hidden.is_some() {
        fill(
            Rect::new(0.0, 0.0, v.x, v.y * 0.5 - 12.0),
            rgba(0.02, 0.02, 0.03, 0.92),
        );
        fill(
            Rect::new(0.0, v.y * 0.5 + 12.0, v.x, v.y * 0.5),
            rgba(0.02, 0.02, 0.03, 0.92),
        );
        for k in 0..4 {
            fill(
                Rect::new(v.x * 0.3, v.y * 0.5 - 10.0 + k as f32 * 5.5, v.x * 0.4, 2.5),
                rgba(0.02, 0.02, 0.03, 0.9),
            );
        }
        let t = if g.p.hide_lock > 0.0 {
            format!("LOCKED IN // {:.0}s", g.p.hide_lock.ceil())
        } else {
            "[E] LEAVE".into()
        };
        shadowed(
            a,
            &t,
            c.x - tw(a, F::Body, &t, fs(8.0)) * 0.5,
            c.y + 24.0,
            fs(8.0),
            p.body,
        );
    }
    if g.p.in_vent {
        let t = "IN THE VENTS // [E] AT A GRATE TO MOVE ON";
        shadowed(
            a,
            t,
            c.x - tw(a, F::Body, t, fs(7.0)) * 0.5,
            v.y * 0.2,
            fs(7.0),
            p.teal,
        );
    }
    if g.p.stun > 0.0 {
        let t = format!("STUNNED {:.1}s", g.p.stun);
        shadowed(
            a,
            &t,
            c.x - tw(a, F::Body, &t, fs(8.0)) * 0.5,
            v.y * 0.4,
            fs(8.0),
            METER_LOW,
        );
    }

    // the compact stamina bar (164x5) over the hotbar panel (188x34)
    let stamina = (g.p.stamina / MAX_STAMINA * 100.0).round();
    let st_col = if g.p.exhausted || stamina < 20.0 {
        METER_LOW
    } else if stamina < 45.0 {
        METER_WARN
    } else {
        stamina_col()
    };
    meter(
        Rect::new(c.x - 82.0, v.y - 50.0, 164.0, 5.0),
        stamina / 100.0,
        st_col,
    );
    drawn.stamina = true;

    let hb = Rect::new(c.x - 94.0, v.y - 42.0, 188.0, 34.0);
    hud_box(hb);
    let slot_w = 34.0;
    let sx0 = hb.x + (hb.w - (slot_w * 5.0 + 2.0 * 4.0)) * 0.5;
    for i in 0..HOTBAR_SIZE.min(5) {
        let r = Rect::new(
            sx0 + i as f32 * (slot_w + 2.0),
            hb.y + 1.0,
            slot_w,
            hb.h - 2.0,
        );
        let selected = i == g.selected;
        let slot = g.slots[i];
        let tint = if slot.is_some() {
            WHITE
        } else {
            rgba(0.48, 0.5, 0.51, 0.72)
        };
        let mul = |c: Color| Color::new(c.r * tint.r, c.g * tint.g, c.b * tint.b, c.a * tint.a);
        let edge = if selected {
            p.amber
        } else {
            lr(hexa(0xffffff47), rgba(0.3, 0.14, 0.18, 0.92))
        };
        sbox(
            r,
            mul(lr(hexa(0x09060ab8), rgba(0.078, 0.043, 0.063, 0.94))),
            mul(edge),
            if selected { 2.0 } else { 1.0 },
            1.0,
        );
        if let Some(sl) = slot {
            let icon = &a.icons[match sl.item {
                Item::Flashlight => 0,
                Item::Taser => 1,
                Item::Crowbar => 2,
                Item::Axe => 3,
                Item::Battery => 4,
                Item::Stim => 5,
            }];
            let (iw, ih) = (icon.width().max(1.0), icon.height().max(1.0));
            let k = (r.w / iw).min(22.0 / ih);
            let s = unit();
            draw_texture_ex(
                icon,
                (r.x + (r.w - iw * k) * 0.5) * s,
                r.y * s,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(iw * k * s, ih * k * s)),
                    ..Default::default()
                },
            );
            let frac = match sl.item {
                Item::Flashlight => Some(g.p.lamp / MAX_LAMP_CHARGE),
                Item::Crowbar => Some(sl.n as f32 / CROWBAR_DURABILITY as f32),
                Item::Axe => Some(sl.n as f32 / AXE_DURABILITY as f32),
                Item::Taser => Some(1.0 - g.taser_cd / g.prof().taser_cooldown),
                _ => None,
            };
            if let Some(f) = frac {
                let by = r.y + 20.0 + lh(F::Body, 6.0) - 2.0;
                meter(Rect::new(r.x + 1.0, by, r.w - 2.0, 3.0), f, meter_safe());
            }
            if sl.item.stack_max() > 1 && sl.n > 1 {
                let t = format!("x{}", sl.n);
                label_outline(
                    a,
                    F::Body,
                    &t,
                    r.x + r.w - 1.0 - tw(a, F::Body, &t, 9.0),
                    r.y + r.h - lh(F::Body, 9.0),
                    9.0,
                    lr(p.ink, rgba(1.0, 0.94, 0.72, 1.0)),
                    rgba(0.0, 0.0, 0.0, 0.9),
                    1.5,
                );
            }
        }
        let n = format!("{}", i + 1);
        label_c(
            a,
            F::Body,
            &n,
            r.x + r.w * 0.5,
            r.y + 20.0,
            6.0,
            mul(p.body),
        );
    }
    drawn.hotbar = true;

    if let Some(e) = &g.escape {
        let r = Rect::new(c.x - 90.0, c.y - 39.0, 180.0, 60.0);
        hud_box(r);
        let content = lh(F::Body, 11.0) + 5.0 + 8.0 + 5.0 + 4.0;
        let mut y = r.y + (r.h - content) * 0.5;
        let t = "BREAK FREE!";
        shadowed(
            a,
            t,
            c.x - tw(a, F::Body, t, 11.0) * 0.5,
            y,
            11.0,
            lr(p.ink, rgba(0.788, 0.663, 0.847, 1.0)),
        );
        y += lh(F::Body, 11.0) + 5.0;
        meter(
            Rect::new(r.x + 10.0, y, 160.0, 8.0),
            e.progress as f32 / ESCAPE_PRESSES as f32,
            lr(METER_WARN, rgba(0.91, 0.72, 0.29, 1.0)),
        );
        y += 13.0;
        meter(
            Rect::new(r.x + 10.0, y, 160.0, 4.0),
            e.remaining / ESCAPE_DURATION,
            METER_LOW,
        );
    }

    // the crimson edge pulse while a hunter hunts close by; frost or heat haze at the extremes
    let danger = g.danger();
    if danger > 0.35 && g.report.is_none() {
        let pulse = 0.5 + 0.5 * (g.time * (3.0 + danger * 6.0)).sin();
        edge(
            v,
            10.0,
            rgba(
                0.6,
                0.05,
                0.1,
                (danger - 0.35) * 0.55 * (0.55 + 0.45 * pulse),
            ),
        );
    }
    let heat = ((g.meters.temperature - 76.0) / 24.0).clamp(0.0, 1.0);
    let cold = ((34.0 - g.meters.temperature) / 34.0).clamp(0.0, 1.0);
    if cold > 0.0 {
        let frost = lr(hex(0x5aa9e0), rgba(0.82, 0.96, 1.0, 1.0));
        edge(v, 14.0, alpha(frost, cold * 0.44 * 0.6));
    } else if heat > 0.0 {
        let haze = lr(hex(0xe0512f), rgba(1.0, 0.74, 0.12, 1.0));
        edge(v, 14.0, alpha(haze, heat * 0.22));
    }
    if g.fade > 0.0 {
        fill(
            Rect::new(0.0, 0.0, v.x, v.y),
            rgba(0.0, 0.0, 0.0, g.fade.min(1.0)),
        );
    }
    drawn
}

fn edge(v: Vec2, e: f32, col: Color) {
    fill(Rect::new(0.0, 0.0, v.x, e), col);
    fill(Rect::new(0.0, v.y - e, v.x, e), col);
    fill(Rect::new(0.0, e, e, v.y - 2.0 * e), col);
    fill(Rect::new(v.x - e, e, e, v.y - 2.0 * e), col);
}
