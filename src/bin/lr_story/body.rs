//! The first-person body, ported from the Last Route runner (src/bin/last_route/game/player.rs and config/mod.rs):
//! walk, sprint, crouch, the grounded jump with gravity, stamina and exhaustion, the eye-aligned head bob and sway,
//! and mouse look with a pitch limit. Units are metres. The facility, level stack, meters and sounds of the original
//! are not here; collision is against the story map's solid tiles.

use macroquad::prelude::{vec2, vec3, Vec2, Vec3};

/// Metres per map tile. Walls, floors and the body all use this scale.
pub const TILE_M: f32 = 2.0;

// Movement (config/mod.rs: 64, 100, 36 px/s over TILE_SIZE 16 = 4.0, 6.25, 2.25 m/s).
pub const WALK_SPEED: f32 = 4.0;
pub const SPRINT_SPEED: f32 = 6.25;
pub const CROUCH_SPEED: f32 = 2.25;
pub const GRAVITY: f32 = 12.0;
pub const JUMP_VELOCITY: f32 = 3.15;
/// The body's half-width for collision.
pub const RADIUS: f32 = 0.24;

// Eye (config/mod.rs: CAMERA_HEIGHT = 1.6 * PLAYER_MODEL_SCALE, CROUCH_EYE_RATIO 0.69).
const MODEL_SCALE: f32 = 1.067;
pub const CAMERA_HEIGHT: f32 = 1.6 * MODEL_SCALE;
pub const CROUCH_EYE_RATIO: f32 = 0.69;
/// How fast the eye settles into a crouch or back up (player.rs crouch_t, per second).
const CROUCH_RATE: f32 = 14.0;

// Head bob (config/mod.rs CAMERA_BOB_*; data/player/bob.json: phase_per_metre 1.9, ease_rate 10, sway_ratio 0.42).
const BOB_WALK: f32 = 0.024 * MODEL_SCALE;
const BOB_SPRINT: f32 = 0.038 * MODEL_SCALE;
const BOB_CROUCH: f32 = 0.012 * MODEL_SCALE;
const BOB_PHASE_PER_METRE: f32 = 1.9;
const BOB_EASE_RATE: f32 = 10.0;
const BOB_SWAY_RATIO: f32 = 0.42;

// Stamina (config/mod.rs: MAX 100, sprint drain 32/s, recovery 21/s, exhausted recovery 25).
pub const MAX_STAMINA: f32 = 100.0;
const SPRINT_DRAIN: f32 = 32.0;
const STAMINA_RECOVERY: f32 = 21.0;
const EXHAUSTED_RECOVERY: f32 = 25.0;

/// Camera and look (config/mod.rs FIELD_OF_VIEW_DEG 78, MOUSE_SENSITIVITY 0.0075; the pitch limit is the one the
/// original reads from data/player/look.json, approximated here).
pub const FOV_DEG: f32 = 78.0;
pub const MOUSE_SENSITIVITY: f32 = 0.0075;
const PITCH_LIMIT: f32 = 1.45;

/// What the player asks for this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// -1 back .. 1 forward.
    pub fwd: f32,
    /// -1 left .. 1 right.
    pub strafe: f32,
    pub sprint: bool,
    pub crouch: bool,
    /// Held state; a hop needs a fresh press.
    pub jump: bool,
    /// Mouse look in radians (already scaled by sensitivity).
    pub look: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// Feet position on the ground plane (x, z) in metres.
    pub pos: (f32, f32),
    /// Height of the jump above the ground (0 on the ground).
    pub feet_y: f32,
    vy: f32,
    jump_prev: bool,
    /// Turn around the vertical axis. 0 faces -z (north); positive turns right.
    pub yaw: f32,
    pub pitch: f32,
    /// 0 standing .. 1 fully crouched.
    pub crouch_t: f32,
    pub crouching: bool,
    pub sprinting: bool,
    pub moving: bool,
    pub grounded: bool,
    pub stamina: f32,
    pub exhausted: bool,
    pub bob_phase: f32,
    /// Metres walked on the ground, for the footstep-like counters the game may read.
    pub walked: f32,
}

impl Body {
    pub fn new(pos: (f32, f32), yaw: f32) -> Body {
        Body {
            pos,
            feet_y: 0.0,
            vy: 0.0,
            jump_prev: false,
            yaw,
            pitch: 0.0,
            crouch_t: 0.0,
            crouching: false,
            sprinting: false,
            moving: false,
            grounded: true,
            stamina: MAX_STAMINA,
            exhausted: false,
            bob_phase: 0.0,
            walked: 0.0,
        }
    }

    /// The horizontal facing as (x, z) on the ground plane. At yaw 0 this is (0, -1).
    pub fn forward2(&self) -> (f32, f32) {
        (-self.yaw.sin(), -self.yaw.cos())
    }

    /// Mouse look: a positive x turns right; the pitch is clamped so the view never flips.
    pub fn look(&mut self, d: Vec2) {
        self.yaw += d.x;
        self.pitch = (self.pitch - d.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// The eye height above the ground at the current crouch, without the bob.
    pub fn eye_height(&self) -> f32 {
        CAMERA_HEIGHT * (1.0 + (CROUCH_EYE_RATIO - 1.0) * self.crouch_t)
    }

    /// The camera position: the eye, with the head bob and sway applied.
    pub fn eye(&self) -> Vec3 {
        let amp = if self.crouching {
            BOB_CROUCH
        } else if self.sprinting {
            BOB_SPRINT
        } else {
            BOB_WALK
        };
        let walking = self.moving && self.grounded;
        let bob = if walking { (self.bob_phase * 2.0).sin() * amp } else { 0.0 };
        let sway = if walking { self.bob_phase.cos() * amp * BOB_SWAY_RATIO } else { 0.0 };
        let right = vec2(self.yaw.cos(), -self.yaw.sin());
        vec3(
            self.pos.0 + right.x * sway,
            self.feet_y + self.eye_height() + bob,
            self.pos.1 + right.y * sway,
        )
    }

    /// The unit view direction, pitch included.
    pub fn view_dir(&self) -> Vec3 {
        let (fx, fz) = self.forward2();
        vec3(fx * self.pitch.cos(), self.pitch.sin(), fz * self.pitch.cos()).normalize()
    }

    /// Advances the body by `dt` seconds. `blocked(tx, ty)` says whether map tile (tx, ty) stops the body; off-map
    /// tiles must answer true. The body does not move while `input.fwd` and `input.strafe` are zero.
    pub fn update(&mut self, dt: f32, input: &Input, blocked: &dyn Fn(i32, i32) -> bool) {
        self.look(input.look);

        self.crouching = input.crouch;
        let target = if self.crouching { 1.0 } else { 0.0 };
        self.crouch_t += (target - self.crouch_t) * (1.0 - (-CROUCH_RATE * dt).exp());

        let mut axes = vec2(input.strafe, input.fwd);
        if axes.length() > 1.0 {
            axes = axes.normalize();
        }
        let moving = axes.length_squared() > 0.0;
        self.moving = moving;

        let wants_sprint = input.sprint && moving && !self.exhausted && self.grounded;
        let sprinting = wants_sprint && self.stamina > 0.0 && !self.crouching;
        self.sprinting = sprinting;
        let speed = if self.crouching {
            CROUCH_SPEED
        } else if sprinting {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };

        if sprinting {
            self.stamina = (self.stamina - SPRINT_DRAIN * dt).max(0.0);
            if self.stamina <= 0.0 {
                self.exhausted = true;
            }
        } else {
            self.stamina = (self.stamina + STAMINA_RECOVERY * dt).min(MAX_STAMINA);
            if self.exhausted && self.stamina >= EXHAUSTED_RECOVERY {
                self.exhausted = false;
            }
        }

        // Walking: the forward and strafe axes in the ground plane, then per-axis slide against the walls.
        if moving {
            let (fx, fz) = self.forward2();
            let (rx, rz) = (self.yaw.cos(), -self.yaw.sin());
            let dx = (fx * axes.y + rx * axes.x) * speed * dt;
            let dz = (fz * axes.y + rz * axes.x) * speed * dt;
            self.move_body(dx, dz, blocked);
        }

        // Jump on a fresh press while grounded; gravity and landing otherwise.
        let jump_edge = input.jump && !self.jump_prev;
        self.jump_prev = input.jump;
        if jump_edge && self.grounded && !self.crouching {
            self.vy = JUMP_VELOCITY;
            self.grounded = false;
        }
        if !self.grounded {
            self.vy -= GRAVITY * dt;
            self.feet_y += self.vy * dt;
            if self.feet_y <= 0.0 {
                self.feet_y = 0.0;
                self.vy = 0.0;
                self.grounded = true;
            }
        }

        // Head bob: the phase advances with the ground covered, and eases back to the nearest rest when it stops.
        if moving && self.grounded {
            self.bob_phase += BOB_PHASE_PER_METRE * speed * dt;
        } else {
            let rest = (self.bob_phase / std::f32::consts::PI).round() * std::f32::consts::PI;
            self.bob_phase += (rest - self.bob_phase) * (1.0 - (-BOB_EASE_RATE * dt).exp());
        }
    }

    fn move_body(&mut self, dx: f32, dz: f32, blocked: &dyn Fn(i32, i32) -> bool) {
        let total = (dx * dx + dz * dz).sqrt();
        let steps = ((total / 0.08).ceil() as i32).max(1);
        let start = self.pos;
        for _ in 0..steps {
            let nx = self.pos.0 + dx / steps as f32;
            if !hits_wall(nx, self.pos.1, blocked) {
                self.pos.0 = nx;
            }
            let nz = self.pos.1 + dz / steps as f32;
            if !hits_wall(self.pos.0, nz, blocked) {
                self.pos.1 = nz;
            }
        }
        self.walked += ((self.pos.0 - start.0).powi(2) + (self.pos.1 - start.1).powi(2)).sqrt();
    }

    /// The map tile the body's feet are on.
    pub fn tile(&self) -> (i32, i32) {
        ((self.pos.0 / TILE_M).floor() as i32, (self.pos.1 / TILE_M).floor() as i32)
    }
}

/// Whether a body of `RADIUS` centred at (x, z) overlaps a blocked tile.
fn hits_wall(x: f32, z: f32, blocked: &dyn Fn(i32, i32) -> bool) -> bool {
    let (x0, x1) = (((x - RADIUS) / TILE_M).floor() as i32, ((x + RADIUS) / TILE_M).floor() as i32);
    let (z0, z1) = (((z - RADIUS) / TILE_M).floor() as i32, ((z + RADIUS) / TILE_M).floor() as i32);
    (z0..=z1).any(|tz| (x0..=x1).any(|tx| blocked(tx, tz)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(_: i32, _: i32) -> bool {
        false
    }

    #[test]
    fn walking_forward_moves_at_walk_speed_along_the_facing() {
        let mut b = Body::new((1.0, 1.0), 0.0); // faces -z
        b.update(0.5, &Input { fwd: 1.0, ..Default::default() }, &open);
        assert!((b.pos.1 - (1.0 - WALK_SPEED * 0.5)).abs() < 1e-3, "z = {}", b.pos.1);
        assert!((b.pos.0 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn walls_stop_the_body() {
        // Wall at tile column 1 (x in [2,4)). The body starts clear at x = 1.0 and walks east into it; its edge
        // (x + RADIUS) stops at the wall face, so it ends just short of x = 2 - RADIUS.
        let wall = |tx: i32, _tz: i32| tx == 1;
        let mut b = Body::new((1.0, 5.0), -std::f32::consts::FRAC_PI_2); // faces +x (yaw -pi/2)
        b.update(1.0, &Input { fwd: 1.0, ..Default::default() }, &wall);
        assert!(b.pos.0 > 1.5 && b.pos.0 <= 2.0 - RADIUS + 1e-3, "x = {}", b.pos.0);
    }

    #[test]
    fn a_jump_leaves_the_ground_and_lands_again() {
        let mut b = Body::new((1.0, 1.0), 0.0);
        b.update(0.016, &Input { jump: true, ..Default::default() }, &open);
        assert!(!b.grounded && b.feet_y > 0.0);
        for _ in 0..200 {
            b.update(0.016, &Input::default(), &open);
        }
        assert!(b.grounded && b.feet_y == 0.0);
    }

    #[test]
    fn crouching_lowers_the_eye() {
        let mut b = Body::new((1.0, 1.0), 0.0);
        let standing = b.eye().y;
        for _ in 0..120 {
            b.update(0.016, &Input { crouch: true, ..Default::default() }, &open);
        }
        assert!(b.eye().y < standing - 0.3, "{} vs {}", b.eye().y, standing);
    }

    #[test]
    fn pitch_is_clamped_and_yaw_turns_right_on_positive_x() {
        let mut b = Body::new((1.0, 1.0), 0.0);
        b.look(vec2(0.3, 100.0));
        assert!((b.pitch + PITCH_LIMIT).abs() < 1e-5);
        assert!((b.yaw - 0.3).abs() < 1e-5);
    }

    #[test]
    fn sprinting_drains_stamina_until_exhausted() {
        let mut b = Body::new((1.0, 1.0), 0.0);
        for _ in 0..400 {
            b.update(0.05, &Input { fwd: 1.0, sprint: true, ..Default::default() }, &open);
        }
        assert!(b.exhausted, "stamina {}", b.stamina);
        assert!(!b.sprinting);
    }
}
