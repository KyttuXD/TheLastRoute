# Last Route patches to miniquad 0.4.11

This is the crates.io miniquad 0.4.11 source with four small changes, all marked "Last Route patch" in the code.
Nothing outside macOS (and the Metal backend, which only macOS and iOS build) behaves differently.

1. `src/graphics/metal.rs`, `new_texture`: an RGBA8 render target is created as BGRA8, because every Metal
   pipeline is built for the view's BGRA8 colour format. Without this, drawing into a macroquad render target fails.
2. `src/graphics/metal.rs`, `apply_scissor_rect`: the y flip uses the height of the current pass (`pass_height`,
   set in `begin_pass`), not the window. macroquad sets a scissor on every draw, so every draw into a smaller
   render target had an out-of-bounds scissor and drew nothing.
3. `src/graphics/metal.rs`, `begin_pass`: a custom pass clears depth only when the pass action is Clear (it
   was always Clear). macroquad draws each call as its own pass with PassAction::Nothing, so depth was wiped
   between draws.
4. `src/native/macos.rs` and `src/lib.rs`: `window::set_frame_pacer(bool)`. The macOS run loop waits on a
   CVDisplayLink at the top of every iteration, which caps it at the display's refresh rate even with the swap
   interval at 0. The static `FRAME_PACER_ENABLED` (default true, the stock behaviour) is read every iteration.

Not implemented in the Metal backend: `texture_read_pixels` (macroquad's `grab_screen` is OpenGL only too), so
screenshots need OpenGL. When miniquad or macroquad is upgraded, check whether upstream fixed 1 to 3 and drop
the ones it did.
