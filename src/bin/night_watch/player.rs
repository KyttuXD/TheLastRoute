#![allow(dead_code)]
//! First-person walker, aim ray and station picker (set C8).
//!
//! Ports the subset of Godot `player_controller_3d.gd` that Night Watch uses
//! (walk / sprint / crouch pace, mouse look, capsule radius 0.24, the
//! interactable picker with its aim cone and line-of-sight slack) and the
//! `night_watch_player.tscn` numbers (fov 78, near 0.06, flashlight spot).
//!
//! Pure logic: no macroquad drawing, input or window calls. Movement and look
//! come from the per-frame `InputFrame`.
//!
//! Conventions: `forward = (-sin yaw * cos pitch, sin pitch, -cos yaw * cos pitch)`,
//! so yaw 0 looks down -Z; a positive mouse x delta turns right (yaw falls);
//! a positive mouse y delta looks down (pitch falls).
//!
//! Movement actions are read from `InputFrame` by name: `move_forward`,
//! `move_back`, `move_left`, `move_right`, `sprint`, `crouch`, and the keyboard
//! look actions `look_left`, `look_right`, `look_up`, `look_down`. The
//! integrator maps keys to these names (see the hook list).

use crate::core::{Bindings, InputFrame};
use crate::gfx::{Aabb, FpCamera, Spot, StationDef};
use macroquad::prelude::{vec3, Vec3};

/// Capsule radius of the player body (m).
pub const RADIUS: f32 = 0.24;
/// Standing and crouched capsule heights (m).
pub const STAND_HEIGHT: f32 = 1.7;
pub const CROUCH_HEIGHT: f32 = 1.08;
/// Eye height when standing (contract: 1.707).
pub const EYE_HEIGHT: f32 = 1.707;
/// Crouched eye: ratio 0.69, never above the short capsule minus 0.08.
pub const CROUCH_EYE_HEIGHT: f32 = {
    let a = EYE_HEIGHT * 0.69;
    let b = CROUCH_HEIGHT - 0.08;
    if a < b {
        a
    } else {
        b
    }
};
pub const WALK_SPEED: f32 = 4.0;
pub const SPRINT_SPEED: f32 = 6.25;
/// Godot crouch speed 36 against walk 64.
pub const CROUCH_SPEED: f32 = WALK_SPEED * 36.0 / 64.0;
/// Pitch limit (rad).
pub const PITCH_LIMIT: f32 = 1.4;
/// Keyboard look rate (rad/s at the default sensitivity).
pub const KEY_LOOK_RATE: f32 = 2.8;
pub const DEFAULT_SENS: f32 = 0.0075;
/// Aim range (m), half angle (rad) and line-of-sight slack (m).
pub const AIM_RANGE: f32 = 4.5;
pub const AIM_HALF_ANGLE: f32 = 0.42;
pub const AIM_RAY_SLACK: f32 = 0.06;
/// Obstacles lower than this above the feet are stepped over.
const STEP_HEIGHT: f32 = 0.3;
/// Longest distance moved in one collision sub-step (m).
const MAX_SUBSTEP: f32 = 0.1;
/// Flashlight (night_watch_player.tscn): offset from the eye, energy, range, half angle.
const FLASH_OFFSET: (f32, f32, f32) = (0.08, -0.05, -0.12);
const FLASH_ENERGY: f32 = 7.5;
const FLASH_RANGE: f32 = 16.0;
const FLASH_HALF_ANGLE_DEG: f32 = 30.0;

pub struct Player {
    pub cam: FpCamera,
    /// No walking (looking still works unless `look_frozen`).
    pub frozen: bool,
    /// No mouse or key look.
    pub look_frozen: bool,
    /// Ignore solids and the footprint.
    pub noclip: bool,
    /// Forced crouch (vent crawl); the `crouch` input action also crouches.
    pub crouch: bool,
    /// Flashlight on (modes read it and call `flashlight_spot`).
    pub flashlight: bool,
    /// Walkable bounds `(min_x, min_z, max_x, max_z)` for the body centre's
    /// capsule edge; `None` means unbounded.
    pub footprint: Option<(f32, f32, f32, f32)>,
    /// Feet position (floor level under the body).
    pub feet: Vec3,
    /// Real ground speed of the last update (m/s), for footstep pacing.
    pub speed: f32,
    /// True while the last update moved the body.
    pub moving: bool,
    /// True while the last update sprinted.
    pub sprinting: bool,
    /// 0 standing .. 1 crouched, eased.
    crouch_t: f32,
}

fn forward_of(yaw: f32, pitch: f32) -> Vec3 {
    vec3(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

/// Circle (radius r at x,z) against a box that matters at the given vertical span.
fn blocked_by(solids: &[Aabb], x: f32, z: f32, r: f32, y_feet: f32, y_head: f32) -> bool {
    for a in solids {
        if a.max.y <= y_feet + STEP_HEIGHT || a.min.y >= y_head {
            continue;
        }
        let cx = x.clamp(a.min.x, a.max.x);
        let cz = z.clamp(a.min.z, a.max.z);
        let dx = x - cx;
        let dz = z - cz;
        if dx * dx + dz * dz < r * r {
            return true;
        }
    }
    false
}

impl Player {
    pub fn new(spawn: Vec3) -> Player {
        let mut p = Player {
            cam: FpCamera { pos: spawn, yaw: 0.0, pitch: 0.0, fov_deg: 78.0, near: 0.06, far: 100.0 },
            frozen: false,
            look_frozen: false,
            noclip: false,
            crouch: false,
            flashlight: false,
            footprint: None,
            feet: spawn,
            speed: 0.0,
            moving: false,
            sprinting: false,
            crouch_t: 0.0,
        };
        p.sync_camera();
        p
    }

    fn eye_height(&self) -> f32 {
        EYE_HEIGHT + (CROUCH_EYE_HEIGHT - EYE_HEIGHT) * self.crouch_t
    }

    fn sync_camera(&mut self) {
        self.cam.pos = vec3(self.feet.x, self.feet.y + self.eye_height(), self.feet.z);
    }

    /// Eye position (y 1.707 above the feet while standing).
    pub fn eye(&self) -> Vec3 {
        vec3(self.feet.x, self.feet.y + self.eye_height(), self.feet.z)
    }

    pub fn forward(&self) -> Vec3 {
        forward_of(self.cam.yaw, self.cam.pitch)
    }

    /// Unit vector to the right of the view on the floor plane.
    pub fn right(&self) -> Vec3 {
        vec3(self.cam.yaw.cos(), 0.0, -self.cam.yaw.sin())
    }

    pub fn is_crouching(&self) -> bool {
        self.crouch_t > 0.5
    }

    /// Move the body to `p` (feet) and face `yaw` / `pitch`.
    pub fn teleport(&mut self, p: Vec3, yaw: f32, pitch: f32) {
        self.feet = p;
        self.cam.yaw = yaw;
        self.cam.pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.speed = 0.0;
        self.moving = false;
        self.sprinting = false;
        self.sync_camera();
    }

    /// Flashlight spot light (Godot: SpotLight3D at (0.08,-0.05,-0.12) from the
    /// eye pivot, energy 7.5, range 16, half angle 30 degrees). `None` when off.
    pub fn flashlight_spot(&self) -> Option<Spot> {
        if !self.flashlight {
            return None;
        }
        let f = self.forward();
        let r = self.right();
        let up = r.cross(f).normalize_or_zero();
        let up = if up.length_squared() < 0.5 { vec3(0.0, 1.0, 0.0) } else { up };
        let pos = self.eye() + r * FLASH_OFFSET.0 + up * FLASH_OFFSET.1 - f * FLASH_OFFSET.2;
        Some(Spot {
            pos,
            dir: f,
            color: [1.0, 1.0, 1.0],
            energy: FLASH_ENERGY,
            range: FLASH_RANGE,
            cos_outer: FLASH_HALF_ANGLE_DEG.to_radians().cos(),
        })
    }

    fn apply_look(&mut self, input: &InputFrame, sens: f32, dt: f32) {
        if self.look_frozen {
            return;
        }
        self.cam.yaw -= input.mouse_delta.0 * sens;
        self.cam.pitch = (self.cam.pitch - input.mouse_delta.1 * sens).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        let lx = (input.is_down("look_right") as i32 - input.is_down("look_left") as i32) as f32;
        let ly = (input.is_down("look_up") as i32 - input.is_down("look_down") as i32) as f32;
        if lx != 0.0 || ly != 0.0 {
            let scale = if sens > 0.0 { sens / DEFAULT_SENS } else { 1.0 };
            self.cam.yaw -= lx * KEY_LOOK_RATE * scale * dt;
            self.cam.pitch = (self.cam.pitch + ly * KEY_LOOK_RATE * scale * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
    }

    fn clamp_footprint(&mut self) {
        if let Some((x0, z0, x1, z1)) = self.footprint {
            if x1 - x0 > 2.0 * RADIUS {
                self.feet.x = self.feet.x.clamp(x0 + RADIUS, x1 - RADIUS);
            }
            if z1 - z0 > 2.0 * RADIUS {
                self.feet.z = self.feet.z.clamp(z0 + RADIUS, z1 - RADIUS);
            }
        }
    }

    /// Slide the body by (dx, dz), one axis at a time, in short sub-steps so a
    /// fast frame cannot tunnel through a thin wall.
    fn move_body(&mut self, dx: f32, dz: f32, solids: &[Aabb]) {
        let len = (dx * dx + dz * dz).sqrt();
        if len <= 0.0 {
            return;
        }
        if self.noclip {
            self.feet.x += dx;
            self.feet.z += dz;
            return;
        }
        let n = ((len / MAX_SUBSTEP).ceil() as i32).max(1);
        let sx = dx / n as f32;
        let sz = dz / n as f32;
        let height = if self.is_crouching() { CROUCH_HEIGHT } else { STAND_HEIGHT };
        for _ in 0..n {
            let nx = self.feet.x + sx;
            if !blocked_by(solids, nx, self.feet.z, RADIUS, self.feet.y, self.feet.y + height) {
                self.feet.x = nx;
            }
            let nz = self.feet.z + sz;
            if !blocked_by(solids, self.feet.x, nz, RADIUS, self.feet.y, self.feet.y + height) {
                self.feet.z = nz;
            }
            self.clamp_footprint();
        }
    }

    /// Per-frame update: look, then walk (WASD 4.0 m/s, sprint 6.25, crouch 2.25).
    /// `sens` is radians per mouse pixel (`Save::mouse_sens`, default 0.0075).
    pub fn update(&mut self, input: &InputFrame, _bindings: &Bindings, solids: &[Aabb], sens: f32) {
        let dt = input.dt.clamp(0.0, 0.1);
        self.apply_look(input, sens, dt);

        // Crouch ease (eye drops over ~0.1 s).
        let wants_crouch = self.crouch || input.is_down("crouch");
        let target = if wants_crouch { 1.0 } else { 0.0 };
        let step = 10.0 * dt;
        self.crouch_t = if (target - self.crouch_t).abs() <= step {
            target
        } else {
            self.crouch_t + step * (target - self.crouch_t).signum()
        };

        self.moving = false;
        self.sprinting = false;
        self.speed = 0.0;
        if !self.frozen && dt > 0.0 {
            let fwd = (input.is_down("move_forward") as i32 - input.is_down("move_back") as i32) as f32;
            let side = (input.is_down("move_right") as i32 - input.is_down("move_left") as i32) as f32;
            let mag = (fwd * fwd + side * side).sqrt();
            if mag > 0.0 {
                let (fx, fz) = (-self.cam.yaw.sin(), -self.cam.yaw.cos());
                let (rx, rz) = (self.cam.yaw.cos(), -self.cam.yaw.sin());
                let wx = (fx * fwd + rx * side) / mag;
                let wz = (fz * fwd + rz * side) / mag;
                let sprint = input.is_down("sprint") && !wants_crouch;
                let speed = if wants_crouch {
                    CROUCH_SPEED
                } else if sprint {
                    SPRINT_SPEED
                } else {
                    WALK_SPEED
                };
                let before = (self.feet.x, self.feet.z);
                self.move_body(wx * speed * dt, wz * speed * dt, solids);
                let moved = ((self.feet.x - before.0).powi(2) + (self.feet.z - before.1).powi(2)).sqrt();
                self.speed = moved / dt;
                self.moving = moved > 1e-5;
                self.sprinting = sprint && self.moving;
            }
        }
        if self.noclip {
            // nothing else to do; noclip skips solids inside move_body
        } else {
            self.clamp_footprint();
        }
        self.sync_camera();
    }

    /// Index of the station under the crosshair, or `None`. Range 4.5 m, half
    /// angle 0.42 rad (widened by each station's `radius` as seen from the eye),
    /// clear line of sight to its focus point (blockers within 0.06 m of the
    /// focus do not count, nor does a solid that contains the focus). The best
    /// alignment wins; ties go to the nearer station on the floor plane.
    pub fn aim(&self, stations: &[StationDef], solids: &[Aabb]) -> Option<usize> {
        let origin = self.eye();
        let look = self.forward();
        let cos_half = AIM_HALF_ANGLE.cos();
        let mut chosen: Option<usize> = None;
        let mut best_align = -1.0f32;
        let mut best_dist = f32::INFINITY;
        for (i, s) in stations.iter().enumerate() {
            if s.disabled || s.prompt.is_empty() {
                continue;
            }
            let focus = s.pos + s.focus;
            let to = focus - origin;
            let dist = to.length();
            if dist <= 0.001 || dist > AIM_RANGE {
                continue;
            }
            let dir = to / dist;
            let align = look.dot(dir).clamp(-1.0, 1.0);
            let angle = align.acos();
            let widen = s.radius.max(0.0).atan2(dist);
            if align < cos_half && angle - widen > AIM_HALF_ANGLE {
                continue;
            }
            if !line_clear(origin, dir, dist, focus, solids) {
                continue;
            }
            let hd = ((s.pos.x - self.feet.x).powi(2) + (s.pos.z - self.feet.z).powi(2)).sqrt();
            if align > best_align + 0.0001 || ((align - best_align).abs() <= 0.0001 && hd < best_dist) {
                chosen = Some(i);
                best_align = align;
                best_dist = hd;
            }
        }
        chosen
    }
}

fn contains_point(a: &Aabb, p: Vec3, pad: f32) -> bool {
    p.x >= a.min.x - pad
        && p.x <= a.max.x + pad
        && p.y >= a.min.y - pad
        && p.y <= a.max.y + pad
        && p.z >= a.min.z - pad
        && p.z <= a.max.z + pad
}

/// Nothing solid stands between `origin` and `focus` (`dir` unit, `dist` long).
fn line_clear(origin: Vec3, dir: Vec3, dist: f32, focus: Vec3, solids: &[Aabb]) -> bool {
    for a in solids {
        // The thing the station is mounted on (panel, monitor) never blocks itself.
        if contains_point(a, focus, AIM_RAY_SLACK) {
            continue;
        }
        if let Some(t) = a.ray(origin, dir) {
            if t >= 0.0 && t < dist - AIM_RAY_SLACK {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Save;

    fn frame(actions: &[&'static str], dt: f32) -> InputFrame {
        let mut f = InputFrame::default();
        for a in actions {
            f.down.insert(*a);
        }
        f.dt = dt;
        f
    }

    fn bind() -> Bindings {
        Bindings::from_save(&Save::default())
    }

    fn station(id: &str, pos: Vec3) -> StationDef {
        StationDef { id: id.to_string(), prompt: format!("USE {id}"), pos, focus: Vec3::ZERO, radius: 0.0, disabled: false }
    }

    fn wall(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }

    #[test]
    fn forward_at_yaw_zero_is_minus_z() {
        let p = Player::new(Vec3::ZERO);
        let f = p.forward();
        assert!(f.x.abs() < 1e-6 && f.y.abs() < 1e-6 && (f.z + 1.0).abs() < 1e-6);
        let mut q = Player::new(Vec3::ZERO);
        q.teleport(Vec3::ZERO, std::f32::consts::FRAC_PI_2, 0.0);
        let f = q.forward();
        assert!((f.x + 1.0).abs() < 1e-5 && f.z.abs() < 1e-5);
    }

    #[test]
    fn eye_is_1_707_above_the_feet() {
        let p = Player::new(vec3(1.0, 0.0, 2.0));
        assert!((p.eye().y - 1.707).abs() < 1e-6);
        assert!((p.cam.pos.y - 1.707).abs() < 1e-6);
    }

    #[test]
    fn walk_speed_and_sprint_speed() {
        let mut p = Player::new(Vec3::ZERO);
        p.update(&frame(&["move_forward"], 0.1), &bind(), &[], DEFAULT_SENS);
        assert!((p.feet.z + 0.4).abs() < 1e-4, "walk 4 m/s for 0.1 s, got {}", p.feet.z);
        let mut q = Player::new(Vec3::ZERO);
        q.update(&frame(&["move_forward", "sprint"], 0.1), &bind(), &[], DEFAULT_SENS);
        assert!((q.feet.z + 0.625).abs() < 1e-4);
        assert!(q.sprinting);
    }

    #[test]
    fn diagonal_is_not_faster() {
        let mut p = Player::new(Vec3::ZERO);
        p.update(&frame(&["move_forward", "move_right"], 0.1), &bind(), &[], DEFAULT_SENS);
        let d = (p.feet.x.powi(2) + p.feet.z.powi(2)).sqrt();
        assert!((d - 0.4).abs() < 1e-4);
        assert!(p.feet.x > 0.0 && p.feet.z < 0.0);
    }

    #[test]
    fn crouch_is_slower_and_lowers_the_eye() {
        let mut p = Player::new(Vec3::ZERO);
        for _ in 0..5 {
            p.update(&frame(&["move_forward", "crouch"], 0.05), &bind(), &[], DEFAULT_SENS);
        }
        assert!(p.eye().y < 1.2);
        assert!(p.speed <= CROUCH_SPEED + 1e-3);
    }

    #[test]
    fn walking_into_a_wall_stops() {
        let solids = [wall(vec3(-5.0, 0.0, -1.2), vec3(5.0, 3.0, -1.0))];
        let mut p = Player::new(Vec3::ZERO);
        for _ in 0..60 {
            p.update(&frame(&["move_forward"], 0.05), &bind(), &solids, DEFAULT_SENS);
        }
        assert!(p.feet.z >= -1.0 + RADIUS - 1e-3, "z = {}", p.feet.z);
        assert!(p.feet.z <= -1.0 + RADIUS + 0.2);
    }

    #[test]
    fn a_fast_frame_does_not_tunnel() {
        let solids = [wall(vec3(-5.0, 0.0, -0.45), vec3(5.0, 3.0, -0.4))];
        let mut p = Player::new(Vec3::ZERO);
        p.update(&frame(&["move_forward", "sprint"], 0.1), &bind(), &solids, DEFAULT_SENS);
        p.update(&frame(&["move_forward", "sprint"], 0.1), &bind(), &solids, DEFAULT_SENS);
        assert!(p.feet.z > -0.4);
    }

    #[test]
    fn sliding_along_a_wall_keeps_moving() {
        let solids = [wall(vec3(-5.0, 0.0, -1.2), vec3(5.0, 3.0, -1.0))];
        let mut p = Player::new(vec3(0.0, 0.0, -0.7));
        for _ in 0..10 {
            p.update(&frame(&["move_forward", "move_right"], 0.05), &bind(), &solids, DEFAULT_SENS);
        }
        assert!(p.feet.x > 0.5);
        assert!(p.feet.z >= -1.0 + RADIUS - 1e-3);
    }

    #[test]
    fn low_obstacles_are_stepped_over_and_noclip_ignores_walls() {
        let low = [wall(vec3(-5.0, 0.0, -1.2), vec3(5.0, 0.2, -1.0))];
        let mut p = Player::new(Vec3::ZERO);
        for _ in 0..40 {
            p.update(&frame(&["move_forward"], 0.05), &bind(), &low, DEFAULT_SENS);
        }
        assert!(p.feet.z < -1.5);
        let tall = [wall(vec3(-5.0, 0.0, -1.2), vec3(5.0, 3.0, -1.0))];
        let mut q = Player::new(Vec3::ZERO);
        q.noclip = true;
        for _ in 0..40 {
            q.update(&frame(&["move_forward"], 0.05), &bind(), &tall, DEFAULT_SENS);
        }
        assert!(q.feet.z < -1.5);
    }

    #[test]
    fn frozen_blocks_walking_but_not_looking() {
        let mut p = Player::new(Vec3::ZERO);
        p.frozen = true;
        let mut f = frame(&["move_forward"], 0.1);
        f.mouse_delta = (10.0, 0.0);
        p.update(&f, &bind(), &[], DEFAULT_SENS);
        assert_eq!(p.feet, Vec3::ZERO);
        assert!(p.cam.yaw < 0.0);
        p.look_frozen = true;
        let yaw = p.cam.yaw;
        p.update(&f, &bind(), &[], DEFAULT_SENS);
        assert_eq!(p.cam.yaw, yaw);
    }

    #[test]
    fn mouse_look_turns_right_and_clamps_pitch() {
        let mut p = Player::new(Vec3::ZERO);
        let mut f = frame(&[], 0.016);
        f.mouse_delta = (100.0, 5000.0);
        p.update(&f, &bind(), &[], DEFAULT_SENS);
        assert!((p.cam.yaw + 0.75).abs() < 1e-5);
        assert!((p.cam.pitch + PITCH_LIMIT).abs() < 1e-6);
        f.mouse_delta = (0.0, -50000.0);
        p.update(&f, &bind(), &[], DEFAULT_SENS);
        assert!((p.cam.pitch - PITCH_LIMIT).abs() < 1e-6);
    }

    #[test]
    fn key_look_turns() {
        let mut p = Player::new(Vec3::ZERO);
        p.update(&frame(&["look_left"], 0.1), &bind(), &[], DEFAULT_SENS);
        assert!(p.cam.yaw > 0.0);
    }

    #[test]
    fn footprint_bounds_the_body() {
        let mut p = Player::new(Vec3::ZERO);
        p.footprint = Some((-1.0, -1.0, 1.0, 1.0));
        for _ in 0..60 {
            p.update(&frame(&["move_forward"], 0.05), &bind(), &[], DEFAULT_SENS);
        }
        assert!((p.feet.z + (1.0 - RADIUS)).abs() < 1e-4);
    }

    #[test]
    fn aim_picks_the_nearer_of_two_stations_on_the_axis() {
        let mut p = Player::new(vec3(0.0, 0.0, 0.0));
        p.teleport(vec3(0.0, 0.0, 0.0), 0.0, 0.0);
        let y = 1.707;
        let st = [station("far", vec3(0.0, y, -3.5)), station("near", vec3(0.0, y, -2.0))];
        assert_eq!(p.aim(&st, &[]), Some(1));
    }

    #[test]
    fn aim_prefers_the_better_aligned_station() {
        let p = Player::new(Vec3::ZERO);
        let y = 1.707;
        let st = [station("side", vec3(0.5, y, -2.0)), station("centre", vec3(0.0, y, -3.0))];
        assert_eq!(p.aim(&st, &[]), Some(1));
    }

    #[test]
    fn aim_ignores_disabled_far_and_outside_the_cone() {
        let p = Player::new(Vec3::ZERO);
        let y = 1.707;
        let mut off = station("off", vec3(0.0, y, -2.0));
        off.disabled = true;
        assert_eq!(p.aim(&[off], &[]), None);
        assert_eq!(p.aim(&[station("far", vec3(0.0, y, -6.0))], &[]), None);
        assert_eq!(p.aim(&[station("side", vec3(3.0, y, -2.0))], &[]), None);
        let mut quiet = station("quiet", vec3(0.0, y, -2.0));
        quiet.prompt = String::new();
        assert_eq!(p.aim(&[quiet], &[]), None);
    }

    #[test]
    fn aim_is_blocked_by_a_wall_but_not_by_its_own_mount() {
        let p = Player::new(Vec3::ZERO);
        let y = 1.707;
        let target = [station("panel", vec3(0.0, y, -3.0))];
        let between = [wall(vec3(-1.0, 0.0, -1.6), vec3(1.0, 3.0, -1.5))];
        assert_eq!(p.aim(&target, &between), None);
        // The panel sits on a wall whose face is the focus point: still usable.
        let mount = [wall(vec3(-1.0, 0.0, -3.2), vec3(1.0, 3.0, -3.0))];
        assert_eq!(p.aim(&target, &mount), Some(0));
    }

    #[test]
    fn aim_follows_the_look_direction() {
        let mut p = Player::new(Vec3::ZERO);
        let y = 1.707;
        let st = [station("east", vec3(3.0, y, 0.0))];
        assert_eq!(p.aim(&st, &[]), None);
        p.teleport(Vec3::ZERO, -std::f32::consts::FRAC_PI_2, 0.0);
        assert_eq!(p.aim(&st, &[]), Some(0));
    }

    #[test]
    fn flashlight_spot_follows_the_flag() {
        let mut p = Player::new(Vec3::ZERO);
        assert!(p.flashlight_spot().is_none());
        p.flashlight = true;
        let s = p.flashlight_spot().expect("on");
        assert!((s.energy - 7.5).abs() < 1e-6 && (s.range - 16.0).abs() < 1e-6);
        assert!((s.cos_outer - 30f32.to_radians().cos()).abs() < 1e-6);
        assert!((s.dir.z + 1.0).abs() < 1e-5);
    }
}
