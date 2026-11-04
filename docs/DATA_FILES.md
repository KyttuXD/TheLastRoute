# Data files: tunable values moved out of code

Every tunable number, timing, list, and per-hunter, per-difficulty or per-region setting lives in JSON under `src/bin/last_route/data/` (or `data/regions/` at the top level). `build.rs` embeds these files, and each one is read through an existing content loader. Code holds behaviour only. Standard values in each file match the game as it was before the move.

Changing a value in a file changes behaviour after a rebuild. The task rule is one test per feature that a changed data value changes behaviour, and that Standard values match the old game. Those tests are written in the feature's own module; this page does not list them.

## Added 2026-11-04

| File | Read by | Controls |
|---|---|---|
| `data/stairs_links.json` (`lr.stairs_links/1`) | `game/transition.rs` (`links()`) | Level links, stair flights and sewer ladders. `landing_offset` (0.6 cells in from the far landing), `prefetch_distance` (8 cells, when the storey ahead is built). See [MECHANICS.md](../MECHANICS.md) for the rule. |
| `data/flights.json` | `world/flight.rs` | Continuous-level flights: storey, slab, rise, max riser, steps per cell, margin, switchback/straight/spiral settings, shape and per-region weights. Not used by the running game yet. |
| `data/hunters/search.json` (`lr.search/1`) | `hunters/brain.rs`, via `content/mod.rs` | Hunter search after a lost sighting: `reach`, `memory`, the search `offsets`, and per-difficulty `hiding_spot_chance`, `last_seen_spot_first`, `unchecked_first`. Standard keeps `hiding_spot_chance` at 0.0, so hunters keep searching beside the runner and the relay rush is not starved. |
| `data/hunters/sabotage.json` (`lr.hunter_sabotage/1`) | `hunters/sabotage.rs`, via `content/mod.rs` | Relay sabotage: `near_cells`, `dwell_s`, `resecure_s`, and per-difficulty `enabled` (false on every difficulty today), `target`, `chance`. |
| `data/hunters/charge.json` (`lr.charge/1`) | `hunters/gimmicks/charge.rs` | Per-hunter charge (Loxelle): passive chance and timings, the charge windup, speed and distances, and the door-charge stun. |
| `data/hunters/nightmare_pounce.json` (`lr.nightmare_pounce/1`) | `hunters/gimmicks/nyxie.rs`, `solyth.rs` | Nightmare pounce for Nyxie and Solyth: `difficulties` (nightmare only), `numbers` (range, rate per eligible second, cooldown, per-chase cap, stumble, stun), party-wide `budget.seconds`, and the wall-sweep `sweep.step`. |
| `data/skins/config.json` (`lr.skin_config/1`) | `shared/skins.rs` | Skin download: `max_bytes`, the host and backend paths, and `timeout_secs`. |
| `data/lobby.json` (`lr.lobby/1`) | `ui/menus.rs` | Difficulty picker: titles, guest title, and the label and blurb of each option. |
| `data/hud.json` (`lr.hud/1`) | `ui/hud.rs` | Danger meter thresholds and pulse timing, meter minimum width, and HUD label templates. |
| `data/perf.json` (`lr.perf/1`) | `app/perf.rs` | Frame-time history length, spike threshold and logging, graph range, guide lines, colour bands, and the names of the timed parts. |
| `data/regions/*.json` (`lr.region/1`) | `packs/regions.rs` | One file per built-in region: floor sets, room roles, prop set and sizes. Current files: apartment, beach, carnival, gothic, greenhouse, hospital, hotel, lab, mall, mansion, maze, prison, sewer. |

## Rules for these files

- Give every file a `schema` key.
- Keep a `_about` or `_note` line that cites the Godot source for the Standard numbers.
- A value the code depends on is checked when the file is parsed. `stairs_links.json` no longer has `flight_steps`: a flight now spans one storey (5.0 m) and its length comes from the flight rules.
- Do not add a number to a `.rs` file. Add it to the nearest data file and read it through the loader.
