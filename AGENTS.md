# AGENTS.md — Last Route (Rust)

Rules for any AI agent or tool working in this repository. KT's instructions here override tool defaults.

## Commits and pushes

- **Authors:** commits may be made by other people (for example other contributors' names). Do not use AI, agent or tool identities as the author or committer, and do not invent a name.
- **No attribution lines at all:** never add a `Co-Authored-By:` line (for anyone, AI or human) or a `Generated with` line to a commit message, PR description, tag, or release. Commits carry only the normal author and message.
- **Enforced by git hooks:** run `git config core.hooksPath .githooks` once in each clone. `.githooks/commit-msg` rejects AI attribution lines, and `.githooks/pre-push` rejects pushes whose new commits name an AI. Do not bypass them with `--no-verify`.
- **Never force-push `main`**, and never rewrite pushed history, without KT's explicit go-ahead in that conversation.
- **Never push secrets or runtime data:** no `.env`, keys, tokens, `dist/`, `*.wasm`, logs, or saves. Check `git status` before committing.
- **Never change the live servers** (vps1, vps2, vps3) or the Godot site without KT's approval of the exact steps.
- **Branches:** work on a branch per task, and merge to `main` only after tests pass.

## Markdown documents

- Every `*.md` file lives under `docs/`, except `README.md` and `AGENTS.md` at the root. New documents go in `docs/` (or a subfolder of it) and link to other docs with resolving relative paths.
- Do not add other root-level `.md` files. Existing exceptions are listed in `TODO.md` for moving.
- Keep document links working when you move or rename a file: search the repo for the old path and update it.

## Build and tests

- Run the native tests with the test executable directly: `cargo test --no-run --lib --bin last_route`, then run the newest `target/<dir>/debug/deps/last_route-*.exe` from the repo folder. Plain `cargo test` aborts on KT's PC.
- Known timing-flaky tests (`headless::dedicated`, `net::http`, `shared::skins`) pass when run alone. Rerun them alone before treating a failure as real.
- Web build: `cargo check --target wasm32-unknown-unknown --bin last_route` must pass before merging.
- On KT's PC, use `CARGO_BUILD_JOBS=4` and at most one cargo build at a time per agent. Do not leave builds or servers running.
- Web deploys: `sh packaging/web/deploy-vps2.sh dist/web` after building the wasm and assembling the page files. See `docs/DEPLOY_VPS2_WEB.md`.

## Performance goals and rules

The goals (KT, 2026-11-12). Frame rates are for a run in play, not the menu:

| Machine | Goal |
|---|---|
| Very low-end PCs | at least 30 fps |
| KT's dual-core MacBook Airs | 60 to 300 fps |
| KT's Framework Laptop 13 (the main PC) | 1000 fps or more, as it is today. Do not lose it. |

The rules that follow from them (how to measure, and the current numbers, are in `docs/PERFORMANCE.md`):

- **Frame time comes first.** A change may not make the stress run's average frame slower than run-to-run noise
  (about 3%) on KT's PC, not even to save memory, without KT's go-ahead. Memory is saved on what is out of sight
  (packed, dropped, built on first use), never by adding work to every frame.
- **Measure both, before and after,** for any change to rendering, meshes, streaming, the frame loop, hunters or
  NPCs: the average frame time and the peak memory of `--stress 40 --stress-scenario mixed`, old and new build run
  back to back at least twice each. Put the four numbers in the commit message.
- **Memory:** every MB counts. A new cache, table or buffer that holds more than about 1 MB needs a reason in the
  commit message and a row in `docs/RAM_PASS.md`.
- **No new work per frame without a number:** no allocation, sort or full pass over the world in the draw or update
  loop unless it is measured and the first rule holds.
- **Compile time and dependencies:** no new crate without KT's go-ahead. Shipped builds keep fat LTO (thin LTO built
  twice as fast and ran 5% slower: `docs/COMPILE_SPEED.md`). A build must stay free of compiler warnings in the game,
  the server, Night Watch, the tests and the web build.

## Agent passes in this repo

The global agent build rules (KT's global CLAUDE.md: edit agents never compile, QA only reads, one build-and-fix agent, then one test-and-fix agent) apply here. This repo's specifics:

- The standard pass is `.claude/workflows/edit-pass.js` (args `{ label, extraRules, tasks: [{ n, files, task }] }`). Every pass compiles into the one shared `CARGO_TARGET_DIR=target/pass` (incremental between passes; the main `target/` is for KT).
- The build-and-fix agent runs, in order: `cargo build --bin last_route`, `cargo test --no-run --lib --bin last_route`, `cargo check --target wasm32-unknown-unknown --bin last_route`, `cargo build --bin lr_server --features server`.
- The test-and-fix agent runs the newest `<target>/debug/deps/last_route-*.exe --test-threads=4`, then `<target>/debug/last_route.exe --headless --test-suite "tests/*.json"`.
- Off-limits to every agent: `src/bin/lr_story/`, `src/bin/lr_story_server/`, `packaging/story-*`, `Cargo.toml`, `Cargo.lock`, `.gitignore`, `docs/LR_STORY.md` (another agent owns them).
- Known failures to ignore: none since 2026-11-08 (the bot suite, the run_flow capture test and the npcs/brain unit tests pass). Timing flakes (`net::http`, `headless::dedicated`, `shared::skins`) pass when rerun alone.

## Working with KT

- Keep chat replies short. Put detail in commit messages and in `docs/`.
- Ask before deleting files, destructive git operations, or anything outward-facing.
