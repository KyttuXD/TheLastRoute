//! The runner's body (player_controller_3d.gd): walk / sprint / crouch at the original's
//! speeds, the grounded hop, stamina and exhaustion, the flashlight battery, the stim pack,
//! and the eye-aligned bobbing camera.

use super::meters::Meters;
use super::Controls;
use crate::audio::Sounds;
use crate::config::*;
use crate::world::Facility;
use macroquad::prelude::*;

/// Strobe period bounds for a forced flicker (player_controller_3d.gd 867-870).
const FLICKER_MIN_PERIOD: f32 = 0.08;
const FLICKER_MAX_PERIOD: f32 = 0.35;

pub struct Player {
    pub pos: (f32, f32),
    pub feet_y: f32,
    vy: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub crouch_t: f32,
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
    /// Walk height of the surface the body stands on: 0.0 on the floor, a deck's height on a catwalk.
    pub height: f32,
    /// Admin run speed: multiplies every movement speed (admin_menu.gd).
    pub speed_scale: f32,
    /// Eye height change during a stair or ladder climb (game/transition.rs), in metres: negative going down.
    pub climb_lift: f32,
    /// Seconds left in a forced flicker (a plunge splash or a hunter event); the
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
            vy: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            crouch_t: 0.0,
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
            climb_lift: 0.0,
            flicker_remaining: 0.0,
            flicker_next_toggle: 0.0,
            flicker_dark: false,
            flicker_seed: 0x2545_F491,
        }
    }

    /// Strobes the beam off and on at random short intervals for `duration` seconds,
    /// then restores the real state (Godot `force_flashlight_flicker`). Called by a
    /// plunge splash or a hunter event; the lamp keeps its charge and the switch keeps its state.
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
        self.vy = 0.0;
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.hidden = None;
        self.hide_lock = 0.0;
        self.in_vent = false;
        self.stun = 0.0;
    }

    pub fn eye_height(&self) -> f32 {
        CAMERA_HEIGHT + (CAMERA_HEIGHT * CROUCH_EYE_RATIO - CAMERA_HEIGHT) * self.crouch_t + self.climb_lift
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
        let bob = if self.moving && self.feet_y <= 0.0 {
            (self.bob_phase * 2.0).sin() * amp
        } else {
            0.0
        };
        vec3(
            self.pos.0,
            self.feet_y + self.eye_height() + bob,
            self.pos.1,
        )
    }

    pub fn look(&mut self, d: Vec2) {
        self.yaw -= d.x;
        self.pitch = (self.pitch - d.y).clamp(-1.45, 1.45);
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

    fn move_body(&mut self, fac: &Facility, dx: f32, dy: f32) {
        let total = (dx * dx + dy * dy).sqrt();
        let steps = ((total / 0.08).ceil() as i32).max(1);
        let start = self.pos;
        for _ in 0..steps {
            let nx = self.pos.0 + dx / steps as f32;
            if self.noclip || fac.body_fits_at(nx, self.pos.1, PLAYER_RADIUS, self.height) {
                self.pos.0 = nx;
            }
            let ny = self.pos.1 + dy / steps as f32;
            if self.noclip || fac.body_fits_at(self.pos.0, ny, PLAYER_RADIUS, self.height) {
                self.pos.1 = ny;
            }
        }
        self.walked += ((self.pos.0 - start.0).powi(2) + (self.pos.1 - start.1).powi(2)).sqrt();
    }

    pub fn update_movement(
        &mut self,
        dt: f32,
        c: &Controls,
        prof: &Profile,
        fac: &Facility,
        meters: &mut Meters,
        _sounds: &Sounds,
    ) {
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
            * self.speed_scale;
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
                (rx * input.x + fx * input.y) * speed * dt,
                (ry * input.x + fy * input.y) * speed * dt,
            );
        }
        let ground = ((self.pos.0 - before.0).powi(2) + (self.pos.1 - before.1).powi(2)).sqrt()
            / dt.max(1e-4);

        if self.feet_y <= 0.0 {
            self.feet_y = 0.0;
            self.vy = 0.0;
            if c.jump && !self.crouching && self.stun <= 0.0 {
                self.vy = PLAYER_JUMP_VELOCITY;
            }
        }
        self.vy -= PLAYER_GRAVITY * dt;
        self.feet_y = (self.feet_y + self.vy * dt).max(0.0);

        if moving && self.feet_y <= 0.0 && ground > 0.3 {
            self.bob_phase += dt * ground * 1.9;
        }
        // Sound.footstep every frame on the floor (player_controller_3d.gd 1987-1993): the cadence is stride / ground
        // speed, no step under 0.3 m/s, feet alternate
        if moving && self.feet_y <= 0.0 {
            let gait = if self.crouching {
                "crouch"
            } else if sprinting {
                "sprint"
            } else {
                "walk"
            };
            crate::audio::sound::footstep(gait, "", ground.min(speed * 1.5));
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
}
