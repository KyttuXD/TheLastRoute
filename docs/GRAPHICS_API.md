# Graphics API (Metal and OpenGL) and the macOS frame pacer

macOS can draw the game through Metal or OpenGL. Every other system uses OpenGL. The choice is made once, before the window opens (miniquad cannot switch afterwards), in `app/gfx.rs`.

## Choosing

Settings, VIDEO tab, GRAPHICS API (macOS only; takes effect the next time the game starts): AUTO, METAL, OPENGL. Saved as `graphics_api` in settings.json (missing means AUTO). The row says it is experimental, with the measured numbers below.

Precedence, first match wins:

1. Not macOS, or `MTLCreateSystemDefaultDevice()` returns null: OpenGL.
2. `--gfx auto|metal|gl`, then env `LR_GFX`, then the saved setting.
3. An explicit `--gfx metal` or `LR_GFX=metal` is honoured over everything below.
4. `--screenshot`, `--self-test` and `--agent-window` use OpenGL: they read the screen back, and the Metal backend cannot (see below).
5. A crash marker (below) forces OpenGL, and the settings row says "OPENGL: LAST METAL START DID NOT FINISH".
6. AUTO is OpenGL everywhere. Owner decision: until Metal is measured faster than OpenGL on a device in the way people play (see "Measured"), AUTO does not pick it. METAL in the setting, `--gfx metal` and `LR_GFX=metal` still give Metal. The switch is `auto_metal_on_apple_silicon` in `data/app/gfx.json` (false); set it to true to give Apple Silicon Metal under AUTO.

The log's first lines say what was picked and why (`graphics api: Gl (AUTO: OpenGL until Metal is measured faster)`).

### Crash guard

A Metal start writes `gfx_boot.marker` (name and frame count in `data/app/gfx.json`) to the per-user folder before the window opens. The marker holds the process id. It is removed by a clean end: after 120 drawn frames, or by an `atexit` hook when the process exits normally earlier (Cmd-Q on the boot screen, a double launch, `--self-test`), unless the run panicked (a panic hook records that). An abort or a kill skips `atexit`, so the marker survives. At the next launch the marker counts as a crash only when its process is gone: a second instance started next to a running Metal one ignores the first's marker. A crash drops AUTO and METAL-in-settings to OpenGL; start once with `--gfx metal` to retry, and a good run clears the marker. Tests (`app::gfx`) cover the marker lifecycle in a temporary folder: write, other instance ignored, crash detected, clean end, panic.

## What Metal needs

- The world shader has a Metal twin (`MSL` in `render/shader.rs`). It must stay line for line with the GLSL. Three tests check it: the lighting constants, the uniform byte layout (macroquad packs uniforms with no padding, so MSL uses `packed_float3` and `packed_float4`), and `msl_source_compiles_with_the_metal_compiler`, which compiles the twin with the system Metal compiler and looks up both entry points (skipped where no Metal device exists).
- Render targets are top-down on Metal and bottom-up on OpenGL, so the three places that draw a render-target texture use `flip_y: !platform::is_metal()` (scene and view in `app/mod.rs`, the skin preview). `is_metal()` reads miniquad once and caches it (call it only after the window is open).
- miniquad 0.4.11 is vendored in `vendor/miniquad`; `vendor/miniquad/LAST_ROUTE_PATCHES.md` lists every change and `vendor/miniquad/last_route.patch` is the exact diff from the crates.io source. Without the Metal fixes the 3D view is black on Metal.

## Known limits of the Metal backend

- No screen read-back: `get_screen_data` (macroquad's `grab_screen`) is OpenGL only and `texture_read_pixels` is unimplemented in miniquad's Metal backend. `save_screen` and the F-key capture return before touching the disk on Metal and log a line (the F-key line says "SCREENSHOT NEEDS OPENGL"). Start with `--gfx gl` to take screenshots. Night Watch always uses OpenGL.
- Indices are u16 only with base element 0; the uniform buffer holds about 5400 draws per frame (the game draws 50-60).
- There is no runtime fallback: a shader or pipeline error is a panic.
- A window on a Retina display: miniquad's Metal view always draws at the backing resolution, while the OpenGL view with `high_dpi: false` draws at 1x. On a 2x display Metal therefore shades about four times the pixels. Not measured (the M4 here is on a 1x 1080p display); do it before AUTO is allowed to pick Metal on a laptop panel.

## Measured (Apple M4, macOS 27, 1920x1080 at 75 Hz, release build, stress `fly`, 12-15 s runs)

Why Metal first looked slow: its command buffer showed 8-12 ms of "GPU time" per frame. That was not shading. With `presentDrawable` left out, the same frame's GPU time is 0.9 ms. The 12 ms is the command buffer waiting for the compositor to hand back a drawable, and in a window the compositor releases one per refresh (75 Hz = 13.3 ms) even with the layer's `displaySyncEnabled` NO (read back as NO). So **a Metal window is held to the display's refresh rate whatever VSYNC says**; fullscreen is not.

| case (vsync off, pacer off) | avg FPS, three runs | note |
|---|---|---|
| OpenGL, window | 264, 265 | no compositor wait |
| Metal, window | 73.8, 73.7, 73.9 | compositor-paced |
| OpenGL, fullscreen | 261, 188, 196 | high variance |
| Metal, fullscreen | 342, 341, 339 | 1.3-1.8x OpenGL, tight |

What mattered in Metal:

- One render encoder per macroquad batch (the stock behaviour, a load and store of colour and depth each time): fullscreen 131-133 FPS. Reusing the encoder across passes onto the same target (patch below): 341 FPS. This is the whole Metal gain.
- Depth32Float instead of Depth32Float_Stencil8: no measurable change (342.5, 341.1 vs 342, 341, 339). Kept anyway: the stencil aspect was never used.
- The 16-lamp loop: removing it from the fragment shader changed nothing in a window (the present wait dominates), and the whole GPU run is 1.2 ms per frame fullscreen, so the loop is not a bottleneck here. The twin keeps it (it must match the GLSL). Not isolated on Intel.
- Mipmaps and filters: the stored-sampler fix below changes what props look like, not the frame time.

AUTO stays OpenGL because most people play in a window, where Metal is slower than OpenGL. When fullscreen play is the target, Metal is faster on Apple Silicon; flip `auto_metal_on_apple_silicon` once the Retina cost and a windowed fix are measured. The Intel MacBook Air (KT-MBA) was not reachable over ssh for this pass, so Metal on Intel is unmeasured.

`LR_METAL_TIMING=1` prints, every 120 frames, where a Metal frame goes (commit to GPU start, GPU run, wait return, drawable acquire, time outside the commit). It costs two clock reads per frame and is off by default.

## The macOS frame pacer, VSYNC and FPS LIMIT

Defaults: VSYNC on, FPS LIMIT 0 (UNLIMITED; the slider's top stop, 1000, is UNLIMITED too; a saved exact 60 stays as the player's choice), as in Godot (`DEFAULT_VSYNC := true`). With VSYNC on the loop is held to the display's refresh, so a 75 Hz screen shows 75 FPS (59-60 on a 60 Hz one): that is vsync working, not a limit. The Settings text under the two rows says so.

miniquad's macOS run loop waits on a CVDisplayLink at the top of every iteration, whatever the swap interval is. Measured (OpenGL, window, vsync off): pacer left on 74.1 FPS, pacer off 264 FPS. So that wait, not OpenGL, was the 60-75 cap with vsync off.

The vendored miniquad exposes `window::set_frame_pacer(bool)` (a shared `AtomicBool` read every loop iteration). `app::pacing::apply_vsync` calls it with the VSYNC setting: on keeps the display-link wait (and sets the swap interval, or the Metal layer's `displaySyncEnabled`, to on); off skips the wait and presents at once. The toggle is live, for both backends. `LR_FRAME_PACER=on|off` forces the wait whatever VSYNC says (a measuring switch). Other systems are not affected.

FPS LIMIT works above the refresh rate when VSYNC is off: `Pacer::wait` sleeps to a fixed frame grid. Measured (OpenGL, vsync off, `--stress-limit N`): 60 gave 58.8, 120 gave 117.7, 200 gave 194.0 FPS. With VSYNC on a cap above the refresh does nothing. In a Metal window the cap cannot exceed the refresh either (see Measured).

With vsync off and no limit the loop has no wait of its own: it uses a core flat out and can tear on a screen that does not match the frame rate. A window that is fully hidden still sleeps (miniquad's occlusion throttle, 60 Hz); a visible but unfocused window does not. Set an FPS LIMIT to keep laptops cool.

### Linux, the browser and phones

Linux (X11 GLX, X11 EGL, Wayland EGL): `apply_vsync` calls the vendored `window::set_swap_interval`; the loop applies it on the GL thread just before the next swap (one relaxed atomic load per frame, the GL call only on a change), so VSYNC is live and FPS LIMIT works above the refresh with VSYNC off. The driver may still ignore the request (a compositor that forces vsync, such as some Wayland setups).

Browser (wasm): requestAnimationFrame paces the loop at the screen's refresh whatever VSYNC and FPS LIMIT say, so the FPS LIMIT wait is skipped (`app/gfx.rs`) and VSYNC has no effect. Phones (Android, iOS): the compositor paces the frame the same way; the rows do not raise the rate. The Settings note says the browser is screen-paced.

## Measuring

```sh
LR_GFX=gl    target/release/last_route --stress 60 --stress-log gl.csv    --stress-vsync off
LR_GFX=metal target/release/last_route --stress 60 --stress-log metal.csv --stress-vsync off
LR_GFX=gl    target/release/last_route --stress 60 --stress-vsync off --stress-limit 120
```

Compare `avg_fps` and `low_1pct_fps` in the `# summary` line, and repeat at least three times: OpenGL fullscreen varies by 30% between runs. `--stress-limit N` caps the run at N FPS through the same pacer as the FPS LIMIT row (0 or absent: none). Nothing in CI opens a window, so Metal itself is a manual check.

## Manual Metal checklist (run before AUTO is allowed to pick Metal)

Start with `--gfx metal` and look at each, comparing with `--gfx gl`:

1. A stress run (`--stress 20 --stress-vsync off`) ends with a summary line and no panic.
2. The 3D view is lit like OpenGL (corridor walls, flashlight cone, lamps), props are not upside down.
3. Menus, the settings panel and the pause menu draw; the minimap is upright.
4. The skin preview on the title/skin screen is upright (it flips with `is_metal()`).
5. The locker camera and the held-item view (render targets) are upright and not black.
6. Pixel-art props and the atlas are sharp (nearest filter, not blurred) up close and far away.
7. Resize and fullscreen toggle do not black out the window.
8. Quit with Cmd-Q on the boot screen, then start with AUTO plus METAL in settings: it must still choose Metal (no stale marker).
