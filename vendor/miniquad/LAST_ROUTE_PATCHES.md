# Last Route patches to miniquad 0.4.11

This is the crates.io miniquad 0.4.11 source (crate checksum `dcbaea978d741f3b60ced666b06a2a1e4a2bc81cb8b9dd29f4a40965e1f59e65`)
with eight small changes, all marked "Last Route patch" in the code. They are four Metal fixes, two Metal speed
changes, one Metal diagnostic, the pacer switch and one GL texture-cache fix (item 8, the only change that reaches the
GL, Linux, Windows and wasm backends). Everything else outside macOS (and the Metal backend, which only macOS and iOS
build) is the untouched crate source.

`last_route.patch` in this folder is the exact `diff -ruN` of upstream `src/` against `vendor/miniquad/src/`. To check or
upgrade: unpack the crate, apply `last_route.patch` (`patch -p1 -d <crate>/src < last_route.patch`) and compare with
`vendor/miniquad/src`; regenerate it with `diff -ruN <crate>/src vendor/miniquad/src` after any edit here. Cargo.lock has
no source or checksum line for miniquad because it is a path dependency; the checksum above is the pin.

Fixes (needed for the game to draw on Metal):

1. `src/graphics/metal.rs`, `new_texture`: an RGBA8 render target is created as BGRA8, because every Metal
   pipeline is built for the view's BGRA8 colour format. Without this, drawing into a macroquad render target fails.
2. `src/graphics/metal.rs`, `apply_scissor_rect`: the y flip uses the height of the current pass (`pass_height`,
   set in `begin_pass`), not the window. macroquad sets a scissor on every draw, so every draw into a smaller
   render target had an out-of-bounds scissor and drew nothing.
3. `src/graphics/metal.rs`, `begin_pass`: a custom pass clears depth only when the pass action is Clear (it
   was always Clear). macroquad draws each call as its own pass with PassAction::Nothing, so depth was wiped
   between draws.
4. `src/graphics/metal.rs`, `texture_set_min_filter`: edits the texture's stored sampler descriptor, as
   `texture_set_mag_filter` and `texture_set_wrap` do, instead of building a fresh one that was never kept. Before, the
   later mag-filter or wrap call rebuilt the sampler from the creation-time descriptor and undid the min filter (a
   `set_filter(Nearest)` then `repeat()` left min Linear, so minified pixel-art was blurred on Metal only). Min, mag, mip
   and wrap now all live in the one stored descriptor.

Speed (see docs/GRAPHICS_API.md, measured fullscreen 131 to 341 FPS from the first):

5. `src/graphics/metal.rs`, `begin_pass` / `end_render_pass` / `commit_frame`: macroquad begins a new pass for every
   batch with PassAction::Nothing; the stock backend ended and restarted a render encoder for each, which made the GPU
   load and store the whole colour and depth target every time. A pass that continues the previous one onto the same
   target without clearing now reuses its encoder; the encoder ends when another target starts, a pass clears, a
   mipmap or pass is deleted, or the frame commits. Depth is Depth32Float (`DEPTH_FORMAT` in
   `src/native/apple/frameworks.rs`) instead of Depth32Float_Stencil8: the stencil aspect was never used.

Diagnostic:

6. `src/graphics/metal.rs`, `commit_frame`: `LR_METAL_TIMING=1` (aarch64) prints per-frame GPU, wait and encode
   times every 120 frames. Off by default.

Pacer switch:

7. `src/native/macos.rs` and `src/lib.rs`: `window::set_frame_pacer(bool)`. The macOS run loop waits on a
   CVDisplayLink at the top of every iteration, which caps it at the display's refresh rate even with the swap
   interval at 0. The static `FRAME_PACER_ENABLED` (default true, the stock behaviour) is read every iteration.

GL / WebGL fix (all GL targets, wasm included):

8. `src/graphics/gl.rs` `delete_texture` and `src/graphics/gl/cache.rs` `GlCache::forget_texture`: upstream has the
   `clear_texture_bindings()` call in `delete_texture` commented out, so the binding cache (`textures[slot]`,
   `stored_texture`) keeps a deleted GL name. The next `new_texture`, `texture_set_filter`, `texture_set_wrap` or
   `texture_update` stores that slot and rebinds it afterwards, which the web shim (`js/gl.js` `validateGLObjectID`)
   reports as "glBindTexture called with an already deleted texture ID N" (harmless there: its IDs never repeat), and
   which on native GL, where names are reused, makes a new texture that got the same name skip its bind so its upload
   goes to the wrong texture. `delete_texture` now resets just the deleted name's cache entries (no GL call, other
   bindings keep their entries) before `glDeleteTextures`. Two unit tests sit at the end of `cache.rs`
   (`cargo test --lib` inside this folder).

Old systems (legacy build only, `cfg(lr_macos_legacy)`; found by reading, then on a Mac Pro 3,1):

9. `src/native/macos.rs` and `src/native/apple/frameworks.rs`: calls that OS X 10.6 or 10.5 do not have are left out
   or asked for first. File drops go through `NSFilenamesPboardType` and `propertyListForType:` (every system since
   10.0): `NSPasteboardTypeFileURL` is a 10.13 data symbol, and a program that asks for it does not start on an
   older AppKit; `readObjectsForClasses:options:` is 10.6. `NSRunningApplication` (10.6)
   is replaced by `-[NSApplication activateIgnoringOtherApps:]` and `NSProcessInfo.processName`.
   `setActivationPolicy:` and `-[NSImage initWithCGImage:size:]` (both 10.6) are called only when the system answers
   to them (this check is in every build; it is always true on a current system). The unused `ImageIO` framework link
   is dropped.

Not implemented in the Metal backend: `texture_read_pixels` (macroquad's `grab_screen` is OpenGL only too), so
screenshots need OpenGL. A Metal window is also held to the refresh rate by the compositor (docs/GRAPHICS_API.md). When
miniquad or macroquad is upgraded, check whether upstream fixed 1 to 5 and drop the ones it did.
