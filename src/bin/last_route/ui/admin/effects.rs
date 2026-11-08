//! The admin switches' effect on a running route, applied around `Game::update` from outside
//! the game code: GOD MODE and NO TARGET cancel a capture (and NO TARGET also drops the hunters'
//! interest in the runner), NOCLIP moves the runner through walls, RUN SPEED scales the step the
//! body took, MAP REVEAL uncovers the loaded floor, and a teleport outside the loaded window
//! lands once the window has streamed there. Then the overlays: HUNTER REVEAL markers, SHOW
//! TRAPS (Ravelle's wires), SHOW COLLISION (wall cells round the runner), the REGION MAP chip,
//! the FPS / SPIKE overlay. (The ADMIN ACTIVE badge was removed on 2026-11-07.)

use super::actions::safe_landing;
use super::look::*;
use super::Admin;
use crate::app::assets::Assets;
use crate::config::*;
use crate::content::Content;
use crate::game::{pdist, Controls, Game};
use crate::hunters::brain::State;
use crate::hunters::gimmicks::Gimmick;
use crate::world::infinite::CHUNK;
use macroquad::prelude::*;

/// Where a world point lands on screen with the run's camera (render/scene.rs `camera`).
fn project(g: &Game, p: Vec3) -> Option<Vec2> {
    let (fwd, right, up) = g.camera_basis();
    let v = p - g.eye();
    let z = v.dot(fwd);
    if z < 0.05 {
        return None;
    }
    let (sw, sh) = (screen_width(), screen_height());
    let t = (g.fov_deg.to_radians() / 2.0).tan();
    let x = v.dot(right) / (z * t * (sw / sh));
    let y = v.dot(up) / (z * t);
    Some(vec2((x + 1.0) / 2.0 * sw, (1.0 - y) / 2.0 * sh))
}

/// A world segment drawn on screen, clipped at the near plane.
fn line3(g: &Game, a: Vec3, b: Vec3, w: f32, col: Color) {
    let (fwd, _, _) = g.camera_basis();
    let eye = g.eye();
    let (za, zb) = ((a - eye).dot(fwd), (b - eye).dot(fwd));
    let near = 0.06;
    if za < near && zb < near {
        return;
    }
    let (mut a, mut b) = (a, b);
    if za < near {
        a = a + (b - a) * ((near - za) / (zb - za));
    } else if zb < near {
        b = b + (a - b) * ((near - zb) / (za - zb));
    }
    if let (Some(p), Some(q)) = (project(g, a), project(g, b)) {
        draw_line(p.x, p.y, q.x, q.y, w, col);
    }
}

impl Admin {
    /// NO TARGET: the runner is not anyone's target (admin_no_target), so hunters drop a chase,
    /// an investigation or a search before they think this frame.
    pub(super) fn no_target_before(&mut self, g: &mut Game) {
        if !self.no_target {
            return;
        }
        for h in g.hunters.iter_mut() {
            if matches!(h.state, State::Chase | State::Investigate | State::Search | State::Peek) {
                h.state = State::Wander;
                h.last_known = None;
                h.wander_target = None;
                h.direct_visible = false;
                h.clear_path();
            }
        }
    }

    pub(super) fn apply_after(&mut self, g: &mut Game, c: &Controls, _content: &Content, dt: f32) {
        // GOD MODE / NO TARGET: a capture or a death this frame is undone; the hunter that did it
        // is calmed (calm_reset) so it does not grab again at once.
        if self.god_mode || self.no_target {
            if let Some(e) = g.escape.as_ref() {
                if !self.pre.escape {
                    let i = e.hunter;
                    g.escape = None;
                    g.p.flashlight_on = self.pre.flashlight;
                    if let Some(h) = g.hunters.get_mut(i) {
                        h.state = State::Idle;
                        h.last_known = None;
                        h.clear_path();
                        h.direct_visible = false;
                        h.apply_stun(2.0, false);
                    }
                }
            }
            if g.report.is_some() && !self.pre.report {
                g.report = None;
                g.p.flashlight_on = self.pre.flashlight;
                // end_run cleared the saved run; an Infinite route puts its checkpoint back
                if g.is_infinite() {
                    g.save_checkpoint();
                }
                let at = g.p.pos;
                for h in g.hunters.iter_mut().filter(|h| pdist(h.pos, at) < 3.0) {
                    h.state = State::Idle;
                    h.last_known = None;
                    h.clear_path();
                    h.apply_stun(2.0, false);
                }
            }
        }

        // the window recentred this frame when the runner jumped by whole chunks
        let moved = (g.p.pos.0 - self.pre.pos.0, g.p.pos.1 - self.pre.pos.1);
        let c32 = CHUNK as f32;
        let shift = if g.is_infinite() && (moved.0.abs() > c32 * 0.5 || moved.1.abs() > c32 * 0.5) {
            ((moved.0 / c32).round() * c32, (moved.1 / c32).round() * c32)
        } else {
            (0.0, 0.0)
        };

        if self.pending_landing {
            let inside = g.p.pos.0 >= 0.0
                && g.p.pos.1 >= 0.0
                && g.p.pos.0 < g.fac.cols as f32
                && g.p.pos.1 < g.fac.rows as f32;
            if inside {
                self.pending_landing = false;
                if let Some(p) = safe_landing(g, g.p.pos.0, g.p.pos.1) {
                    g.p.pos = p;
                } else {
                    g.say("ADMIN // NO SAFE CELL NEAR THERE");
                }
            }
            return;
        }

        let free =
            g.escape.is_none() && g.p.hidden.is_none() && g.report.is_none() && g.p.stun <= 0.0;
        if self.noclip && free {
            // straight through walls, at walk / sprint speed times RUN SPEED
            let input = vec2(c.strafe, c.fwd);
            let input = if input.length() > 1.0 {
                input.normalize()
            } else {
                input
            };
            let speed = if c.sprint {
                PLAYER_SPRINT_SPEED
            } else {
                PLAYER_WALK_SPEED
            } * self.run_speed;
            let (fx, fy) = g.forward2();
            let (rx, ry) = (g.p.yaw.cos(), -g.p.yaw.sin());
            let d = (
                (rx * input.x + fx * input.y) * speed * dt,
                (ry * input.x + fy * input.y) * speed * dt,
            );
            let mut p = (
                self.pre.pos.0 - shift.0 + d.0,
                self.pre.pos.1 - shift.1 + d.1,
            );
            p.0 = p.0.clamp(0.3, g.fac.cols as f32 - 0.3);
            p.1 = p.1.clamp(0.3, g.fac.rows as f32 - 0.3);
            g.p.pos = p;
        } else if self.run_speed != 1.0 && free && shift == (0.0, 0.0) {
            // admin_speed_scale: the extra part of this frame's step, with the body's collision
            let extra = (
                moved.0 * (self.run_speed - 1.0),
                moved.1 * (self.run_speed - 1.0),
            );
            let steps = ((extra.0.hypot(extra.1) / 0.08).ceil() as i32).max(1);
            for _ in 0..steps {
                let nx = g.p.pos.0 + extra.0 / steps as f32;
                if g.fac.body_fits(nx, g.p.pos.1, PLAYER_RADIUS) {
                    g.p.pos.0 = nx;
                }
                let ny = g.p.pos.1 + extra.1 / steps as f32;
                if g.fac.body_fits(g.p.pos.0, ny, PLAYER_RADIUS) {
                    g.p.pos.1 = ny;
                }
            }
        }

        if self.map_reveal {
            if self.map_reveal_undo.is_none() {
                self.map_reveal_undo = Some(g.map_reveal_snapshot());
            }
            g.reveal_whole_window();
        } else if let Some(u) = self.map_reveal_undo.take() {
            g.map_reveal_restore(u);
        }
    }

    pub(super) fn draw_overlays(&mut self, g: &Game, a: &Assets) {
        let u = px_per_unit();
        let ui = Ui::new(a, u, false);
        if self.show_collision {
            let (px, py) = (g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
            let col = Color::new(TEAL.r, TEAL.g, TEAL.b, 0.55);
            for y in py - 6..=py + 6 {
                for x in px - 6..=px + 6 {
                    if !g.fac.is_wall_cell(x, y) {
                        continue;
                    }
                    let (x0, y0, x1, y1) = (x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0);
                    for (ax, ay, bx, by) in [
                        (x0, y0, x1, y0),
                        (x1, y0, x1, y1),
                        (x1, y1, x0, y1),
                        (x0, y1, x0, y0),
                    ] {
                        line3(g, vec3(ax, 0.02, ay), vec3(bx, 0.02, by), 1.5, col);
                    }
                }
            }
            // the runner's capsule footprint
            let n = 20;
            for i in 0..n {
                let (t0, t1) = (
                    i as f32 / n as f32 * std::f32::consts::TAU,
                    (i + 1) as f32 / n as f32 * std::f32::consts::TAU,
                );
                let r = PLAYER_RADIUS;
                line3(
                    g,
                    vec3(g.p.pos.0 + t0.cos() * r, 0.02, g.p.pos.1 + t0.sin() * r),
                    vec3(g.p.pos.0 + t1.cos() * r, 0.02, g.p.pos.1 + t1.sin() * r),
                    2.0,
                    STABLE,
                );
            }
        }
        if self.show_traps {
            for h in g.hunters.iter() {
                if let Gimmick::Ravelle(r) = &h.gimmick {
                    for w in r.wires.iter().filter(|w| !w.triggered) {
                        line3(
                            g,
                            vec3(w.a.0, 0.35, w.a.1),
                            vec3(w.b.0, 0.35, w.b.1),
                            2.5,
                            AMBER,
                        );
                        for end in [w.a, w.b] {
                            if let Some(p) = project(g, vec3(end.0, 0.35, end.1)) {
                                draw_circle(p.x, p.y, 1.5 * u, ACCENT_EDGE);
                            }
                        }
                    }
                }
            }
        }
        // Hunter identities are an admin inspection aid, never part of the player view. Keep
        // the reveal available while the admin panel is open, but hide names as soon as the
        // player returns to the route.
        if self.hunter_reveal && self.open {
            for h in g.hunters.iter() {
                let Some(p) = project(g, vec3(h.pos.0, 1.9, h.pos.1)) else {
                    continue;
                };
                let chasing = h.active && h.state == State::Chase;
                let col = if h.stun > 999.0 {
                    MUTED
                } else if chasing {
                    ALERT
                } else if h.active {
                    AMBER
                } else {
                    BODY
                };
                let s = 3.0 * u;
                draw_triangle(
                    vec2(p.x, p.y + s),
                    vec2(p.x - s * 0.7, p.y - s * 0.3),
                    vec2(p.x + s * 0.7, p.y - s * 0.3),
                    col,
                );
                let label = format!(
                    "{} {}M",
                    h.def.name.to_uppercase(),
                    pdist(h.pos, g.p.pos) as i32
                );
                let w = ui.tw(&label, 6.0);
                ui.text(
                    &label,
                    p.x - w / 2.0 + 1.0,
                    p.y - 10.0 * u + 1.0,
                    6.0,
                    Color::new(0.0, 0.0, 0.0, 0.8),
                );
                ui.text(&label, p.x - w / 2.0, p.y - 10.0 * u, 6.0, col);
            }
        }
        if self.region_map {
            let region = g
                .fac
                .region_at(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32)
                .to_uppercase();
            let label = format!("REGION // {region}");
            let w = ui.tw(&label, 6.0) + 8.0 * u;
            let r = Rect::new((screen_width() - w) / 2.0, 23.0 * u, w, 9.0 * u);
            ui.boxed(r, PANEL, PANEL_EDGE);
            ui.text_center(&label, r, 6.0, AMBER);
        }
        if self.perf_overlay && !self.frame_ms.is_empty() {
            let n = self.frame_ms.len() as f32;
            let avg = self.frame_ms.iter().sum::<f32>() / n;
            let spike = self.frame_ms.iter().cloned().fold(0.0, f32::max);
            let r = Rect::new(6.0 * u, 18.0 * u, 96.0 * u, 34.0 * u);
            ui.boxed(r, alpha(PANEL, 0.88), PANEL_EDGE);
            ui.text(
                &format!("FPS {:.0}  {:.1} MS", 1000.0 / avg.max(0.01), avg),
                r.x + 3.0 * u,
                r.y + 2.0 * u,
                6.0,
                INK,
            );
            ui.text(
                &format!("SPIKE {:.1} MS", spike),
                r.x + 3.0 * u,
                r.y + 10.0 * u,
                6.0,
                if spike > 50.0 { ALERT } else { BODY },
            );
            let base = r.y + r.h - 2.0 * u;
            let bw = (r.w - 6.0 * u) / 240.0;
            for (i, ms) in self.frame_ms.iter().enumerate() {
                let h = (ms / 50.0).min(1.0) * 14.0 * u;
                let col = if *ms > 33.4 {
                    ALERT
                } else if *ms > 17.5 {
                    STAMINA_WARN
                } else {
                    STABLE
                };
                draw_rectangle(r.x + 3.0 * u + i as f32 * bw, base - h, bw.max(1.0), h, col);
            }
        }
    }

}

const STAMINA_WARN: Color = Color::new(0.949, 0.733, 0.384, 1.0);
