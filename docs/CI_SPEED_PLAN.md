# CI speed plan

Goal: a reasonable compile time on every platform, macOS and Windows first. Source: the CI audit
(last 40 runs, job-level data only; step-level timings were not pulled, so the split between cold
compile, link and test run is inferred). Workflows: `.github/workflows/test.yml` and `build.yml`.
This file is a plan. The "Already done" table lists what has landed; the rest is open.

## Where we are

| Job | Typical | Worst | Failure notes |
|---|---|---|---|
| test (windows-latest) | 8-10 min | 18 min | 4 failed, 4 cancelled of 19 completed |
| test (ubuntu-22.04) | 4-6 min | 7 min | fails at headless suites or cargo test |
| build windows | 3-5 min | 6 min | cache key `windows`, release profile |
| build linux | 2-4 min | 4 min | one packaging failure |
| build macos (Intel, arm64, universal) | 1-2 min | 2 min | build_app.sh and zip failures |

Overall failure rate: test about 21%, build about 15%. No job has `timeout-minutes`. The build
workflow has `contents: write` at workflow level. Actions artifact storage is full (uploads are
`continue-on-error`, so this is silent).

Main cause of slow Windows tests (inferred): `test.yml` (cache key `test-server-<os>`, `ci` profile) and
`build.yml` (cache key `windows`, `release` profile) each compile the crate in separate caches.

## Already done

| Platform | Change | Effect |
|---|---|---|
| All | Toolchain pinned (`rust-toolchain.toml` and `dtolnay/rust-toolchain` at 1.99.1) | Same compiler locally and in CI; no surprise rebuilds from a moving stable |
| All | Tests build once with the `ci` profile (lib, last_route and lr_server in one cargo run), and the bins reuse that build | Removed one full crate compile in test.yml |
| macOS | Intel and Apple Silicon slices build in parallel jobs, then a short `macos-universal` job joins them | Wall time is the slower slice, not the sum |
| macOS | `split-debuginfo=unpacked` in `.cargo/config.toml` | Faster links |
| Release | `debug = 0` for release, line tables only for `ci` | Smaller objects, faster link |
| Local | `fast` profile (see COMPILE_SPEED.md) | Local iteration only |
| All | Separate cache key for the `--features server` crate graph: `test-server-<os>` in test.yml, `linux-server` in build.yml (its Linux packaging also builds `lr_server`); plain game builds keep `windows`, `macos-*` and the `nw-*` and `lrs-*` keys | A run no longer overwrites the cache of a different dependency graph |
| All | Build steps stamp start and end (UTC) and append `step, os, cache-hit, elapsed seconds` to the run summary (`GITHUB_STEP_SUMMARY`), even when the step fails; build.yml, test.yml, nightly-lrs.yml and nightly-nw.yml | The "Measure first" table below can be filled from run summaries instead of `gh run view` |
| All | One job per target (Windows, Linux, macOS Intel, macOS Apple Silicon in build.yml; a matrix or separate jobs in test.yml and the nightlies), the same pinned toolchain in every workflow, `Swatinem/rust-cache` in every job, and no new crate dependency | Targets build in parallel; wall time is the slowest target |
| All | `actions/checkout@v6` and `actions/upload-artifact@v6` (2026-11-12) | Both run on Node 24; the "Node.js 20 is deprecated" warning on every job is gone |
| Linux | The apt step installs `libasound2-dev` only and refreshes the package index only if the install fails (2026-11-12). The game loads X11 and OpenGL at run time, so their `-dev` packages, `pkg-config` and `xvfb` were never needed to build or test | The step was 22 to 24 s |
| test | After the unit tests only the game is built for the headless suites; the server, Night Watch and the two LR Story programs are type-checked (`cargo check --bins`) in place of built and linked (2026-11-12) | The step was 95 s |
| test | `timeout-minutes: 40` (2026-11-12) | Bounds a hung run |
| Release | Thin LTO was measured and rejected (2026-11-12): the build is 2.2 times faster, the game 5% slower (COMPILE_SPEED.md) | The release builds stay as they are; they are the floor of build.yml |
| test | Split into two halves that run side by side (2026-11-12): `unit` (the game's unit tests) and `suites` (library and server tests, the game built for the headless suites, the other programs checked) | A push is checked in the time of the longer half |
| build macOS Intel | Cross-compiled on the Apple Silicon runner (2026-11-12) | The Intel runner took 8 to 9 minutes for the same build |
| Release | `codegen-units = 16` with fat LTO (2026-11-12): plays the same, builds about 40% faster | Every release job (Windows, Linux, both Macs, web) |

Goal (KT, 2026-11-12): every job of a push done in under 5 minutes.

Reached on the first run after these changes (commit d38dd05; the two test halves and the Intel build started with
empty caches, so they should not get slower):

| Job | Before | After |
|---|---|---|
| test | 7 min 32 s to 8 min 42 s | unit 4 min 30 s, suites 2 min 49 s (side by side) |
| build windows | 5 min 41 s | 4 min 47 s |
| build macos-intel | 8 min 59 s | 3 min 9 s |
| build macos-arm64 | 4 min 35 s | 4 min 3 s |
| build linux | 4 min 36 s | 2 min 31 s |
| deploy-web | 3 min 7 s | 2 min 13 s |

The margin is thin on two: the unit half (244 s of it is one `cargo test`, compile and run) and Windows. If the unit
half grows past five minutes, the next step is to run its tests in two shards.

## Step timings on a push (run of 2026-11-12, commit 0b82938, warm caches)

| Job | Whole job | Slowest steps |
|---|---|---|
| test (ubuntu-22.04) | 7 min 32 s | `cargo test` 301 s, `cargo build --bins` 95 s, apt 22 s |
| build linux | 4 min 36 s | `packaging/linux/build.sh` 219 s (two fat-LTO release builds), apt 24 s |
| build windows | 5 min 41 s | `cargo build --release` 294 s |
| build macos-intel | 8 min 59 s | Build the Intel app 478 s |
| build macos-arm64 | 4 min 35 s | Build the Apple Silicon app 241 s |
| deploy-web | 3 min 07 s | Build the wasm 142 s |

The release builds are almost all fat-LTO time on the game crate, which the cache cannot hold. The test job is the
unit test compile and run (about half each).

## Next, per platform

Saving figures are estimates until step timings are measured (see "Measure first").

### Windows (first)

| Change | Expected saving | Risk |
|---|---|---|
| One shared `rust-cache` key and one profile for build and test (`shared-key: win-ci`, build from the `ci` binaries that the test job already produced, or make build reuse the test artifacts) | 2-4 min per run, removes the second cold compile | Medium: release binaries must still be optimised; if release must differ, share only the dependency cache by using the same profile for dependencies |
| sccache with `RUSTC_WRAPPER=sccache` and the GitHub cache backend | 3-6 min on cold runs (cache eviction, dependency bumps); little on warm | Low-medium: incremental compilation must be off (CI already is); cache size limits |
| `cargo nextest` for the test step | 1-2 min if the suite is large; faster failure output | Low: doctests are not run by nextest, so keep a `cargo test --doc` step if any exist |
| Split lib and bin tests into parallel jobs | Wall time down by the shorter half | Low, but doubles compile unless the cache is shared first; do after the shared key |
| `timeout-minutes: 25` on the job | Caps the 18 min worst case and hung runs | Low |
| Self-test stays out of CI (needs OpenGL and audio) | n/a | n/a |

### macOS

The jobs are already fast (1-2 min); failures cost more than speed here.

| Change | Expected saving | Risk |
|---|---|---|
| Check the `.app` exists and print its path before `zip` in `packaging/macos/universal.sh` (not part of this file) | Fewer red runs, fewer reruns | Low |
| Shared cache key per slice (`shared-key: macos-arm64`, `macos-x86_64`) between build and any future test job | 1-2 min if a macOS test job is added | Low |
| sccache on both slice jobs | 1-3 min on cold runs | Low |
| `timeout-minutes: 15` per job | Bounds runner cost | Low |
| Keep the toolchain pinned with explicit `targets` | Avoids a toolchain download on each run | Low |

### Linux

| Change | Expected saving | Risk |
|---|---|---|
| Cache apt packages or skip `apt-get update` (only `install`) | 20-60 s per job | Low: stale package index can fail the install; keep a fallback to update on failure |
| Shared cache key between test and build | 1-2 min | Medium, as for Windows |
| lld or mold linker | 10-40 s per link | Low: confirm the linker is on the runner first |
| `timeout-minutes: 15` | Bounds hangs | Low |

### All platforms

| Change | Expected saving | Risk |
|---|---|---|
| `permissions: contents: read` at workflow level in build.yml, `contents: write` only on jobs that publish to nightly | No speed gain; smaller blast radius | Low: a missing write on a publish job fails that job, so test on a branch |
| Publish to the VPS as its own job gated on the nightly job (secrets only there) | Test and build finish without waiting for SSH | Medium: needs artifact hand-off between jobs; artifact storage is full, so use a run-scoped cache as the macOS slices do |
| `concurrency` group per ref with `cancel-in-progress` for test | Saves runner time on stacked pushes | Low: do not cancel main publish runs |
| `timeout-minutes` on every job | Bounds worst case | Low |

## Order of work

1. Fix the two failing headless suites (red test job on main is worth more than any speed-up).
2. Measure first: `gh run view <id> --json jobs` for step timings, so cold compile, link and test are separate numbers.
3. Shared cache keys and one profile per platform (Windows first).
4. Timeouts and per-job permissions (no risk to speed, quick wins on safety).
5. sccache on Windows and macOS; re-measure over five runs and keep it only if the median drops.
6. nextest, then the lib/bin test split, only if test time still dominates.
7. Linux apt cache and linker.

## Measure first

Record for each job before and after a change: cold and warm total, compile step, link step,
test step, cache size. Keep the table in this file so a change that does not help can be reverted.

The workflows now write the compile step's elapsed seconds and the cache-hit flag to each run's summary page
("Build times"). Copying those numbers into the table below, over several runs, is still open; the table is empty.

| Job | Cold before | Warm before | Cold after | Warm after |
|---|---|---|---|---|
| test windows | | | | |
| build windows | | | | |
| macOS arm64 | | | | |
| macOS Intel | | | | |
| test linux | | | | |
