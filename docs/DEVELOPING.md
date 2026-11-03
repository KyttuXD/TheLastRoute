# Developing KYTTU: The Last Route

This is for people working on the code. For how to play, see [the README](../README.md).

## Run

```sh
cargo run --release
cargo run -- --screenshot shots/now.png   # draws ~120 frames, saves a PNG, exits
cargo run -- --self-test shots/test.json  # plays the core loop with scripted input, writes results
```

The self-test needs a display and an audio device, so it doesn't run on hosted CI runners.

## Headless runs, tests and AI-agent control

```sh
last_route --headless [--port 47600]                       # no window: JSON lines on stdin/stdout (and TCP)
last_route --agent-window [--port 47600]                   # same protocol with the game window (screenshots)
last_route --test-suite "tests/*.json" [--out report.json] # headless test suites, JSON report
last_route --bot --scenario=survive --seed=ALPHA --minutes=5 --out=.playtest/alpha
```

The protocol keeps the original playtest bridge's commands and JSON shapes (`hello`, `observe`,
`act`, `step`, `start_run`, `admin`, `screenshot`, ...) plus direct queries for agents; see
[AGENT_CLI.md](AGENT_CLI.md). The simulation runs thousands of times faster than real time
headless.

Tests: `cargo test --locked --lib --bin last_route --bin lr_server --features server`. The
golden tests that compare against the reference output need `PARITY_DIR` set and are ignored by
default.

## Building

- **Windows:** `cargo build --release` → `target/release/last_route.exe` (icon embedded).
  A running `last_route.exe` locks `target/release/last_route.exe`, so cargo cannot replace it and
  the build fails with a file-lock error. Either close the game first, or build into another target
  directory, for example `cargo build --release --target-dir target/release-check`.
- **macOS (Apple Silicon and Intel):** on a Mac, `sh packaging/macos/build_app.sh` →
  `dist/Kyttu's Lab.app`, one universal app for both chip types.
- **Linux:** install `pkg-config libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev`, then
  `sh packaging/linux/build.sh` → `dist/last-route-linux-x86_64.tar.gz`.
- **CI:** `.github/workflows/build.yml` builds the three platforms on a push (artifacts only, kept
  for 3 days); `.github/workflows/test.yml` runs the unit tests and the headless suites on
  Windows and Ubuntu.
- The same code also checks with `cargo check --target x86_64-unknown-linux-gnu` and
  `--target aarch64-apple-darwin`.

## Offline project docs

The app recognizes the local reference library at `C:\Users\Kyttu\Documents\Projects\Docs` when
it exists. Set `KYTTU_DOCS` to use the same docs from another location. The helper APIs in
`src/shared/paths.rs` only read files beneath that directory and never package the library into
the game.

## Code layout

The package builds three executables from one library:

- `src/lib.rs` (crate `last_route_core`) — the shared core every game uses: `config`, `content`,
  `world`, `hunters`, `game` (player, items, interactions, capture and escape, the streamed
  window), `render`, `audio`, and `shared` (assets, paths, checkpoints, admin switches)
- `src/bin/last_route/` — KYTTU: The Last Route: `app` (main loop, menus, screenshot and self-test
  modes), `ui` (HUD, map, menus and run report), `headless` (agent and test modes), plus `net`
  and `updater`, which are not compiled yet
- `src/bin/night_watch/` — KYTTU: Night Watch (placeholder, not started)
- `src/bin/lr_server/` — the dedicated server (`lr_server`)

Core modules:

- `src/config` — tuning numbers and difficulty profiles (see [MECHANICS.md](../MECHANICS.md))
- `src/content` — the data-driven content registry (`data/*.json` and mod packs)
- `src/world` — the seeded random generator, the facility floor generator and the Infinite chunks
- `src/hunters` — the shared hunter brain and each hunter's own tricks
- `src/game` — the run logic
- `src/render` — shader, world meshes, the painted textures (`textures/`), Blockbench and Blender
  models, scene and held item
- `src/audio` — the synthesised sound cues (`cues/`)
- `data/` — the game's JSON content; `assets/` — textures and models
- `tools/prop_convert` — converts the blocky prop GLBs to .bbmodel
