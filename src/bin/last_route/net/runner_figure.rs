//! The figure a remote runner wears in the 3D view. The body is the character model the game already loads for the
//! NPC teammates (`Assets::npc_body`: the base hunter model parsed at the runner's height). Each runner's look is
//! generated from its skin id and its peer id, so the same runner always looks the same.

use super::world_sync::{RemoteView, SKIN_DEFAULT};
use crate::game::Game;
use crate::render::bbmodel::Model;
use crate::render::builder::Builder;
use crate::render::{scene, shader};
use crate::shared::assets::Assets;
use macroquad::prelude::*;

/// The skins the room knows, as the SKINS table of `world_sync` lists them. An id outside it draws as DEFAULT.
const KNOWN_SKINS: [&str; 1] = [SKIN_DEFAULT];
/// Thickness of a trim mark, in metres, laid on the body's surface.
const TRIM_THICKNESS: f32 = 0.02;

/// The trim a runner's look wears on the front and back of its body, so it reads from either side.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Trim {
    Badge,
    Stripes,
    Belt,
    Dots,
}

/// A runner's colours and trim pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    body: Color,
    trim: Color,
    pattern: Trim,
}

/// Draws the remote runners in the 3D view, lit like the scene. Each one is the body model at its position and yaw,
/// tinted by its look and marked by its trim. Draws nothing when `remotes` is empty, so a single-player frame is unchanged.
pub fn draw_runners(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    if remotes.is_empty() {
        return;
    }
    set_camera(&scene::camera(g, rt));
    gl_use_material(mat);
    shader::apply(mat, &scene::lighting(g), &g.world.lamps);
    match a.npc_body.as_ref() {
        Some(body) => {
            for r in remotes {
                let look = look_of(&r.skin, r.peer);
                let at = vec3(r.x, 0.0, r.y);
                let mut verts = vec![];
                let mut idx = vec![];
                body.build(&mut verts, &mut idx, at, r.yaw, 0.0, 0.0, look.body);
                draw_mesh(&Mesh {
                    vertices: verts,
                    indices: idx,
                    texture: Some(body.texture.clone()),
                });
                let mut marks = Builder::new(Some(a.white.clone()));
                trim_marks(&mut marks, body, at, r.yaw, &look);
                marks.draw_and_clear();
            }
        }
        None => {
            let mut b = Builder::new(Some(a.white.clone()));
            for r in remotes {
                b.cube(vec3(r.x, 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, look_of(&r.skin, r.peer).body, 2.0);
            }
            b.draw_and_clear();
        }
    }
}

/// The look of a runner: its skin id and peer id hashed to a body hue, a trim hue across the colour wheel, and a pattern.
fn look_of(skin: &str, peer: u32) -> Look {
    let bits = mix(known_skin(skin), peer);
    let hue = unit(bits);
    let sat = 0.45 + 0.3 * unit(bits >> 16);
    let val = 0.8 + 0.15 * unit(bits >> 32);
    let pattern = match (bits >> 48) % 4 {
        0 => Trim::Badge,
        1 => Trim::Stripes,
        2 => Trim::Belt,
        _ => Trim::Dots,
    };
    Look {
        body: hsv(hue, sat, val),
        trim: hsv((hue + 0.5).fract(), 0.9, 1.0),
        pattern,
    }
}

/// The skin a runner draws with: its own id when the room knows it, else DEFAULT.
fn known_skin(skin: &str) -> &str {
    if KNOWN_SKINS.iter().any(|known| *known == skin) {
        skin
    } else {
        SKIN_DEFAULT
    }
}

/// FNV-1a over the skin id and the peer id, finished with splitmix64 so every input bit reaches every output bit.
/// Fixed constants, so a runner's look is the same on every build and platform.
fn mix(skin: &str, peer: u32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in skin.bytes().chain(peer.to_le_bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// A value in [0, 1) from the low 16 bits of `bits`.
fn unit(bits: u64) -> f32 {
    (bits & 0xffff) as f32 / 65536.0
}

/// HSV to RGB, all components in [0, 1).
fn hsv(h: f32, s: f32, v: f32) -> Color {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::new(r, g, b, 1.0)
}

/// The trim of a look on the body: marks on the front and back of the model (a mirrored pair, so the direction the
/// body faces does not matter), in the trim colour and unlit so they glow in the dark like the old figures did.
fn trim_marks(b: &mut Builder, body: &Model, at: Vec3, yaw: f32, look: &Look) {
    let s = body.scale;
    let rot = Mat3::from_rotation_y(yaw);
    let base = body.min.y * s;
    let height = (body.max.y - body.min.y) * s;
    let (cx, cz) = ((body.min.x + body.max.x) * 0.5 * s, (body.min.z + body.max.z) * 0.5 * s);
    let half_depth = (body.max.z - body.min.z) * s * 0.5;
    let mut mark = |x: f32, y: f32, w: f32, h: f32| {
        for face in [-1.0, 1.0] {
            let local = vec3(cx + x, base + y * height, cz + face * (half_depth + TRIM_THICKNESS * 0.5));
            b.cube_basis(at + rot * local, vec3(w, h, TRIM_THICKNESS), rot, look.trim, 2.0);
        }
    };
    match look.pattern {
        Trim::Badge => mark(0.0, 0.72, 0.12, 0.12),
        Trim::Stripes => {
            mark(-0.1, 0.6, 0.03, 0.3);
            mark(0.1, 0.6, 0.03, 0.3);
        }
        Trim::Belt => mark(0.0, 0.5, 0.26, 0.04),
        Trim::Dots => {
            for x in [-0.08, 0.0, 0.08] {
                mark(x, 0.7, 0.04, 0.04);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_keeps_its_look_and_unknown_skins_draw_as_default() {
        assert_eq!(look_of(SKIN_DEFAULT, 3), look_of(SKIN_DEFAULT, 3));
        assert_eq!(look_of("NO_SUCH_SKIN", 3), look_of(SKIN_DEFAULT, 3));
        assert_ne!(look_of(SKIN_DEFAULT, 1).body, look_of(SKIN_DEFAULT, 2).body, "two runners on one skin differ");
    }

    #[test]
    fn colours_stay_in_range() {
        for peer in 0..16 {
            let look = look_of(SKIN_DEFAULT, peer);
            for c in [look.body, look.trim] {
                assert!((0.0..=1.0).contains(&c.r) && (0.0..=1.0).contains(&c.g) && (0.0..=1.0).contains(&c.b));
            }
        }
    }
}
