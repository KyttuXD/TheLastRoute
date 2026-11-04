# Godot re-sync, 2026-11-04

Read-only audit of the Godot project (`KyttusLabApp`) against `src/bin/last_route`. Godot line numbers were read directly. Rust evidence comes from grep, so "missing" means no matching identifier was found, not that the code was traced. Items marked "verify" were not fully checked.

Size key: S small (under a day, mostly data or a constant), M medium (a feature in one module), L large (new geometry or a new system).

Godot paths are under `KyttusLabApp/`. Rust paths are under `src/bin/last_route/`.

## 1. Sewers and water

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 1 | S-L | Waterfall plunge and swim missing. | `game/player/player_controller_3d.gd:1154-1229` (begin_plunge, arc, swim phase; `PLUNGE_SWIM_SPEED` 1.5 at :357) | No `plunge`, `swim` or `soaked` identifier. | Port the plunge state machine into `game/player.rs`; add swim input. |
| 2 | M | Flood wading missing. | `player_controller_3d.gd:336-338` (wading_depth), :365-372 (depth thresholds, noise), :1237-1256 (`_update_wading`, tiring at `WADE_TIRING_DEPTH`) | No `wade` or `wading`. | Read per-cell depth from the flood grid; apply speed, noise, splash. |
| 3 | S-M | Soaked status and drip noise missing. | `player_controller_3d.gd:362-364` (`SOAKED_SECONDS` 30, `SOAKED_NOISE_SCALE` 1.45, `SOAKED_DRIP_EVERY` 1.1-2.4), :1255 onward | None. | Add a `soaked_left` timer to `game/player.rs`; put the constants in data. |
| 4 | M | Water has no depth or surface height. | `world/generation/sewer_route_generator.gd:95-96` (`CHANNEL_DEPTH` 1.0, `WATER_SURFACE` -0.3) | `world/route/sewer/core.rs:47-53` has only the `WATER` code. | Add a surface-height value; carry it into sewer mesh and collision. |
| 5 | S | Flood reach and cap numbers unverified. | `sewer_route_generator.gd:910-911` (`FLOOD_REACH` 12, `FLOOD_MAX_CELLS` 160) | `world/route/sewer/breach.rs:49-50` | Test that asserts 12 and 160 (Godot-parity values). |
| 6 | M | Sewer ladders, manholes, grates, elevators open. | `sewer_route_generator.gd:727` (`_ladder_wall`), :762 (`_shaft_cell`) | `world/route/sewer/core.rs` has `LadderLink`; `game/ladder.rs` exists; TODO.md:383 still open. | Finish the remaining link kinds; tick TODO.md only after validation. |
| 7 | S | Sewer loot table unused. | `sewer_route_generator.gd:662` (`_pickups`) | TODO.md:760 and :385 (`data/loot/sewer.json`). | Wire the table into the pickup roll. |
| 8 | M | Sewer level bed missing. | `game/audio/level_ambience.gd` | `audio/cues/ambience.rs` has per-level beds, no sewer entry. TODO.md:797. | Add the sewer bed mapping. |
| 9 | S-M | Positional water loops partly ported. | `game/audio/emitter_loops.gd:29` (`falls`, `outflow`, `flood`, `lava`) | `audio/cues/ambience.rs:7` mentions FallsLoop and flood; no lava loop. TODO.md:798. | Add the missing loop kinds. |
| 10 | S | Extra relay chance unconfirmed. | `sewer_route_generator.gd:27` (`SEWER_EXTRA_RELAY_CHANCE` .55) | Not checked; see `world/route/pickups.rs`. | Check the relay spawn roll. |
| 11 | S | Hunter affinity lists need a parity check. | `sewer_route_generator.gd:388` (`_hunter_spawns`) | `world/route/sewer/hunters.rs:43-45` (`SEWER_PREFER`, `SEWER_AVOID`) | Compare both lists to Godot. |

## 2. Inferno (level -3)

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 12 | L | Lava pits not ported. | `audio/emitter_loops.gd:44-45` (lava loop), `config/room_catalog.gd` (lava rooms) | `world/inferno.rs` carves ledges and basins only. TODO.md:809. | Add a lava cell kind and a damage or stun rule. |
| 13 | M-L | Cages and spider lair not ported. | `config/secret_creatures.gd` | `world/inferno.rs` has no cage geometry. TODO.md:633. | Port the cage props and lair spawn. |
| 14 | M | Inferno spider spawn missing on -3. | `config/secret_creatures.gd` | `hunters/gimmicks/` has no inferno spawn rule. TODO.md:633. | Add the spawn rule to the inferno chunk. |
| 15 | M | Player ignores inferno floor heights. | n/a | `render/world_mesh.rs:741-879` draws heights; player stays at base. TODO.md:282. | Sample `Inferno::height_at` in the player ground-height path. |

## 3. Catwalks and level 0

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 16 | M | Level-0 catwalks not ported. | `night_watch/openair/openair_world.gd:145-151` (night watch geometry); facility rules `hunters/ai/streamed_hunter_navigation.gd:548-576` | `world/facility.rs:151-156` has `DeckKind::Catwalk` and `set_walk_surface`, no geometry. | Add catwalk cells to the level-0 generator. |
| 17 | S-M | Hunter sight through breakables unchecked. | `streamed_hunter_navigation.gd:576` (`SEE_THROUGH_BREAKABLES`: `chainedGate`, `lockedCatwalkGate`, `sluiceGate`) | No matching list. | Add the same see-through list (as data) to hunter sight. |
| 18 | S | Level-0 maze regions not compared. | `world/generation/streamed_route_generator.gd` (279 KB), `docs/REGION_RULES.md` | `world/route/layout.rs`, `furnished.rs` | Diff the region list against REGION_RULES.md. |

## 4. Hunters, abilities and numbers

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 19 | M | Burst, intercept, slam missing. | `hunters/ai/hard_abilities.gd:55` (BURST), :60 (INTERCEPT), :65 (SLAM), `update` at :179 | Only Nyxie's pounce (`hunters/gimmicks/nyxie.rs:6-7`). | Port the shared HardAbilities module. |
| 20 | S | Pounce and stun numbers: verify data file. | `hard_abilities.gd:15-19` (FREQUENCY, BUDGET 360/45/25), :51-53 (`LEAP_SPEED` 7.2, `BONK_STUN` .8) | `nyxie.rs:464` quotes the same numbers in a comment. | Confirm the JSON data matches; a comment is not proof. |
| 21 | - | Stamina numbers match. | `config/game_config.gd:67-71` | `config/mod.rs:48-50` | None. |
| 22 | S | Brain timing constants. | `hunters/ai/hunter_brain.gd:30-43` (`STUCK_CENTER_AFTER` .25, `CENTER_STEER_SECONDS` .8, `UNREACHABLE_GIVE_UP` 1.6) | `hunters/brain.rs:331-343` has arrival and waypoint values, no STUCK or CENTER constants. | Add the constants or confirm they are inlined. |
| 23 | S | Hearing and sight constants. | `hunters/ai/hunter_seats.gd:14-16` (`HEAR_NOISE` 2.0, `HEAR_DISTANCE` 12, `SIGHT_DISTANCE` 14), `config/game_config.gd:221-225` | Not checked; `hunters/brain.rs`. | Compare. |
| 24 | S | Void sight requirement. | `hunters/gimmicks/void_cyan_gimmick.gd:7` (`SIGHT_REQUIRED` 2.5) | `hunters/gimmicks/void/` not checked. | Compare. |
| 25 | S | Ravelle tripwire text likely mismatched. | `hunters/gimmicks/ravelle_gimmick.gd:50` ("TRIPWIRE // STUNNED") | Not checked. | Compare message and stun length. |
| 26 | S | Relay rush threshold unverified. | `hunters/ai/hunter_director.gd:39` (`RELAY_RUSH_COUNT` 7), :10-13 (`RELAY_RUSH_DELAY`) | `game/hunting.rs:126`, `game/progression.rs:41` | Test asserting 7 and the delay. |
| 27 | S | "did you hear that" subtext still present. | Not in runtime `game/` code. | `game/hunting.rs:253` | Remove the line. |
| 28 | S | STUNNED messages still present. | `ravelle_gimmick.gd:50` only (`infinite_route_test.gd:4317` is a test file). | `game/items.rs:156` ("TASER HIT // N STUNNED"), :196 ("N STUNNED Xs") | Remove or reword as requested. |

## 5. NPC teammates

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 29 | M | Only the "door" task is built. | `player/npc_teammates.gd` has no relay or locker task either | `game/npc.rs:76-77` | Decide whether relay and locker tasks are in scope. |
| 30 | M | NPC storey crossing. | `npc_teammates.gd:34` (`STOREY_KINDS` stairs, spiral, ladder, ramp), :396-567 (`_planned_link`, begin and tick crossing) | No storey-link functions in `game/npc.rs`. | Port `_planned_link` and the crossing state. Stairs and ladders stay direct, no fade or halt. |
| 31 | S-M | NPC chat missing. | `npc_teammates.gd:52` (`CHAT_COOLDOWN` 30), :637 (`_talk`) | No chat function. | Port cooldown and line picker (lines as data). |
| 32 | M | NPC command ops partly missing. | `npc_teammates.gd:47-51` (OPS: teleport, goto, follow, unfollow, smart, chat, freeze, resume, ignore, target, clear) | Only spawn, remove and door task. | Add remaining ops to the admin path. |
| 33 | M | NPC down and revive missing. | `npc_teammates.gd:235` (`down`), :261 (`revived`) | None. | Add a down state that stops movement, and a revive call. |
| 34 | M | Teammate waypoints stored but unused. | `hud/teammate_waypoints.gd`, `hud/waypoint_overlay.gd` | `app/settings.rs:7`, :282 | Add a waypoint overlay for teammates. |

## 6. Multiplayer, hiding and interactables

| # | Size | Gap | Godot | Rust | Fix |
|---|------|-----|-------|------|-----|
| 35 | M | NPC sync rate unchecked. | `npc_teammates.gd:35` (`SEND_HZ` 12) | `net/world_sync.rs`, `net/protocol.rs` not compared. | Confirm rate and packet budget. |
| 36 | M | Hiding spots partly present. | `hunters/ai/hunter_brain.gd:306` (`may_grab_hidden`), :632, :1142 (search the spot nearest last sighting) | `config/mod.rs:133` (`HIDING_SPOT`); hunter search unverified. | Check hunter search and grab rules. Standard `hiding_spot_chance` stays 0.0. |
| 37 | S-M | Interactable parity incomplete. | `assets/interactables/` (lift, vent, safe_zone, heater_station, music_box, coin, battery) | `game/interact.rs`, `world/route/music_boxes.rs`, `pickups.rs` cover some; vent, lift, heater unconfirmed. | List Rust interactable kinds and diff against Godot. |

## Suggested order

1. Quick text and data cleanup (all S): items 27, 28, then the verification sweep 5, 10, 11, 20, 22-26, 21 (no change). These are cheap and de-risk the later work.
2. Water player state: items 1-3 together in `game/player.rs` (citing `player_controller_3d.gd:1154-1256`), then item 4 (surface height) and item 5 test.
3. Sewer completion: items 6, 7, 8, 9 (sewer bed and lava loop kind).
4. Hunters: item 19 (shared HardAbilities), item 17, item 36.
5. Inferno: item 15 first (small), then 14, 12, 13.
6. Level 0: items 18, then 16.
7. NPC teammates: settle scope (item 29) first, then 33, 32, 31, 30, 34, 35.
8. Interactable diff (item 37) any time; it only produces a list.

## Rules while porting

- Numbers, timings and lists go in JSON under `src/bin/last_route/data/`, not in code.
- Bots and chased fake players never trigger the player's danger readout, heartbeat or chase cue.
- Tick TODO.md only after validation.
