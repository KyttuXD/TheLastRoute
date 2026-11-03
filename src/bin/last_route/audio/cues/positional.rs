//! AudioStreamPlayer3D as Godot 4.7.2 mixes it to a stereo output, fitted against Godot's own mixed output (movie
//! writer capture of an emitter with Glyra's hum settings at 10 positions around a Camera3D; residual < -54 dB at
//! 2-4 m):
//!
//! - distance: ATTENUATION_INVERSE_DISTANCE `linear_to_db(1 / (d / unit_size + eps)) + volume_db`, capped at max_db,
//!   then multiplied by the linear fade `max(0, 1 - d / max_distance)` (the docs: max_distance "always works in a linear
//!   fashion"). Measured 0.856 per channel straight ahead at 2 m = 1.4125 * (1 - 2/14) * 0.7071.
//! - stereo pan: equal power, `L = g * sqrt((1 - k x) / 2)`, `R = g * sqrt((1 + k x) / 2)` with x = the source
//!   direction's component along the listener's right and k = 0.6 at the project's panning strength
//!   (audio/general/3d_panning_strength 0.5 x panning_strength 1.0). Measured: 90 deg right L/R = 0.4473/0.8944 of g
//!   (sqrt .2 / sqrt .8), 45 deg = sqrt .288 / sqrt .712. Front and back are identical.
//! - Not modelled: the attenuation shelf filter (5 kHz, up to -24 dB as the gain drops). For the hum (sines of
//!   260-440 Hz) it only shifts phase slightly (the -12..-17 dB residuals measured at 8-13.5 m); its level effect is
//!   under 0.1 dB.

#[derive(Clone, Copy, Debug)]
pub struct Emitter3D {
    pub unit_size: f32,
    pub max_distance: f32,
    pub volume_db: f32,
    pub max_db: f32,
    /// Height of the emitter above its node (m).
    pub height: f32,
}

/// The measured pan depth at the project's panning strength (0.5 global x 1.0 per node).
pub const PAN_DEPTH: f32 = 0.6;

/// Glyra's hum (hunter_view_3d.gd 1572-1577): unit 4 m, max 14 m, +3 dB, 1.5 m up; bus Master (the 3D player's default).
pub const HUM: Emitter3D = Emitter3D {
    unit_size: 4.0,
    max_distance: 14.0,
    volume_db: 3.0,
    max_db: 3.0,
    height: 1.5,
};
/// MusicBoxTune (music_box_tune.gd): unit 5 m, max 12 m, +6 dB (capped at max_db 3), 0.4 m above the box.
pub const TUNE: Emitter3D = Emitter3D {
    unit_size: 5.0,
    max_distance: 12.0,
    volume_db: 6.0,
    max_db: 3.0,
    height: 0.4,
};

impl Emitter3D {
    /// The distance gain (linear).
    pub fn distance_gain(&self, d: f32) -> f32 {
        let mut db = 20.0 * (1.0 / (d / self.unit_size + 1e-5)).log10() + self.volume_db;
        if db > self.max_db {
            db = self.max_db;
        }
        let mut g = 10f32.powf(db / 20.0);
        if self.max_distance > 0.0 {
            g *= (1.0 - d / self.max_distance).max(0.0);
        }
        g
    }

    /// Left / right gains for a source at `local` in the listener's frame (x right, y up, -z forward), metres.
    pub fn stereo_gains(&self, local: [f32; 3]) -> (f32, f32) {
        let d = (local[0] * local[0] + local[1] * local[1] + local[2] * local[2]).sqrt();
        let g = self.distance_gain(d);
        let x = if d > 1e-6 { local[0] / d } else { 0.0 };
        (
            g * ((1.0 - PAN_DEPTH * x) * 0.5).sqrt(),
            g * ((1.0 + PAN_DEPTH * x) * 0.5).sqrt(),
        )
    }

    /// `stereo_gains` for an emitter node at `node` (feet) heard from `eye` with the camera's `right` / `up` / `forward`.
    pub fn gains_from(
        &self,
        node: [f32; 3],
        eye: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        forward: [f32; 3],
    ) -> (f32, f32) {
        let d = [
            node[0] - eye[0],
            node[1] + self.height - eye[1],
            node[2] - eye[2],
        ];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        self.stereo_gains([dot(d, right), dot(d, up), -dot(d, forward)])
    }
}
