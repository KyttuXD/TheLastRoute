//! Visible light cones: Kathy's search beam (hunters/gimmicks/kathy.rs, drawn like her Godot eye beam in
//! hunter_view_3d.gd `_build_beam_visual`: yellow while she patrols, red while she chases) and a remote runner's
//! flashlight (net/runner_figure.rs draws it for every runner whose sample says the lamp is on).
//!
//! A cone is an open four-sided pyramid. Its apex sits at the light and its alpha falls from the apex to the rim, so
//! the beam is brightest where it leaves the lamp. It is drawn with the world material in [`BEAM_MODE`]: unlit, fogged
//! like the rest of the room, and blended by its vertex alpha. The caller binds the world material and the white
//! texture (`render/scene.rs`, `net/runner_figure.rs`).

use super::bbmodel::Model;
use crate::hunters::gimmicks::kathy::{BEAM_HALF_ANGLE, BEAM_RANGE};
use crate::hunters::gimmicks::Gimmick;
use macroquad::prelude::*;

/// The vertex mode (normal.w) of an unlit, fogged surface that draws with its texture times its vertex colour,
/// alpha included (`render/shader.rs`: modes above 2.5 keep the alpha, modes above 1.5 take the fog path).
pub const BEAM_MODE: f32 = 3.0;

/// Kathy's neutral beam: the colour of her eye (the atlas pixel (10, 14) of kathy_atlas.png, 239 195 0).
pub const KATHY_BEAM_RGB: (f32, f32, f32) = (239.0 / 255.0, 195.0 / 255.0, 0.0);
/// Kathy's beam while she is chasing (hunter_view_3d.gd BEAM_CHASE_COLOR).
pub const KATHY_CHASE_RGB: (f32, f32, f32) = (1.0, 0.10, 0.08);
/// A runner's flashlight colour (the shader's beam colour, render/shader.rs).
pub const FLASHLIGHT_RGB: (f32, f32, f32) = (1.0, 0.95, 0.86);
/// Half angle of a runner's flashlight cone (radians): the lit cone of the shader falls off between about 21 and 37 degrees.
pub const FLASHLIGHT_HALF_ANGLE: f32 = 0.35;
/// The height a runner's lamp is held at, standing and crouched (metres above the feet).
const FLASHLIGHT_HEIGHT: f32 = 1.25;
const FLASHLIGHT_HEIGHT_CROUCH: f32 = 0.85;
/// How far the lamp hand sits ahead of the feet, along the aim (metres).
const FLASHLIGHT_FORWARD: f32 = 0.3;

/// Alpha at the apex and at the rim of Kathy's beam, and of a flashlight.
const KATHY_ALPHA: (f32, f32) = (0.30, 0.06);
const FLASHLIGHT_ALPHA: (f32, f32) = (0.22, 0.03);

/// One open pyramid from `apex` along `dir`, `length` long, its base square `half_width` from the axis. The vertices
/// carry the colour `rgb` with `alpha.0` at the apex and `alpha.1` on the base. The mesh has no texture: the caller
/// binds the white texture, so the vertex colour is the whole colour.
pub fn cone_mesh(apex: Vec3, dir: Vec3, length: f32, half_width: f32, rgb: (f32, f32, f32), alpha: (f32, f32)) -> Mesh {
    let dir = dir.normalize_or_zero();
    let side = if dir.y.abs() < 0.9 { dir.cross(Vec3::Y) } else { dir.cross(Vec3::X) }.normalize_or_zero();
    let up = side.cross(dir);
    let centre = apex + dir * length;
    let rim = [
        centre + side * half_width + up * half_width,
        centre - side * half_width + up * half_width,
        centre - side * half_width - up * half_width,
        centre + side * half_width - up * half_width,
    ];
    let normal = vec4(dir.x, dir.y, dir.z, BEAM_MODE);
    let col = |a: f32| -> [u8; 4] { Color::new(rgb.0, rgb.1, rgb.2, a).into() };
    let mut vertices = Vec::with_capacity(5);
    vertices.push(Vertex { position: apex, uv: Vec2::ZERO, color: col(alpha.0), normal });
    for p in rim {
        vertices.push(Vertex { position: p, uv: Vec2::ZERO, color: col(alpha.1), normal });
    }
    let mut indices = Vec::with_capacity(12);
    for k in 0..4u16 {
        indices.extend_from_slice(&[0, 1 + k, 1 + (k + 1) % 4]);
    }
    Mesh { vertices, indices, texture: None }
}

/// Where Kathy's eyes are, in her model's units: the middle of her two eye locators (their outliner origins, the rest
/// pose). Without them, a point on her centre line at 85% of her height.
pub fn kathy_eye_units(model: &Model) -> Vec3 {
    let at = |name: &str| model.groups.iter().find(|g| g.name == name).map(|g| g.origin);
    match (at("L-Eye-Attachment"), at("R-Eye-Attachment")) {
        (Some(l), Some(r)) => (l + r) * 0.5,
        _ => vec3(
            (model.min.x + model.max.x) * 0.5,
            model.min.y + (model.max.y - model.min.y) * 0.85,
            (model.min.z + model.max.z) * 0.5,
        ),
    }
}

/// Kathy's search beam in the scene's frame: from her eye through her body transform `root`, along her model's front
/// (-Z), `BEAM_RANGE` long and `BEAM_HALF_ANGLE` wide, red while she chases.
pub fn kathy_cone(root: Mat4, eye: Vec3, chasing: bool) -> Mesh {
    let apex = root.transform_point3(eye);
    let dir = root.transform_vector3(Vec3::NEG_Z);
    let rgb = if chasing { KATHY_CHASE_RGB } else { KATHY_BEAM_RGB };
    cone_mesh(apex, dir, BEAM_RANGE, BEAM_RANGE * BEAM_HALF_ANGLE.tan(), rgb, KATHY_ALPHA)
}

/// Whether hunter `gimmick` shows a beam this frame: only Kathy, and only while she is active, not held by a grab, not
/// faded out and not stunned (Godot hides her beam during a grab, hunter_view_3d.gd).
pub fn kathy_beam_shown(gimmick: &Gimmick, active: bool, held: bool, faded: bool, stun: f32) -> bool {
    matches!(gimmick, Gimmick::Kathy) && active && !held && !faded && stun <= 0.0
}

/// Where a runner's lamp is when no held flashlight model gives its tip (no item in the hand, or the body hidden): from
/// its feet at `feet`, forward along its yaw, at the height it holds the lamp (standing or crouched).
pub fn flashlight_apex(feet: Vec3, yaw: f32, crouching: bool) -> Vec3 {
    let (fx, fz) = (-yaw.sin(), -yaw.cos());
    let height = if crouching { FLASHLIGHT_HEIGHT_CROUCH } else { FLASHLIGHT_HEIGHT };
    feet + vec3(fx * FLASHLIGHT_FORWARD, height, fz * FLASHLIGHT_FORWARD)
}

/// A runner's flashlight beam leaving its lamp at `apex` (the tip of the flashlight it holds, `net/held_wire.rs`), aimed
/// by its yaw and pitch the way its camera looks (`game/player.rs` camera_basis). The beam reaches the flashlight's
/// throw (data/render/ambient.json).
pub fn flashlight_cone(apex: Vec3, yaw: f32, pitch: f32) -> Mesh {
    let (fx, fz) = (-yaw.sin(), -yaw.cos());
    let dir = vec3(fx * pitch.cos(), pitch.sin(), fz * pitch.cos());
    let range = super::ambient::table().flashlight().range;
    cone_mesh(apex, dir, range, range * FLASHLIGHT_HALF_ANGLE.tan(), FLASHLIGHT_RGB, FLASHLIGHT_ALPHA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centre_of_rim(m: &Mesh) -> Vec3 {
        m.vertices[1..].iter().fold(Vec3::ZERO, |a, v| a + v.position) / 4.0
    }

    #[test]
    fn kathy_gets_a_beam_while_she_is_out_and_free() {
        let k = Gimmick::Kathy;
        assert!(kathy_beam_shown(&k, true, false, false, 0.0), "an active, free Kathy shows her beam");
        assert!(!kathy_beam_shown(&k, false, false, false, 0.0), "a dormant Kathy shows none");
        assert!(!kathy_beam_shown(&k, true, true, false, 0.0), "no beam while a grab holds her");
        assert!(!kathy_beam_shown(&k, true, false, true, 0.0), "no beam while she is faded out");
        assert!(!kathy_beam_shown(&k, true, false, false, 0.5), "no beam while she is stunned");
        assert!(!kathy_beam_shown(&Gimmick::None, true, false, false, 0.0), "only Kathy has a beam");
    }

    #[test]
    fn kathys_cone_starts_at_her_eye_and_opens_along_her_facing_at_search_range() {
        // her model front is -Z; a body turned -90 degrees about Y faces +X
        let root = Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let m = kathy_cone(root, vec3(0.0, 1.5, 0.0), false);
        assert_eq!(m.vertices.len(), 5, "apex and four rim corners");
        assert_eq!(m.indices.len(), 12, "four side triangles");
        let apex = m.vertices[0].position;
        assert!((apex - vec3(0.0, 1.5, 0.0)).length() < 1e-4, "the apex is at her eye: {apex:?}");
        let rim = centre_of_rim(&m);
        assert!((rim - apex).normalize().dot(vec3(1.0, 0.0, 0.0)) > 0.999, "the beam opens along her facing: {:?}", rim - apex);
        assert!(((rim - apex).length() - BEAM_RANGE).abs() < 1e-3, "the beam is {BEAM_RANGE} m long");
    }

    #[test]
    fn kathys_beam_is_yellow_on_patrol_and_red_on_the_chase() {
        let patrol = kathy_cone(Mat4::IDENTITY, Vec3::ZERO, false);
        let chase = kathy_cone(Mat4::IDENTITY, Vec3::ZERO, true);
        let c = patrol.vertices[1].color;
        assert!(c[0] > 200 && c[1] > 150 && c[2] < 30, "yellow on patrol: {c:?}");
        let r = chase.vertices[1].color;
        assert!(r[0] > 200 && r[1] < 60 && r[2] < 60, "red on the chase: {r:?}");
        assert!((patrol.vertices[0].color[3] as f32 - KATHY_ALPHA.0 * 255.0).abs() <= 1.0, "the apex is the brightest");
        assert!(patrol.vertices[0].color[3] > patrol.vertices[1].color[3], "the alpha falls to the rim");
        assert_eq!(patrol.vertices[0].normal.w, BEAM_MODE);
    }

    #[test]
    fn a_flashlight_beam_follows_the_aim_and_the_crouch() {
        // yaw 0 faces -Z; level aim from the standing lamp
        let lamp = flashlight_apex(Vec3::ZERO, 0.0, false);
        assert!((lamp.y - FLASHLIGHT_HEIGHT).abs() < 1e-4, "held at standing height: {lamp:?}");
        let m = flashlight_cone(lamp, 0.0, 0.0);
        let apex = m.vertices[0].position;
        assert!((apex - lamp).length() < 1e-5, "the beam leaves the lamp");
        assert!((centre_of_rim(&m) - apex).normalize().dot(vec3(0.0, 0.0, -1.0)) > 0.999);
        // looking up 45 degrees: the beam climbs
        let up = flashlight_cone(lamp, 0.0, std::f32::consts::FRAC_PI_4);
        assert!(centre_of_rim(&up).y > up.vertices[0].position.y + 1.0, "an upward aim lifts the beam");
        // crouched: the lamp is lower
        assert!(flashlight_apex(Vec3::ZERO, 0.0, true).y < lamp.y);
    }

    #[test]
    fn a_beam_leaves_the_lamp_it_is_given_not_the_feet() {
        // the held flashlight's tip, out at the hand, is where the light starts
        let tip = vec3(0.4, 1.1, -0.3);
        let m = flashlight_cone(tip, 1.0, 0.2);
        assert!((m.vertices[0].position - tip).length() < 1e-5);
    }
}
