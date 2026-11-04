# Graphics API (Metal and OpenGL) and the macOS frame pacer

macOS can draw the game through Metal or OpenGL. Every other system uses OpenGL. The choice is made once, before the window opens (miniquad cannot switch afterwards), in `app/gfx.rs`.

## Choosing

Settings, VIDEO tab, GRAPHICS API (macOS only; takes effect the next time the game starts): AUTO, METAL, OPENGL. Saved as `graphics_api` in settings.json (missing means AUTO).

Precedence, first match wins:

1. Not macOS, or `MTLCreateSystemDefaultDevice()` returns null: OpenGL.
2. `--gfx auto|metal|gl`, then env `LR_GFX`, then the saved setting.
3. An explicit `--gfx metal` or `LR_GFX=metal` is honoured over everything below.
4. `--screenshot`, `--self-test` and `--agent-window` use OpenGL: they read the screen back, and the Metal backend cannot (see below).
5. A crash-guard marker left by an earlier Metal start forces OpenGL.
6. AUTO is Metal on Apple Silicon (aarch64) and OpenGL on Intel, until Metal is measured on the Intel MacBook Air (KT-MBA). METAL in the setting gives Metal on Intel too.

The log's first lines say what was picked and why (`graphics api: Metal (AUTO on Apple Silicon)`).

### Crash guard

A Metal start writes `gfx_boot.marker` (name and frame count in `data/app/gfx.json`) to the per-user folder before the window opens, and deletes it after 120 drawn frames. A panic inside miniquad's Metal backend aborts the process, so the marker survives and the next AUTO or METAL-in-settings launch uses OpenGL. To try Metal again, start once with `--gfx metal`; a good run removes the marker. A run that quits before 120 frames also leaves the marker.

## What Metal needs

- The world shader has a Metal twin (`MSL` in `render/shader.rs`). It must stay line for line with the GLSL; two tests check the lighting constants and the uniform byte layout (macroquad packs uniforms with no padding, so MSL uses `packed_float3` and `packed_float4`).
- Render targets are top-down on Metal and bottom-up on OpenGL, so the three places that draw a render-target texture use `flip_y: !platform::is_metal()` (scene and view in `app/mod.rs`, the skin preview).
- miniquad 0.4.11 is vendored in `vendor/miniquad` with three Metal fixes and the pacer switch; see `vendor/miniquad/LAST_ROUTE_PATCHES.md`. Without the three fixes the 3D view is black on Metal.

## Known limits of the Metal backend

- No screen read-back: `get_screen_data` (macroquad's `grab_screen`) is OpenGL only and `texture_read_pixels` is unimplemented in miniquad's Metal backend. `save_screen` and the F-key capture do nothing on Metal and log a line (the F-key line says "SCREENSHOT NEEDS OPENGL"). Start with `--gfx gl` to take screenshots. Night Watch always uses OpenGL.
- Indices are u16 only with base element 0; the uniform buffer holds about 5400 draws per frame (the game draws 50-60).
- There is no runtime fallback: a shader or pipeline error is a panic.
- Measured on an M4 (macOS 27, 75 Hz display, stress `fly`): Metal is GPU-bound at about 8-11 ms per frame (`MTLCommandBuffer` GPUStartTime to GPUEndTime), and unlimited it ran slower than OpenGL. Merging macroquad's one-encoder-per-draw passes, and not waiting for the GPU every frame, made no measurable difference, so the cost is not that. It is not understood yet; measure before promising Metal gains on any Mac.

## The macOS frame pacer

miniquad's macOS run loop waits on a CVDisplayLink at the top of every iteration, whatever the swap interval is. That capped the loop at the display's refresh rate even with VSYNC off (73-75 FPS on a 75 Hz panel, on OpenGL and Metal alike). It is not an OpenGL limit.

The vendored miniquad exposes `window::set_frame_pacer(bool)` (a shared `AtomicBool` read every loop iteration). `app::pacing::apply_vsync` calls it with the VSYNC setting: on keeps the display-link wait (and sets the swap interval, or the Metal layer's `displaySyncEnabled`, to on); off skips the wait and presents at once. The toggle is live, for both backends, and also applies to `--stress-vsync`. Other systems are not affected.

With vsync off on a display that does not match the frame rate, expect tearing; the FPS LIMIT row still holds the loop to a fixed grid.

## Measuring

```sh
LR_GFX=gl    target/release/last_route --stress 60 --stress-log gl.csv    --stress-vsync off
LR_GFX=metal target/release/last_route --stress 60 --stress-log metal.csv --stress-vsync off
```

Compare `avg_fps` and `low_1pct_fps` in the `# summary` line. Repeat with `--stress-vsync on` for the display-paced numbers. Nothing in CI opens a window, so Metal itself is a manual check: the run must finish with a summary line and no panic.
