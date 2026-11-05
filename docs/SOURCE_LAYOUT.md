# Source layout

One package, `last_route` ([Cargo.toml](../Cargo.toml)), builds a small shared library and three executables. Each executable keeps its own code and assets in its own folder under `src/bin/`.

## Crates and folders

| Target | Folder | What lives there |
|---|---|---|
| Library `last_route_core` | [src/lib.rs](../src/lib.rs), [src/packs](../src/packs), [src/shared](../src/shared) | Only what two or more executables share: the mod pack types (`packs`, which `lr_server` builds against alone) and the paths every executable uses (`shared::paths`). |
| `last_route` (the game) | [src/bin/last_route](../src/bin/last_route) | The game and everything it draws, plays and loads (see below). |
| `night_watch` | [src/bin/night_watch](../src/bin/night_watch) | Night Watch: shell, framework, modes, mask, save, its `calls/` data and its reference `assets/`. |
| `lr_server` (needs the `server` feature) | [src/bin/lr_server](../src/bin/lr_server) | The dedicated server: accounts, rooms, saves, moderation, relay. |

`night_watch` and `lr_server` import nothing from the game. `lr_server` uses `last_route_core::packs`, `night_watch` uses `last_route_core::shared::paths`.

## Inside `src/bin/last_route`

- `app/`, `ui/`, `headless/`, `net/`, `updater/`, `story/`: the front end, menus, test and agent modes, networking and the in-game updater.
- `audio/`, `config/`, `content/`, `game/`, `hunters/`, `render/`, `world/`: the game's own modules. They declare themselves in [main.rs](../src/bin/last_route/main.rs), so code inside the game writes `crate::world::...` as before.
- Inferno terrain: `world/inferno_terrain.rs` (noisy floor and ceiling of level -3, a pure function of world text and cell; read through `Facility::ground_height` / `ceiling_height`, drawn by `render/world_mesh.rs`, followed by `game/player.rs`, `hunters/brain.rs` and `game/npc.rs`). Data: `data/inferno.json` `terrain`.
- Continuous levels: `world/flight.rs` and `world/flight_links.rs` (flights, links), `game/level_stack.rs` (resident stack), `game/resident_hunters.rs` (hunters per resident floor), `game/hunter_pipes.rs` (water and valve floods for hunters), `hunters/flight_edges.rs` (hunter stair, ladder and drop edges), `render/stack_actors.rs` (bodies at absolute y). See [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md).
- `shared/`: the game-only shared items (`assets`, `checkpoint`, `admin_flags`, `safe_room`). It re-exports `paths` from the library, so `crate::shared::paths` keeps working.
- `assets/` (pictures and Blockbench models; the Blender GLB sources they were converted from are in `tools/props/source_glb`, not embedded) and `data/` (JSON content): embedded into the executable by [build.rs](../build.rs), under the same keys as before (`hunters/aki.bbmodel`, `props/...`).
- `audio/quad-snd` is a vendored, patched crate (`[patch.crates-io]` in Cargo.toml).

## Inside `src/bin/lr_server`

- `accounts/` (`account_service`, `guest_names`), `rooms/` (`room_service`, `reserved_rooms`, `public_route`, `hunter_id_aliases`), `saves/` (`game_saves`), `packs/` (`mod_packs`, `pack_kinds`, `pack_check`, `regions`), `relay/` (`enet_relay`), `ops/` (`moderation`, `crash_reports`, `env_file`), `common/` (`util`, `upstream`), and `server/` (the HTTP router: `mod`, `routes`, `support`, `tests`). Each folder has a `mod.rs`; `main.rs` stays at the root.
- `main.rs` re-exports the modules (`use accounts::account_service;` and so on), so code still writes `crate::account_service::...`. Tests live in their module files.
- The folder is called `common`, not `core`, so it does not shadow Rust's `core` crate.
- Runtime data: `LASTROUTE_DATA_DIR` (or `KYTTU_DATA_DIR`) sets the folder; the default is `.data` under the working directory (`server/mod.rs`), not `data/`.

## Data and assets

- The game's data and assets live in `src/bin/last_route/data` and `src/bin/last_route/assets`.
- [data/regions](../data/regions) stays at the top level because the library's `packs` module reads it. `build.rs` embeds it under the `regions/` keys.
- Night Watch's reference copy of the Godot rules and cast is in `src/bin/night_watch/assets`. It is not embedded yet.
- The build script runs once per package, so every executable still embeds the same files. Splitting the embed per executable needs a package per executable, which is not done.

## Where new code goes

1. Code only the game uses goes under `src/bin/last_route/`.
2. Code only Night Watch uses goes under `src/bin/night_watch/`; code only `lr_server` uses goes under `src/bin/lr_server/`.
3. Code that two or more executables use goes in the library (`src/lib.rs` and its modules), and must not import any bin module.

## Tests and verification

- Tests moved with their code: `cargo test --lib` now runs the library (packs, paths) only; the game's tests run with `cargo test --bin last_route`.
- CI (`.github/workflows/test.yml`) runs `--lib --bin last_route --bin lr_server --features server`, then the headless suite `tests/*.json`.
- The moved game modules carry `#![allow(dead_code)]` at their root. These items were the library's `pub` API, so nothing flagged them unused; prune them as the port settles.
- Night Watch still has dead-code warnings until its modes are wired into the shell.

## What moved (2026-11-03)

- `src/{audio,config,content,game,hunters,render,world}` into `src/bin/last_route/`.
- `src/shared/{assets,checkpoint,admin_flags,safe_room}.rs` into `src/bin/last_route/shared/`.
- `assets/` (except Night Watch's) and `data/` (except `regions/`) into `src/bin/last_route/`.
- `assets/night_watch` into `src/bin/night_watch/assets`.
- `src/night_watch` into `src/bin/night_watch/` (earlier).
- Paths updated in `build.rs`, `Cargo.toml` (quad-snd patch), `tools/parity/gen_prop_tables.mjs`, `tools/props/fidelity_report.mjs` and `packaging/macos/build_app.sh`.
