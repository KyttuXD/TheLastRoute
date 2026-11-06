//! View-frustum culling for the floor's meshes. The six planes are read off the same view-projection matrix the camera
//! draws with (`Camera3D::matrix`, the hiding and the grab cameras included), so a box this test calls outside is outside
//! what the GPU would clip too. The test is conservative: a box is dropped only when it lies wholly beyond one plane by
//! more than the margin, so anything that touches the view is kept (and an unreadable plane never drops anything).

use macroquad::prelude::*;

/// Metres of slack past every plane before a box counts as outside, so rounding never drops a visible mesh.
pub const FRUSTUM_MARGIN_M: f32 = 0.75;

/// The six clip planes (left, right, bottom, top, near, far), each `(nx, ny, nz, d)` with a unit normal pointing into the
/// view: a point `p` is inside a plane when `n . p + d >= 0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// The frustum of a view-projection matrix in OpenGL clip space (-w..w on every axis), the space macroquad's
    /// `Camera3D::matrix` is in on every backend (the Metal shader remaps depth itself).
    pub fn from_view_proj(m: Mat4) -> Frustum {
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        let raw = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2];
        let mut planes = [Vec4::ZERO; 6];
        for (out, p) in planes.iter_mut().zip(raw) {
            let len = p.truncate().length();
            // a degenerate or non-finite plane stays zero, which every box passes: nothing is ever dropped by it
            *out = if len.is_finite() && len > 1e-12 { p / len } else { Vec4::ZERO };
        }
        Frustum { planes }
    }

    /// The frustum of a perspective camera, built the way macroquad builds `Camera3D::matrix`.
    pub fn from_perspective(position: Vec3, target: Vec3, up: Vec3, fovy: f32, aspect: f32, z_near: f32, z_far: f32) -> Frustum {
        Frustum::from_view_proj(Mat4::perspective_rh_gl(fovy, aspect, z_near, z_far) * Mat4::look_at_rh(position, target, up))
    }

    /// True when the box `lo`..`hi` may show in the view: it is not wholly outside any plane by more than `margin`
    /// metres. An empty (inverted) box has nothing to draw and reads as outside.
    pub fn intersects_box(&self, lo: Vec3, hi: Vec3, margin: f32) -> bool {
        if lo.x > hi.x || lo.y > hi.y || lo.z > hi.z {
            return false;
        }
        for p in &self.planes {
            // the box corner farthest along the plane's normal: if even that one is behind the plane, the box is out
            let far = vec3(
                if p.x >= 0.0 { hi.x } else { lo.x },
                if p.y >= 0.0 { hi.y } else { lo.y },
                if p.z >= 0.0 { hi.z } else { lo.z },
            );
            // written so that a NaN keeps the box
            if p.x * far.x + p.y * far.y + p.z * far.z + p.w < -margin {
                return false;
            }
        }
        true
    }

    /// [`Self::intersects_box`] for the box a transform `xf` makes of the local box `lo`..`hi` (its eight corners' box).
    pub fn intersects_transformed_box(&self, xf: Mat4, lo: Vec3, hi: Vec3, margin: f32) -> bool {
        if lo.x > hi.x || lo.y > hi.y || lo.z > hi.z {
            return false;
        }
        let mut wlo = Vec3::splat(f32::MAX);
        let mut whi = Vec3::splat(f32::MIN);
        for i in 0..8 {
            let c = vec3(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let w = xf.transform_point3(c);
            wlo = wlo.min(w);
            whi = whi.max(w);
        }
        self.intersects_box(wlo, whi, margin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking down -Z from the origin, 90 degrees tall, square, near 0.1, far 100.
    fn forward() -> Frustum {
        Frustum::from_perspective(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y, 90f32.to_radians(), 1.0, 0.1, 100.0)
    }

    fn cube(c: Vec3, half: f32) -> (Vec3, Vec3) {
        (c - Vec3::splat(half), c + Vec3::splat(half))
    }

    #[test]
    fn a_box_in_front_and_on_the_axis_is_inside() {
        let (lo, hi) = cube(vec3(0.0, 0.0, -10.0), 1.0);
        assert!(forward().intersects_box(lo, hi, 0.0));
    }

    #[test]
    fn a_box_behind_the_camera_is_outside() {
        let (lo, hi) = cube(vec3(0.0, 0.0, 10.0), 1.0);
        assert!(!forward().intersects_box(lo, hi, 0.0));
        // far behind and off to a side is outside too
        let (lo, hi) = cube(vec3(30.0, 0.0, 30.0), 1.0);
        assert!(!forward().intersects_box(lo, hi, 0.0));
    }

    #[test]
    fn a_box_far_to_a_side_above_or_below_is_outside() {
        let f = forward();
        for c in [vec3(50.0, 0.0, -10.0), vec3(-50.0, 0.0, -10.0), vec3(0.0, 50.0, -10.0), vec3(0.0, -50.0, -10.0)] {
            let (lo, hi) = cube(c, 1.0);
            assert!(!f.intersects_box(lo, hi, 0.0), "{c:?}");
        }
    }

    #[test]
    fn a_box_beyond_the_far_plane_is_outside() {
        let (lo, hi) = cube(vec3(0.0, 0.0, -150.0), 1.0);
        assert!(!forward().intersects_box(lo, hi, 0.0));
    }

    #[test]
    fn a_box_straddling_a_plane_is_kept() {
        let f = forward();
        // straddling the right plane (x = -z at 90 degrees): centred on it
        let (lo, hi) = cube(vec3(10.0, 0.0, -10.0), 2.0);
        assert!(f.intersects_box(lo, hi, 0.0));
        // straddling the near plane
        assert!(f.intersects_box(vec3(-0.5, -0.5, -0.5), vec3(0.5, 0.5, 0.5), 0.0));
        // a huge box around the camera (a room's floor mesh) holds the eye and is kept
        assert!(f.intersects_box(vec3(-45.0, -1.0, -45.0), vec3(45.0, 4.0, 45.0), 0.0));
        // a long wall that crosses the view from far left to far right
        assert!(f.intersects_box(vec3(-80.0, 0.0, -20.0), vec3(80.0, 3.0, -19.9), 0.0));
    }

    #[test]
    fn the_margin_keeps_a_box_just_outside_a_plane() {
        let f = forward();
        // a thin box just past the right plane: its near face corner is at x = 10.3 when the plane passes x = 10 at z = -10
        let lo = vec3(10.3, -0.1, -10.1);
        let hi = vec3(10.6, 0.1, -9.9);
        assert!(!f.intersects_box(lo, hi, 0.0), "outside with no margin");
        assert!(f.intersects_box(lo, hi, 1.0), "kept by a metre of margin");
        // a box well past the margin stays outside
        assert!(!f.intersects_box(lo + vec3(5.0, 0.0, 0.0), hi + vec3(5.0, 0.0, 0.0), 1.0));
    }

    #[test]
    fn an_empty_box_is_outside_and_a_nan_box_is_kept() {
        let f = forward();
        assert!(!f.intersects_box(Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 1.0));
        assert!(f.intersects_box(Vec3::splat(f32::NAN), Vec3::splat(f32::NAN), 0.0), "an unreadable box is drawn, never dropped");
        // a degenerate matrix never drops anything
        let z = Frustum::from_view_proj(Mat4::ZERO);
        let (lo, hi) = cube(vec3(1000.0, 1000.0, 1000.0), 1.0);
        assert!(z.intersects_box(lo, hi, 0.0));
    }

    #[test]
    fn a_turned_camera_keeps_what_it_faces_and_drops_what_is_behind() {
        // looking along +X from (5, 1, 5)
        let eye = vec3(5.0, 1.0, 5.0);
        let f = Frustum::from_perspective(eye, eye + Vec3::X, Vec3::Y, 70f32.to_radians(), 16.0 / 9.0, 0.05, 60.0);
        let (lo, hi) = cube(eye + vec3(12.0, 0.0, 1.0), 1.0);
        assert!(f.intersects_box(lo, hi, 0.0));
        let (lo, hi) = cube(eye - vec3(12.0, 0.0, 0.0), 1.0);
        assert!(!f.intersects_box(lo, hi, 0.0));
    }

    #[test]
    fn a_transformed_box_is_tested_by_its_world_corners() {
        let f = forward();
        let xf = Mat4::from_translation(vec3(0.0, 0.0, -10.0));
        assert!(f.intersects_transformed_box(xf, Vec3::splat(-1.0), Vec3::splat(1.0), 0.0));
        let behind = Mat4::from_translation(vec3(0.0, 0.0, 10.0));
        assert!(!f.intersects_transformed_box(behind, Vec3::splat(-1.0), Vec3::splat(1.0), 0.0));
        // turned a quarter about Y: a long box along X now runs along Z, into the view
        let turned = Mat4::from_translation(vec3(0.0, 0.0, -20.0)) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!(f.intersects_transformed_box(turned, vec3(-15.0, -1.0, -1.0), vec3(15.0, 1.0, 1.0), 0.0));
    }

    /// A tiny deterministic generator, so the property test below is repeatable without a dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
        }

        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.next()
        }
    }

    /// Whether `p` projects inside the clip volume of `m` (what the GPU would keep).
    fn in_clip(m: Mat4, p: Vec3) -> bool {
        let c = m * p.extend(1.0);
        c.w > 0.0 && c.x.abs() <= c.w && c.y.abs() <= c.w && c.z.abs() <= c.w
    }

    #[test]
    fn culling_never_removes_a_box_that_has_a_visible_point() {
        let mut rng = Lcg(0x5eed_1234_abcd_ef01);
        let mut kept_visible = 0;
        let mut dropped = 0;
        for case in 0..400 {
            let eye = vec3(rng.range(-50.0, 50.0), rng.range(0.5, 3.0), rng.range(-50.0, 50.0));
            let yaw = rng.range(0.0, std::f32::consts::TAU);
            let pitch = rng.range(-0.6, 0.6);
            let dir = vec3(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos());
            let fovy = rng.range(50.0, 100.0).to_radians();
            let aspect = rng.range(1.0, 2.0);
            let (near, far) = (0.05, rng.range(30.0, 90.0));
            let m = Mat4::perspective_rh_gl(fovy, aspect, near, far) * Mat4::look_at_rh(eye, eye + dir, Vec3::Y);
            let f = Frustum::from_view_proj(m);
            for _ in 0..40 {
                let c = eye + vec3(rng.range(-80.0, 80.0), rng.range(-6.0, 6.0), rng.range(-80.0, 80.0));
                let half = vec3(rng.range(0.1, 25.0), rng.range(0.1, 4.0), rng.range(0.1, 25.0));
                let (lo, hi) = (c - half, c + half);
                // a grid of points over the box, corners included
                let mut visible = false;
                'grid: for ix in 0..=6 {
                    for iy in 0..=4 {
                        for iz in 0..=6 {
                            let p = lo + (hi - lo) * vec3(ix as f32 / 6.0, iy as f32 / 4.0, iz as f32 / 6.0);
                            if in_clip(m, p) {
                                visible = true;
                                break 'grid;
                            }
                        }
                    }
                }
                let kept = f.intersects_box(lo, hi, FRUSTUM_MARGIN_M);
                if visible {
                    assert!(kept, "case {case}: a box with a visible point was culled: {lo:?}..{hi:?} from {eye:?} toward {dir:?}");
                    kept_visible += 1;
                } else if !kept {
                    dropped += 1;
                }
            }
        }
        // the generator must exercise both outcomes, or the test proves nothing
        assert!(kept_visible > 200, "{kept_visible} visible boxes");
        assert!(dropped > 1000, "{dropped} dropped boxes");
    }
}
