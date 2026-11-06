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

## Web build and deploy

The browser build is `wasm32-unknown-unknown`; the page and its scripts are `packaging/web/` (`index.html`, `update.js`, plus `audio.js` and
`mq_js_bundle.js` from the macroquad/quad-snd bundles), copied next to `last_route.wasm` in `dist/web`. `sh packaging/web/deploy-vps2.sh [FOLDER]`
(default `dist/web`) deploys that folder to `vps2:/srv/lr-rust-web`: it writes `version.json` from the sha256 of `last_route.wasm`, uploads the five
game files by tar over ssh (`--no-same-owner`, moved into place from a staging folder), uploads `version.json` last so open tabs only see a new
version once every file is in place, then checks the wasm sha256 on the server and through the live URL. It never touches the Caddyfile.
`DRY_RUN=1` prints the plan without uploading. Open tabs poll `version.json` (`update.js`) and show the UPDATING screen
(`ui/update_screen.rs`, numbers in `data/ui/update_screen.json`) when its build changes: a 10 second countdown, UPDATE NOW, and LATER during a run.
`LR_SHOT_UPDATE=1` with `--screenshot` draws that screen on a native build.

The page also registers the game's JavaScript bridges as miniquad plugins: pointer lock and localStorage (`app/web_pointer.rs`,
`app/web_storage.rs`), the room relay and directory (`net/web_bridge.rs`), and the file chooser (`shared/file_pick.rs`, the `lr_pick_*`
functions; see `docs/SKIN_SYNC.md`). A new `extern "C"` import in the game needs its function in `index.html` in the same deploy.

## Offline project docs

The app recognizes the local reference library at `C:\Users\Kyttu\Documents\Projects\Docs` when
it exists. Set `KYTTU_DOCS` to use the same docs from another location. The helper APIs in
`src/shared/paths.rs` only read files beneath that directory and never package the library into
the game.

## Code layout

The package builds three executables from one library:

- `src/lib.rs` (crate `last_route_core`) — only what executables share: `packs` and `shared::paths`
- `src/bin/last_route/` — KYTTU: The Last Route: the front end (`app`, `ui`, `headless`, `net`, `updater`) and the
  game modules (`config`, `content`, `world`, `hunters`, `game`, `render`, `audio`, `shared`), plus its `data/` and `assets/`
- `src/bin/night_watch/` — KYTTU: Night Watch (shell, framework and modes; see [NIGHT_WATCH_MODES.md](NIGHT_WATCH_MODES.md))
- `src/bin/lr_server/` — the dedicated server (`lr_server`)

Game modules (all under `src/bin/last_route/`; the layout is in [SOURCE_LAYOUT.md](SOURCE_LAYOUT.md)):

- `config` — tuning numbers and difficulty profiles (see [MECHANICS.md](../MECHANICS.md))
- `content` — the data-driven content registry (`data/*.json` and mod packs)
- `world` — the seeded random generator, the facility floor generator and the Infinite chunks
- `hunters` — the shared hunter brain and each hunter's own tricks
- `game` — the run logic
- `render` — shader, world meshes, the painted textures (`textures/`), Blockbench and Blender
  models, scene and held item
- `audio` — the synthesised sound cues (`cues/`)
- `data/` and `assets/` — the game's JSON content, textures and models (region files are in `src/bin/last_route/data/regions/`)
- `tools/prop_convert` — the first converter (the blocky `_0` GLBs to one-per-prop .bbmodel, now unused by the game); `tools/props/glb_to_bbmodel.mjs` is the current one (see docs/PROPS_BBMODEL.md)
