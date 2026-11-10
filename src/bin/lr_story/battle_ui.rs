//! The action-battle screen furniture: the boss bar, the small bar over a normal enemy, floating hit numbers, the
//! VICTORY / DEFEATED banners; plus the flat floor telegraphs and the
//! projectile spheres that are drawn inside the 3D camera, and the battle keys (left mouse attacks, right mouse held
//! blocks; the number keys, the wheel and F belong to the hotbar and are read in main.rs).
//!
//! There is no battle menu: the fight is real time and runs in rpg.rs (`ActionBattle`); `Game` in play.rs owns it. This
//! file only reads that state, sends the player's presses back, and draws. The rules that are not pixels (bar
//! fractions, phase marks, shake, hit number fade, telegraph geometry, which verdict to announce) are plain functions
//! and tested.

use crate::hud::{self, hp_frac, scale_for};
use crate::play::Game;
use crate::rpg::{ActionBattle, EnemyState, Projectile, Telegraph};
use crate::model::AttackKind;
use crate::ui;
use macroquad::prelude::*;

// ---- rules that are not pixels ----

/// The HP-percent marks of a boss bar as fractions (0..1): only marks strictly between 0 and 100, sorted, no repeats.
pub fn phase_marks(phases: &[i32]) -> Vec<f32> {
    let mut marks: Vec<i32> = phases.iter().copied().filter(|p| (1..100).contains(p)).collect();
    marks.sort_unstable();
    marks.dedup();
    marks.into_iter().map(|p| p as f32 / 100.0).collect()
}

/// The two stretches of an enemy bar as fractions of its width: the real fill, and the pale "recent damage" trail
/// behind it. The trail itself eases in rpg.rs (`trail_hp`); it is never drawn shorter than the fill.
pub fn bar_segments(hp: i32, trail_hp: f32, max_hp: i32) -> (f32, f32) {
    let fill = hp_frac(hp, max_hp);
    let trail = if max_hp <= 0 { 0.0 } else { (trail_hp / max_hp as f32).clamp(0.0, 1.0) };
    (fill, trail.max(fill))
}

/// Whether the small bar over a normal enemy shows: it is hidden while the enemy is unhurt or dead.
pub fn small_bar_visible(hp: i32, max_hp: i32, dead: bool) -> bool {
    !dead && hp > 0 && hp < max_hp
}

/// The screen offset in pixels of a shake of strength `shake` (the state's value, 0 when still) at time `t`.
pub fn shake_offset(shake: f32, t: f32) -> (f32, f32) {
    if shake <= 0.0 {
        return (0.0, 0.0);
    }
    let a = shake.min(1.0) * 10.0;
    ((t * 83.0).sin() * a, (t * 71.0 + 1.3).cos() * a * 0.6)
}

/// The camera offset in metres (sideways, up) of a shake: 5 cm at full strength.
pub fn camera_shake(shake: f32, t: f32) -> (f32, f32) {
    let (x, y) = shake_offset(shake, t);
    (x * 0.005, y * 0.005)
}

/// Seconds a floating number lives.
pub const HIT_SECS: f32 = 1.1;

/// How far a floating number has risen after `age` seconds, in the 0..1 range of its travel.
pub fn hit_rise(age: f32) -> f32 {
    let t = (age / HIT_SECS).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(2)
}

/// A floating number's opacity: solid, then gone by `HIT_SECS`.
pub fn hit_alpha(age: f32) -> f32 {
    if age <= 0.6 {
        1.0
    } else {
        (1.0 - (age - 0.6) / (HIT_SECS - 0.6)).clamp(0.0, 1.0)
    }
}

/// The share of the player's swing cooldown left, 0 (ready) to 1 (just swung).
pub fn swing_frac(left: f32) -> f32 {
    (left / 0.45).clamp(0.0, 1.0)
}

/// What the end banner says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Won,
    Lost,
}

/// Seconds the banner shows, and the part of it spent fading in and out.
const BANNER_END: f32 = 3.2;
const BANNER_FADE: f32 = 0.5;

/// The banner's opacity `age` seconds after the fight ended.
pub fn banner_alpha(age: f32) -> f32 {
    if !(0.0..BANNER_END).contains(&age) {
        0.0
    } else {
        (age / BANNER_FADE).min((BANNER_END - age) / BANNER_FADE).clamp(0.0, 1.0)
    }
}

/// The flag a won battle sets, and the one a lost battle sets (play.rs).
pub fn won_flag(id: &str) -> String {
    format!("won_{id}")
}

pub fn lost_flag(id: &str) -> String {
    format!("lost_{id}")
}

/// Which verdict to announce when the battle has already gone from the game: the flag that is set now and was not set
/// when the fight began. None when neither is new (the fight was left some other way).
pub fn infer_verdict(had_won: bool, had_lost: bool, won_now: bool, lost_now: bool) -> Option<Verdict> {
    if won_now && !had_won {
        Some(Verdict::Won)
    } else if lost_now && !had_lost {
        Some(Verdict::Lost)
    } else {
        None
    }
}

// ---- telegraph geometry (metres on the floor, as (x, z)) ----

/// A unit direction, or straight ahead (-z) when `dir` is zero or not a number.
pub fn unit_dir(dir: (f32, f32)) -> (f32, f32) {
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    if len > 1e-4 && len.is_finite() {
        (dir.0 / len, dir.1 / len)
    } else {
        (0.0, -1.0)
    }
}

/// How full a telegraph is, 0 to 1 (`t` seconds in, `total` seconds long).
pub fn telegraph_fill(t: f32, total: f32) -> f32 {
    if total <= 0.0 {
        1.0
    } else {
        (t / total).clamp(0.0, 1.0)
    }
}

/// `n + 1` points on an arc of `radius` round `centre`, `sweep` radians wide and centred on `dir`.
pub fn arc_points(centre: (f32, f32), dir: (f32, f32), radius: f32, sweep: f32, n: usize) -> Vec<(f32, f32)> {
    let d = unit_dir(dir);
    let base = d.1.atan2(d.0);
    let n = n.max(1);
    (0..=n)
        .map(|i| {
            let a = base - sweep / 2.0 + sweep * i as f32 / n as f32;
            (centre.0 + a.cos() * radius, centre.1 + a.sin() * radius)
        })
        .collect()
}

/// The four corners of a lane from `centre` along `dir` for `length` metres, `half_w` to each side, in drawing order.
pub fn lane_quad(centre: (f32, f32), dir: (f32, f32), length: f32, half_w: f32) -> [(f32, f32); 4] {
    let d = unit_dir(dir);
    let side = (-d.1, d.0);
    let end = (centre.0 + d.0 * length, centre.1 + d.1 * length);
    [
        (centre.0 + side.0 * half_w, centre.1 + side.1 * half_w),
        (end.0 + side.0 * half_w, end.1 + side.1 * half_w),
        (end.0 - side.0 * half_w, end.1 - side.1 * half_w),
        (centre.0 - side.0 * half_w, centre.1 - side.1 * half_w),
    ]
}

/// The width of an enemy melee cone, radians: the same arc the hit test uses (`rpg::ENEMY_ARC_DEG`).
pub const MELEE_SWEEP: f32 = crate::rpg::ENEMY_ARC_DEG * std::f32::consts::PI / 180.0;

// ---- the battle keys ----

/// Ends the block (the pause menu and the screens call this so a held right button cannot stick).
pub fn release(g: &mut Game) {
    if g.battle.is_some() {
        g.set_block(false);
    }
}

// ---- the battle's own clock and banner ----

/// The battle UI's state: its clock (for the shake), which battle it is following, and the end banner.
pub struct BattleUi {
    id: String,
    time: f32,
    /// A fight is in progress (so its end is still to be announced).
    active: bool,
    had_won: bool,
    had_lost: bool,
    announced: bool,
    banner: Option<(Verdict, f32)>,
}

impl Default for BattleUi {
    fn default() -> BattleUi {
        BattleUi::new()
    }
}

impl BattleUi {
    pub fn new() -> BattleUi {
        BattleUi { id: String::new(), time: 0.0, active: false, had_won: false, had_lost: false, announced: false, banner: None }
    }

    /// Moves the clocks on and notices how a fight ends (from its outcome, or from the flag it set when it is gone).
    pub fn update(&mut self, g: &Game, dt: f32) {
        self.time += dt;
        if let Some((_, age)) = self.banner.as_mut() {
            *age += dt;
            if *age >= BANNER_END {
                self.banner = None;
            }
        }
        match g.battle.as_ref() {
            Some(st) => {
                if !self.active || self.id != st.battle {
                    self.active = true;
                    self.id = st.battle.clone();
                    self.had_won = g.flags.contains(&won_flag(&self.id));
                    self.had_lost = g.flags.contains(&lost_flag(&self.id));
                    self.announced = false;
                    self.banner = None;
                }
                if !self.announced {
                    let verdict = match &st.outcome {
                        Some(crate::rpg::Outcome::Won) => Some(Verdict::Won),
                        Some(crate::rpg::Outcome::Lost) => Some(Verdict::Lost),
                        None => None,
                    };
                    if let Some(v) = verdict {
                        self.banner = Some((v, 0.0));
                        self.announced = true;
                    }
                }
            }
            None => {
                if self.active {
                    self.active = false;
                    if !self.announced {
                        let v = infer_verdict(
                            self.had_won,
                            self.had_lost,
                            g.flags.contains(&won_flag(&self.id)),
                            g.flags.contains(&lost_flag(&self.id)),
                        );
                        if let Some(v) = v {
                            self.banner = Some((v, 0.0));
                        }
                    }
                    self.announced = true;
                }
            }
        }
    }

    /// The battle's presses: left mouse attacks, the right button held blocks. Items are the hotbar's (1-9 select, F use,
    /// read in main.rs). Call it only while the world is live (no pause, no open screen, no talk).
    pub fn handle(&mut self, g: &mut Game) {
        if g.battle.is_some() {
            if is_mouse_button_pressed(MouseButton::Left) {
                g.player_attack();
            }
            g.set_block(is_mouse_button_down(MouseButton::Right));
        }
    }

    /// The shake as a camera offset in metres (sideways, up), for main.rs.
    pub fn camera_shake_for(&self, g: &Game) -> (f32, f32) {
        g.battle.as_ref().map_or((0.0, 0.0), |b| camera_shake(b.shake, self.time))
    }

    /// Draws the battle over the scene. `proj` turns a world point into a screen point (None when it is off screen).
    pub fn draw(&self, g: &Game, proj: &dyn Fn(Vec3) -> Option<Vec2>) {
        let (sw, sh) = (screen_width(), screen_height());
        let k = scale_for(sh);
        if let Some(st) = g.battle.as_ref() {
            if let Some(def) = g.pack.battles.get(&st.battle) {
                let dead = matches!(st.state, EnemyState::Dead);
                if def.boss && !dead {
                    self.draw_boss_bar(st, &def.name, &phase_marks(&def.phases), def.max_hp, sw, k);
                } else if small_bar_visible(st.enemy_hp, def.max_hp, dead) {
                    if let Some(p) = proj(vec3(st.enemy_pos.0, 2.25, st.enemy_pos.1)) {
                        draw_small_bar(st, &def.name, def.max_hp, p, k);
                    }
                }
            }
            for h in &st.hits {
                self.draw_hit(h, proj, sw, sh, k);
            }
            if st.blocking {
                ui::text_centered("BLOCK", sw / 2.0, sh / 2.0 + 62.0 * k, 16.0 * k, ui::ACCENT);
            }
            let swing = swing_frac(st.player_swing);
            if swing > 0.0 {
                let (w, h) = (46.0 * k, 4.0 * k);
                let r = Rect::new(sw / 2.0 - w / 2.0, sh / 2.0 + 40.0 * k, w, h);
                draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.0, 0.0, 0.5));
                draw_rectangle(r.x, r.y, r.w * (1.0 - swing), r.h, ui::ACCENT);
            }
        }
        if let Some((v, age)) = self.banner {
            self.draw_banner(g, v, age, sw, sh, k);
        }
    }

    /// The big bar, top centre: the name, a red fill with the pale trail behind it, the phase marks and the numbers.
    fn draw_boss_bar(&self, st: &ActionBattle, name: &str, marks: &[f32], max_hp: i32, sw: f32, k: f32) {
        let w = (sw - 80.0).min(780.0 * k);
        let top = 18.0 * k;
        ui::title_centered(&name.to_uppercase(), sw / 2.0, top + 28.0 * k, 32.0 * k, ui::TEXT);
        let bar = Rect::new((sw - w) / 2.0, top + 40.0 * k, w, 22.0 * k);
        draw_trail_bar(bar, st, max_hp, ui::BAD, marks);
        let hp = format!("{} / {}", st.enemy_hp.max(0), max_hp);
        ui::text_shadow(&hp, bar.x + bar.w - ui::measure(&hp, 13.0 * k) - 8.0 * k, bar.y + bar.h - 6.0 * k, 13.0 * k, ui::TEXT);
        if st.phase > 0 {
            // the same number the floating "Phase N" word shows (rpg hurt_enemy)
            let p = format!("Phase {}", st.phase);
            ui::text_shadow(&p, bar.x + 8.0 * k, bar.y + bar.h - 6.0 * k, 13.0 * k, ui::ACCENT);
        }
    }

    /// One floating number. Enemy numbers rise from over the enemy's head; the player's own (damage taken, heals) rise
    /// from just under the crosshair, since the body is the camera.
    fn draw_hit(&self, h: &crate::rpg::Hit, proj: &dyn Fn(Vec3) -> Option<Vec2>, sw: f32, sh: f32, k: f32) {
        let rise = hit_rise(h.age);
        let (pos, size) = if h.on_enemy {
            match proj(vec3(h.pos.0, 2.0 + rise * 0.9, h.pos.1)) {
                Some(p) => (p, 38.0 * k),
                None => return,
            }
        } else {
            (vec2(sw / 2.0, sh * 0.66 - rise * 60.0 * k), 34.0 * k)
        };
        let col = if h.heal {
            ui::GOOD
        } else if h.on_enemy {
            ui::ACCENT
        } else {
            ui::BAD
        };
        let a = hit_alpha(h.age);
        ui::title_centered(&h.text, pos.x + 2.0, pos.y + 2.0, size, ui::fade(ui::SHADOW, a));
        ui::title_centered(&h.text, pos.x, pos.y, size, ui::fade(col, a));
    }

    fn draw_banner(&self, _g: &Game, v: Verdict, age: f32, sw: f32, sh: f32, k: f32) {
        let a = banner_alpha(age);
        if a <= 0.0 {
            return;
        }
        let (title, col) = match v {
            Verdict::Won => ("VICTORY", ui::GOOD),
            Verdict::Lost => ("DEFEATED", ui::BAD),
        };
        let y = sh * 0.34 - (1.0 - hud::ease_out(age / BANNER_FADE)) * 14.0;
        let size = 84.0 * k;
        ui::title_centered(title, sw / 2.0 + 3.0, y + 3.0, size, ui::fade(ui::SHADOW, a));
        ui::title_centered(title, sw / 2.0, y, size, ui::fade(col, a));
        let rule = 220.0 * k;
        draw_line(sw / 2.0 - rule / 2.0, y + 18.0 * k, sw / 2.0 + rule / 2.0, y + 18.0 * k, 2.0, ui::fade(col, a));
        let sub = match v {
            Verdict::Won => String::new(),
            Verdict::Lost => "You come to again.".to_string(),
        };
        if !sub.is_empty() {
            ui::text_centered(&sub, sw / 2.0, y + 52.0 * k, 22.0 * k, ui::fade(ui::TEXT, a));
        }
    }
}

/// A health bar with the pale recent-damage trail behind its fill, the hit flash and the phase marks over it.
fn draw_trail_bar(r: Rect, st: &ActionBattle, max_hp: i32, color: Color, marks: &[f32]) {
    let (fill, trail) = bar_segments(st.enemy_hp, st.trail_hp, max_hp);
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_rectangle(r.x, r.y, r.w * trail, r.h, Color::new(0.95, 0.9, 0.8, 0.85));
    let f = st.flash.clamp(0.0, 1.0);
    let c = Color::new(color.r + (1.0 - color.r) * f, color.g + (1.0 - color.g) * f, color.b + (1.0 - color.b) * f, 1.0);
    draw_rectangle(r.x, r.y, r.w * fill, r.h, c);
    for m in marks {
        let x = r.x + r.w * m;
        draw_line(x, r.y - 3.0, x, r.y + r.h + 3.0, 2.0, ui::fade(ui::TEXT, 0.9));
    }
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, ui::fade(ui::BORDER, 0.9));
}

/// The small bar and name tag over a normal enemy's head (`p` is where its head lands on screen).
fn draw_small_bar(st: &ActionBattle, name: &str, max_hp: i32, p: Vec2, k: f32) {
    let w = 96.0 * k;
    let bar = Rect::new(p.x - w / 2.0, p.y, w, 9.0 * k);
    ui::text_centered(name, p.x, p.y - 6.0 * k, 15.0 * k, ui::TEXT);
    draw_trail_bar(bar, st, max_hp, ui::BAD, &[]);
}

// ---- inside the 3D camera ----

/// Height of the telegraphs above the floor (metres), just over the floor blocks.
const FLAT_Y: f32 = 0.05;

fn flat_fan(centre: (f32, f32), pts: &[(f32, f32)], color: Color) {
    if pts.len() < 2 {
        return;
    }
    let mut vertices = vec![Vertex::new(centre.0, FLAT_Y, centre.1, 0.0, 0.0, color)];
    vertices.extend(pts.iter().map(|p| Vertex::new(p.0, FLAT_Y, p.1, 0.0, 0.0, color)));
    let mut indices: Vec<u16> = Vec::with_capacity((pts.len() - 1) * 3);
    for i in 0..pts.len() - 1 {
        indices.extend_from_slice(&[0, (i + 1) as u16, (i + 2) as u16]);
    }
    draw_mesh(&Mesh { vertices, indices, texture: None });
}

fn flat_quad(q: [(f32, f32); 4], color: Color) {
    let vertices: Vec<Vertex> = q.iter().map(|p| Vertex::new(p.0, FLAT_Y, p.1, 0.0, 0.0, color)).collect();
    draw_mesh(&Mesh { vertices, indices: vec![0, 1, 2, 0, 2, 3], texture: None });
}

fn outline(pts: &[(f32, f32)], color: Color) {
    for w in pts.windows(2) {
        draw_line_3d(vec3(w[0].0, FLAT_Y + 0.005, w[0].1), vec3(w[1].0, FLAT_Y + 0.005, w[1].1), color);
    }
}

/// One telegraph, flat on the floor in red: a faint full shape, a bright rim, and a fill that grows to the full shape as
/// the strike nears (a circle for a slam, a cone for a melee swing, a lane for a charge or a shot).
pub fn draw_telegraph(t: &Telegraph) {
    let f = telegraph_fill(t.t, t.total);
    let faint = Color::new(0.95, 0.15, 0.12, 0.22);
    let rim = Color::new(1.0, 0.35, 0.28, 0.95);
    let fill = Color::new(0.95, 0.12, 0.10, 0.30 + 0.30 * f);
    match t.kind {
        AttackKind::Slam => {
            let full = arc_points(t.centre, (1.0, 0.0), t.radius, std::f32::consts::TAU, 40);
            flat_fan(t.centre, &full, faint);
            outline(&full, rim);
            flat_fan(t.centre, &arc_points(t.centre, (1.0, 0.0), t.radius * f, std::f32::consts::TAU, 40), fill);
        }
        AttackKind::Melee => {
            let reach = if t.length > 0.0 { t.length } else { crate::rpg::MELEE_REACH };
            let full = arc_points(t.centre, t.dir, reach, MELEE_SWEEP, 16);
            flat_fan(t.centre, &full, faint);
            outline(&full, rim);
            flat_fan(t.centre, &arc_points(t.centre, t.dir, reach * f, MELEE_SWEEP, 16), fill);
        }
        AttackKind::Charge | AttackKind::Shot => {
            let half = t.radius.max(0.25);
            let len = t.length.max(0.5);
            let q = lane_quad(t.centre, t.dir, len, half);
            flat_quad(q, faint);
            outline(&[q[0], q[1], q[2], q[3], q[0]], rim);
            flat_quad(lane_quad(t.centre, t.dir, len * f, half), fill);
        }
    }
}

/// A projectile as a glowing sphere: a bright core and a faint larger halo.
pub fn draw_projectile(p: &Projectile) {
    let c = vec3(p.pos.0, 1.0, p.pos.1);
    draw_sphere(c, p.radius * 1.9, None, Color::new(1.0, 0.45, 0.15, 0.25));
    draw_sphere(c, p.radius, None, Color::new(1.0, 0.85, 0.4, 1.0));
}

/// Everything of the fight that lives on the floor and in the air: the telegraphs and the projectiles. Call it inside the
/// 3D camera, after the opaque scene.
pub fn draw_world(st: &ActionBattle) {
    for t in &st.telegraphs {
        draw_telegraph(t);
    }
    for p in &st.projectiles {
        draw_projectile(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_marks_are_sorted_fractions_inside_the_bar() {
        assert_eq!(phase_marks(&[33, 66]), vec![0.33, 0.66]);
        assert_eq!(phase_marks(&[66, 33, 33, 0, 100, -5, 150]), vec![0.33, 0.66]);
        assert!(phase_marks(&[]).is_empty());
    }

    #[test]
    fn the_trail_is_never_shorter_than_the_fill_and_stays_in_the_bar() {
        assert_eq!(bar_segments(30, 30.0, 30), (1.0, 1.0));
        let (fill, trail) = bar_segments(15, 24.0, 30);
        assert_eq!((fill, trail), (0.5, 0.8), "the pale stretch is the recent damage");
        let (fill, trail) = bar_segments(15, 3.0, 30);
        assert_eq!((fill, trail), (0.5, 0.5), "a trail below the fill is lifted to it");
        assert_eq!(bar_segments(-4, 0.0, 30), (0.0, 0.0));
        assert_eq!(bar_segments(10, 99.0, 30).1, 1.0, "clamped to the bar");
        assert_eq!(bar_segments(10, 10.0, 0), (0.0, 0.0), "no max, no bar");
    }

    #[test]
    fn the_small_bar_shows_only_for_a_hurt_living_enemy() {
        assert!(!small_bar_visible(30, 30, false));
        assert!(small_bar_visible(12, 30, false));
        assert!(!small_bar_visible(0, 30, false));
        assert!(!small_bar_visible(12, 30, true));
    }

    #[test]
    fn the_shake_is_still_at_zero_and_bounded() {
        assert_eq!(shake_offset(0.0, 3.0), (0.0, 0.0));
        assert_eq!(shake_offset(-1.0, 3.0), (0.0, 0.0));
        assert_eq!(camera_shake(0.0, 1.0), (0.0, 0.0));
        for i in 0..50 {
            let (x, y) = shake_offset(5.0, i as f32 * 0.13);
            assert!(x.abs() <= 10.0 && y.abs() <= 6.0);
            let (cx, cy) = camera_shake(5.0, i as f32 * 0.13);
            assert!(cx.abs() <= 0.05 + 1e-6 && cy.abs() <= 0.03 + 1e-6, "a few centimetres at most");
        }
    }

    #[test]
    fn floating_numbers_rise_and_fade_out() {
        assert_eq!(hit_rise(0.0), 0.0);
        assert_eq!(hit_rise(5.0), 1.0);
        assert!(hit_rise(0.3) > 0.3 && hit_rise(0.3) < 1.0);
        assert_eq!(hit_alpha(0.2), 1.0);
        assert!(hit_alpha(0.85) > 0.0 && hit_alpha(0.85) < 1.0);
        assert_eq!(hit_alpha(HIT_SECS), 0.0);
        assert_eq!(hit_alpha(3.0), 0.0);
        let mut last = 1.0;
        for i in 0..30 {
            let a = hit_alpha(i as f32 * 0.04);
            assert!(a <= last + 1e-6, "never gets brighter");
            last = a;
        }
    }

    #[test]
    fn the_swing_meter_runs_from_full_to_ready() {
        assert_eq!(swing_frac(0.0), 0.0);
        assert_eq!(swing_frac(0.45), 1.0);
        assert_eq!(swing_frac(9.0), 1.0);
        assert!(swing_frac(0.2) > 0.0 && swing_frac(0.2) < 1.0);
    }

    #[test]
    fn the_banner_fades_in_holds_and_out() {
        assert_eq!(banner_alpha(-1.0), 0.0);
        assert_eq!(banner_alpha(0.0), 0.0);
        assert!(banner_alpha(0.25) > 0.0 && banner_alpha(0.25) < 1.0);
        assert_eq!(banner_alpha(1.5), 1.0);
        assert!(banner_alpha(BANNER_END - 0.2) < 1.0);
        assert_eq!(banner_alpha(BANNER_END), 0.0);
    }

    #[test]
    fn a_gone_battle_is_announced_from_the_flag_it_newly_set() {
        assert_eq!(infer_verdict(false, false, true, false), Some(Verdict::Won));
        assert_eq!(infer_verdict(false, false, false, true), Some(Verdict::Lost));
        assert_eq!(infer_verdict(false, true, false, true), None, "an old defeat is not news");
        assert_eq!(infer_verdict(true, false, true, false), None);
        assert_eq!(infer_verdict(false, true, true, true), Some(Verdict::Won), "won now, lost before");
        assert_eq!(infer_verdict(false, false, false, false), None);
        assert_eq!(won_flag("boss"), "won_boss");
        assert_eq!(lost_flag("boss"), "lost_boss");
    }

    #[test]
    fn directions_are_unit_length_with_a_forward_default() {
        let d = unit_dir((3.0, 4.0));
        assert!((d.0 - 0.6).abs() < 1e-6 && (d.1 - 0.8).abs() < 1e-6);
        assert_eq!(unit_dir((0.0, 0.0)), (0.0, -1.0));
        assert_eq!(unit_dir((f32::NAN, 1.0)), (0.0, -1.0));
    }

    #[test]
    fn the_telegraph_fills_as_the_strike_nears() {
        assert_eq!(telegraph_fill(0.0, 1.0), 0.0);
        assert_eq!(telegraph_fill(0.5, 1.0), 0.5);
        assert_eq!(telegraph_fill(2.0, 1.0), 1.0);
        assert_eq!(telegraph_fill(0.3, 0.0), 1.0, "no windup is already full");
    }

    #[test]
    fn an_arc_stays_on_its_radius_and_spans_its_sweep() {
        let pts = arc_points((2.0, 3.0), (0.0, -1.0), 1.6, MELEE_SWEEP, 8);
        assert_eq!(pts.len(), 9);
        for p in &pts {
            let r = ((p.0 - 2.0).powi(2) + (p.1 - 3.0).powi(2)).sqrt();
            assert!((r - 1.6).abs() < 1e-4);
        }
        // the middle of the arc is straight ahead (-z)
        let mid = pts[4];
        assert!((mid.0 - 2.0).abs() < 1e-4 && (mid.1 - (3.0 - 1.6)).abs() < 1e-4);
        // a full circle closes on itself
        let ring = arc_points((0.0, 0.0), (1.0, 0.0), 2.0, std::f32::consts::TAU, 12);
        assert!((ring[0].0 - ring[12].0).abs() < 1e-4 && (ring[0].1 - ring[12].1).abs() < 1e-4);
    }

    #[test]
    fn a_lane_runs_from_the_centre_along_its_direction() {
        let q = lane_quad((1.0, 1.0), (1.0, 0.0), 6.0, 0.5);
        let xs: Vec<f32> = q.iter().map(|p| p.0).collect();
        let zs: Vec<f32> = q.iter().map(|p| p.1).collect();
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) >= 1.0 - 1e-5);
        assert!((xs.iter().cloned().fold(f32::MIN, f32::max) - 7.0).abs() < 1e-5);
        assert!((zs.iter().cloned().fold(f32::MIN, f32::max) - 1.5).abs() < 1e-5);
        assert!((zs.iter().cloned().fold(f32::MAX, f32::min) - 0.5).abs() < 1e-5);
    }
}
