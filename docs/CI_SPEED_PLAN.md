# CI speed plan

Goal: a reasonable compile time on every platform, macOS and Windows first. Source: the CI audit
(last 40 runs, job-level data only; step-level timings were not pulled, so the split between cold
compile, link and test run is inferred). Workflows: `.github/workflows/test.yml` and `build.yml`.
This file is a plan; it changes no workflow.

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

Main cause of slow Windows tests (inferred): `test.yml` (cache key `test-<os>`, `ci` profile) and
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

| Job | Cold before | Warm before | Cold after | Warm after |
|---|---|---|---|---|
| test windows | | | | |
| build windows | | | | |
| macOS arm64 | | | | |
| macOS Intel | | | | |
| test linux | | | | |
