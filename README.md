This build contains the original game's own files, for the private use of the person who owns the game, on their own computer. It must not be shared or published.

# Last Route — Rust port

A Rust + macroquad port of KYTTU: The Last Route (the Godot 4 project in
`LastRoute Godot/KyttusLabApp`). CREATE ROOM starts the Infinite route like the original:
an endless streamed facility, district by district, with stairwells down to deeper levels and
more hunters as you recover relays. The finite floor-by-floor route (relays, then the service
lift) is kept too, and a run saves itself at each level; the title's username button opens the
account menu, where a saved run can be continued or cleared.

## Run

```sh
cargo run --release
cargo run -- --screenshot shots/now.png   # draws ~120 frames, saves a PNG, exits
cargo run -- --self-test shots/test.json  # plays the core loop with scripted input, writes results
```

## Headless runs, tests and AI-agent control

```sh
last_route --headless [--port 47600]                       # no window: JSON lines on stdin/stdout (and TCP)
last_route --agent-window [--port 47600]                   # same protocol with the game window (screenshots)
last_route --test-suite "tests/*.json" [--out report.json] # headless test suites, JSON report
last_route --bot --scenario=survive --seed=ALPHA --minutes=5 --out=.playtest/alpha
```

The protocol keeps the original playtest bridge's commands and JSON shapes (`hello`, `observe`,
`act`, `step`, `start_run`, `admin`, `screenshot`, ...) plus direct queries for agents; see
`docs/AGENT_CLI.md`. The simulation runs thousands of times faster than real time headless.

## Windows, macOS and Linux

The same code builds on all three (checked with `cargo check --target x86_64-unknown-linux-gnu`
and `--target aarch64-apple-darwin`; macOS/Linux binaries have to be linked on those systems).

- **Windows:** `cargo build --release` → `target/release/last_route.exe` (icon embedded).
- **macOS (Apple Silicon and Intel):** on a Mac, `sh packaging/macos/build_app.sh` →
  `dist/Kyttu's Lab.app`, one universal app for both chip types, with the original's logo.
- **Linux:** install `pkg-config libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev`, then
  `sh packaging/linux/build.sh` → `dist/last-route-linux-x86_64.tar.gz` (binary, desktop entry, icon).
- `.github/workflows/build.yml` builds all three on a push (artifacts only), for a private repository.

Saves and settings live in `%APPDATA%\LastRoute` (Windows), `~/Library/Application Support/LastRoute`
(macOS) or `~/.local/share/LastRoute` (Linux). The original's fonts are Courier New Bold and Impact;
on Linux without them, Liberation Mono/Sans Bold stand in.

## Offline project docs

The app recognizes the local reference library at `C:\Users\Kyttu\Documents\Projects\Docs` when it exists.
Set `KYTTU_DOCS` to use the same docs from another location; the helper APIs in `src/app/paths.rs`
only read files beneath that directory and never package the library into the game.

## Controls (the original's default bindings)

| Action | Key |
| --- | --- |
| Move | W A S D |
| Look | Mouse (I J K L / arrow keys too) |
| Sprint / crouch / hop | Shift (hold) / C (hold) / Space |
| Use held item | Left click |
| Interact (relays, pickups, lockers, lift, stairs, safe zone) | E or right click |
| Flashlight / taser | F / G |
| Hotbar | 1-5, mouse wheel |
| Drop held item | Q |
| Map | M |
| Pause | Escape |
| Hide HUD / fullscreen | F1 / F11 |

## Layout

- `src/app` — the main loop, menus, assets, screenshot and self-test modes
- `src/config` — tuning numbers and difficulty profiles (see MECHANICS.md)
- `src/content` — the data-driven content registry (data/*.json + mod packs)
- `src/world` — seeded random generator, the facility floor generator and the Infinite chunks
- `src/hunters` — the shared hunter brain and each hunter's own tricks
- `src/game` — the run: player, items, interactions, capture and escape, the streamed window
- `src/render` — shader, world meshes, the original's painted textures (`textures/`), Blockbench
  and Blender models, scene and held item
- `src/ui` — HUD, map, menus (account, loading) and run report
- `src/audio` — the original's synthesised sound cues (`cues/`)
- `data/` — the original's JSON content; `assets/` — the original's textures and models
- `tools/prop_convert` — converts the original's blocky prop GLBs to .bbmodel
