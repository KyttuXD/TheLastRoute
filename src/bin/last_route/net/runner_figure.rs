//! The figure a remote runner wears in the 3D view. The body is the blocky mannequin the game loads for the runners and
//! the NPC teammates (`Assets::npc_body`, parsed at the runner's height), textured with the mannequin atlas by default.
//! A runner whose skin is on disk (see `Assets::skin_texture`) wears that skin's PNG on the same model. A skin that is
//! DEFAULT, or not known yet, draws the atlas.

use super::world_sync::{RemoteView, SKIN_DEFAULT};
use crate::game::Game;
use crate::render::scene;
use crate::render::shader;
use crate::shared::assets::Assets;
use macroquad::prelude::*;

/// Which texture a runner's body draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The mannequin atlas (the default skin, or a skin not known yet).
    Atlas,
    /// The cached skin with this id.
    Skin(String),
}

/// The texture choice for a runner: DEFAULT is the atlas; a skin id is used only when `has_skin` says its texture is
/// ready; any other id falls back to the atlas.
pub fn pick(skin: &str, has_skin: &dyn Fn(&str) -> bool) -> Pick {
    if skin != SKIN_DEFAULT && has_skin(skin) {
        Pick::Skin(skin.to_string())
    } else {
        Pick::Atlas
    }
}

/// Draws the remote runners in the 3D view, lit like the scene. Each one is the mannequin at its position and yaw,
/// wearing its skin's texture. Draws nothing when `remotes` is empty, so a single-player frame is unchanged.
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
                let texture = match pick(&r.skin, &|id| a.skin_texture(id).is_some()) {
                    Pick::Skin(id) => a.skin_texture(&id).unwrap_or_else(|| body.texture.clone()),
                    Pick::Atlas => body.texture.clone(),
                };
                let mut verts = vec![];
                let mut idx = vec![];
                body.build(&mut verts, &mut idx, vec3(r.x, 0.0, r.y), r.yaw, 0.0, 0.0, WHITE);
                draw_mesh(&Mesh {
                    vertices: verts,
                    indices: idx,
                    texture: Some(texture),
                });
            }
        }
        None => {
            // The mannequin failed to load: a plain grey post keeps the runner visible in the 3D view.
            let mut b = crate::render::builder::Builder::new(Some(a.white.clone()));
            for r in remotes {
                b.cube(vec3(r.x, 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, GRAY, 2.0);
            }
            b.draw_and_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runner_uses_the_atlas_and_an_unknown_skin_falls_back() {
        let none = |_: &str| false;
        assert_eq!(pick(SKIN_DEFAULT, &|_| true), Pick::Atlas, "DEFAULT is always the atlas");
        assert_eq!(pick("NO_SUCH_SKIN", &none), Pick::Atlas, "an unknown id draws the atlas");
        let known = "ABC";
        assert_eq!(pick(known, &|id| id == known), Pick::Skin(known.into()));
        assert_eq!(pick(known, &none), Pick::Atlas, "a skin not loaded yet draws the atlas");
    }
}
