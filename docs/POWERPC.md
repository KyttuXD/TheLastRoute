# PowerPC

Status (2026-11-12): the game **builds, links and passes its headless suites** as a big-endian PowerPC Linux program,
64-bit and 32-bit, in the qemu user emulator. It has never run on a PowerPC machine and never opened a window there.
A much smaller zero-dependency port for weaker machines is started in the separate `LastRoute-RustLite` folder.

| Target | Machines | State |
|---|---|---|
| `powerpc64-unknown-linux-gnu` | G5 Macs and POWER under Linux | builds; 78 of 78 headless tests and 3854 of 3855 unit tests pass under `qemu-ppc64` |
| `powerpc-unknown-linux-gnu` | G3/G4 Macs under Linux | builds; 78 of 78 headless tests pass under `qemu-ppc` |
| Mac OS X on PowerPC (10.4, 10.5) | any PowerPC Mac on its own system | not possible: rustc has no `powerpc-apple-darwin` target |

A PowerPC Mac therefore needs a Linux on it (Debian ports `ppc64` or `powerpc`, Void PPC glibc, Adelie is musl and
will not load this build).

## Getting a program to try

- **From GitHub:** the `nightly-legacy` pre-release (https://github.com/KyttuXD/TheLastRoute/releases/tag/nightly-legacy)
  holds `last_route-linux-powerpc64-UNTESTED.tar.gz` and `last_route-linux-powerpc32-UNTESTED.tar.gz`. To rebuild
  them: Actions > `powerpc` > Run workflow (manual only, like `macos-legacy`). Each of its two jobs builds the game,
  runs the headless suites under qemu and replaces its file on the release. It is built on Ubuntu 22.04, so it needs
  glibc 2.35 or newer.
- **On a PC:** `sh packaging/powerpc/build.sh [64|32] [suites|test]`. Needs `gcc-powerpc64-linux-gnu` or
  `gcc-powerpc-linux-gnu`, and `qemu-user` for `suites` (the headless suites, about 4 minutes) and `test` (the unit
  tests as well, about 35 minutes under emulation).
- `sh packaging/powerpc/check.sh [64|32]` only type-checks and needs no PowerPC compiler.

On the machine: the program is one file. It loads X11 and OpenGL at run time and needs `libasound.so.2` (ALSA).

## What it needed

1. **64-bit atomics (32-bit only).** 32-bit PowerPC has no 64-bit atomic instruction. `quad-rand` is vendored
   (`vendor/quad-rand`, one patch): its state is two 32-bit halves there. The game's 64-bit atomics come from
   `shared/atomic64.rs`: the standard types where they exist, a mutex around the number on 32-bit PowerPC. New code
   must import `AtomicU64`/`AtomicI64` from there, not from `std`. Test-only files (`hunters/bulk_bench.rs`,
   `net/ws_test_hub.rs`) and `lr_server` still use `std`.
2. **Sound byte order.** quad-snd asked ALSA for little-endian float samples and wrote them in the machine's order,
   which is noise on a big-endian machine. It now asks for the big-endian float format there
   (`audio/quad-snd/src/alsa_snd.rs`).
3. **The link.** Ubuntu has no big-endian PowerPC `libasound` to link against, so `build.sh` links against a
   stand-in with the same name and the 19 functions the game calls. The real library is used on the machine; under
   qemu the stand-in loads and the game runs without sound.

The one unit test that fails under qemu is `world::route::prop_tables::tests::generated_file_matches_generator`:
it starts the PC's `node` from inside the emulated program, with the PowerPC library path, and `node` does not start.
It is not a PowerPC fault. The 32-bit build's unit tests have not been run.

Nothing else needed a change: the headless suites (route generation, saves, the bot runs, the stairs) give the same
results as on a PC, which covers the byte order of the game's own data handling.

## Not known until it runs on a machine

- **Graphics.** The game asks for OpenGL 3.2 or GLES 2-class shaders. Old PowerPC video cards (Radeon 9x00, GeForce
  FX) stop at OpenGL 2.1; the legacy renderer in [MACOS_SNOW_LEOPARD.md](MACOS_SNOW_LEOPARD.md) is Mac-only today.
  A G5 with a newer card under Mesa, or a POWER machine, is the likeliest to show a picture. The renderer's byte
  order (texture uploads, vertex colours) has not been seen on a big-endian GPU driver.
- **Sound.** The big-endian format request is untested on hardware.
- **TLS.** `ring` builds its C fallback for big-endian PowerPC and links; no network test has run there.
- **Speed and memory.** Nothing measured. See [RAM_PASS.md](RAM_PASS.md) for what the game holds on a PC.
