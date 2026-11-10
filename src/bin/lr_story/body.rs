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
#[allow(dead_code)]
pub const FOV_DEG: f32 = 78.0;
const PITCH_LIMIT: f32 = crate::look::PITCH_LIMIT;

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
    /// How much of the bob shows, 0..1: eases up while the body walks on the ground and back down when it stops, so the
    /// view settles instead of snapping flat.
    bob_weight: f32,
    /// Metres walked on the ground, for the footstep-like counters the game may read.
    pub walked: f32,
    /// Scales walking and sprinting speed (a story's player profile sets it). 1.0 is the normal speed; crouching is not
    /// scaled.
    pub speed: f32,
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
            bob_weight: 0.0,
            walked: 0.0,
            speed: 1.0,
        }
    }

    /// The horizontal facing as (x, z) on the ground plane. At yaw 0 this is (0, -1).
    pub fn forward2(&self) -> (f32, f32) {
        (-self.yaw.sin(), -self.yaw.cos())
    }

    /// Mouse look: a positive x turns right (yaw goes down, as last_route's Player::look does: the forward is
    /// (-sin yaw, -cos yaw), so a smaller yaw swings it toward +x); a positive y looks down. The pitch is clamped.
    pub fn look(&mut self, d: Vec2) {
        self.yaw -= d.x;
        self.pitch = (self.pitch - d.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Whether the body is moving up (or at the top of a jump). Meaningful while it is off the ground.
    pub fn rising(&self) -> bool {
        self.vy >= 0.0
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
        let amp = amp * self.bob_weight;
        let bob = (self.bob_phase * 2.0).sin() * amp;
        let sway = self.bob_phase.cos() * amp * BOB_SWAY_RATIO;
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
            SPRINT_SPEED * self.speed
        } else {
            WALK_SPEED * self.speed
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
        let mut covered = 0.0;
        if moving {
            let (fx, fz) = self.forward2();
            let (rx, rz) = (self.yaw.cos(), -self.yaw.sin());
            let dx = (fx * axes.y + rx * axes.x) * speed * dt;
            let dz = (fz * axes.y + rz * axes.x) * speed * dt;
            covered = self.move_body(dx, dz, blocked);
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

        // Head bob: the phase advances with the ground actually covered (a body pushed against a wall does not bob), and
        // eases back to the nearest rest when it stops. The bob's weight eases the same way instead of snapping to zero.
        let bobbing = covered > 1e-6 && self.grounded;
        let ease = 1.0 - (-BOB_EASE_RATE * dt).exp();
        if bobbing {
            self.bob_phase += BOB_PHASE_PER_METRE * covered;
        } else {
            let rest = (self.bob_phase / std::f32::consts::PI).round() * std::f32::consts::PI;
            self.bob_phase += (rest - self.bob_phase) * ease;
        }
        let target = if bobbing { 1.0 } else { 0.0 };
        self.bob_weight += (target - self.bob_weight) * ease;
    }

    /// Moves the body and returns the ground it covered in metres. A step is taken when it is clear, or when the body
    /// already overlaps a solid tile (one turned solid around it) and the step overlaps less, so it can always walk out.
    fn move_body(&mut self, dx: f32, dz: f32, blocked: &dyn Fn(i32, i32) -> bool) -> f32 {
        let total = (dx * dx + dz * dz).sqrt();
        let steps = ((total / 0.08).ceil() as i32).max(1);
        let start = self.pos;
        for _ in 0..steps {
            let nx = self.pos.0 + dx / steps as f32;
            if can_step(self.pos, (nx, self.pos.1), blocked) {
                self.pos.0 = nx;
            }
            let nz = self.pos.1 + dz / steps as f32;
            if can_step(self.pos, (self.pos.0, nz), blocked) {
                self.pos.1 = nz;
            }
        }
        let moved = ((self.pos.0 - start.0).powi(2) + (self.pos.1 - start.1).powi(2)).sqrt();
        self.walked += moved;
        moved
    }
}

/// Whether the body may go from `from` to `to`: the target is clear, or the body is already inside a solid tile and the
/// target overlaps strictly less of it (so the way out is always open, and the way deeper never is).
fn can_step(from: (f32, f32), to: (f32, f32), blocked: &dyn Fn(i32, i32) -> bool) -> bool {
    if !hits_wall(to.0, to.1, blocked) {
        return true;
    }
    hits_wall(from.0, from.1, blocked) && overlap_area(to.0, to.1, blocked) < overlap_area(from.0, from.1, blocked)
}

/// The area (m^2) of the body's square that lies inside blocked tiles.
fn overlap_area(x: f32, z: f32, blocked: &dyn Fn(i32, i32) -> bool) -> f32 {
    let (x0, x1) = (((x - RADIUS) / TILE_M).floor() as i32, ((x + RADIUS) / TILE_M).floor() as i32);
    let (z0, z1) = (((z - RADIUS) / TILE_M).floor() as i32, ((z + RADIUS) / TILE_M).floor() as i32);
    let mut area = 0.0;
    for tz in z0..=z1 {
        for tx in x0..=x1 {
            if blocked(tx, tz) {
                let w = ((x + RADIUS).min((tx + 1) as f32 * TILE_M) - (x - RADIUS).max(tx as f32 * TILE_M)).max(0.0);
                let h = ((z + RADIUS).min((tz + 1) as f32 * TILE_M) - (z - RADIUS).max(tz as f32 * TILE_M)).max(0.0);
                area += w * h;
            }
        }
    }
    area
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
        assert!((b.yaw + 0.3).abs() < 1e-5);
        // A positive x turns right: from facing north (-z), the forward swings toward +x (east).
        let (fx, _) = b.forward2();
        assert!(fx > 0.0, "turned right, forward x = {fx}");
    }

    #[test]
    fn speed_scales_walking_and_sprinting_but_not_crouching() {
        assert_eq!(Body::new((1.0, 1.0), 0.0).speed, 1.0, "the default speed is normal");

        let mut walk = Body::new((1.0, 1.0), 0.0);
        walk.speed = 2.0;
        walk.update(0.5, &Input { fwd: 1.0, ..Default::default() }, &open);
        assert!((walk.pos.1 - (1.0 - 2.0 * WALK_SPEED * 0.5)).abs() < 1e-3, "walk z = {}", walk.pos.1);

        let mut sprint = Body::new((1.0, 1.0), 0.0);
        sprint.speed = 2.0;
        sprint.update(0.5, &Input { fwd: 1.0, sprint: true, ..Default::default() }, &open);
        assert!((sprint.pos.1 - (1.0 - 2.0 * SPRINT_SPEED * 0.5)).abs() < 1e-3, "sprint z = {}", sprint.pos.1);

        let mut crouch = Body::new((1.0, 1.0), 0.0);
        crouch.speed = 2.0;
        crouch.update(0.5, &Input { fwd: 1.0, crouch: true, ..Default::default() }, &open);
        assert!((crouch.pos.1 - (1.0 - CROUCH_SPEED * 0.5)).abs() < 1e-3, "crouch z = {}", crouch.pos.1);
    }

    #[test]
    fn a_body_inside_a_tile_that_became_solid_can_walk_out_but_not_deeper() {
        let wall = |tx: i32, _tz: i32| tx == 1; // x in [2, 4)
        // Centre at x = 2.1: the right edge (2.34) is inside the wall, the left edge (1.86) is clear.
        let mut west = Body::new((2.1, 5.0), std::f32::consts::FRAC_PI_2); // faces -x
        assert!(hits_wall(west.pos.0, west.pos.1, &wall));
        west.update(0.25, &Input { fwd: 1.0, ..Default::default() }, &wall);
        assert!(west.pos.0 < 1.5 && !hits_wall(west.pos.0, west.pos.1, &wall), "x = {}", west.pos.0);

        let mut east = Body::new((2.1, 5.0), -std::f32::consts::FRAC_PI_2); // faces +x, deeper
        east.update(0.25, &Input { fwd: 1.0, ..Default::default() }, &wall);
        assert!(east.pos.0 <= 2.1 + 1e-4, "went deeper: x = {}", east.pos.0);
    }

    #[test]
    fn the_bob_advances_on_the_ground_covered_not_the_ground_asked_for() {
        let mut free = Body::new((1.0, 20.0), 0.0);
        free.update(0.25, &Input { fwd: 1.0, ..Default::default() }, &open); // 1 m
        assert!((free.bob_phase - BOB_PHASE_PER_METRE).abs() < 1e-3, "phase {}", free.bob_phase);

        // Pressed against a wall (tile row -1 is solid; the body faces north at z = RADIUS): no ground, no bob.
        let wall = |_tx: i32, tz: i32| tz < 0;
        let mut stuck = Body::new((1.0, RADIUS), 0.0);
        stuck.bob_phase = 1.0;
        for _ in 0..10 {
            stuck.update(0.05, &Input { fwd: 1.0, ..Default::default() }, &wall);
        }
        assert!(stuck.bob_phase <= 1.0 + 1e-4, "phase advanced to {}", stuck.bob_phase);
        assert!(stuck.bob_weight < 0.05, "weight {}", stuck.bob_weight);
    }

    #[test]
    fn the_bob_eases_out_after_the_body_stops() {
        let mut b = Body::new((1.0, 40.0), 0.0);
        for _ in 0..20 {
            b.update(0.05, &Input { fwd: 1.0, ..Default::default() }, &open);
        }
        assert!(b.bob_weight > 0.9, "weight {}", b.bob_weight);
        b.update(0.016, &Input::default(), &open);
        assert!(b.bob_weight > 0.5 && b.bob_weight < 1.0, "weight right after stopping {}", b.bob_weight);
        for _ in 0..120 {
            b.update(0.05, &Input::default(), &open);
        }
        assert!(b.bob_weight < 1e-3);
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
