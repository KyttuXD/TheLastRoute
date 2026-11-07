//! Which GL pipeline (material) is bound. The binding is global state that lives on after the pass that set it: macroquad keeps
//! the last `gl_use_material` until `gl_use_default_material`, through `set_camera` and `set_default_camera`. A pass that binds
//! the world material and leaves by an early return, or that simply never restores, would draw everything after it (the
//! blit to the window, the HUD, the menus) with the lit world shader, which paints them black.
//!
//! That is how the co-op black screen happened: the remote runners' pass bound the world material and only the held-item layer
//! put the default back, and with nothing in hand the layer draws nothing, so the whole frame, the UI included, was drawn with
//! the world shader for as long as another runner was in the room. So the world material is only bound through a
//! [`WorldMaterial`] guard, which restores the default when it goes out of scope on every path, and the frame ends with
//! [`reset`] before any screen-space drawing.

use macroquad::prelude::*;

/// The two pipeline bindings a pass moves between. The game uses [`Macroquad`]; the tests record the calls.
pub trait Pipeline {
    /// Binds the world material.
    fn use_world(&mut self);
    /// Binds macroquad's default material.
    fn use_default(&mut self);
}

/// The real binding: macroquad's `gl_use_material` for the world material, `gl_use_default_material` for the rest.
pub struct Macroquad<'m>(&'m Material);

impl Pipeline for Macroquad<'_> {
    fn use_world(&mut self) {
        gl_use_material(self.0);
    }
    fn use_default(&mut self) {
        gl_use_default_material();
    }
}

/// The world material bound for as long as this value lives; the default material is bound again when it is dropped, on a
/// return, an early return or an unwind alike.
#[must_use = "the world material is released when the guard is dropped"]
pub struct WorldMaterial<P: Pipeline> {
    pipeline: P,
}

impl<P: Pipeline> WorldMaterial<P> {
    /// Binds the world material through `pipeline`.
    pub fn enter(mut pipeline: P) -> Self {
        pipeline.use_world();
        WorldMaterial { pipeline }
    }
}

impl<P: Pipeline> Drop for WorldMaterial<P> {
    fn drop(&mut self) {
        self.pipeline.use_default();
    }
}

/// Binds `mat` until the returned guard is dropped.
pub fn world_material(mat: &Material) -> WorldMaterial<Macroquad<'_>> {
    WorldMaterial::enter(Macroquad(mat))
}

/// Binds the default material. The frame calls this before it draws in screen space, so that no pass above can leave the
/// world shader on the window, the HUD or the menus.
pub fn reset() {
    gl_use_default_material();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A pipeline that only remembers what was bound, in order.
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<&'static str>>>);

    impl Recorder {
        fn bound(&self) -> &'static str {
            self.0.borrow().last().copied().unwrap_or("default")
        }
    }

    impl Pipeline for Recorder {
        fn use_world(&mut self) {
            self.0.borrow_mut().push("world");
        }
        fn use_default(&mut self) {
            self.0.borrow_mut().push("default");
        }
    }

    /// The runners' pass in outline: it binds the world material, then may leave early (the mannequin failed to load) or after
    /// drawing the figures. It returns at once for an empty room without binding anything.
    fn runners_pass(p: &Recorder, remotes: usize, mannequin: bool) {
        if remotes == 0 {
            return;
        }
        let _world = WorldMaterial::enter(p.clone());
        if !mannequin {
            return;
        }
    }

    #[test]
    fn the_default_material_is_back_after_every_way_out_of_a_pass() {
        for (remotes, mannequin) in [(0, true), (1, true), (3, true), (1, false)] {
            let p = Recorder::default();
            runners_pass(&p, remotes, mannequin);
            assert_eq!(p.bound(), "default", "{remotes} remotes, mannequin {mannequin}");
        }
    }

    #[test]
    fn the_world_material_is_bound_for_exactly_as_long_as_the_guard_lives() {
        let p = Recorder::default();
        let guard = WorldMaterial::enter(p.clone());
        assert_eq!(p.bound(), "world");
        drop(guard);
        assert_eq!(*p.0.borrow(), ["world", "default"]);
    }

    /// The co-op black screen: another runner in the room and nothing in hand, so the held-item layer draws nothing and used to
    /// be the only thing that unbound the world material. The frame is the passes in order, every frame.
    #[test]
    fn a_frame_with_another_runner_and_an_empty_hand_ends_on_the_default_material() {
        use crate::render::viewmodel::{LayerAction, LayerContent, LayerState};
        let mut layer = LayerState::<u32>::new();
        for frame in 0..4 {
            let p = Recorder::default();
            // the scene binds and releases the world material
            drop(WorldMaterial::enter(p.clone()));
            runners_pass(&p, 1, true);
            // the held-item layer: an empty hand is cleared once and then skipped, and binds no material either way
            let action = layer.next(LayerContent::Empty, 1);
            assert_eq!(action, if frame == 0 { LayerAction::ClearOnly } else { LayerAction::Skip }, "frame {frame}");
            assert_eq!(p.bound(), "default", "frame {frame}: the window and the HUD are drawn next");
        }
    }

    #[test]
    fn the_guard_releases_the_material_while_a_pass_unwinds() {
        let p = Recorder::default();
        let q = p.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _world = WorldMaterial::enter(q);
            panic!("a pass fails half way");
        }));
        assert!(r.is_err());
        assert_eq!(p.bound(), "default");
    }

    /// Every `gl_use_material` in the game sits behind the guard, except the skin preview's, which pairs it with the default on
    /// the next line. A new bare call is the same leak waiting to happen.
    #[test]
    fn no_pass_binds_the_world_material_outside_the_guard() {
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, hits);
                } else if path.extension().is_some_and(|x| x == "rs") {
                    let name = path.to_string_lossy().replace('\\', "/");
                    if name.ends_with("render/pipeline.rs") || name.ends_with("ui/skin_preview.rs") {
                        continue;
                    }
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (i, line) in text.lines().enumerate() {
                        let code = line.split("//").next().unwrap_or("");
                        if code.contains("gl_use_material(") {
                            hits.push(format!("{name}:{}", i + 1));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/last_route"), &mut hits);
        assert!(hits.is_empty(), "use render::pipeline::world_material instead: {hits:?}");
    }
}
