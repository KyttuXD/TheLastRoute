//! The runner's body (player_controller_3d.gd): walk / sprint / crouch at the original's
//! speeds, the grounded hop, stamina and exhaustion, the flashlight battery, the stim pack,
//! and the eye-aligned bobbing camera.

use super::meters::Meters;
use super::level_stack::{body_fits_at_y, floor_y_of, ground_y, LevelStack, STEP_UP};
use super::Controls;
use crate::audio::Sounds;
use crate::config::*;
use crate::world::facility::LinkKind;

use crate::world::Facility;
use macroquad::prelude::*;

/// Strobe period bounds for a forced flicker (player_controller_3d.gd 867-870).
const FLICKER_MIN_PERIOD: f32 = 0.08;
const FLICKER_MAX_PERIOD: f32 = 0.35;

/// How fast the body's height follows the stair tread under it, per second (an exponential ease).
const STAIR_FOLLOW_RATE: f32 = 40.0;

/// The fall tunables (air control, the gap that makes a fall, the drop that reports a landing) are data:
/// data/swim/fall.json, read through `swim::fall_rules()`.
use super::swim::fall_rules;

/// The camera-feel tunables (data/player/bob.json): bob phase per metre walked (the step-locked cadence), the rad/s
/// the phase eases back to rest at, the lateral sway as a share of the bob amplitude, and the gap between the crouched
/// eye and the top of the shortened capsule (player_controller_3d.gd 997-1020, 1001-1002).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BobRules {
    pub phase_per_metre: f32,
    pub ease_rate: f32,
    pub sway_ratio: f32,
    pub crouch_eye_clearance: f32,
}

impl Default for BobRules {
    fn default() -> Self {
        BobRules { phase_per_metre: 1.9, ease_rate: 10.0, sway_ratio: 0.42, crouch_eye_clearance: 0.08 }
    }
}

pub fn bob_rules() -> &'static BobRules {
    static RULES: std::sync::OnceLock<BobRules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        let d = BobRules::default();
        let v = crate::content::embedded::data_under("player/bob.json")
            .next()
            .and_then(|(_, t)| serde_json::from_str::<serde_json::Value>(t).ok());
        let get = |k: &str, dv: f32| {
            v.as_ref().and_then(|v| v.get(k)).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(dv)
        };
        BobRules {
            phase_per_metre: get("phase_per_metre", d.phase_per_metre),
            ease_rate: get("ease_rate", d.ease_rate),
            sway_ratio: get("sway_ratio", d.sway_ratio),
            crouch_eye_clearance: get("crouch_eye_clearance", d.crouch_eye_clearance),
        }
    })
}

/// A fall that just ended: the body touched the ground at `y` (absolute feet y) after dropping `drop_height` metres,
/// at `speed` m/s, at `pos`. `Player::take_landing` hands it over once; the game applies the effects (stun, stamina,
/// noise) from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub y: f32,
    pub drop_height: f32,
    pub speed: f32,
    pub pos: (f32, f32),
}

/// The walking height of the stair flight under `pos` (`Stair::surface_rel`, relative to the level floor), or None off the stairs. Only
/// the facility's `Stairs` flights count: a sewer ladder is climbed by hand (game/ladder.rs) and is never a stair here.
pub fn stair_height_at(fac: &Facility, pos: (f32, f32)) -> Option<f32> {
    fac.stairs
        .iter()
        .filter(|s| matches!(s.kind, LinkKind::Stairs))
        .find_map(|s| s.surface_rel(fac.level, pos.0, pos.1))
}

/// Whether a body of `PLAYER_RADIUS` centred at (x, y) and standing at `h` above the level floor fits, with no neighbour
/// level resident. The facility rule (`body_fits_at_y`) covers the flight wells and the lane dividers.
pub(crate) fn body_fits(fac: &Facility, x: f32, y: f32, h: f32) -> bool {
    body_fits_at_y(fac, &LevelStack::EMPTY, x, y, floor_y_of(fac.level) + h, PLAYER_RADIUS)
}

pub struct Player {
    pub pos: (f32, f32),
    /// Height of the jump above the walking surface (0 on the ground).
    pub feet_y: f32,
    /// Absolute y of the current level's floor (`level * STOREY`, `level_stack::floor_y_of`); the game sets it whenever
    /// the level changes. Everything the renderer sees stays relative to it, so it moves nothing on screen.
    pub floor_y: f32,
    /// Whether the feet are on the ground (set by `update_movement`); footsteps, head bob and landing read it.
    pub grounded: bool,
    vy: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub crouch_t: f32,
    /// Metres the eye is lowered while the runner stands in sewer water (0 = dry); the feet stay on the walkway.
    pub water_sink: f32,
    pub crouching: bool,
    pub sprinting: bool,
    pub moving: bool,
    pub stamina: f32,
    pub exhausted: bool,
    pub lamp: f32,
    pub flashlight_on: bool,
    pub noise: f32,
    pub noise_burst: f32,
    pub noise_burst_timer: f32,
    pub bob_phase: f32,
    /// Whether jump was held last frame: a hop needs a fresh press (Godot just_pressed), not a held key.
    pub jump_prev: bool,
    pub stim_inject: f32,
    pub stim_boost: f32,
    pub stim_crash: f32,
    /// The hiding spot the runner is inside.
    pub hidden: Option<usize>,
    /// Ceryth's trap: locked inside this long.
    pub hide_lock: f32,
    pub in_vent: bool,
    pub vent_entry: usize,
    pub vent_time: f32,
    /// Movement and items locked (Ilyra's pounce, Ravelle's wire).
    pub stun: f32,
    pub pesticide: f32,
    pub walked: f32,
    /// Admin noclip: the body ignores walls and props (admin_menu.gd).
    pub noclip: bool,
    /// Height of the walking surface under the body: 0.0 on the floor, the tread's height on a stair flight (eased
    /// toward it in `update_movement`).
    pub height: f32,
    /// Admin run speed: multiplies every movement speed (admin_menu.gd).
    pub speed_scale: f32,
    /// While the body falls: the highest absolute y of the fall (its start, or the top of the arc), for the drop height.
    /// None when the body is not falling.
    fall_top: Option<f32>,
    /// The landing the last fall left, until `take_landing` takes it.
    landing: Option<Landing>,
    /// The body's velocity over the last frame, in metres a second, from the displacement `update_movement` made. It is
    /// not cleared by a label change or by `respawn_on_level`, so the momentum carries onto the next floor.
    pub move_vel: (f32, f32),
    /// Seconds left in a forced flicker (a pool landing or a hunter event); the
    /// beam strobes until it runs out and the player's own switch cannot override it.
    flicker_remaining: f32,
    flicker_next_toggle: f32,
    /// The strobe's dark half: the beam is forced off while this is set.
    flicker_dark: bool,
    flicker_seed: u32,
}

impl Player {
    pub fn new(pos: (f32, f32)) -> Player {
        Player {
            pos,
            feet_y: 0.0,
            floor_y: 0.0,
            grounded: true,
            vy: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            crouch_t: 0.0,
            water_sink: 0.0,
            crouching: false,
            sprinting: false,
            moving: false,
            stamina: MAX_STAMINA,
            exhausted: false,
            lamp: MAX_LAMP_CHARGE,
            flashlight_on: false,
            noise: 0.0,
            noise_burst: 0.0,
            noise_burst_timer: 0.0,
            bob_phase: 0.0,
            jump_prev: false,
            stim_inject: 0.0,
            stim_boost: 0.0,
            stim_crash: 0.0,
            hidden: None,
            hide_lock: 0.0,
            in_vent: false,
            vent_entry: 0,
            vent_time: 0.0,
            stun: 0.0,
            pesticide: 0.0,
            walked: 0.0,
            noclip: false,
            height: 0.0,
            speed_scale: 1.0,
            fall_top: None,
            landing: None,
            move_vel: (0.0, 0.0),
            flicker_remaining: 0.0,
            flicker_next_toggle: 0.0,
            flicker_dark: false,
            flicker_seed: 0x2545_F491,
        }
    }

    /// Strobes the beam off and on at random short intervals for `duration` seconds,
    /// then restores the real state (Godot `force_flashlight_flicker`). Called by a
    /// pool landing or a hunter event; the lamp keeps its charge and the switch keeps its state.
    pub fn force_flashlight_flicker(&mut self, duration: f32) {
        self.flicker_remaining = self.flicker_remaining.max(duration);
        self.flicker_next_toggle = 0.0;
    }

    /// True while a flicker holds the beam, so the player's [F] does not fight it.
    pub fn flashlight_seized(&self) -> bool {
        self.flicker_remaining > 0.0
    }

    /// The beam as the world sees it: the player's switch, a charged lamp, and not in the
    /// dark half of a flicker.
    pub fn beam_lit(&self) -> bool {
        self.flashlight_on && self.lamp > 0.0 && !self.flicker_dark
    }

    fn update_flashlight_flicker(&mut self, dt: f32) {
        if self.flicker_remaining <= 0.0 {
            return;
        }
        self.flicker_remaining -= dt;
        self.flicker_next_toggle -= dt;
        if self.flicker_remaining <= 0.0 {
            // Back to the real state rather than wherever the last random toggle left it.
            self.flicker_remaining = 0.0;
            self.flicker_dark = false;
            return;
        }
        if self.flicker_next_toggle <= 0.0 {
            self.flicker_dark = !self.flicker_dark;
            self.flicker_next_toggle = self.next_flicker_period();
        }
    }

    fn next_flicker_period(&mut self) -> f32 {
        self.flicker_seed = self
            .flicker_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let unit = (self.flicker_seed >> 8) as f32 / (1u32 << 24) as f32;
        FLICKER_MIN_PERIOD + (FLICKER_MAX_PERIOD - FLICKER_MIN_PERIOD) * unit
    }

    /// A new floor: start one cell south of the lobby centre, facing north.
    pub fn place(&mut self, start: (f32, f32)) {
        self.pos = (start.0, start.1 + 1.0);
        self.feet_y = 0.0;
        self.water_sink = 0.0;
        self.grounded = true;
        self.height = 0.0;
        self.vy = 0.0;
        self.fall_top = None;
        self.landing = None;
        self.move_vel = (0.0, 0.0);
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.hidden = None;
        self.hide_lock = 0.0;
        self.in_vent = false;
        self.stun = 0.0;
    }

    pub fn eye_height(&self) -> f32 {
        // the crouched eye never sits above the shortened capsule (Godot 1001-1002)
        let cap = crate::world::route::sewer::core::CROUCH_COLLISION_HEIGHT as f32 - bob_rules().crouch_eye_clearance;
        let crouched = (CAMERA_HEIGHT * CROUCH_EYE_RATIO).min(cap);
        CAMERA_HEIGHT + (crouched - CAMERA_HEIGHT) * self.crouch_t
    }

    /// Absolute y of the feet' walking surface: the level's floor plus the stair or deck lift (`height`).
    pub fn y(&self) -> f32 {
        self.floor_y + self.height
    }

    /// Absolute y of the eye without head bob: `y()` plus the jump and the standing or crouched eye height, minus the
    /// water sink.
    pub fn eye_y(&self) -> f32 {
        self.y() + self.feet_y + self.eye_height() - self.water_sink
    }

    /// Puts the feet on the walking surface under the body (the floor, a deck, a flight tread, the Inferno's noisy floor):
    /// `height` is the surface relative to the level floor and `feet_y` stays 0. A spawn, a resume and a level change use it.
    pub fn stand_on_ground(&mut self, fac: &Facility) {
        self.set_abs_y(self.floor_y + super::level_stack::local_ground(fac, self.pos.0, self.pos.1));
    }

    /// Puts the feet at absolute `y` on the spot (a ladder holds the body on its shaft): the height absorbs the difference to
    /// the level floor, and the jump offset, vertical speed and any fall end; the body is grounded.
    pub fn set_abs_y(&mut self, y: f32) {
        self.height = y - self.floor_y;
        self.feet_y = 0.0;
        self.vy = 0.0;
        self.grounded = true;
        self.fall_top = None;
    }

    /// Vertical speed in metres a second (up positive): the jump and the fall; 0 on the ground.
    pub fn vy(&self) -> f32 {
        self.vy
    }

    /// Whether the body is in a fall (no ground under it, `update_movement` integrates its height).
    pub fn falling(&self) -> bool {
        self.fall_top.is_some()
    }

    /// The landing the last fall left, once.
    pub fn take_landing(&mut self) -> Option<Landing> {
        self.landing.take()
    }

    /// Facing on the grid: (-sin yaw, -cos yaw), as the original's controller.
    pub fn forward2(&self) -> (f32, f32) {
        (-self.yaw.sin(), -self.yaw.cos())
    }

    pub fn camera_basis(&self) -> (Vec3, Vec3, Vec3) {
        let (fx, fz) = self.forward2();
        let fwd = vec3(
            fx * self.pitch.cos(),
            self.pitch.sin(),
            fz * self.pitch.cos(),
        )
        .normalize();
        let right = vec3(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = right.cross(fwd).normalize();
        (fwd, right, up)
    }

    pub fn eye(&self) -> Vec3 {
        let amp = if self.crouching {
            CAMERA_BOB_CROUCH
        } else if self.sprinting {
            CAMERA_BOB_SPRINT
        } else {
            CAMERA_BOB_WALK
        };
        let bob = if self.moving && self.grounded {
            (self.bob_phase * 2.0).sin() * amp
        } else {
            0.0
        };
        // lateral sway: cos(phase) * amplitude * ratio along the view's right (Godot 1017)
        let sway = if self.moving && self.grounded { self.bob_phase.cos() * amp * bob_rules().sway_ratio } else { 0.0 };
        // View-relative: `eye_y()` minus the floor, so the level's offset never moves the picture.
        vec3(
            self.pos.0 + self.yaw.cos() * sway,
            self.eye_y() - self.floor_y + bob,
            self.pos.1 - self.yaw.sin() * sway,
        )
    }

    pub fn look(&mut self, d: Vec2) {
        self.yaw -= d.x;
        let lim = crate::config::look::cfg().pitch_limit_rad;
        self.pitch = (self.pitch - d.y).clamp(-lim, lim);
    }

    pub fn recover_stamina(&mut self, dt: f32) {
        let regen = if self.stim_crash > 0.0 {
            STIM_CRASH_REGEN_SCALE
        } else {
            1.0
        };
        self.stamina = (self.stamina + STAMINA_RECOVERY * regen * dt).min(MAX_STAMINA);
        if self.exhausted && self.stamina >= STAMINA_EXHAUSTED_RECOVERY {
            self.exhausted = false;
        }
    }

    /// Relabels the floor the body is measured from without moving it: `y()` is unchanged (the height absorbs the
    /// difference), and the jump offset, vertical speed, grounded flag, position, facing, bob and velocity carry over.
    pub fn rebase_floor(&mut self, new_floor_y: f32) {
        let y = self.y();
        self.floor_y = new_floor_y;
        self.height = y - new_floor_y;
    }

    fn move_body(&mut self, fac: &Facility, stack: &LevelStack, dx: f32, dy: f32) {
        let total = (dx * dx + dy * dy).sqrt();
        let steps = ((total / 0.08).ceil() as i32).max(1);
        let start = self.pos;
        for _ in 0..steps {
            let nx = self.pos.0 + dx / steps as f32;
            if self.noclip || self.fits(fac, stack, nx, self.pos.1) {
                self.pos.0 = nx;
            }
            let ny = self.pos.1 + dy / steps as f32;
            if self.noclip || self.fits(fac, stack, self.pos.0, ny) {
                self.pos.1 = ny;
            }
        }
        self.walked += ((self.pos.0 - start.0).powi(2) + (self.pos.1 - start.1).powi(2)).sqrt();
    }

    /// Whether the body fits at (x, z) standing on its current surface (feet y = floor + height, not the jump offset).
    fn fits(&self, fac: &Facility, stack: &LevelStack, x: f32, z: f32) -> bool {
        body_fits_at_y(fac, stack, x, z, self.floor_y + self.height, PLAYER_RADIUS)
    }

    pub fn update_movement(
        &mut self,
        dt: f32,
        c: &Controls,
        prof: &Profile,
        fac: &Facility,
        stack: &LevelStack,
        meters: &mut Meters,
        _sounds: &Sounds,
    ) {
        let jump_edge = c.jump && !self.jump_prev;
        self.jump_prev = c.jump;
        self.noise_burst_timer = (self.noise_burst_timer - dt).max(0.0);
        self.stun = (self.stun - dt).max(0.0);
        self.pesticide = (self.pesticide - dt).max(0.0);
        self.update_flashlight_flicker(dt);
        if self.in_vent {
            self.vent_time += dt;
        }
        if self.stim_inject > 0.0 {
            self.stim_inject -= dt;
            if self.stim_inject <= 0.0 {
                self.stamina = MAX_STAMINA;
                self.exhausted = false;
                self.stim_boost = STIM_BOOST_SECONDS;
                crate::audio::sound::recharge(MAX_STAMINA);
            }
        } else if self.stim_boost > 0.0 {
            self.stim_boost -= dt;
            if self.stim_boost <= 0.0 {
                self.stim_crash = STIM_CRASH_SECONDS;
            }
        } else if self.stim_crash > 0.0 {
            self.stim_crash -= dt;
        }
        // the crash heart: heartbeat(.65) at once, then every 0.8 s (player_controller_3d.gd 3346-3353)
        crate::audio::sound::stim_crash(dt, self.stim_boost <= 0.0 && self.stim_crash > 0.0);

        let mut input = vec2(c.strafe, c.fwd);
        if self.stun > 0.0 {
            input = Vec2::ZERO;
        }
        if input.length() > 1.0 {
            input = input.normalize();
        }
        let moving = input.length_squared() > 0.0;
        self.crouching = c.crouch;
        let target = if self.crouching { 1.0 } else { 0.0 };
        self.crouch_t += (target - self.crouch_t) * (1.0 - (-14.0 * dt).exp());
        // A flight is no special ground for the speed (Godot has no stair code): walk, sprint and crouch follow the flat
        // rules on it, and a sprint held through a fall keeps its full speed (the air control scale is for walking).
        let airborne = self.falling();
        let wants_sprint = c.sprint && moving && !self.exhausted;
        let mut sprinting = false;
        let speed = if self.crouching {
            PLAYER_CROUCH_SPEED * prof.crouch_speed
        } else if wants_sprint && self.stamina > 0.0 {
            sprinting = true;
            PLAYER_SPRINT_SPEED
                * prof.sprint_speed
                * if self.stim_boost > 0.0 {
                    STIM_SPEED_SCALE
                } else {
                    1.0
                }
        } else {
            PLAYER_WALK_SPEED * prof.player_speed
        } * meters.movement_scale()
            * self.speed_scale
            * fall_rules().air_scale(airborne && !sprinting);
        self.sprinting = sprinting;
        self.moving = moving;
        self.noise = if !moving {
            0.0
        } else if self.crouching {
            NOISE_CROUCH
        } else if sprinting {
            NOISE_SPRINT
        } else {
            NOISE_WALK
        };
        if self.noise_burst_timer > 0.0 {
            self.noise = self.noise.max(self.noise_burst);
        }
        if sprinting {
            let scale = if self.stim_boost > 0.0 {
                STIM_DRAIN_SCALE
            } else {
                1.0
            };
            // Native 0x14008a600: frame delta is multiplied by the profile/stim
            // drain scale, then the meter is clamped at zero.
            let native_sprint_drain = STAMINA_SPRINT_DRAIN * scale * meters.drain_scale();
            self.stamina = (self.stamina - native_sprint_drain * dt).max(0.0);
            if self.stamina <= 0.0 {
                self.exhausted = true;
            }
        } else {
            self.recover_stamina(dt);
        }
        if self.flashlight_on {
            // Native 0x140067e40/0x14008a600: idle drain plus the sprint
            // surcharge, all applied once per frame and clamped at zero.
            let mut native_lamp_drain = LAMP_IDLE_DRAIN * prof.lamp_drain;
            if sprinting {
                native_lamp_drain += LAMP_SPRINT_DRAIN * prof.sprint_drain;
            }
            self.lamp = (self.lamp - native_lamp_drain * meters.drain_scale() * dt).max(0.0);
        }

        let (fx, fy) = self.forward2();
        let (rx, ry) = (self.yaw.cos(), -self.yaw.sin());
        let before = self.pos;
        if moving {
            self.move_body(
                fac,
                stack,
                (rx * input.x + fx * input.y) * speed * dt,
                (ry * input.x + fy * input.y) * speed * dt,
            );
        }
        let speed_now = ((self.pos.0 - before.0).powi(2) + (self.pos.1 - before.1).powi(2)).sqrt()
            / dt.max(1e-4);
        self.move_vel = ((self.pos.0 - before.0) / dt.max(1e-4), (self.pos.1 - before.1) / dt.max(1e-4));

        // The ground under the body in the absolute frame (the active level, else a resident neighbour). On a flight, or
        // past its end on the neighbour's floor, the body's height follows it; on the level's own deck the deck is the
        // lowest the jump offset can go (`min_feet`) and the height stays 0.
        let under = ground_y(fac, stack, self.pos.0, self.pos.1, self.y() + self.feet_y);
        let rel = under.y - self.floor_y;
        // a neighbour's surface is followed only when it is within a step of the feet (the flight's own continuation);
        // a floor far below the feet is a hole or a shaft (a ladder's), which the body does not drop into by easing
        let neighbour = under.level != fac.level && (under.y - self.y()).abs() <= STEP_UP;
        let on_flight = stair_height_at(fac, self.pos).is_some();
        // any surface of the level's own window (the floor, a catwalk deck, the Inferno's noisy floor, above or below the
        // level floor) is carried in `height` (relative to the level floor); `feet_y` is only the jump offset, so `y()` is
        // the true absolute y and the collision and the ladder read the same number
        let on_deck = under.level == fac.level;
        let lifted = neighbour || on_flight || on_deck;
        // No ground within a step under a standing body (a drop cell, a ladder shaft, a hole): the body falls. A jump in
        // the air over such a place falls too, from the height of its walking surface.
        if !self.falling() && !self.noclip && !on_flight {
            let rest = self.y() + if self.grounded { self.feet_y } else { 0.0 };
            if fall_rules().starts_fall(rest - under.y) {
                self.height += self.feet_y;
                self.feet_y = 0.0;
                self.grounded = false;
                self.fall_top = Some(self.y());
            }
        }
        if self.falling() {
            self.fall_step(dt, fac, stack);
            self.after_move(dt, moving, sprinting, speed, speed_now);
            return;
        }
        let (surface, min_feet) = if lifted { (rel, 0.0) } else { (0.0, rel) };
        if self.feet_y <= min_feet {
            self.feet_y = min_feet;
            self.vy = 0.0;
            if jump_edge && !self.crouching && self.stun <= 0.0 {
                self.vy = PLAYER_JUMP_VELOCITY;
            }
        }
        // On the Inferno's noisy floor a grounded body walking downhill sticks to the surface instead of dropping in
        // small hops behind it (the data's `ground_snap`); a jump (vy above 0) or a real drop is left alone.
        if let Some(t) = fac.terrain() {
            if self.grounded && self.vy <= 0.0 && self.feet_y - min_feet <= t.rules().ground_snap {
                self.feet_y = min_feet;
                self.vy = 0.0;
            }
        }
        self.vy -= PLAYER_GRAVITY * dt;
        self.feet_y = (self.feet_y + self.vy * dt).max(min_feet);
        self.grounded = self.feet_y <= min_feet;
        // The body rises and sinks with the flight under it: the height eases toward the surface it now stands on, and
        // back to the floor off the stairs, so it never snaps through a tread.
        self.height += (surface - self.height) * (1.0 - (-STAIR_FOLLOW_RATE * dt).exp());
        // a deck or a noisy cave floor is continuous by construction (the slope per cell is capped), so the body stands
        // on it exactly: only a flight's treads are eased
        if on_deck && !neighbour && !on_flight {
            self.height = surface;
        }

        self.after_move(dt, moving, sprinting, speed, speed_now);
    }

    /// One frame of a fall: the height integrates the vertical speed directly (so `y()` is the true absolute y), the
    /// horizontal control was applied by the caller, and the body touches down when its feet reach the ground under it
    /// (measured from where the frame began, so a floor between two frames is never passed).
    fn fall_step(&mut self, dt: f32, fac: &Facility, stack: &LevelStack) {
        let y0 = self.y();
        let ground = ground_y(fac, stack, self.pos.0, self.pos.1, y0);
        self.vy -= PLAYER_GRAVITY * dt;
        let y1 = y0 + self.vy * dt;
        self.grounded = false;
        self.fall_top = self.fall_top.map(|t| t.max(y1));
        // a surface far above the feet is not ground (the body is under an overhang); only one within a step is
        if y1 <= ground.y && ground.y <= y0 + STEP_UP {
            let top = self.fall_top.take().unwrap_or(y0);
            let speed = self.vy.abs();
            self.height = ground.y - self.floor_y;
            self.feet_y = 0.0;
            self.vy = 0.0;
            self.grounded = true;
            let drop_height = top - ground.y;
            if fall_rules().reports_landing(drop_height) {
                self.landing = Some(Landing { y: ground.y, drop_height, speed, pos: self.pos });
            }
        } else {
            self.height = y1 - self.floor_y;
        }
    }

    /// The head bob and the footsteps of the frame (only on the ground).
    fn after_move(&mut self, dt: f32, moving: bool, sprinting: bool, speed: f32, speed_now: f32) {
        if moving && self.grounded && speed_now > 0.3 {
            self.bob_phase = (self.bob_phase + dt * speed_now * bob_rules().phase_per_metre).rem_euclid(std::f32::consts::TAU);
        } else {
            // ease back to rest instead of freezing mid-stride (Godot 1009-1011: move_toward 0)
            let step = dt * bob_rules().ease_rate;
            self.bob_phase = if self.bob_phase.abs() <= step { 0.0 } else { self.bob_phase - step * self.bob_phase.signum() };
        }
        // Sound.footstep every frame on the floor (player_controller_3d.gd 1987-1993): the cadence is stride / ground
        // speed, no step under 0.3 m/s, feet alternate
        if moving && self.grounded {
            let gait = if self.crouching {
                "crouch"
            } else if sprinting {
                "sprint"
            } else {
                "walk"
            };
            crate::audio::sound::footstep(gait, "", speed_now.min(speed * 1.5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = 0.01;

    #[test]
    fn flicker_turns_the_beam_off_and_back_on_within_its_duration() {
        let mut p = Player::new((0.0, 0.0));
        p.flashlight_on = true;
        p.lamp = MAX_LAMP_CHARGE;
        assert!(p.beam_lit());

        p.force_flashlight_flicker(1.0);
        assert!(p.flashlight_seized());

        let mut elapsed = 0.0;
        let mut saw_dark = false;
        let mut saw_lit_after_dark = false;
        while p.flashlight_seized() {
            p.update_flashlight_flicker(STEP);
            elapsed += STEP;
            assert!(elapsed < 1.0 + 2.0 * STEP, "flicker ran past its duration");
            if p.flashlight_seized() {
                if !p.beam_lit() {
                    saw_dark = true;
                } else if saw_dark {
                    saw_lit_after_dark = true;
                }
            }
        }
        assert!(saw_dark, "the beam never went dark during the flicker");
        assert!(
            saw_lit_after_dark,
            "the beam never came back on during the flicker"
        );
        assert!(p.beam_lit());
        assert!(p.flashlight_on);
    }

    #[test]
    fn flicker_restores_the_players_switch_and_never_lights_an_off_beam() {
        let mut p = Player::new((0.0, 0.0));
        p.lamp = MAX_LAMP_CHARGE;
        p.force_flashlight_flicker(0.5);
        for _ in 0..60 {
            p.update_flashlight_flicker(STEP);
            assert!(!p.beam_lit());
        }
        assert!(!p.flashlight_seized());
        assert!(!p.flashlight_on);
        assert!(!p.beam_lit());
    }

    use crate::audio::Sounds;
    use crate::config::{Difficulty, CAMERA_HEIGHT};
    use crate::content::Content;
    use crate::game::{Controls, Game};
    use crate::world::catalog::Catalog;
    use crate::world::facility::{Cell, Stair};

    const FRAME: f32 = 1.0 / 60.0;

    /// The first window of a fixed run that has a stair flight, and that flight (an upward one when it has one).
    fn flight_game() -> (Game, Stair) {
        let content = Content::load();
        let catalog = Catalog::load();
        for n in 0..40 {
            for level in [0, -1] {
                let text = format!("ROOM-TEST-{n}");
                let g = Game::new_infinite_sim(Difficulty::Standard, 7, &text, level, &content, &catalog);
                let flight = g
                    .fac
                    .stairs
                    .iter()
                    .copied()
                    .filter(|s| matches!(s.kind, LinkKind::Stairs))
                    .max_by_key(|s| s.dir);
                if let Some(s) = flight {
                    return (g, s);
                }
            }
        }
        panic!("no window in the first 40 test rooms has a stair flight");
    }

    /// The flight's walk path (landing A to foot B, 0.25 m apart) on the level that holds `s`.
    fn path(g: &Game, s: Stair) -> Vec<(f32, f32)> {
        s.walk_path(g.fac.level, 0.25)
    }

    /// A point on the flight, `k` samples along its walk path.
    fn on_flight(g: &Game, s: Stair, k: usize) -> (f32, f32) {
        let p = path(g, s);
        p[k.min(p.len() - 1)]
    }

    /// The yaw that faces along the direction `d` (forward is (-sin, -cos)).
    fn yaw_along(d: (f32, f32)) -> f32 {
        (-d.0).atan2(-d.1)
    }

    /// Stands the body at the flight's first path point facing the next ones.
    fn start_on_flight(g: &mut Game, s: Stair) {
        let p = path(g, s);
        g.p.pos = p[0];
        g.p.yaw = yaw_along((p[3].0 - p[0].0, p[3].1 - p[0].1));
    }

    /// A point just behind the landing A, off the stair.
    fn off_stair(s: Stair) -> (f32, f32) {
        let (cx, cy) = s.landing().center();
        (cx - s.run.0 as f32, cy - s.run.1 as f32)
    }

    /// Runs the body for `secs` under `c`, through the same call the game makes each frame.
    fn run_body(g: &mut Game, secs: f32, c: &Controls) {
        let prof = g.prof();
        let sounds = Sounds::default();
        for _ in 0..(secs / FRAME).round() as i32 {
            g.p.update_movement(FRAME, c, &prof, &g.fac, &LevelStack::EMPTY, &mut g.meters, &sounds);
        }
    }

    #[test]
    fn walking_records_a_velocity_and_a_storey_change_keeps_it() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s) = flight_game();
        start_on_flight(&mut g, s);
        run_body(&mut g, 0.15, &Controls { fwd: 1.0, ..Default::default() });
        let v = g.p.move_vel;
        assert!(v.0.hypot(v.1) > 0.5, "the walk left a velocity: {v:?}");
        // the change to the storey below (flight_game's levels are 0 and -1, so `level - 1` is always allowed)
        let to = g.inf.as_ref().map_or(0, |i| i.level) - 1;
        g.go_to_level(to, g.p.pos, g.p.yaw, &content, &catalog);
        assert_eq!(g.p.move_vel, v, "the momentum is carried across the level change");
    }

    #[test]
    fn a_body_standing_still_has_no_velocity() {
        let (mut g, s) = flight_game();
        g.p.pos = off_stair(s);
        run_body(&mut g, 0.2, &Controls::default());
        assert_eq!(g.p.move_vel, (0.0, 0.0));
    }

    #[test]
    fn stepping_onto_a_flight_raises_the_body_by_the_tread_heights() {
        let (mut g, s) = flight_game();
        // from the landing, walk onto the flight along its path
        start_on_flight(&mut g, s);
        let start = g.p.pos;
        run_body(&mut g, 0.15, &Controls { fwd: 1.0, ..Default::default() });
        let moved = ((g.p.pos.0 - start.0).powi(2) + (g.p.pos.1 - start.1).powi(2)).sqrt();
        assert!(moved > 0.2, "the body walked onto the flight: {moved} m");
        let surface = s.surface_rel(g.fac.level, g.p.pos.0, g.p.pos.1).expect("the body is on the flight");
        assert!(stair_height_at(&g.fac, g.p.pos).is_some());
        assert!((g.p.height - surface).abs() < 0.1, "height {} follows the tread {surface}", g.p.height);
    }

    #[test]
    fn the_eye_follows_the_body_up_the_steps_and_the_pitch_is_unchanged() {
        let (mut g, s) = flight_game();
        g.p.pos = on_flight(&g, s, 12);
        g.p.pitch = 0.3;
        run_body(&mut g, 2.0, &Controls::default());
        let surface = s.surface_rel(g.fac.level, g.p.pos.0, g.p.pos.1).unwrap();
        let eye = g.p.eye();
        assert!(
            (eye.y - (CAMERA_HEIGHT + surface)).abs() < 1e-3,
            "eye {} is the standing eye on the tread {surface}",
            eye.y
        );
        assert_eq!(g.p.pitch, 0.3);
    }

    /// One frame of the body on the flight window.
    fn step_body(g: &mut Game, c: &Controls) {
        let prof = g.prof();
        let sounds = Sounds::default();
        g.p.update_movement(FRAME, c, &prof, &g.fac, &LevelStack::EMPTY, &mut g.meters, &sounds);
    }

    #[test]
    fn a_jump_on_a_flight_lands_back_on_it_with_y_continuous() {
        let (mut g, s) = flight_game();
        g.p.pos = on_flight(&g, s, 12);
        run_body(&mut g, 0.5, &Controls::default());
        assert!(g.p.grounded && stair_height_at(&g.fac, g.p.pos).is_some());
        let hop = Controls { jump: true, ..Default::default() };
        let (mut last, mut max_step, mut peak) = (g.p.y() + g.p.feet_y, 0.0_f32, 0.0_f32);
        let mut left_ground = false;
        for i in 0..150 {
            let c = if i == 0 { hop.clone() } else { Controls::default() };
            step_body(&mut g, &c);
            let y = g.p.y() + g.p.feet_y;
            max_step = max_step.max((y - last).abs());
            last = y;
            peak = peak.max(g.p.feet_y);
            left_ground |= !g.p.grounded;
            assert!(!g.p.falling(), "a hop on a flight is not a fall (frame {i})");
        }
        assert!(left_ground && peak > 0.3, "the press hops on a stair: peak {peak} m");
        assert!(max_step <= 0.25, "y stays continuous through the hop: worst frame step {max_step}");
        assert!(g.p.grounded && g.p.feet_y == 0.0 && g.p.take_landing().is_none(), "landed on the flight, no fall landing");
        let tread = stair_height_at(&g.fac, g.p.pos).expect("still on the flight");
        assert!((g.p.height - tread).abs() < 0.05, "standing on the tread again: {} vs {tread}", g.p.height);
    }

    #[test]
    fn a_jump_still_works_on_the_floor() {
        let (mut g, s) = flight_game();
        let hop = Controls { jump: true, ..Default::default() };
        g.p.pos = off_stair(s);
        run_body(&mut g, FRAME, &Controls::default());
        run_body(&mut g, FRAME, &hop);
        assert!(g.p.feet_y > 0.0, "a fresh press hops on the floor");
    }

    /// The horizontal speed (m/s) of one frame of `c` from `at` facing `yaw`.
    fn frame_speed(g: &mut Game, at: (f32, f32), yaw: f32, c: &Controls) -> f32 {
        g.p.pos = at;
        g.p.yaw = yaw;
        // settle onto the surface under the spot first (the height eases to a tread), with no key held
        run_body(g, 0.5, &Controls::default());
        g.p.stamina = MAX_STAMINA;
        g.p.exhausted = false;
        step_body(g, c);
        g.p.move_vel.0.hypot(g.p.move_vel.1)
    }

    #[test]
    fn sprint_crouch_and_walk_on_a_flight_run_at_the_flat_speeds() {
        let (mut g, s) = flight_game();
        let prof = g.prof();
        let ms = g.meters.movement_scale();
        let (walk, sprint, crouch) = (
            PLAYER_WALK_SPEED * prof.player_speed * ms,
            PLAYER_SPRINT_SPEED * prof.sprint_speed * ms,
            PLAYER_CROUCH_SPEED * prof.crouch_speed * ms,
        );
        assert!(crouch < walk && walk < sprint, "{crouch} {walk} {sprint}");
        let p = path(&g, s);
        let (on, yaw_on) = (p[12], yaw_along((p[15].0 - p[12].0, p[15].1 - p[12].1)));
        // a flat spot: just behind the landing, walking away from the flight
        let (off, yaw_off) = (off_stair(s), yaw_along((-(s.run.0 as f32), -(s.run.1 as f32))));
        let fwd = Controls { fwd: 1.0, ..Default::default() };
        let run = Controls { fwd: 1.0, sprint: true, ..Default::default() };
        let duck = Controls { fwd: 1.0, crouch: true, ..Default::default() };
        for (name, c, want) in [("walk", &fwd, walk), ("sprint", &run, sprint), ("crouch", &duck, crouch)] {
            // the crouch settles over a few frames; the speed does not wait for it
            let flat = (0..4).map(|k| frame_speed(&mut g, off, yaw_off + k as f32 * std::f32::consts::FRAC_PI_2, c)).fold(0.0_f32, f32::max);
            let stairs = frame_speed(&mut g, on, yaw_on, c);
            assert!((flat - want).abs() < 0.01, "{name} on the flat: {flat} vs {want}");
            assert!((stairs - want).abs() < 0.01, "{name} on a flight: {stairs} vs the flat {want}");
        }
        frame_speed(&mut g, on, yaw_on, &run);
        assert!(g.p.sprinting, "the sprint flag is set on a flight");
    }

    #[test]
    fn a_sprinting_climb_and_a_hopping_climb_keep_y_continuous_and_never_fall() {
        use crate::world::flight::FlightShape::*;
        let mut checked = 0;
        for (shape, level) in [(Switchback, 0), (Straight, -1), (Spiral, 0)] {
            let Some((mut g, s)) = shape_game(shape, level) else { continue };
            // sprint all the way, then hop every 20 frames
            for hop in [false, true] {
                g.p.set_abs_y(g.p.floor_y);
                g.p.stamina = MAX_STAMINA;
                g.p.exhausted = false;
                let (steps, grounded_always, lag, fell) = climb_steps_with(&mut g, s, |i| Controls {
                    fwd: 1.0,
                    sprint: true,
                    jump: hop && (i / 10) % 2 == 0,
                    ..Default::default()
                });
                assert!(steps.len() > 20, "{shape:?}: the body walked the flight");
                let worst = steps.iter().cloned().fold(0.0, f32::max);
                assert!(worst < 0.25, "{shape:?} hop {hop}: a frame step of {worst} m");
                assert!(lag < 0.3, "{shape:?} hop {hop}: the height lags the tread by {lag} m");
                assert!(!fell, "{shape:?} hop {hop}: no fall started on the flight");
                assert!(grounded_always || hop, "{shape:?}: a sprint alone stays on the treads");
            }
            checked += 1;
        }
        assert!(checked >= 1, "at least one shape was found");
    }

    #[test]
    fn a_sprint_held_through_a_fall_keeps_its_full_speed() {
        let (mut g, _, run) = drop_game();
        g.p.yaw = yaw_along((run.0 as f32, run.1 as f32));
        let start = g.p.pos;
        let c = Controls { fwd: 1.0, sprint: true, ..Default::default() };
        let dt = 1.0 / 120.0;
        for _ in 0..16 {
            step_in_stack(&mut g, &c, dt);
        }
        assert!(g.p.falling(), "still in the air");
        assert!(g.p.sprinting, "the sprint carries into the fall");
        let moved = (g.p.pos.0 - start.0).hypot(g.p.pos.1 - start.1);
        let prof = g.prof();
        let full = PLAYER_SPRINT_SPEED * prof.sprint_speed * g.meters.movement_scale() * 16.0 * dt;
        assert!(moved > full * 0.85 && moved < full * 1.15, "moved {moved} m, a full sprint gives {full} m (air control would give {})", full * fall_rules().air_control);
    }

    #[test]
    fn leaving_the_stair_returns_the_height_to_zero() {
        let (mut g, s) = flight_game();
        g.p.pos = on_flight(&g, s, 12);
        run_body(&mut g, 2.0, &Controls::default());
        assert!(g.p.height.abs() > 0.1, "up on the flight: {}", g.p.height);
        g.p.pos = off_stair(s);
        run_body(&mut g, 2.0, &Controls::default());
        assert!(g.p.height.abs() < 1e-3, "back on the floor: {}", g.p.height);
    }
    #[test]
    fn y_is_the_floor_plus_the_height_and_the_eye_sits_on_it() {
        let mut p = Player::new((1.0, 1.0));
        p.floor_y = -5.0;
        p.height = 0.75;
        assert_eq!(p.y(), -5.0 + 0.75);
        p.feet_y = 0.2;
        assert!((p.eye_y() - (p.y() + 0.2 + p.eye_height())).abs() < 1e-6);
    }

    #[test]
    fn the_water_sink_lowers_only_the_eye() {
        let mut p = Player::new((1.0, 1.0));
        p.floor_y = -5.0;
        let dry = p.eye_y();
        assert_eq!(p.water_sink, 0.0);
        p.water_sink = 0.8;
        assert!((p.eye_y() - (p.y() + p.feet_y + p.eye_height() - 0.8)).abs() < 1e-6);
        assert!((dry - p.eye_y() - 0.8).abs() < 1e-6, "the eye drops by the sink");
        assert_eq!(p.y(), -5.0, "the feet stay on the walkway");
        // the hunters' sight line reads `eye_y() - y()`: the lowered eye
        assert!((p.eye_y() - p.y() - (p.eye_height() - 0.8)).abs() < 1e-6);
        p.place((2.0, 2.0));
        assert_eq!(p.water_sink, 0.0, "placing the runner dries the eye");
    }

    #[test]
    fn changing_the_floor_moves_the_absolute_eye_but_not_the_view() {
        let mut p = Player::new((1.0, 1.0));
        p.height = 0.5;
        let (abs0, view0) = (p.eye_y(), p.eye().y);
        p.floor_y = -10.0;
        assert!((p.eye_y() - (abs0 - 10.0)).abs() < 1e-5, "the absolute eye moved by the floor");
        assert!((p.eye().y - view0).abs() < 1e-5, "the picture did not move");
    }

    #[test]
    fn grounded_follows_the_hop() {
        let (mut g, s) = flight_game();
        g.p.pos = off_stair(s);
        run_body(&mut g, 0.1, &Controls::default());
        assert!(g.p.grounded);
        run_body(&mut g, FRAME, &Controls { jump: true, ..Default::default() });
        assert!(!g.p.grounded && g.p.feet_y > 0.0, "in the air after the press");
        run_body(&mut g, 3.0, &Controls::default());
        assert!(g.p.grounded && g.p.feet_y == 0.0, "landed");
    }

    #[test]
    fn a_floor_offset_does_not_change_the_walk() {
        let (mut a, s) = flight_game();
        let (mut b, _) = flight_game();
        b.p.floor_y = -5.0;
        for g in [&mut a, &mut b] {
            start_on_flight(g, s);
            run_body(g, 0.15, &Controls { fwd: 1.0, jump: true, ..Default::default() });
        }
        assert_eq!(a.p.pos, b.p.pos);
        assert_eq!(a.p.feet_y, b.p.feet_y);
        assert_eq!(a.p.height, b.p.height);
        assert!((a.p.eye().y - b.p.eye().y).abs() < 1e-5);
    }

    /// A real window with a Stairs flight of `shape` on `level` (scanning ROOM-TEST-n, n < 300).
    fn shape_game(shape: crate::world::flight::FlightShape, level: i32) -> Option<(Game, Stair)> {
        let content = Content::load();
        let catalog = Catalog::load();
        for n in 0..300 {
            let text = format!("ROOM-TEST-{n}");
            let g = Game::new_infinite_sim(Difficulty::Standard, 7, &text, level, &content, &catalog);
            let found = g
                .fac
                .stairs
                .iter()
                .copied()
                .find(|s| matches!(s.kind, LinkKind::Stairs) && s.shape == shape);
            if let Some(s) = found {
                return Some((g, s));
            }
        }
        None
    }

    #[test]
    fn the_body_fits_along_the_whole_walk_path_of_every_shape() {
        use crate::world::flight::FlightShape::*;
        let mut checked = 0;
        for (shape, level) in [(Switchback, 0), (Spiral, 0), (Straight, -1)] {
            let Some((g, s)) = shape_game(shape, level) else { continue };
            let p = path(&g, s);
            assert!(p.len() > 8, "{shape:?}: a real path");
            // The walk path ends in the centre of the OTHER end's cell: that cell is the other storey's floor, held
            // by the other level's facility, so only the samples strictly inside it are not this level's flight.
            let other = if s.dir < 0 { s.foot() } else { s.landing() };
            let (ox, oy) = (other.x as f32, other.y as f32);
            for (i, pt) in p.iter().enumerate() {
                if pt.0 > ox - 0.25 && pt.0 < ox + 1.25 && pt.1 > oy - 0.25 && pt.1 < oy + 1.25 {
                    continue;
                }
                let h = s
                    .surface_rel(g.fac.level, pt.0, pt.1)
                    .unwrap_or_else(|| panic!("{shape:?}: path sample {i} {pt:?} is on the flight"));
                // a spiral's end radial is a rail (the slab of upper floor stands a storey over the lowest step): the
                // route crosses it where the walker changes owner, and a body is kept off it
                if s.near_cliff(g.fac.level, pt.0, pt.1, 0.3) {
                    continue;
                }
                assert!(body_fits(&g.fac, pt.0, pt.1, h), "{shape:?}: the body fits at path sample {i} {pt:?}");
            }
            checked += 1;
        }
        assert!(checked >= 1, "at least one shape was found in the scanned windows");
    }

    /// Every flight found over a sweep of windows on levels 0 and -1 (all three shapes, both ends of the connection):
    /// the body fits at every sample of its walk path except inside the other end's cell.
    #[test]
    fn the_body_fits_along_the_walk_path_of_every_flight_in_a_sweep() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut seen = std::collections::BTreeMap::new();
        let mut bad = Vec::new();
        for level in [0, -1] {
            for n in 0..60 {
                let text = format!("ROOM-TEST-{n}");
                let g = Game::new_infinite_sim(Difficulty::Standard, 7, &text, level, &content, &catalog);
                for s in g.fac.stairs.iter().copied().filter(|s| matches!(s.kind, LinkKind::Stairs)) {
                    *seen.entry((format!("{:?}", s.shape), level)).or_insert(0) += 1;
                    let other = if s.dir < 0 { s.foot() } else { s.landing() };
                    let (ox, oy) = (other.x as f32, other.y as f32);
                    for (i, pt) in s.walk_path(g.fac.level, 0.25).into_iter().enumerate() {
                        if pt.0 > ox - 0.25 && pt.0 < ox + 1.25 && pt.1 > oy - 0.25 && pt.1 < oy + 1.25 {
                            continue;
                        }
                        if s.near_cliff(g.fac.level, pt.0, pt.1, 0.3) {
                            continue;
                        }
                        let fits = s.surface_rel(g.fac.level, pt.0, pt.1).is_some_and(|h| body_fits(&g.fac, pt.0, pt.1, h));
                        if !fits {
                            bad.push(format!("{:?} L{level} {text} stair {:?} sample {i} {pt:?}", s.shape, s.cell));
                            break;
                        }
                    }
                }
            }
        }
        assert!(seen.keys().any(|(sh, _)| sh == "Spiral"), "the sweep met a spiral: {seen:?}");
        assert!(seen.keys().any(|(sh, _)| sh == "Switchback") && seen.keys().any(|(sh, _)| sh == "Straight"), "{seen:?}");
        assert!(bad.is_empty(), "the body is blocked on the walk path ({seen:?}): {bad:#?}");
    }

    #[test]
    fn rebase_keeps_y_velocity_and_facing() {
        let mut p = Player::new((3.0, 4.0));
        p.floor_y = -5.0;
        p.height = 0.6;
        p.feet_y = 0.3;
        p.vy = 1.5;
        p.grounded = false;
        p.yaw = 0.7;
        p.bob_phase = 2.0;
        p.move_vel = (1.0, -2.0);
        let (y, eye) = (p.y(), p.eye_y());
        p.rebase_floor(0.0);
        assert!((p.y() - y).abs() < 1e-5 && (p.eye_y() - eye).abs() < 1e-5);
        assert_eq!(p.floor_y, 0.0);
        assert!((p.height - (-4.4)).abs() < 1e-5);
        assert_eq!((p.feet_y, p.vy, p.grounded), (0.3, 1.5, false));
        assert_eq!((p.pos, p.yaw, p.bob_phase, p.move_vel), ((3.0, 4.0), 0.7, 2.0, (1.0, -2.0)));
    }

    #[test]
    fn rebase_up_then_down_is_identity() {
        let mut p = Player::new((1.0, 1.0));
        p.floor_y = -5.0;
        p.height = 1.25;
        let (f, h) = (p.floor_y, p.height);
        p.rebase_floor(0.0);
        p.rebase_floor(f);
        assert_eq!(p.floor_y, f);
        assert!((p.height - h).abs() < 1e-5);
    }

    /// Walks the flight of `s` along its path: every frame's change of y(), whether the body stayed grounded, and the
    /// worst lag of the height behind the tread under it.
    fn climb_steps(g: &mut Game, s: Stair) -> (Vec<f32>, bool, f32) {
        let (steps, grounded, lag, _) = climb_steps_with(g, s, |_| Controls { fwd: 1.0, ..Default::default() });
        (steps, grounded, lag)
    }

    /// `climb_steps` with the controls of frame `i` chosen by `controls`; the fourth value is whether a fall started.
    fn climb_steps_with(g: &mut Game, s: Stair, controls: impl Fn(usize) -> Controls) -> (Vec<f32>, bool, f32, bool) {
        start_on_flight(g, s);
        let prof = g.prof();
        let sounds = Sounds::default();
        let p = path(g, s);
        let (mut steps, mut grounded, mut worst_lag, mut fell) = (Vec::new(), true, 0.0_f32, false);
        let mut last = g.p.y() + g.p.feet_y;
        for frame in 0..(p.len() * 40) {
            let here = g.p.pos;
            let k = p
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let da = (a.1 .0 - here.0).hypot(a.1 .1 - here.1);
                    let db = (b.1 .0 - here.0).hypot(b.1 .1 - here.1);
                    da.total_cmp(&db)
                })
                .map_or(0, |(i, _)| i);
            if k + 3 >= p.len() {
                break;
            }
            let t = p[k + 3];
            g.p.yaw = yaw_along((t.0 - here.0, t.1 - here.1));
            g.p.update_movement(FRAME, &controls(frame), &prof, &g.fac, &LevelStack::EMPTY, &mut g.meters, &sounds);
            steps.push((g.p.y() + g.p.feet_y - last).abs());
            last = g.p.y() + g.p.feet_y;
            fell |= g.p.falling();
            grounded &= g.p.grounded;
            if let Some(h) = stair_height_at(&g.fac, g.p.pos) {
                worst_lag = worst_lag.max((h - g.p.height).abs());
            }
        }
        (steps, grounded, worst_lag, fell)
    }

    #[test]
    fn a_body_climbing_a_flight_never_steps_more_than_a_quarter_metre_per_frame() {
        use crate::world::flight::FlightShape::*;
        let mut checked = 0;
        for (shape, level) in [(Switchback, 0), (Straight, -1), (Spiral, 0)] {
            let Some((mut g, s)) = shape_game(shape, level) else { continue };
            let (steps, grounded, lag) = climb_steps(&mut g, s);
            assert!(steps.len() > 20, "{shape:?}: the body walked the flight");
            let worst = steps.iter().cloned().fold(0.0, f32::max);
            assert!(worst < 0.25, "{shape:?}: a frame step of {worst} m");
            assert!(lag < 0.25, "{shape:?}: the height lags the tread by {lag} m");
            assert!(grounded, "{shape:?}: the body left the ground on the flight");
            checked += 1;
        }
        assert!(checked >= 1, "at least one shape was found");
    }

    #[test]
    fn a_jump_over_the_well_does_not_teleport() {
        let mut p = Player::new((2.0, 2.0));
        p.floor_y = 0.0;
        p.height = 0.0;
        p.feet_y = 0.8;
        p.vy = 2.0;
        let (y, eye) = (p.y(), p.eye_y());
        p.rebase_floor(-5.0);
        assert_eq!(p.feet_y, 0.8, "the jump offset survives the relabel");
        assert!((p.y() - y).abs() < 1e-5 && (p.eye_y() - eye).abs() < 1e-5);
    }

    /// A start window of a fixed run with a ladder going down, both levels resident, and the body standing in the hatch
    /// cell at the upper floor (as a body that just let go of the top of the shaft).
    fn shaft_game() -> (Game, Stair) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
        let s = g
            .fac
            .stairs
            .iter()
            .copied()
            .find(|s| s.is_ladder() && s.dir < 0)
            .expect("the start window has a ladder down");
        g.p.pos = s.cell.center();
        g.p.set_abs_y(floor_y_of(g.fac.level));
        g.keep_stack(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(g.fac.level - 1), "the level below is resident beside the ladder");
        (g, s)
    }

    /// One frame of the body against the game's own level stack.
    fn step_in_stack(g: &mut Game, c: &Controls, dt: f32) {
        let prof = g.prof();
        let sounds = Sounds::default();
        let empty = LevelStack::EMPTY;
        let stack = g.inf.as_ref().map_or(&empty, |i| &i.stack);
        g.p.update_movement(dt, c, &prof, &g.fac, stack, &mut g.meters, &sounds);
    }

    /// Falls from the hatch to the floor below at `dt` a frame: the time it took and the landing.
    fn fall_down_the_shaft(g: &mut Game, dt: f32) -> (f32, Landing) {
        let mut t = 0.0;
        while t < 3.0 {
            step_in_stack(g, &Controls::default(), dt);
            t += dt;
            if let Some(l) = g.p.take_landing() {
                return (t, l);
            }
        }
        panic!("the body never landed: y {} falling {}", g.p.y(), g.p.falling());
    }

    #[test]
    fn fall_time_matches_sqrt_2_storey_over_g() {
        let (mut g, _) = shaft_game();
        let (t, l) = fall_down_the_shaft(&mut g, 1.0 / 240.0);
        let want = (2.0 * STOREY_DROP / PLAYER_GRAVITY).sqrt();
        assert!((t - want).abs() < 0.06, "fell in {t} s, free fall takes {want} s");
        assert!((l.y - (g.fac.level - 1) as f32 * STOREY_DROP).abs() < 1e-3, "landed on the floor below: {}", l.y);
    }

    /// One storey in metres (the drop from a hatch to the floor below).
    const STOREY_DROP: f32 = crate::world::vertical::STOREY;

    #[test]
    fn landing_reports_height_and_speed() {
        let (mut g, s) = shaft_game();
        let (_, l) = fall_down_the_shaft(&mut g, 1.0 / 120.0);
        assert!((l.drop_height - STOREY_DROP).abs() < 0.05, "dropped a storey: {}", l.drop_height);
        let v = (2.0 * PLAYER_GRAVITY * STOREY_DROP).sqrt();
        assert!((l.speed - v).abs() < 0.6, "touched down at about {v} m/s, not {}", l.speed);
        assert!(((l.pos.0 - s.cell.center().0).hypot(l.pos.1 - s.cell.center().1)) < 1e-3, "no sideways drift: {:?}", l.pos);
        assert!(!g.p.falling() && g.p.grounded && g.p.take_landing().is_none(), "the landing is taken once");
    }

    /// A start window on the first sewer level with a waterfall void, the level below resident, and a drop cell whose
    /// neighbour in direction `run` is a drop too (open air a falling body may steer through). A ladder shaft is one
    /// cell under a slab, so it cannot show the steering: the void is the place a body falls freely.
    fn drop_game() -> (Game, Cell, (i32, i32)) {
        let (content, catalog) = (Content::load(), Catalog::load());
        for n in 0..60 {
            let text = format!("ROOM-TEST-{n}");
            let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &text, -1, &content, &catalog);
            let found = g.fac.drop_cells().iter().copied().find_map(|c| {
                [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .into_iter()
                    .find(|d| g.fac.is_drop(Cell::new(c.x + d.0, c.y + d.1)) && g.fac.is_drop(Cell::new(c.x - d.1, c.y + d.0)))
                    .map(|d| (c, d))
            });
            if let Some((c, d)) = found {
                g.p.pos = c.center();
                g.p.set_abs_y(floor_y_of(g.fac.level));
                g.keep_stack(&content, &catalog);
                assert!(g.inf.as_ref().unwrap().stack.has(g.fac.level - 1), "the level under the void is resident");
                return (g, c, d);
            }
        }
        panic!("no window in the first 60 test rooms has a waterfall void");
    }

    #[test]
    fn air_control_moves_the_body_in_a_fall() {
        let (mut g, _, run) = drop_game();
        g.p.yaw = yaw_along((run.0 as f32, run.1 as f32));
        let start = g.p.pos;
        let c = Controls { fwd: 1.0, ..Default::default() };
        let dt = 1.0 / 120.0;
        for _ in 0..30 {
            step_in_stack(&mut g, &c, dt);
        }
        assert!(g.p.falling(), "still in the air after a quarter second");
        let moved = (g.p.pos.0 - start.0).hypot(g.p.pos.1 - start.1);
        let prof = g.prof();
        let want = PLAYER_WALK_SPEED * prof.player_speed * g.meters.movement_scale() * fall_rules().air_control * 30.0 * dt;
        assert!(moved > 0.0, "the body steers in the air");
        assert!(moved < want * 1.15 && moved > want * 0.85, "moved {moved} m, air speed gives {want} m");
    }

    #[test]
    fn a_drop_under_fall_min_gap_is_a_step() {
        let (mut g, s) = flight_game();
        g.p.pos = off_stair(s);
        // 0.4 m above the floor: a short drop, eased as a step, no fall and no landing
        g.p.feet_y = fall_rules().min_gap * 0.65;
        g.p.grounded = true;
        run_body(&mut g, 1.5, &Controls::default());
        assert!(!g.p.falling() && g.p.take_landing().is_none());
        // 1.5 m above the floor: a fall that reports its landing
        g.p.feet_y = 1.5;
        g.p.grounded = true;
        run_body(&mut g, 0.05, &Controls::default());
        assert!(g.p.falling(), "a drop over the gap is a fall");
        run_body(&mut g, 1.5, &Controls::default());
        let l = g.p.take_landing().expect("the fall landed");
        assert!((l.drop_height - 1.5).abs() < 0.05, "{}", l.drop_height);
        assert!(g.p.height.abs() < 1e-3 && g.p.grounded);
    }

    #[test]
    fn rebase_during_a_fall_keeps_y() {
        let mut p = Player::new((3.0, 4.0));
        p.floor_y = 0.0;
        p.height = -2.0;
        p.vy = -4.0;
        p.grounded = false;
        p.fall_top = Some(0.0);
        let y = p.y();
        p.rebase_floor(-5.0);
        assert!((p.y() - y).abs() < 1e-5, "a relabel moves nothing");
        assert!(p.falling() && p.vy == -4.0 && !p.grounded, "the fall carries on");
    }

    #[test]
    fn set_abs_y_puts_the_feet_there_and_ends_a_fall() {
        let mut p = Player::new((1.0, 1.0));
        p.floor_y = -5.0;
        p.feet_y = 0.5;
        p.vy = 3.0;
        p.grounded = false;
        p.fall_top = Some(1.0);
        p.set_abs_y(-2.0);
        assert_eq!((p.y(), p.feet_y, p.vy, p.grounded, p.falling()), (-2.0, 0.0, 0.0, true, false));
    }

    #[test]
    fn a_held_jump_hops_once_until_released() {
        let (mut g, s) = flight_game();
        g.p.pos = off_stair(s);
        run_body(&mut g, 0.1, &Controls::default());
        let held = Controls { jump: true, ..Default::default() };
        run_body(&mut g, FRAME, &held);
        assert!(g.p.feet_y > 0.0, "the press hops");
        run_body(&mut g, 3.0, &held);
        assert!(g.p.grounded && g.p.feet_y == 0.0, "still held, landed, no second hop");
        run_body(&mut g, FRAME, &Controls::default());
        run_body(&mut g, FRAME, &held);
        assert!(g.p.feet_y > 0.0, "a fresh press hops again");
    }

    #[test]
    fn the_crouched_eye_is_capped_under_the_shortened_capsule() {
        let mut p = Player::new((1.0, 1.0));
        p.crouch_t = 1.0;
        let cap = crate::world::route::sewer::core::CROUCH_COLLISION_HEIGHT as f32 - bob_rules().crouch_eye_clearance;
        assert!(p.eye_height() <= cap + 1e-6, "{} over {cap}", p.eye_height());
        p.crouch_t = 0.0;
        assert!((p.eye_height() - CAMERA_HEIGHT).abs() < 1e-6, "standing is untouched");
    }

    #[test]
    fn the_bob_phase_eases_to_rest_when_the_walk_stops() {
        let (mut g, s) = flight_game();
        g.p.pos = off_stair(s);
        run_body(&mut g, 0.1, &Controls::default());
        g.p.bob_phase = 2.0;
        run_body(&mut g, 0.1, &Controls::default());
        assert!(g.p.bob_phase > 0.0 && g.p.bob_phase < 2.0, "eased, not held or snapped: {}", g.p.bob_phase);
        run_body(&mut g, 1.0, &Controls::default());
        assert_eq!(g.p.bob_phase, 0.0);
    }

    #[test]
    fn the_eye_sways_sideways_only_while_walking() {
        let mut p = Player::new((1.0, 1.0));
        p.grounded = true;
        p.bob_phase = 0.0;
        p.moving = true;
        assert!((p.eye().x - p.pos.0).abs() > 1e-5, "lateral sway while walking");
        p.moving = false;
        assert_eq!((p.eye().x, p.eye().z), p.pos);
    }

    #[test]
    fn the_runner_walks_up_to_a_sewer_railing_and_no_further() {
        use super::super::rail_tests::samples;
        let mut checked = 0;
        for s in samples().iter().take(8) {
            for (c, bit) in s.fac.rail_edges().iter().step_by(5) {
                let st = match *bit {
                    crate::world::facility::RAIL_N => (0.0, -1.0),
                    crate::world::facility::RAIL_E => (1.0, 0.0),
                    crate::world::facility::RAIL_S => (0.0, 1.0),
                    _ => (-1.0, 0.0),
                };
                let mut p = Player::new(c.center());
                p.floor_y = floor_y_of(s.fac.level);
                if !p.fits(&s.fac, &LevelStack::EMPTY, p.pos.0, p.pos.1) {
                    continue;
                }
                // a hard run at the water: the walk and then a lunge, in the frame sizes the game steps in
                for _ in 0..30 {
                    p.move_body(&s.fac, &LevelStack::EMPTY, st.0 * 0.12, st.1 * 0.12);
                }
                p.move_body(&s.fac, &LevelStack::EMPTY, st.0 * 3.0, st.1 * 3.0);
                let cell = crate::world::Cell::new(p.pos.0.floor() as i32, p.pos.1.floor() as i32);
                assert_eq!(cell, *c, "{}: the runner went through the railing of {c:?} to {:?}", s.name, p.pos);
                assert!(p.fits(&s.fac, &LevelStack::EMPTY, p.pos.0, p.pos.1));
                checked += 1;
            }
        }
        assert!(checked > 20, "{checked}");
    }
}
