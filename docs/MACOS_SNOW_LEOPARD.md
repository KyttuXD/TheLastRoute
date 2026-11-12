# Intel Snow Leopard port status

Target: Intel x86_64 OS X 10.6.8. This is not PowerPC and is not yet a working game port.
The normal game artifact still requires macOS 10.12 or later.

## Implemented renderer foundation

`LR_MACOS_LEGACY=1` enables an Intel-only experimental build configuration in
the game and vendored miniquad. Normal builds do not enable it. It:

- removes the renderer's Metal, MetalKit and Vision framework dependencies;
- requests a legacy OpenGL context instead of OpenGL 3.2 Core;
- resolves APPLE vertex-array and EXT framebuffer entry points;
- converts GLES 1.00 shader inputs to desktop GLSL 1.20;
- uses legacy alpha textures instead of red-channel swizzling;
- replaces Lion/Retina fullscreen, mouse-coordinate, DPI and scroll calls;
- forces OpenGL even if Metal was previously selected in saved settings.

The `renderer` job of `.github/workflows/macos-legacy.yml` builds this configuration, audits
linked frameworks, tests shader conversion and legacy Apple shader compilation,
and runs headless gameplay suites. It intentionally uses the supported 10.12
compiler runtime until the runtime/dependency backport is ready. Its reports
are not a Snow Leopard-compatible game download.

## Runtime route

[MacPorts' Rust Portfile](https://github.com/macports/macports-ports/blob/master/lang/rust/Portfile)
maintains Rust 1.78.0 for macOS 10.11 and earlier. It applies
`patch-emulate_tls.diff` below Darwin 11, because native thread-local storage arrived
in OS X 10.7. Upstream rustup's binaries do not provide this backport.

With MacPorts Rust installed on the target Mac, run from the repository:

```sh
sh packaging/macos/legacy/probe.sh
```

The probe compiles directly with `/opt/local/bin/rustc` (override using
`LEGACY_RUSTC=/absolute/path/to/rustc`). It checks allocation, threads, TLS
isolation, files and local TCP, and prints OS/compiler/dynamic-library information.
It creates and removes only its own temporary files. It does not install software.
Passing this probe does not establish game or graphics compatibility.

The `runtime` job of the same workflow fetches the actual
MacPorts Darwin 10 bootstrap (Rust 1.77.0, published under its 1.78.0 release),
its matching standard library and the legacy-support compatibility library.
Downloads are SHA-256 checked and extracted into a private runner directory;
there is no system installation. It compiles and executes the standalone probe
automatically. Modern-host success still does not establish 10.6.8 execution.

After the runtime probe, CI attempts a real game build with that legacy compiler.
It resolves a separate dependency graph in a disposable `git archive` source
copy using MSRV-aware Cargo resolution, then builds with the matching legacy
Cargo. The checkout's release manifests and lockfile are hash-checked for
changes. No experimental executable is packaged or labeled 10.6.8-compatible
until the remaining compiler and runtime issues are resolved.

The standalone runtime probe is also available for developer investigation; it
is not a prerequisite imposed on players. The deliverable must be a self-contained
game, not a requirement for players to install Rust or MacPorts.

## CI

One workflow, `macos-legacy` (`.github/workflows/macos-legacy.yml`), holds both jobs. It runs by hand only
(Actions > macos-legacy > Run workflow, or `gh workflow run macos-legacy`), so pushes and pull requests get no
extra check. The normal Intel build stays in `build.yml` (`macos-intel`, deployment target 10.12 from
`packaging/macos/build_app.sh`). The OpenGL 2.1 smoke test prints `SKIPPED` on a machine with no accelerated
pixel format (hosted runners); run it on a real Mac.

## Legacy dependency graph (2026-11-12)

Found by building the staged copy with Rust 1.77 (`cargo check`, Linux host and `--target x86_64-apple-darwin`).
`packaging/macos/legacy/build-experiment.sh` applies these to the staged copy only; the release `Cargo.toml`
and `Cargo.lock` are not touched.

| Crate | Problem with 1.77 | Staged fix |
|---|---|---|
| `winresource` (build) | pulls `toml` 1.x, edition 2024, unreadable by Cargo 1.77 | line removed (it is a Windows-host-only build dependency anyway) |
| `fontdue` 0.9.5 | `cast_signed` | pinned to 0.9.3 |
| `macroquad` 0.4.16 | const float math (1.82) | 0.4.14 copied into the stage and patched: `size_of` imports, `Color::from_rgba`/`from_hex` not `const`, miniquad requirement set to the vendored 0.4.11 |
| `indexmap`, `zeroize`, `wasm-bindgen`, `web-sys` | declare a newer rust-version | lowered by Cargo's MSRV-aware resolver |

`packaging/macos/legacy/stage-dependencies.sh` does all of this to a disposable copy; `build-experiment.sh` calls it.

The game's own code now builds with 1.77 too. What that took (all in the normal source, same behaviour everywhere):
`Waker::noop` replaced by `shared::task::noop_waker`; inline `const { }` array elements replaced by a const item;
float division in `const fn` colour helpers replaced by the `ui::style::UNIT` table; two borrowed temporaries
(`render/scene.rs`, `world/route/layout.rs`) given names. Keep new code inside Rust 1.77 if the legacy build matters:
the check is `cargo check` under 1.77 on the staged copy (what the `runtime` job does on a Mac).

Checked on Linux with rustup's 1.77 and `--target x86_64-apple-darwin` (`LR_MACOS_LEGACY=1`): the whole game
type-checks. That check cannot compile `ring`'s C or link; the `runtime` job does both and packages
`last_route-macos-10.6-UNTESTED.zip` (deployment target 10.6, MacPorts legacy-support library beside the executable).

Open risk: `rfd` 0.17 (the SKIN tab file panel) uses `objc2-app-kit`, which is written for 10.12+. It compiles;
whether it runs on 10.6.8 is unknown. If it does not, the legacy build needs its own `NSOpenPanel` call.

## Remaining work

- Verify the patched runtime on real 10.6.8; modern GitHub macOS runners cannot do this.
- Get the `runtime` job's game build and link green in CI (it type-checks locally).
- Run `last_route-macos-10.6-UNTESTED.zip` on the 2008 Mac Pro and report what happens (window, GL, audio, HTTPS).
- Verify the legacy renderer and remaining platform calls on Snow Leopard.
- Audit linked Apple frameworks, audio, file dialogs and TLS dependencies.
- Build and test the actual game, retaining gameplay and other OS behavior.

No 10.6.8, 10.5 or 10.0 game compatibility is established by this investigation.
