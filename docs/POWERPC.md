# PowerPC

Status (2026-11-12): the game **type-checks** for 64-bit big-endian PowerPC Linux. Nothing has been linked or run on
PowerPC. A much smaller zero-dependency port for weaker machines is started in the separate `LastRoute-RustLite` folder.

| Target | Machines | State |
|---|---|---|
| `powerpc64-unknown-linux-gnu` | G5 Macs and POWER under Linux | `sh packaging/powerpc/check.sh` passes: every crate and the game, no source change needed |
| `powerpc-unknown-linux-gnu` | G3/G4 Macs under Linux | stops in `quad-rand` 0.2.3: it uses `AtomicU64`, which 32-bit PowerPC does not have |
| Mac OS X on PowerPC (10.4, 10.5) | any PowerPC Mac on its own system | not possible: rustc has no `powerpc-apple-darwin` target |

## What 32-bit needs

1. `quad-rand` patched to keep its state without a 64-bit atomic (a vendored copy and one `[patch.crates-io]` line,
   as miniquad and quad-snd already have). `Cargo.toml` is off-limits to agents (AGENTS.md), so this waits for KT.
2. The game's own 64-bit atomics (47 uses in 10 files, for example `net/relay.rs`, `render/scene.rs`,
   `shared/skins.rs`) moved to a small wrapper that uses a mutex where `target_has_atomic = "64"` is false.

## Not known until it runs

- Linking: needs `gcc-powerpc64-linux-gnu`, and X11, GL and ALSA libraries for PowerPC. This PC has none.
- Byte order: the code that reads raw bytes (`app/pacing.rs`, `render/shader.rs`, quad-snd) was not audited for
  big-endian. PNG, glTF and JSON loading go through crates that handle it.
- Graphics: the game asks for OpenGL 3.2 or GLES 2-class shaders. Old PowerPC video cards (Radeon 9x00, GeForce FX)
  stop at OpenGL 2.1; the legacy renderer in [MACOS_SNOW_LEOPARD.md](MACOS_SNOW_LEOPARD.md) is Mac-only today.
- TLS: `ring` builds its C fallback for big-endian PowerPC; untested.
