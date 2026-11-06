# Archived: the level-0 catwalks

Removed on 2026-11-06. KT: "remove the catwalk platforms, they're not how I wanted them to be".

This folder is outside `src/`, so none of it is embedded in the game or compiled. It is here so the catwalks can be
redone without digging through history for the data.

## What they were

Raised walkways (decks) hung 2.5 m over the level-0 maze floor, with grating, rails, support beams, lamps and a ladder
or stairs link down to the floor. Hunters and the runner could stand on them. In the maze (and only in regions the data
allowed) each 45 x 45 cell chunk could get a lattice of decks, kept only where a link reached them.

## How they were generated

- `route/catwalk.rs` built a seeded deck lattice per chunk (`size`, `spacing`, `width`, gaps, landings, all in
  `data/regions/catwalk.json`), pruned to what the landings reach.
- `Facility::apply_catwalks` (world/facility.rs, called from `world/route/live.rs::chunk`) lifted the cells that were
  open, clear ground and whose removal did not cut the floor in two, asked `route/catwalk_links.rs` for ladder/stairs
  links (`data/regions/catwalk_links.json`: allowed regions, per-chunk count, link clearance), kept the deck cells joined
  to a link, and stored them in `Facility::walk_surface` / `catwalk_links`.
- Rendering: `render/catwalk.rs` (grating, rails, supports, lamps; `data/render/catwalk.json`), the deck and ladder
  boxes in `render/world_mesh.rs`, and the lamps in `render/scene.rs`.
- Movement: `game/ladder.rs` (`Climb::catwalk`, walk-in at the deck face), `Facility::body_fits_at*` (per-cell walk
  height), `game/level_stack.rs` / `game/player.rs` (standing on a deck).
- Hunters: `hunters/decks.rs` + `data/hunters/decks.json` (who may climb or jump), `hunters/flight_edges.rs`
  (`CatwalkLadder` and `CatwalkJump` edges, `EdgePermit`), `hunters/brain.rs` (layers, stalking the foot of a deck,
  landing stun), `hunter_landing` noise event, `Facility::deck_open` / `nav_open`.

## What is in this folder

| File | What |
|------|------|
| `data/regions/catwalk.json` | Deck lattice rules and the 2.5 m deck height |
| `data/regions/catwalk_links.json` | Which regions get catwalks, links per chunk, link clearance |
| `data/render/catwalk.json` | Grating, rail, support and lamp look |
| `data/hunters/decks.json` | Per-hunter climb / jump-down rules |
| `docs/CATWALKS.md` | The design note (Godot geometry, the Rust deck model, hunters on decks) |

Also removed with them: `data/hunters/flights.json` keys `catwalk_ladder_enabled`, `catwalk_jump_enabled`,
`jump_extra_cost`, `jump_max_drop_m`, `jump_gravity`, `jump_arc_reach_m`, `jump_arc_samples`; `data/hunters/noise_events.json`
key `landing_thud_volume_scale` and kind `hunter_landing`; `data/render/scene_lights.json` key `catwalk_scan_radius_m`;
the copy `src/bin/lr_story/assets/data/regions/catwalk_links.json`.

## Why they were removed

Not how KT wanted them. No bug drove it; the design needs redoing from KT's description.

## Restoring from git history

The last commit on `main` that still has the catwalks is:

```
0d342d7900b112949f3e58d6d7f27078dc884c8a   (deploy-vps2.sh uploads keys.js too)
```

The removed source files are not copied here; read them with git:

```sh
git show 0d342d7:src/bin/last_route/world/route/catwalk.rs
git show 0d342d7:src/bin/last_route/world/route/catwalk_links.rs
git show 0d342d7:src/bin/last_route/render/catwalk.rs
git show 0d342d7:src/bin/last_route/hunters/decks.rs
git diff 0d342d7 HEAD -- src/bin/last_route    # everything the removal touched
```

To bring the feature back: put the data files here back under `src/bin/last_route/data/` (same sub-folders), restore
the four source files above, and re-apply the removed parts of `world/facility.rs` (`DeckKind`, `WalkSurface`,
`walk_surface`, `catwalk_links`, `apply_catwalks*`, `catwalk_flight`, `deck_open`, `nav_open`, the height rule in
`fits`), `world/route/live.rs` (the `apply_catwalks` call), `render/scene.rs` and `render/world_mesh.rs`,
`game/ladder.rs`, `hunters/flight_edges.rs`, `hunters/brain.rs`, `hunters/path_scheduler.rs`,
`hunters/noise_events.rs`, `game/hunting.rs` and the tests that went with them. `git diff 0d342d7 HEAD` lists them all.
The Godot layout goldens (`layout_matches_godot_goldens`) never covered catwalks: they run before the catwalk step.

Night Watch's "catwalk" rooms on the guard roof (`src/bin/night_watch`) are a different thing and were left alone.
