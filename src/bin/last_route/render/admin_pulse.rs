//! The admin find-player marker: a tall beam and two expanding rings at the target runner's live
//! position, drawn with no depth test so it shows through walls. It lasts `pulse_secs`
//! (data/admin/menu.json, "hunter_view") and fades out. The caller keeps one [`Pulse`], calls
//! [`Pulse::update`] with the frame time and the target's current position, then [`Pulse::draw`]
//! inside the 3D pass (hook in render/scene.rs).

use crate::ui::admin::hunter_view;
use macroquad::prelude::*;

/// One running marker.
#[derive(Clone, Debug, PartialEq)]
pub struct Pulse {
    /// The target's live position (x, y, z); refreshed each frame by `update`.
    pub at: [f32; 3],
    pub age: f32,
    pub duration: f32,
}

impl Pulse {
    pub fn new(at: [f32; 3], duration: f32) -> Pulse {
        Pulse { at, age: 0.0, duration }
    }

    /// Advances the clock and follows the target; false once the marker has run its time.
    pub fn update(&mut self, dt: f32, live_pos: [f32; 3]) -> bool {
        self.age += dt;
        self.at = live_pos;
        self.alive()
    }

    pub fn alive(&self) -> bool {
        self.age < self.duration
    }

    /// 1 at the start, 0 at the end.
    pub fn alpha(&self) -> f32 {
        (1.0 - self.age / self.duration).clamp(0.0, 1.0)
    }

    /// The radius of ring `i` (0 or 1): both expand to `max` over a second, the second half a cycle behind.
    pub fn ring_radius(&self, i: usize, max: f32) -> f32 {
        let phase = (self.age + i as f32 * 0.5).fract();
        phase * max
    }

    /// Draws the beam and the rings with the depth test off, then puts it back on.
    pub fn draw(&self, beam_height: f32, ring_max: f32) {
        if !self.alive() {
            return;
        }
        let a = self.alpha();
        let gl = unsafe { get_internal_gl() };
        gl.quad_gl.depth_test(false);
        let top = vec3(self.at[0], self.at[1] + beam_height, self.at[2]);
        let base = vec3(self.at[0], self.at[1], self.at[2]);
        draw_line_3d(base, top, Color::new(1.0, 0.85, 0.2, 0.9 * a));
        for i in 0..2 {
            let r = self.ring_radius(i, ring_max);
            let fade = a * (1.0 - r / ring_max).clamp(0.0, 1.0);
            let n = 32;
            for k in 0..n {
                let a0 = std::f32::consts::TAU * k as f32 / n as f32;
                let a1 = std::f32::consts::TAU * (k + 1) as f32 / n as f32;
                let p0 = base + vec3(a0.cos() * r, 0.05, a0.sin() * r);
                let p1 = base + vec3(a1.cos() * r, 0.05, a1.sin() * r);
                draw_line_3d(p0, p1, Color::new(1.0, 0.85, 0.2, fade));
            }
        }
        gl.quad_gl.depth_test(true);
    }
}

/// A marker from the data duration.
pub fn start(at: [f32; 3]) -> Pulse {
    Pulse::new(at, hunter_view::cfg().pulse_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_follows_its_target_and_ends_after_its_duration() {
        let mut p = Pulse::new([0.0, 0.0, 0.0], 2.0);
        assert!(p.update(1.0, [3.0, 0.0, 4.0]));
        assert_eq!(p.at, [3.0, 0.0, 4.0]);
        assert!((p.alpha() - 0.5).abs() < 1e-6);
        assert!(!p.update(1.5, [3.0, 0.0, 4.0]));
        assert_eq!(p.alpha(), 0.0);
    }

    #[test]
    fn two_rings_are_half_a_cycle_apart_and_stay_in_range() {
        let p = Pulse::new([0.0; 3], 6.0);
        let max = 4.0;
        for t in [0.0, 0.2, 0.7] {
            let mut q = p.clone();
            q.age = t;
            let (a, b) = (q.ring_radius(0, max), q.ring_radius(1, max));
            assert!((0.0..=max).contains(&a) && (0.0..=max).contains(&b));
            assert!((a - b).abs() > 0.1, "the rings are apart");
        }
    }

    #[test]
    fn the_data_duration_is_used() {
        assert_eq!(start([0.0; 3]).duration, hunter_view::cfg().pulse_secs);
    }
}
