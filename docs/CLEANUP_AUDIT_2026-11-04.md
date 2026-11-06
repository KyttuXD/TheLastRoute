# Cleanup audit, 2026-11-04

Read-only audit after the stairs project (phase 8 still in progress). Nothing was deleted or edited. Files that are modified or were added in the last day are excluded. Sizes marked "~" are approximate; the full `target/` scan did not finish.

Tiers: A = safe to delete now (regenerable or ignored output, untracked scratch). B = delete only after KT confirms. C = looks old but is used, keep.

## 1. Summary

| Tier | Reclaimable | Notes |
|---|---|---|
| A | about 39.5 GB measured (target/debug 10.6 GB, 20 scratch target dirs 28.5 GB, src/target 0.39 GB), plus roughly 90 unmeasured scratch dirs | All ignored build output, regenerable with cargo. No tracked file qualified as Tier A. |
| B | about 1.6 GB (git worktree) plus about 10.5 MB tracked files, plus 30 MB assets/props if its unfinished audit clears | Tracked-but-dead or superseded files, docs, one-shot workflows, worktrees. |
| C | not reclaimable (target/ci 2.0 GB, reference/ 112 MB, source_glb 9.3 MB, live docs, CI, packaging) | Keep; reasons in section 4. |

Timing: do not touch `target/stairs-*` or anything built in the last 20 minutes. Another agent is building in `target/stairs-p8` now, so run Tier A target cleanup only when no cargo process is active.

## 2. Tier A: safe to delete now

All paths are relative to the repo root unless noted. All are untracked or ignored.

| Path | Size | Evidence | Regenerate |
|---|---|---|---|
| `target/debug` | ~10.6 GB | Gitignored cargo output, idle since Nov 3 20:27. No reference in .github, packaging, tools or tests. | `cargo build` |
| `target/<scratch>/` (about 110 per-pass dirs: admin*, claude-*, codex-*, coop*, crash-fix, grab-nightmare*, haiku*, nm-pass, nw-*, pass*, perf*, playtest, precommit*, skins*, void_*, vps2skins, wire-34 and similar) | 20 measured dirs = ~28.5 GB; remaining ~90 unmeasured | Per-branch cargo caches from finished verification passes (Nov 3 to 4). Not referenced by workflows, packaging, tools, tests or docs. Exclude `stairs-*`, `ci`, `release`. | Rebuilt on demand |
| `src/target/` (only `wire-34/`) | ~0.39 GB | Stray cargo target inside `src/`. Nothing references it; cargo builds at repo root. | Not needed |
| `target/test-reports`, `target/_list_raw.txt`, `target/codex-player-list-check.txt` | small | Generated test output and a scratch listing. | Test runs |
| Old session scratchpad under `AppData/Local/Temp/claude/.../scratchpad` (outside the repo, 390 entries) | unmeasured | Not referenced by the repo. Keep `TODO.before*.md` until the TODO cleanup is done. | Not needed |

## 3. Tier B: needs KT's OK

### Git worktrees and branches
| Item | Size | Recommendation |
|---|---|---|
| Worktree `scratchpad/ayttu-wt` (detached 3aa3f66) | ~1.6 GB | Check `git status` for unpushed work, then `git worktree remove`. |
| Worktrees `Temp/lr-qa`, `lr-tip`, `rooms-fix` (detached) | unmeasured | Same: check for uncommitted work first, then `git worktree remove`. Never delete the dirs by hand. |
| Local branch `release/2026-11-04-batch` (not on origin) | n/a | Decide: push, merge or delete after the batch lands. |

### Duplicate or generated tracked files
| Path | Size | Recommendation |
|---|---|---|
| `src/bin/night_watch/assets/calls/*.json` (8 files) | ~32 KB | Byte-identical to `src/bin/night_watch/calls/`, which `data.rs` embeds. Confirm packaging does not read `assets/calls`, then delete this copy. |
| `src/bin/night_watch/assets/cams/mixed/*.png` (8 files) | ~1.5 MB | Hash-identical to `cams/classic`. Grep for the "mixed" mode name first, then delete. |
| `docs/admin_screens/*.png` (~90 files) | ~8.6 MB | No references found. Delete or move out of the repo. |
| `tools/props/out/variants/arcadeCabinet_{0,1,2}.bbmodel` | ~76 KB | Documented as temp output. Untrack and gitignore `tools/props/out/`; regenerate with `glb_to_bbmodel.mjs` if `fidelity_report.mjs` needs them. |

### Docs
| Path | Size | Recommendation |
|---|---|---|
| `docs/GODOT_AUDIT_2026-11-04.md` | 25.8 KB | Archive after KT confirms TODO.md covers its gaps. |
| `docs/GODOT_RESYNC_2026-11-04.md` | 10.4 KB | Archive with the audit above. |
| `docs/TODO_AUDIT_2026-11-04.md` | 1.0 KB | Keep until KT decides its three open questions, then delete and fix the pointer at TODO.md line 951. |
| `docs/BUG_TEST_REPORT.md` | 3.2 KB | Confirm findings are fixed or in TODO.md, then archive. |
| `docs/CI_SPEED_PLAN.md` | 6.3 KB | Archive once the plan is done or dropped. |
| `docs/COMPILE_SPEED.md` | 0.7 KB | Merge the fast-profile note into `docs/DEVELOPING.md`, delete. |
| `docs/AGENT_WORKFLOW.md` | 3.3 KB | Merge into `docs/WORKFLOW_RULES.md`, delete. |
| `src/bin/last_route/data/rooms/NOTES.md` (34.1 KB), `audio/cues/NOTES.md` (19.7 KB), `render/textures/NOTES.md` (12.4 KB), `audio/quad-snd/README.md` (1.4 KB), `src/bin/night_watch/assets/README.md` (3.9 KB) | ~71 KB | Move to `docs/` or merge; this depends on KT's decision about the markdown placement rule. Do not delete. |

### Workflows and tools
| Path | Size | Recommendation |
|---|---|---|
| `.claude/workflows/route-port.js`, `route-port-2.js`, `-3.js`, `-3a.js`, `-3b.js`, `-3b2.js`, `-3b3.js`, `-3b4.js` | ~54 KB | One-shot launchers with no references. Delete once KT confirms the port batches are done. |
| `tools/playtest/` (2 ps1 scripts) | small | Delete if playtest is retired. |
| `tools/nightly-mac/` (3 scripts) | ~12 KB | Keep if Mac testers still use it, else delete. Not called by `packaging/nightly.sh` or CI. |
| `run-local.cmd` | 681 B | No references; delete if not used. |

### Code and assets
| Path | Size | Recommendation |
|---|---|---|
| `#![allow(dead_code)]` in the mod.rs files of audio, config, content, game, hunters, render, shared, plus audio/engine.rs, sound.rs, world.rs, app/public_route.rs, net/mod.rs, render/textures/mod.rs | code | After stairs phase 8 merges, remove the allow module by module and delete what rustc reports as dead. Never delete by grep alone. |
| `src/bin/last_route/assets/props` | 30 MB | The reference audit did not finish. Cross-check `data/props/*.json` texture fields against the file list before any deletion. |

## 4. Tier C: looks old, keep

| Path | Why |
|---|---|
| `target/release` | Used by `.github/workflows/build.yml`, `packaging/linux/build.sh`, `tools/stress/run_kt_mba.sh`. |
| `target/ci` (~2.0 GB) | Used by `.github/workflows/test.yml` (`--profile ci`). Can be rebuilt if space is needed. |
| `target/stairs-*` (p0, p1, p2, p34, w1, w23, direct, fix, fixcrash, testfix, p8) | Stairs project in progress. Becomes Tier A when KT signs off on stairs. |
| `.data/` | Runtime saves, moderation state, crash reports. User data; never print or delete without KT. |
| `reference/LR_screenshots/` (about 49 MB of the 112 MB `reference/`) | Indexed by `INDEX.txt` and cited in TODO.md. Duplicate rooms_list/rooms_picks pairs stay because the index likely names both. KT may later move the folder out of git. |
| `tools/props/source_glb/` (9.3 MB), `tools/props/`, `tools/prop_convert/` | Used by build.rs, CREDITS.md, docs and prop_model.rs. |
| `tools/stress/`, `tools/mods/`, `tools/parity/`, `tools/qa/` | Documented or tested (STRESS_TESTS.md, MODDING_SPEC.md, SOURCE_LAYOUT.md); `tools/qa/README.md` documents `check.mjs`. One report suggested B for qa; kept as C because of the conflict. |
| `packaging/*` (linux, macos, windows, nightly.sh, publish-vps.sh, update-manifest.mjs) | Called by build.yml and build.rs. |
| `.github/workflows/build.yml`, `test.yml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml` | Active CI and toolchain config. |
| `.claude/workflows/edit-pass.js`, `godot-feature-audit.js`, `godot-rust-parity.js`, `props.js`, `props-build.js`, `stairs-w23.js`, `stairs-p9.js` | Reusable or in use; stairs scripts wait until phase 9 lands (`stairs-p9.js` is untracked and in flux). |
| `docs/GODOT_FEATURE_AUDIT.md` (71.7 KB), `STAIRS_CONTINUOUS_DESIGN.md`, `GIMMICKS.md`, `PERF_TRIANGLES.md`, `GODOT_RUST_PARITY.md`, `PARITY.md`, `OPEN_QUESTIONS.md`, `SEWERS.md`, `STRESS_TESTS.md`, `AGENT_CLI.md`, `LOCAL_CI.md`, `WORKFLOW_RULES.md` | Referenced by TODO.md, code comments, workflows or CI. GODOT_FEATURE_AUDIT is archived only once its gaps close. |
| `README.md`, `MECHANICS.md`, `TODO.md`, `CREDITS.md` | Explicitly kept. |
| `legacy_chunk` in `world/infinite.rs`, `src/bin/last_route/data/regions/maze_*.json`, `data/hunters/*.json`, `data/props/*.json` | Loaded by directory scan or id-built paths, not by literal filename. |
| `src/bin/last_route/data/hunters/flights.json`, `data/resident_hunters.json`, `game/hunter_pipes.rs`, `game/resident_hunters.rs`, `hunters/flight_edges.rs`, `render/stack_actors.rs` | Untracked stairs phase 8 work. |

## 5. Delete plan

Run from the repo root. Run each batch's verification before the next batch.

**Batch 1: Tier A disk leftovers (no git changes)**
1. Wait until no cargo or game process is building in `target/`.
2. Delete `src/target/`.
3. Delete every `target/*` dir except `stairs-*`, `ci`, `release` and anything modified in the last hour. Then delete `target/debug` and the three loose files (`test-reports`, `_list_raw.txt`, `codex-player-list-check.txt`).
4. Delete old session scratchpad contents, keeping `TODO.before*.md`.
- Verify: `git status --short` is unchanged; `du -sh target`; `cargo build --bin last_route` still works (rebuilds only what is deleted).

**Batch 2: Tier A tracked files (`git rm`)**
- Empty. No tracked file qualified as Tier A in this audit. Candidates become eligible only after KT confirms them as Tier B.
- Verify: nothing to do.

**Batch 3: docs archive moves (after KT OK)**
1. `mkdir docs/archive`, then `git mv` GODOT_AUDIT_2026-11-04, GODOT_RESYNC_2026-11-04, BUG_TEST_REPORT, CI_SPEED_PLAN into it. Merge COMPILE_SPEED and AGENT_WORKFLOW into their targets and `git rm` them.
2. Delete or move `docs/admin_screens/` if KT agrees.
3. Move the five src/ NOTES/README files to `docs/` only if KT decides on the placement rule.
- Verify: grep that no remaining doc links to a moved path (fix TODO.md pointers); every relative link in `docs/` resolves.

**Batch 4: dead code and duplicate assets (needs a build and full tests)**
1. Remove `assets/calls` and `cams/mixed` duplicates after the reference checks; untrack `tools/props/out/`.
2. After stairs phase 8 merges, remove `#![allow(dead_code)]` module by module and delete the symbols rustc reports.
3. Delete the eight `route-port*.js` workflows, `run-local.cmd`, and `tools/playtest` or `tools/nightly-mac` if KT approves.
4. Finish the `assets/props` cross-check before touching it.
- Verify: `cargo build` for all bins, then the full suites that CI runs (`cargo test --lib`, `lr_server`, headless, `last_route --test-suite`), plus a Night Watch launch to confirm calls and cams load. Then the test-fixer agent.

**Batch 5: worktrees, branches, remote**
1. For each extra worktree (`ayttu-wt`, `lr-qa`, `lr-tip`, `rooms-fix`), check `git status` and `git log` for unpushed work, then `git worktree remove`.
2. Decide on local `release/2026-11-04-batch` and inspect the 2 local tags (names not yet checked).
3. Remote: `origin` has only `main`, and GitHub has only the rolling `nightly` and `nightly-server` releases, so there is nothing to delete.
- Verify: `git worktree list`, `git branch -a`, `git ls-remote --heads origin`.
