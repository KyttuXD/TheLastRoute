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

## Working with KT

- Keep chat replies short. Put detail in commit messages and in `docs/`.
- Ask before deleting files, destructive git operations, or anything outward-facing.
