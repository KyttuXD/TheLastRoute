# Driving Last Route from an agent (headless CLI, protocol, tests, bot)

The Rust port runs the same `Game` simulation the window plays **without a window, GPU or audio
device**, at a fixed 60 Hz step, so a run is fully deterministic: same seed + same inputs = same
state. An agent (an LLM, a script, a bot) talks to it in **JSON lines**: one JSON object per line
in, one JSON object per line out. The command names and shapes are the Godot playtest bridge's
(`dev/playtest/playtest_bridge.gd`, protocol 1), so a script written for the original's bridge
(`hello`, `start_run`, `observe`, `act`, `step`, `admin`, `log`, `quit` ...) drives the port too.

```
last_route --headless                       # protocol on stdin/stdout
last_route --headless --port 47600          # ... and on TCP 127.0.0.1:47600 (also --playtest=47600)
last_route --headless --port 47600 --no-stdin   # TCP only (keeps serving after stdin closes)
last_route --headless --seed ALPHA --difficulty hard --mode infinite   # start a run at once
last_route --agent-window [--port N] [--realtime]   # same protocol, the game window draws the run; screenshot works
last_route --test-suite tests/*.json [--out report.json] [--only NAME]  # run suite files, JSON report
last_route --bot --scenario=collect_relays --seed=ALPHA --minutes=3 --target=5 --out=.playtest/alpha
last_route --self-test out.json             # unchanged (windowed self-test)
last_route --screenshot out.png             # unchanged (windowed screenshot)
```

Build with `cargo build` in `app/`; the binary is `target/debug/last_route.exe` (Windows).
Logs and banners go to **stderr**; stdout carries only replies (and, for `--test-suite`, the report).

## Conventions

* **Grid.** The floor is a grid of 1 m cells. A position is `[x, z]` (the original's world x and
  world z; in `observe` the player's `pos` is `[x, height, z]` like the bridge). A cell's centre is
  `cell + 0.5`. On the Infinite route positions are in the live 135x135 window (3x3 chunks of 45);
  `observe.world.origin` is the window's absolute offset and the window recentres as you walk
  (event `window_recentred`), shifting every position by 45.
* **Facing.** `yaw` 0 faces north (-z); forward is `(-sin yaw, -cos yaw)`. Positive `look[0]` turns
  right, positive `look[1]` looks down. `pitch` is radians, up positive, clamped to +-1.4 (`data/player/look.json`).
* **Time.** One frame = 1/60 s (`start_run` `fixed_fps` changes it). Nothing advances unless you
  `step` (or `bot`, `goto`, `step_until`, `interact`, `look`, `teleport` which settle 1 frame).
* **Replies.** Every reply has `"ok"`. Errors are `{"ok": false, "error": "..."}`. If the request has
  `"id"`, the reply echoes it.
* **Seeds.** A number is used as-is; a word (`"ALPHA"`) is hashed (FNV-1a 32) to the run seed.
  Finite floors are `"<seed>-D<depth>"`, Infinite levels `"<seed>:L<level>"`.
* **Safety.** Agent runs never touch the player's saved checkpoint. TCP binds 127.0.0.1 only.

## Quick start (copy these lines)

```json
{"cmd":"hello"}
{"cmd":"start_run","seed":12345,"difficulty":"standard","mode":"finite"}
{"cmd":"objects"}
{"cmd":"goto","to":"relay","index":0}
{"cmd":"interact","kind":"RELAY"}
{"cmd":"events"}
{"cmd":"step","frames":60,"act":{"move":[0,1],"hold":{"sprint":true}}}
{"cmd":"map","radius":8}
{"cmd":"report"}
{"cmd":"quit"}
```

PowerShell: `Get-Content cmds.jsonl | .\target\debug\last_route.exe --headless`.
Python: `subprocess.Popen([exe, "--headless"], stdin=PIPE, stdout=PIPE, text=True)`, write a line,
`readline()` the reply.

A good loop for an LLM: `observe` (or `state`) -> decide -> `goto` / `act`+`step` / `interact` ->
`events` to see what happened. Use `teleport` + `admin god` when you only want to test a mechanic.

## Commands

### Bridge commands (same names and shapes as playtest_bridge.gd)

| cmd | request | reply |
|---|---|---|
| `hello` | | `protocol` (1), `games`, `game`, `version`, `revision` ("rust-port"), `debug_run`, `fixed_fps`, `physics_hz`, `headless`, `commands`, `pulses` |
| `start_run` | `seed` (number or text, default 12345), `difficulty` (`relaxed`/`standard`/`hard`), `mode` (`finite`/`infinite`, default finite), `depth` (finite, default 1), `level` (infinite, 0 = top, negative = down), `bot_seed`, `fixed_fps`, `god`, `no_target` | `seed`, `seed_text`, `difficulty`, `mode`, `obs` |
| `leave_run` | | ends the run |
| `observe` | | `obs` (below) |
| `act` | `act: {...}` (or the act keys at top level) | sets inputs, does not step |
| `step` | `frames` (1..7200, default 1), `act`, `observe` (default true), `events` (bool), `stop_on_end` (default true) | `frames` stepped, `frame`, `obs`, `events` |
| `action` | `name` (a pulse) | one-frame press on the next step |
| `admin` | `name` + args (below) | |
| `log` | `since` | `lines` [{seq, kind (error/warning), text, frame}], `seq`, `errors`, `warnings` (panics inside a command are logged as errors) |
| `screenshot` | `path` | windowed only (`--agent-window`): `path`, `size`; headless: the bridge's error "no rendered image (headless run: ...)" |
| `time_scale` | `scale` | accepted; affects `--agent-window --realtime` only |
| `ui` / `press` | | the port has no menus in agent mode: `ui` returns no buttons, `press` errors |
| `quit` | `code` | exits with `code` |

**act** (inputs persist until changed, like the bridge):
```json
{"move":[strafe_right, forward], "look":[turn_right, look_down],
 "hold":{"sprint":true, "crouch":false, "jump":false, "interact":false},
 "pulse":["interact","flashlight"], "look_delta":[radians_right, radians_down], "slot":2}
```
`move`/`look` are sticks in -1..1 (look turns at 2.8 rad/s at full stick). `pulse` presses for one
frame: `interact`, `flashlight`, `taser`, `tool_use`, `drop_item`, `hotbar_previous`, `hotbar_next`,
`jump`, `slot_1`..`slot_5`, `shop_1`..`shop_4` (Zellia's store); `inventory`, `toggle_map`,
`pause`, `ui_*` are accepted and ignored. `hold.interact` is "hold [E]" (winding the music box).

**admin** (also `npc` {count, smart}: bots round the crosshair spot, and, in `--agent-window` only, `remote` {peer, x, z, yaw, h, clip: idle|walk|sprint|chase|crouch|jump|fall|climb|swim|stun|down, moving, sprinting, crouching, slim, vy, skin, hide: [cell x, cell y, facing x, facing y], hide_for} to draw a made-up other runner and check its pose, `clear: true` removes them): `god` {on}, `no_target` {on}, `noclip` {on}, `speed` {scale 0.25..3}, `teleport` {x, y, z}
(x/z grid, y ignored), plus port helpers `give` {item: flashlight|taser|crowbar|axe|battery|stim,
count}, `set` {lamp, stamina, temperature, ambient, exposure, noise, coins, yaw, pitch}, `summon` {id, count} (the admin summon: hunters on the ring round you), `wake`
{index, state: "wander"|"idle"} (activate a hunter), `hunter_to` {index, x, z | near: metres in
front of you}, `flicker` {seconds, default 1.4} (force_flashlight_flicker: the beam strobes and [F] is locked). `god`/`no_target` mean no capture and nothing ends the run (no_target: hunters do
not sense you at all); the in-game admin overlay's switches (`ui::admin::flags()`) act the same.

### Port commands (direct queries and helpers)

| cmd | request | reply key |
|---|---|---|
| `state` | `sections` (optional list) | `state`: run, player, meters, hotbar, hunters, relays, focus |
| `player` | | position, `cell`, `yaw`, `pitch`, `stamina`, `exhausted`, `lamp`, `flashlight_on`, `crouching`, `sprinting`, `noise`, `hidden` (locker index or null), `in_vent`, `stun`, `stim`, `walked`, `in_safe_zone`, `room`, `local_climate` |
| `meters` | | `stamina`, `lamp`, `exposure`, `noise`, `temperature`, `ambient`, `heat_locked`, `overheating`, `freezing` |
| `hotbar` | | `selected`, `slots` [{item, label, count} or null] x5, `taser_cooldown`, `melee_cooldown` |
| `npcs` | | [{id, name, slot, level, pos, distance, smart, joined, walking}] (NPC teammates) |
| `hunters` | | [{index, id, name, tier, active, state (idle/wander/investigate/chase/search), pos, facing, distance, visible, stun, slow, speed, last_known, taser_immune, melee_immune}] |
| `relays` | | [{index, cell, pos, taken, distance}] |
| `focus` | | what [E] would hit now: {kind, index, pos, distance, prompt} or null |
| `objects` | | start, exit, has_lift, safe_zone, hunter_spawn, relays, batteries, pickups, coins, lockers, stairs, light_switches, heater, music_box, vents |
| `rooms` | | [{index, type, label, region, rect [x,z,w,h], center, climate}] |
| `map` | `radius` (around you), `discovered_only` (default true) | `rows_text` (one string per z row): `#` wall `.` floor `?` unseen `P` you `H` hunter `R` relay `B` battery `L` locker `X` lift `S` safe zone `>`/`<` stairs down/up; `origin` [x,z] of the first char |
| `discovered` | `cells` (bool) | `count` (+ `cells` [[x,z]...]) |
| `events` | `since` (seq), `peek` | events since the last call (or since `seq`) |
| `report` | | run report (`ended`, hunter, cause, depth, score, relays, coins, seed, difficulty, frames, time, walked, shots, hits) |
| `director` | | hunter director: mode, relays_collected, relay_rush_active, stats {shares, flanks, covers, scatters, released, rushes} |
| `nav` | | reachability from the start: relays/batteries/lockers reachable, exit_reachable, unreachable list |
| `hash` | | `hash` (PlaytestObserver.state_hash: frame, scene, run, player, world, hunters, floats snapped to 0.01; FNV-1a 64) |
| `look` | `yaw`, `pitch`, `dyaw`, `dpitch`, `at` [x,z] + `height`, `face` north/south/east/west | `player`, `focus` (after 1 settle frame) |
| `teleport` | `{x, z}`, `{cell:[x,z]}`, or `{to, index}` with to = relay, battery, locker, lift, stairs, stairs_down, stairs_up, safe_zone, heater, music_box, vent, light_switch, coin, pickup, hunter, start | `player`, `focus` (placed beside the object, facing it: the next `interact` hits it) |
| `teleport` `stairs_down` / `stairs_up` | `{to: "stairs_down"}` or `{to: "stairs_up"}` | places you beside the nearest flight end (kept for tests and bots); you then walk the flight, there is no swap |
| `interact` | `kind` (refuse unless the focus is that kind, e.g. "RELAY"), `hold_frames` | `focus_before`, `focus_after`, `events`, `run` |
| `goto` | `{x, z}` or `{to, index}`, `max_frames` (1800), `radius` | walks there with the bot's A* navigator: `arrived`, `frames`, `distance`, `player`, `focus` |
| `step_until` | `expect` (list, see Expectations), `max_frames` (600), `act`, `bot` (let the bot drive), `check_every` | `met`, `frames`, `unmet` |
| `bot` | `frames`, `bot_seed`, `reset`, `debug`, `bot_events`, `observe` | the bot plays N frames: `bot` stats (+ `bot_events`) |
| `walk_flight` | `dir` (`down` default, `up`), `frames` (default 900), `stop_on_level_change` | walks the nearest flight end to end along its walk path. `stop_on_level_change: true` stops on the frame the level label changes (the walk never rebuilds or moves the body; only an admin level jump does). Kept for `tests/*.json` and the bots |
| `stairs_report` | | the stair-link diagnostic: `level`, window `origin`, window `center`, `resident_levels`, the `player` (absolute cell, y, `on_flight`) and the nearest 3 links of the active level (`flights`, ladders included and marked): `kind`, `dir`, `to_level`, `shape`, `run`, absolute `landing` and `foot` cells, `distance`, `well_cells` (count), `surface_rel_landing` and `surface_rel_foot` (tread height above the active floor, null off the flight), `neighbour_resident` and `twin` (true when the other level holds the matching record at this link's far end; null while that level is not resident). Run it standing at a stairwell that does not link and send it with the seed and the level |
| `help` | | command and pulse lists |

Focus / interactable kinds (the original's `RouteInteractable3D.Kind` names): `RELAY`, `BATTERY`,
`TOOL_PICKUP`, `COIN`, `HIDING_SPOT`, `SAFE_ZONE`, `LIFT`, `STAIRS`, `LIGHT_SWITCH`,
`HEATER_STATION`, `MUSIC_BOX`, `VENT`, `SHOP`.

### The observation (`observe`, `step`)

The bridge's LastRouteBot observation, plus port blocks:
```
frame, time, scene ("main_3d" | "infinite_route" | "run_report" | "main_menu"), paused, fps, game,
log {errors, warnings, seq},
run {active, over, depth, score, relays, relays_total, coins, debug_run, death {hunter_id, via, detail} | null,
     seed, difficulty, mode, level, statistics {rooms_discovered, relays_collected, elapsed_seconds,
     distance_m, visited_levels, encountered_hunters, hunters_encountered, items_collected, chases_started, chunks_visited}},
player {pos [x,h,z], cell [x,z], yaw, pitch, vel, hidden, stamina, lamp, flashlight, mode (idle/walk/sprint/crouch/hidden/vent/captured),
        noise, frozen, spectator, item, in_safe_room, exhausted},
focus {kind, index, pos, distance, prompt} | {},
world {chunk [cx, level, cz], level, loaded_chunks, region, relays[8], batteries[4], safe_zones[4], hiding_spots[6]  (each {id, x, y, dist, capacity}, nearest first; y is the grid z),
       origin, size},
hunters [{entity, id, state (IDLE/WANDER/INVESTIGATE/CHASE/SEARCH/PEEK), pos [x,z], dist, los, stuck, stun, active, visible}]  (nearest first),
observe also reports `level` (derived label), `y` (absolute feet y) and `resident` (the floors simulated in the stack) for the player and each hunter.
interactables [{kind, pos [x,h,z], dist, completed}]  (<= 30 within 45 m),
online {...}, meters {...}, hotbar {...}, escape {remaining, progress, needed, hunter} | null, message, toast, director {...}
```

### Events (`events`, `step` with `events:true`)

Each event: `{seq, frame, type, ...}`. Types: `run_started`, `floor_changed` {depth, level,
level_seed}, `window_recentred` {center}, `relay_collected` {relays_collected, relays_total, score},
`coins_changed`, `hotbar_changed` {slots}, `flashlight` {on}, `heater` {state}, `hid` {locker},
`left_hiding`, `entered_vent`, `left_vent`, `hunter_activated`, `hunter_spawned`, `hunter_state`
{id, from, to}, `hunter_encounter` {hunter, state, dist}, `captured` {hunter, presses_needed,
seconds}, `escaped`, `message` {text}, `toast` {text}, `run_ended` {report}.

### The run's rules an agent needs

* Recover every relay on a finite floor, then `interact` the `LIFT` to go one depth down.
  The route hunter wakes after `relays` = 3 (relaxed), 1 (standard), 0 (hard).
* Infinite route: no lift; `STAIRS` change level; more hunters join as relays come in.
* A capture starts the struggle: press `interact` 6 times within 3.2 s (one pulse per step, e.g.
  `{"cmd":"step","frames":2,"act":{"pulse":["interact"]}}` six times) or the run ends.
* Lockers (`HIDING_SPOT`) hide you; `interact` again to leave. The safe zone restores lamp and stamina.
* The flashlight drains `lamp`; batteries (`tool_use` with a battery selected) add 25.

## Expectations (step_until and suite files)

`{"path": "run.relays_collected", "op": ">=", "value": 3}`. Paths are dot separated, numbers
index lists, a final `len` is a length. Ops: `==` `!=` `<` `<=` `>` `>=` `approx` (`tol`)
`exists` `missing` `truthy` `falsy` `contains` `matches` (substring, case-insensitive) `in`.
`"var": "name.path"` compares with a reply saved by `save_as`. Over lists:
`{"any": "hunters", "where": [{"path": "state", "value": "chase"}]}`, `all`, `none`,
`{"count": "objects.coins", "where": {...}, "op": "==", "value": 1}`.
The document has the sections `run`, `player`, `meters`, `hotbar`, `hunters`, `relays`, `focus`,
`objects`, `rooms`, `report`, `director`, `nav`, `map`, `discovered`, `obs`, `events` (all this
run), `bot` (bot stats) and `last` (the previous reply).

```json
{"cmd":"step_until","expect":[{"path":"run.escape","op":"exists"}],"max_frames":300}
{"cmd":"step_until","bot":true,"expect":[{"path":"run.depth","op":">=","value":2}],"max_frames":20000}
```

## Test suites (`--test-suite`)

Suites live in `app/tests/*.json`:
```json
{
  "suite": "name", "description": "...", "ported_from": "tests/validate_x.gd",
  "defaults": {"seed": 12345, "difficulty": "standard", "mode": "finite"},
  "tests": [
    {"name": "recovering a relay scores 100",
     "start_run": {"no_target": true},
     "steps": [
       {"cmd": "teleport", "to": "relay", "index": 0},
       {"cmd": "interact", "kind": "RELAY"},
       {"expect": [{"path": "run.score", "value": 100}]},
       {"cmd": "hash", "save_as": "h"},
       {"cmd": "nope", "expect_ok": false}
     ]}
  ]
}
```
Each test starts a run from `defaults` + `start_run` (`"start_run": false` = no run). A step is a
protocol command and/or `expect`; options: `save_as`, `expect_ok` (default true), `repeat`, `label`.
A `step_until` step fails the test when not met (`"must_meet": false` to allow it).

Output: the JSON report on stdout (and `--out FILE`):
`{version, pass, seconds, totals {suites, suites_passed, tests, passed, failed, unreadable},
suites [{suite, file, description, ported_from, pass, seconds, tests [{name, pass, steps, failures, seconds, frames}]}]}`;
progress on stderr in the original validator's form (`== suite`, `  FAIL  test: message`,
`-- suite NAME PASS 0.12s`, `RESULT: PASS (...)`). Exit 0 all pass, 1 any failure, 2 nothing ran.
Globs are expanded by the program (PowerShell does not); a directory runs all its `*.json`.

| suite | covers (original) |
|---|---|
| protocol | bridge answers, errors, act/step round trip, events, state (validate_playtest.gd) |
| determinism | same seed + inputs = same hash, scripted / bot / Infinite (playtest_check.sh) |
| facility | 9 rooms, 5 relays, everything reachable, seeded sameness (validate_facility, pickup_reach, room) |
| meters | stamina, exhaustion, flashlight drain/death, warming, climate, battery stacks, stim, safe zone |
| run_flow | relay scoring, activation per difficulty, lift, capture -> struggle -> escape / report, god / no_target (run_report) |
| hunters | director (relays mirrored, relay rush), taser on immune hunters, crowbar stun + durability, noclip, run speed |
| infinite | 135x135 window, stairs, recentring, population growth (streamed_world, level_links) |
| interactables | light switch, heater, locker, pickups, coins, vents, music box |
| bot | collect_relays, reach_depth, explore_coverage, goto (playtest_scenarios.gd) |

## The bot (`--bot` / `--playtest-runner`, or the `bot` command)

A port of `last_route_bot.gd`: escape a capture by mashing [E]; stay hidden while a threat is near
(leave 3 s after it clears); on a threat (a chaser within 20 m in sight or 9 m, a searcher within
7 m) run to the safe zone, else a locker, else 25 m away; otherwise recover relays (skipping camped
ones), fetch a battery under 30 % lamp, take tools, ride the lift (finite) / take stairs after
150-300 s on a level (infinite), else explore the least-visited 9 m block. A* on the hunters' grid,
2-cell look-ahead steering, stuck recovery (jump, interact, strafe, replan, blacklist) and soft-lock
recovery (teleport within 6 m in debug runs), idle watchdog.

Runner options (`--key=value` or `--key value`): `scenario` (survive, collect_relays, reach_depth,
explore_coverage, goto_check with `goal_x`/`goal_z`), `seed` (PLAYTEST), `difficulty`, `mode`
(infinite), `minutes` (2), `bot_seed`, `target`, `out` (.playtest/<name>), `name`, `fixed_fps`,
`hash_frames`, `deadline`, limits `max_stuck` 3, `max_soft_locks` 0, `max_unreachable` 6,
`max_idle_s` 90, `max_errors` 0, and `--json` (report on stdout). Writes `report.json` (game,
scenario, seed, pass, failures, bot stats, outcome, game_seconds, frames, perf, deaths, state_hash,
final {depth, relays, score, director}) and `events.jsonl`; prints
`PLAYTEST PASS|FAIL: last_route/<scenario> seed=... game_s=... frames=... stuck=... soft_locks=... unreachable=... errors=... deaths=... relays=... depth=... chunks=...`.
Exit 0 pass, 1 thresholds failed, 2 could not run. Survive restarts 180 frames after a death.

## Differences from the Godot bridge

* No menus, online play, Night Watch, sequencer/flythrough (rendering tools) in agent mode.
* `screenshot` needs `--agent-window`; `time_scale` only matters there with `--realtime`.
* Extra commands: everything in "Port commands"; admin `give`/`set`/`wake`/`hunter_to`.
* Mod packs from the user folder still load (as in the game); the checkpoint is never written.
