//! The original's painted surfaces (render/textures paints them byte for byte) as GPU textures,
//! painted once per kind and kept for the session.

use super::textures::TexRef;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHE: RefCell<HashMap<(&'static str, &'static str), Option<Texture2D>>> = RefCell::new(HashMap::new());
}

/// The texture of a painted kind (nearest sampling; the world shader repeats UVs), or None
/// for a kind the painter tables do not know.
pub fn texture(r: TexRef) -> Option<Texture2D> {
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((r.group, r.kind))
            .or_insert_with(|| {
                let (w, h, px) = r.paint()?;
                let t = Texture2D::from_rgba8(w, h, &px);
                t.set_filter(FilterMode::Nearest);
                Some(t)
            })
            .clone()
    })
}
