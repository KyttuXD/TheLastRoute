//! The original's painted surfaces (render/textures paints them byte for byte) as GPU textures,
//! painted once per kind and kept for the session.

use super::textures::TexRef;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHE: RefCell<HashMap<(&'static str, &'static str), Option<Texture2D>>> = RefCell::new(HashMap::new());
}

/// Uploads one painted surface (nearest sampling; the world shader repeats UVs).
#[cfg(not(test))]
fn upload(w: u16, h: u16, px: &[u8]) -> Texture2D {
    let t = Texture2D::from_rgba8(w, h, px);
    t.set_filter(FilterMode::Nearest);
    t
}

/// Test only: a distinct raw handle that is never uploaded, so the world mesh builders run with no GL behind them
/// (they only clone and compare textures; see `Assets::headless`).
#[cfg(test)]
fn upload(_w: u16, _h: u16, _px: &[u8]) -> Texture2D {
    use macroquad::miniquad::{RawId, TextureId};
    thread_local! {
        static NEXT: std::cell::Cell<u32> = const { std::cell::Cell::new(1000) };
    }
    let n = NEXT.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    Texture2D::from_miniquad_texture(TextureId::from_raw_id(RawId::OpenGl(n)))
}

/// The texture of a painted kind (nearest sampling; the world shader repeats UVs), or None
/// for a kind the painter tables do not know.
pub fn texture(r: TexRef) -> Option<Texture2D> {
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((r.group, r.kind))
            .or_insert_with(|| {
                let (w, h, px) = r.paint()?;
                Some(upload(w, h, &px))
            })
            .clone()
    })
}
