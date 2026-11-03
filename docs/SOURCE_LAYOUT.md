# Source layout (DRAFT)

Status: DRAFT. Describes the current tree and a proposed move. No source file has been moved. The proposed move is blocked, see "Blocker" below.

## Crate shape

- One package, `last_route` ([Cargo.toml](../Cargo.toml)), with two targets built from it:
  - the shared library `last_route_core`, root [src/lib.rs](../src/lib.rs), which declares `audio`, `config`, `content`, `game`, `hunters`, `night_watch`, `render`, `shared`, `world`;
  - three executables under [src/bin](../src/bin): `last_route` ([main.rs](../src/bin/last_route/main.rs)), `lr_server` ([main.rs](../src/bin/lr_server/main.rs), needs the `server` feature), `night_watch` ([main.rs](../src/bin/night_watch/main.rs), placeholder window only).
- `last_route/main.rs` re-exports the shared modules with `pub use last_route_core::{audio, config, content, game, hunters, render, shared, world};`, so inside the game binary `crate::<module>` resolves to the library module.
- `lr_server` does not re-export anything. Its only library use is `last_route_core::content::packs::Pack` and `last_route_core::content::regions` in [pack_check.rs](../src/bin/lr_server/pack_check.rs).
- `src/audio/quad-snd` is a vendored, patched crate (`[patch.crates-io]` in Cargo.toml). It is not counted in the audio module below.

## Module table

Counts are `.rs` files under each module, excluding `src/audio/quad-snd`. "Pub items" counts top-level `pub fn/struct/enum/trait/const/static/type` declarations, a rough size measure, not an API inventory. "Users" lists the binary files that reference the module as `crate::<module>` (game binary) or `last_route_core::<module>` (server).

| Module | Files | Pub items (approx.) | Classification | Users (binary files) |
|---|---|---|---|---|
| `audio` ([mod](../src/audio/mod.rs)) | 16 | 175 | GAME | last_route: 6 files (`app/mod.rs`, `headless/sim.rs`, `ui/admin/look.rs`, `ui/menus.rs`, `ui/settings_panel.rs`, `ui/style.rs`) |
| `config` ([mod](../src/config/mod.rs)) | 2 | 111 | GAME | last_route: 15 files (app, headless, net/world_sync, ui) |
| `content` ([mod](../src/content/mod.rs)) | 7 | 39 | SHARED | last_route: 12 files; lr_server: [pack_check.rs](../src/bin/lr_server/pack_check.rs) |
| `game` ([mod](../src/game/mod.rs)) | 10 | 54 | GAME | last_route: 16 files (app, headless, net/runner_figure, net/world_sync, ui) |
| `hunters` ([mod](../src/hunters/mod.rs)) | 31 | 80 | GAME | last_route: 9 files (app, headless, ui/map, ui/menus, ui/admin) |
| `night_watch` ([mod](../src/night_watch/mod.rs)) | 3 | 8 | GAME (unused, see note) | none. night_watch binary does not import it |
| `render` ([mod](../src/render/mod.rs)) | 27 | 243 | GAME | last_route: 4 files (app/mod.rs, app/perf.rs, net/runner_figure.rs, ui/admin/actions.rs) |
| `shared` ([mod](../src/shared/mod.rs)) | 6 | 24 | GAME | last_route: 5 files (app/mod.rs, app/updater.rs, net/runner_figure.rs, net/world_sync.rs, ui/admin/mod.rs) |
| `world` ([mod](../src/world/mod.rs)) | 31 | 310 | GAME | last_route: 12 files (app, headless, net/world_sync, ui) |
| `lib.rs` ([file](../src/lib.rs)) | 1 | n/a (module list) | n/a | n/a |

Classification groups:

- GAME: `audio`, `config`, `game`, `hunters`, `render`, `shared`, `world`, and `night_watch` (no users; the rule counts "nothing outside last_route" as GAME).
- SHARED: `content` (last_route and lr_server).
- NIGHT: none. The night_watch binary imports no library module.
- SERVER: none. lr_server's only library module is `content`, which is also used by last_route.

Note on `night_watch`: the module ([src/night_watch/mask/vent_lights.rs](../src/night_watch/mask/vent_lights.rs), about 300 lines) is not used by any binary, and the night_watch binary is a placeholder. The classification above follows the rule literally. Treat it as "future NIGHT" and re-check when the mode is ported.

### Binary files under src/bin

- `last_route/`: `main.rs`; `app/` (log, mod, pacing, perf, platform, profile, screenshot, selftest, settings, stress, updater); `headless/` (bot, expect, mod, protocol, runner, sim, suite, window); `net/` (api, backend, chat, config, http, mod, pending, protocol, relay, runner_figure, saves, session, store, tests, world_sync); `ui/` (boot, hud, map, menus, mod, pause, radar_ping, report, settings_panel, style, and `admin/`: actions, effects, hub, look, menu, mod, prefs); `updater/version.rs`.
- `lr_server/`: `main.rs` plus 17 modules (account_service, crash_reports, enet_relay, env_file, game_saves, guest_names, hunter_id_aliases, mod_packs, moderation, pack_check, pack_kinds, public_route, regions, reserved_rooms, room_service, upstream, util) and `server/` (mod, routes, support, tests).
- `night_watch/main.rs`: single file.

## Where new code goes

1. Code that only the game binary (`last_route`) uses goes under `src/bin/last_route/`.
2. Code used by the game and by another binary (`lr_server` or `night_watch`) stays in a library module under `src/` (`src/lib.rs` and its modules).
3. Code used only by `lr_server` goes under `src/bin/lr_server/`. Code used only by `night_watch` goes under `src/bin/night_watch/`.
4. A new library module must be declared in [src/lib.rs](../src/lib.rs) before other crates can use it.

## Proposed move: GAME modules into src/bin/last_route

Proposal: move the GAME modules from `src/` to `src/bin/last_route/` and keep the `last_route` binary's paths unchanged by declaring them there.

Modules that would move: `audio`, `config`, `game`, `hunters`, `render`, `shared`, `world`, and `night_watch` (if the rule is applied).

Import path changes if the move is made:

- [src/bin/last_route/main.rs](../src/bin/last_route/main.rs) line 12: `pub use last_route_core::{audio, config, content, game, hunters, render, shared, world};` becomes `pub use last_route_core::content;` plus local `mod` declarations for each moved module. The `crate::<module>` paths in the game binary (about 138 references) keep their spelling.
- Inside the moved modules, library-internal references to `content` change from `crate::content::` to `last_route_core::content::`. Counts: [game](../src/game/mod.rs) and its submodules (8 references in total, [endless.rs](../src/game/endless.rs) has 3), [hunters](../src/hunters/brain.rs) and its mod file (4 in total), [render](../src/render/world_mesh.rs) (12 in total, 8 of them in world_mesh.rs), [shared/assets.rs](../src/shared/assets.rs) (1), [world](../src/world/route/regions.rs) and its submodules (16 in total, [route/regions.rs](../src/world/route/regions.rs) has 5).
- Inside the moved modules, references to other moved modules keep their `crate::` spelling, because they resolve inside the game binary.

### Blocker

The move cannot be done as a pure path change. `content` (used by `lr_server`) depends on `config`, `shared` and `world`. `world` depends on `content`, `hunters`, `render` and `shared`. `render` depends on `game`, and `game` depends on `audio`. So `lr_server` transitively needs every module except `night_watch`. Moving any of them out of `last_route_core` breaks `content`, and breaks `lr_server` through it.

Options (not decided):

1. Keep the modules in `src/` and document them as shared-core, with the classification above as a note.
2. Move `content` as well and give `lr_server` its own copy or a narrower module, which is a real refactor.
3. Move only `night_watch` (no users) into `src/bin/night_watch/`, which is safe.

Recommendation for the next step: decide between options 1 and 2 before any file moves. Until then, new game-only code goes under `src/bin/last_route/` only when no library module imports it.

## Verification

- All links in this file are relative paths inside this repository.
- Checked against the tree at the time of writing. Re-run the grep for `crate::<module>` and `last_route_core::<module>` under `src/bin` after any change.
