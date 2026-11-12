# Last Route patches to macroquad 0.4.16

The crates.io source (`src/` and the licences; examples, tests and `js/` are left out) with two changes, each marked
"Last Route patch": one in `src/text/atlas.rs`, one in `src/quad_gl.rs`.

**The glyph atlas row test.** `Atlas::cache_sprite` decided whether a sprite fits the current row with
`cursor_x + width < image.width`, then placed it at `cursor_x + GAP` (GAP is 2). A sprite that passed by one or two
pixels overran the row, and the overflow path doubled the atlas in both directions instead of starting a new row.
Measured in the game: 45 glyphs at 15 px took a 512x512 atlas to 1024x1024, and 250 took it to 2048x2048, which is
16 MB of RGBA held in memory plus the same again as a texture, nearly all of it empty. The test is now
`cursor_x + GAP + width <= image.width`. Nothing else changes: sprites keep their gap and the atlas still doubles
when it is really full.

**`QuadGl::geometry_with`.** `geometry` copies a slice of vertices into the batch buffer. The game keeps the floor's
meshes in an 18-byte vertex (`src/bin/last_route/render/packed_mesh.rs`) and unpacks them each frame; with
`geometry_with(vertex_count, indices, write)` the unpacked vertices are written straight into the batch buffer, in
place of into a buffer of the game's and copied from there. `geometry` now clamps as before and calls it, so every
other caller behaves as it did. The Snow Leopard build's 0.4.14 copy does not get this one: the game falls back to the
copy there (`cfg(lr_macos_legacy)`).

To check or upgrade: `diff -ru <crate>/src vendor/macroquad/src` shows these two changes. `Cargo.toml` here carries the
crate's own dependency table; `packaging/macos/legacy/stage-dependencies.sh` applies the atlas fix to the 0.4.14 copy
the Snow Leopard build uses.
