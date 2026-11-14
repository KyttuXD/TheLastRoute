# Last Route patch to macroquad 0.4.16

The crates.io source (`src/` and the licences; examples, tests and `js/` are left out) with one change, marked
"Last Route patch" in `src/text/atlas.rs`.

**The glyph atlas row test.** `Atlas::cache_sprite` decided whether a sprite fits the current row with
`cursor_x + width < image.width`, then placed it at `cursor_x + GAP` (GAP is 2). A sprite that passed by one or two
pixels overran the row, and the overflow path doubled the atlas in both directions instead of starting a new row.
Measured in the game: 45 glyphs at 15 px took a 512x512 atlas to 1024x1024, and 250 took it to 2048x2048, which is
16 MB of RGBA held in memory plus the same again as a texture, nearly all of it empty. The test is now
`cursor_x + GAP + width <= image.width`. Nothing else changes: sprites keep their gap and the atlas still doubles
when it is really full.

**No `image` crate.** `src/png_lite.rs` is a small PNG reader and writer, and `Image::from_file_with_format`,
`Texture2D::from_file_with_format` and `Image::export_png` use it. The game only loads PNG and only asks for 8-bit
RGBA. Gone with `image`: `png`, `flate2`, `fdeflate`, `crc32fast`, `simd-adler32`, `color_quant`, `bytemuck`,
`byteorder`, `num-traits` and a second copy of `miniz_oxide`. `ImageFormat` is now this crate's own one-variant enum
and `Error::ImageError` holds a `String`. Checked: all 2238 pictures in the repository (PNG files, pictures inside
GLB models, bbmodel textures) decode to the same bytes as with the `image` crate
(`src/bin/last_route/shared/png_parity_tests.rs`, run by hand). The Snow Leopard build's 0.4.14 copy keeps `image`.

Two build-only changes besides, neither of which changes what the crate does:

- `Cargo.toml` names `miniz_oxide` in place of `image`.
- `src/lib.rs` starts with an `#![allow(...)]` for the lints newer compilers raise on the upstream code, so a build of
  the game shows the game's own warnings only.

To check or upgrade: `diff -ru <crate>/src vendor/macroquad/src` shows the atlas hunk and the `allow` line. `Cargo.toml` here carries the
crate's own dependency table; `packaging/macos/legacy/stage-dependencies.sh` applies the same fix to the 0.4.14 copy
the Snow Leopard build uses.

Dependency trim (2026-11-13): `macroquad_macro` is gone (the game hand-expands the entry point, so `#[macroquad::main]`,
`#[macroquad::test]` and the scene `CapabilityTrait` derive are removed), and so are the unused optional `log` and
`backtrace` dependencies with the `log-rs` and `glam-serde` features. `set_panic_handler` always reports "Macroquad compiled
without backtrace". The Snow Leopard build's 0.4.14 copy is untouched.
