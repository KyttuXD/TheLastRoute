# PowerPC

Status (2026-11-12): the game **type-checks** for 64-bit and 32-bit big-endian PowerPC Linux. Nothing has been linked
or run on PowerPC. A much smaller zero-dependency port for weaker machines is started in the separate `LastRoute-RustLite` folder.

| Target | Machines | State |
|---|---|---|
| `powerpc64-unknown-linux-gnu` | G5 Macs and POWER under Linux | `sh packaging/powerpc/check.sh` passes |
| `powerpc-unknown-linux-gnu` | G3/G4 Macs under Linux | `sh packaging/powerpc/check.sh 32` passes |
| Mac OS X on PowerPC (10.4, 10.5) | any PowerPC Mac on its own system | not possible: rustc has no `powerpc-apple-darwin` target |

## What 32-bit needed

32-bit PowerPC has no 64-bit atomic instruction.

1. `quad-rand` is vendored (`vendor/quad-rand`, one patch, `[patch.crates-io]`): its state is two 32-bit halves there.
2. The game's 64-bit atomics come from `shared/atomic64.rs`: the standard types where they exist, a mutex around the
   number on 32-bit PowerPC. New code must import `AtomicU64`/`AtomicI64` from there, not from `std`.
   Test-only files (`hunters/bulk_bench.rs`, `net/ws_test_hub.rs`) and `lr_server` still use `std`.

## Not known until it runs

- Linking: needs `gcc-powerpc64-linux-gnu`, and X11, GL and ALSA libraries for PowerPC. This PC has none.
- Byte order: the code that reads raw bytes (`app/pacing.rs`, `render/shader.rs`, quad-snd) was not audited for
  big-endian. PNG, glTF and JSON loading go through crates that handle it.
- Graphics: the game asks for OpenGL 3.2 or GLES 2-class shaders. Old PowerPC video cards (Radeon 9x00, GeForce FX)
  stop at OpenGL 2.1; the legacy renderer in [MACOS_SNOW_LEOPARD.md](MACOS_SNOW_LEOPARD.md) is Mac-only today.
- TLS: `ring` builds its C fallback for big-endian PowerPC; untested.
