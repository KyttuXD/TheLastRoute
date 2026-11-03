//! Exposure, noise meter and temperature (game/player/player_meters.gd). The facility warms on
//! a shortening interval; the body chases it; the safe zone pulls back toward 60; reaching 100
//! latches the heat for the rest of the run (Solyth). Meters carry across floors, as in the
//! original.

pub struct Meters {
    pub exposure: f32,
    pub noise_meter: f32,
    pub temperature: f32,
    pub ambient: f32,
    pub heat_locked: bool,
    noise_input: f32,
    rise_timer: f32,
}

pub struct Situation {
    pub flashlight_on: bool,
    pub noise: f32,
    pub crouching: bool,
    pub hidden: bool,
    pub in_safe_zone: bool,
    pub heater_on: bool,
    /// The room's climate offset (hot and cold spots; 0 in halls).
    pub local: f32,
}

impl Default for Meters {
    fn default() -> Self {
        Meters {
            exposure: 0.0,
            noise_meter: 0.0,
            temperature: 60.0,
            ambient: 60.0,
            heat_locked: false,
            noise_input: 0.0,
            rise_timer: 0.0,
        }
    }
}

impl Meters {
    pub fn add_exposure(&mut self, a: f32) {
        self.exposure = (self.exposure + a).clamp(0.0, 100.0);
    }

    pub fn add_noise(&mut self, a: f32) {
        self.noise_input += a;
    }

    pub fn add_temperature(&mut self, a: f32) {
        self.temperature = (self.temperature + a).clamp(0.0, 100.0);
        if self.temperature >= 100.0 {
            self.heat_locked = true;
        }
    }

    pub fn overheating(&self) -> bool {
        self.temperature >= 90.0
    }

    pub fn freezing(&self) -> bool {
        self.temperature <= 10.0
    }

    /// Lamp/stamina drain multiplier (overheating burns supplies faster).
    pub fn drain_scale(&self) -> f32 {
        if self.overheating() {
            1.35
        } else {
            1.0
        }
    }

    /// Movement multiplier (freezing slows you).
    pub fn movement_scale(&self) -> f32 {
        if self.freezing() {
            0.72
        } else {
            1.0
        }
    }

    pub fn update(&mut self, dt: f32, s: &Situation) {
        // exposure
        let mut target = 0.0;
        if !s.hidden {
            target = 18.0
                + if s.flashlight_on { 55.0 } else { 0.0 }
                + (s.noise / 10.5).clamp(0.0, 1.0) * 40.0;
            if s.crouching {
                target *= 0.55;
            }
        }
        let ease = (1.8 * dt).clamp(0.0, 1.0);
        self.exposure =
            (self.exposure + (target.clamp(0.0, 100.0) - self.exposure) * ease).clamp(0.0, 100.0);
        // noise meter
        let mut rise = std::mem::take(&mut self.noise_input);
        if !s.hidden {
            rise += (s.noise / 10.5).clamp(0.0, 1.0) * 8.0 * dt;
        }
        self.noise_meter = (self.noise_meter + rise - 5.0 * dt).clamp(0.0, 100.0);
        // ambient drift
        if s.heater_on {
            self.ambient = (self.ambient + 3.0 * dt).min(100.0);
        } else {
            let step = ((self.ambient - 60.0) / 5.0).floor().max(0.0);
            let interval = (5.0 - step * 0.5).max(0.5);
            self.rise_timer += dt;
            if self.rise_timer >= interval {
                self.rise_timer -= interval;
                self.ambient = (self.ambient + 0.85).clamp(0.0, 100.0);
            }
        }
        // body
        let local_ambient = (self.ambient + s.local).clamp(0.0, 100.0);
        let diff = local_ambient - self.temperature;
        let proposed = (diff * 1.6).clamp(-6.4, 6.4);
        let rate = if dt > 0.0 && (proposed * dt).abs() > diff.abs() {
            diff / dt
        } else {
            proposed
        };
        self.temperature = (self.temperature + rate * dt).clamp(0.0, 100.0);
        if diff.abs() < 1e-3 {
            // the intended snap (GIMMICKS.md section 27): the float approach otherwise stalls short of 100
            self.temperature = local_ambient;
        }
        if s.in_safe_zone {
            let d = 60.0 - self.temperature;
            self.temperature += d.clamp(-14.0 * dt, 14.0 * dt);
        }
        if self.temperature >= 100.0 - 1e-3 {
            self.heat_locked = true;
        }
        if self.heat_locked {
            self.temperature = self.temperature.max(90.0);
        }
    }
}
