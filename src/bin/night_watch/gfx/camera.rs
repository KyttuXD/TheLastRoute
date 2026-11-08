#![allow(dead_code)]
//! First-person camera, the camera-feed render target, and the `Gfx` bundle.

use super::art::Textures;
use super::shader::LitShader;
use macroquad::prelude::*;

/// Pitch limit in radians.
pub const PITCH_LIMIT: f32 = 1.4;

#[derive(Clone, Copy, Debug)]
pub struct FpCamera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for FpCamera {
    fn default() -> FpCamera {
        FpCamera { pos: Vec3::ZERO, yaw: 0.0, pitch: 0.0, fov_deg: 78.0, near: 0.06, far: 100.0 }
    }
}

impl FpCamera {
    pub fn new(pos: Vec3, yaw: f32, pitch: f32) -> FpCamera {
        FpCamera { pos, yaw, pitch, ..FpCamera::default() }
    }

    /// (-sin yaw cos pitch, sin pitch, -cos yaw cos pitch): yaw 0 looks down -Z.
    pub fn forward(&self) -> Vec3 {
        vec3(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    /// Apply mouse motion in screen px with the sensitivity in rad/px; pitch is clamped to +-1.4.
    pub fn look(&mut self, dx: f32, dy: f32, sens: f32) {
        self.yaw -= dx * sens;
        self.pitch = (self.pitch - dy * sens).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn camera3d(&self) -> Camera3D {
        Camera3D {
            position: self.pos,
            target: self.pos + self.forward(),
            up: Vec3::Y,
            fovy: self.fov_deg.to_radians(),
            z_near: self.near,
            z_far: self.far,
            ..Default::default()
        }
    }

    pub fn apply(&self) {
        set_camera(&self.camera3d());
    }
}

/// A 480x270 render target the camera feeds are drawn into.
pub struct FeedTarget {
    pub rt: RenderTarget,
}

pub const FEED_W: u32 = 480;
pub const FEED_H: u32 = 270;

impl FeedTarget {
    /// Needs a live GL context.
    pub fn new() -> FeedTarget {
        let rt = render_target_ex(FEED_W, FEED_H, RenderTargetParams { sample_count: 1, depth: true });
        rt.texture.set_filter(FilterMode::Nearest);
        FeedTarget { rt }
    }

    /// Start drawing into the target from a camera at `pos` looking at `target`; clears to near black.
    pub fn begin(&self, pos: Vec3, target: Vec3, up: Vec3) {
        set_camera(&Camera3D {
            position: pos,
            target,
            up,
            fovy: 62.0f32.to_radians(),
            aspect: Some(16.0 / 9.0),
            render_target: Some(self.rt.clone()),
            z_near: 0.05,
            z_far: 80.0,
            ..Default::default()
        });
        clear_background(Color::new(0.02, 0.03, 0.03, 1.0));
    }

    pub fn end(&self) {
        set_default_camera();
    }

    /// Blit the feed (render targets are upside down, so Y is flipped).
    pub fn draw_to_rect(&self, x: f32, y: f32, w: f32, h: f32, tint: Color) {
        draw_texture_ex(
            &self.rt.texture,
            x,
            y,
            tint,
            DrawTextureParams { dest_size: Some(vec2(w, h)), flip_y: true, ..Default::default() },
        );
    }
}

/// Shared GPU resources, created once after the window exists.
pub struct Gfx {
    pub tex: Textures,
    pub shader: LitShader,
    pub feed: FeedTarget,
    pub white: Texture2D,
}

impl Gfx {
    pub fn new() -> Gfx {
        let white = Texture2D::from_rgba8(1, 1, &[255, 255, 255, 255]);
        white.set_filter(FilterMode::Nearest);
        Gfx { tex: Textures::build(), shader: LitShader::new(), feed: FeedTarget::new(), white }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_at_yaw_zero_is_minus_z() {
        let c = FpCamera::default();
        let f = c.forward();
        assert!(f.x.abs() < 1e-6 && f.y.abs() < 1e-6 && (f.z + 1.0).abs() < 1e-6);
    }

    #[test]
    fn forward_follows_yaw_and_pitch() {
        let mut c = FpCamera::default();
        c.yaw = std::f32::consts::FRAC_PI_2;
        let f = c.forward();
        assert!((f.x + 1.0).abs() < 1e-5 && f.z.abs() < 1e-5, "yaw +90 degrees looks down -X");
        c.pitch = 0.5;
        assert!(c.forward().y > 0.0);
        assert!((c.forward().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn pitch_is_clamped() {
        let mut c = FpCamera::default();
        c.look(0.0, -100000.0, 0.0075);
        assert_eq!(c.pitch, PITCH_LIMIT);
        c.look(0.0, 100000.0, 0.0075);
        assert_eq!(c.pitch, -PITCH_LIMIT);
    }

    #[test]
    fn camera3d_uses_the_contract_numbers() {
        let c = FpCamera::new(vec3(1.0, 1.7, 2.0), 0.0, 0.0);
        let cam = c.camera3d();
        assert!((cam.fovy - 78.0f32.to_radians()).abs() < 1e-6);
        assert_eq!(cam.z_near, 0.06);
        assert_eq!(cam.z_far, 100.0);
        assert_eq!(cam.target, vec3(1.0, 1.7, 1.0));
    }
}
