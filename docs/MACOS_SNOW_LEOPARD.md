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

`.github/workflows/macos-legacy-renderer.yml` builds this configuration, audits
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

The `Intel Snow Leopard runtime bootstrap` CI workflow now fetches the actual
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

## Remaining work

- Verify the patched runtime on real 10.6.8; modern GitHub macOS runners cannot do this.
- Isolate and pin legacy dependencies: the current locked graph includes crates
  requiring Rust 1.85. Do not downgrade the normal release dependency graph.
- Verify the legacy renderer and remaining platform calls on Snow Leopard.
- Audit linked Apple frameworks, audio, file dialogs and TLS dependencies.
- Build and test the actual game, retaining gameplay and other OS behavior.

No 10.6.8, 10.5 or 10.0 game compatibility is established by this investigation.
