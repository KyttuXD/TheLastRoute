//! Flythrough camera: a Catmull-Rom spline through the shot's camera points. The flight is noclip with
//! input frozen; the stress runner's fly mode (app/stress.rs `fly`) supplies the noclip switch.
use super::shot::{CamPoint, Shot};

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

fn cr(p0: f32, p1: f32, p2: f32, p3: f32, u: f32) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * u
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u * u
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u * u * u)
}

fn at(p: &[CamPoint], i: isize) -> &CamPoint {
    &p[i.clamp(0, p.len() as isize - 1) as usize]
}

/// The camera pose at time `t`; passes exactly through each control point at its time.
pub fn pose(shot: &Shot, t: f32) -> Pose {
    let p = &shot.camera;
    let last = p.len() - 1;
    if t <= p[0].t || last == 0 {
        return Pose { pos: p[0].pos, yaw: p[0].yaw, pitch: p[0].pitch };
    }
    if t >= p[last].t {
        let e = &p[last];
        return Pose { pos: e.pos, yaw: e.yaw, pitch: e.pitch };
    }
    let i = p.windows(2).position(|w| t >= w[0].t && t < w[1].t).unwrap_or(0) as isize;
    let (a, b) = (at(p, i), at(p, i + 1));
    let u = (t - a.t) / (b.t - a.t).max(1e-6);
    let (z, c) = (at(p, i - 1), at(p, i + 2));
    let mut pos = [0.0; 3];
    for k in 0..3 {
        pos[k] = cr(z.pos[k], a.pos[k], b.pos[k], c.pos[k], u);
    }
    Pose { pos, yaw: cr(z.yaw, a.yaw, b.yaw, c.yaw, u), pitch: cr(z.pitch, a.pitch, b.pitch, c.pitch, u) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_passes_through_control_points() {
        let shots = super::super::shot::embedded_shots();
        let s = &shots[0];
        for c in &s.camera {
            let q = pose(s, c.t);
            for k in 0..3 {
                assert!((q.pos[k] - c.pos[k]).abs() < 1e-3);
            }
            assert!((q.yaw - c.yaw).abs() < 1e-3);
        }
    }
}
