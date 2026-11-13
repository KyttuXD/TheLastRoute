# Compile speed

Measured on KT's Framework Laptop 13 (Linux, `CARGO_BUILD_JOBS=4`), 2026-11-12. The CI side is in
[CI_SPEED_PLAN.md](CI_SPEED_PLAN.md).

## Where the time goes

A clean dev build of the game takes 68 s. The game crate itself is 46 s of that (one crate of about 190,000 lines);
the 98 dependency units share the rest, the slowest being rustls (8.6 s), image (8.2 s), macroquad (6.2 s),
lr_update (4.8 s), cc (4.6 s) and miniquad (4.1 s), built four at a time.

| What | Time |
|---|---|
| Clean build, dev | 68 s |
| Clean build, dev with `debug = 0` | 65 s (and a 43 MB program in place of 99 MB) |
| Clean build, dev with `opt-level = 1` | 62 s |
| Clean build, both | 57 s |
| Rebuild after touching one file (no real change) | about 3 s |
| `cargo check` after touching one file | about 2 s |
| `cargo check`, first time | 13 s |
| Release build (fat LTO, one codegen unit) | 107 s clean, 96 s with the dependencies built |
| Release build with thin LTO and 16 codegen units | 48 s clean, 39 s with the dependencies built |

What follows from it:

- The profile switches buy little (11 s of 68 with both), and `opt-level = 2` is what makes a dev build playable, so
  the dev profile stays. Use `cargo check` while editing (2 s) and `--profile fast` when a quicker build matters.
- **Thin LTO is not used for shipped builds.** It builds in under half the time but the stress run's average frame
  was 5% slower (0.857 ms against 0.814 ms, three runs each), and frame time comes first
  ([PERFORMANCE.md](PERFORMANCE.md)).
- The one thing that would cut the 46 s is splitting the game crate into several crates, so that an edit rebuilds
  one of them. That is a large refactor and is not started.

## What was done (2026-11-12)

- The game's dependency tree on Linux went from 105 crates to 68 over the day (the `gltf`, `audrey` and Ogg stacks
  are gone; see the git log), and the lockfile from 160 packages to 147.
- The `image` crate and nine crates under it are gone: the vendored macroquad reads and writes PNG with its own
  `png_lite.rs` (miniz_oxide does the zlib part). The game's Linux tree is 51 crates (it was 63, and 105 at the start of
  the day), the lockfile 133 packages; a clean dev build is 65 s (it was 68).
- What cannot go without rewriting it: `sha2` and its six crates (LR Story uses them, and the server's `hmac` and
  `scrypt` need the real crate), `rustls`/`ring`/`webpki-roots` (HTTPS), `rfd` (the file chooser on Windows and Linux),
  `serde_json` (217 files use it).
- Every program, the tests and the web build compile without warnings. Before: 107 in the game, 26 in the vendored
  crates, 24 in Night Watch, 5 in the server. Three were real (two needless `mut`, a dead assignment in a test).
  The vendored crates carry one `allow` line each for upstream code. About 150 items carry
  `#[cfg_attr(not(test), allow(dead_code))]` or `#[allow(dead_code)]` with the note "not wired into the game yet":
  code that is written and tested but that nothing in the game calls (the room panel, the account panel, the update
  badge, the startup gate, the mouse lock owner are whole files of it). They are listed by
  `grep -rn "not wired into the game yet" src`. Wire them in or delete them; the note should not stay for ever.

## Profiles

- `dev`: `opt-level = 2`, dependencies at 3, line tables only.
- `fast` (`cargo build --profile fast`): `opt-level = 1`, dependencies at 2, incremental.
- `ci`: dev with line tables only; what CI tests with.
- `release`: fat LTO, one codegen unit, no debug info; what is shipped.

`.cargo/config.toml` only sets macOS `split-debuginfo=unpacked`.

## Measuring

Cold: `cargo build --bin last_route --timings` into an empty target folder; the report lands in
`<target>/cargo-timings/`. Warm: touch one source file and build again.
