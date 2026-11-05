# Continuous levels: stairs, ladders and drops as one space

Status: implemented (phases 0-9). Owner request (KT, repeated): levels must connect "like they are together", not through a player teleport, a storey swap, or a fake connection.

All line numbers come from five read-only investigator reports on a working tree that other agents were editing. Re-check each one with `git show HEAD:path` before editing. Paths are under `src/bin/last_route/` unless noted.

Related docs: [CATWALKS.md](CATWALKS.md) (the existing same-world deck model), [PERF_TRIANGLES.md](PERF_TRIANGLES.md), [SOURCE_LAYOUT.md](SOURCE_LAYOUT.md), [DATA_FILES.md](DATA_FILES.md), [GODOT_RUST_PARITY.md](GODOT_RUST_PARITY.md), [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).

Source files referred to: [vertical.rs](../src/bin/last_route/world/vertical.rs), [endless.rs](../src/bin/last_route/game/endless.rs), [ladder.rs](../src/bin/last_route/game/ladder.rs), [transition.rs](../src/bin/last_route/game/transition.rs), [stairs.rs](../src/bin/last_route/render/stairs.rs), [world_mesh.rs](../src/bin/last_route/render/world_mesh.rs), [level_links.rs](../src/bin/last_route/world/level_links.rs), [swim_effects.rs](../src/bin/last_route/game/swim_effects.rs), [world_sync.rs](../src/bin/last_route/net/world_sync.rs), [stairs_links.json](../src/bin/last_route/data/stairs_links.json).

---

## 1. What is fake today

This table is the history: every row is fixed (phases 1-9, see the "as built" sections at the end).

Every level is its own coordinate space (floor at y = 0) and only one level's `Facility` (`Game.fac`) exists at a time. Every connection is a storey swap.

### 1.1 The swap and the teleports

| # | Where | What is fake |
|---|-------|--------------|
| 1 | `game/endless.rs:1165-1178` `Game::change_storey` | `sync_out`, store hunters (`store_level0_hunters` 1442, `store_floor_hunters` 1468), `inf.level = to`, `load_window` (706-771; `self.fac`/`self.fix` replaced at 764-765), `arrive` closure, `restore_level_hunters` (1488), `announce("LEVEL n")`. The core swap. Every connection type calls it. |
| 2 | `game/endless.rs:1216-1236` `walked_level_change` | Walking a flight is a real ramp, but when `level_from_height` flips (`world/vertical.rs:144-150`, threshold 2.9375 m) it calls `change_storey`. The `arrive` closure (1228-1234) may snap the body with `nearest_open_or` and then hard-sets `g.p.height = stair_height_at(new fac).unwrap_or(0.0)` (1233). Absolute y pops by about 1.7 to 3 m. |
| 3 | **Fixed (phase 5).** `game/transition.rs:82-90` `start_stairs`, `game/interact.rs:50-56` `take_flight`, `game/endless.rs:1182-1208` `take_stairs` | Interact on a stairwell changes level on the pressed frame. The body is teleported to `nearest_open_or(far_cell - run*landing_offset)` (1197-1201), yaw reset (1202), fallback `fac.start.center()` (1204). The far cell is found by arithmetic: `cell + run * flight_steps` (1189-1190). |
| 4 | **Fixed (phase 6).** `game/ladder.rs:256-262` + `216-221` `place_beside_ladder` | At the end of a climb it calls `change_storey` and puts the body 0.75 m beside the shaft with `climb_lift = 0`. The eye drops 5 m and x/z jumps in one frame. The climb itself (`lift_at` 93-95, `player.rs:208`) is only a camera offset on a single-level world. x/z is pinned to the rung line (`ladder.rs:265`). |
| 5 | **Fixed (phase 7).** `game/swim_effects.rs:334-386` `try_begin_plunge` / `advance_plunge` | The waterfall is a scripted arc with input and collision off (`game/mod.rs:776-779`). On the splash frame it calls `change_storey(LOWER_LEVEL, ..., \|_\| {})` (372) and then rewrites `p.pos` from the arc (377). |
| 6 | `game/endless.rs:878-913` `prefetch_connections` | Builds the other level's `Facility` ahead (`inf.prefetched`, radius `prefetch_distance` 8 in `stairs_links.json`). It is a cache for the swap. It is never drawn or simulated alongside the current level. |

### 1.2 Geometry that does not span a storey

- `world/vertical.rs:17` `STOREY = WALL_HEIGHT = 5.0`, but a flight up rises `RISE_UP 0.75/m * STAIR_FLIGHT 4 = 3.0 m` and a flight down sinks `RISE_DOWN 1.0/m * 3 cells = 3.0 m` then a flat `LANDING` (`vertical.rs:41, 47, 111-116`). The comments at 33-47 say these are sized just over `LEVEL_CHANGE_HEIGHT 2.9375` (`vertical.rs:36`) so the swap triggers.
- In absolute y the upper flight runs 5.0 (at A) to 2.0 (at B). The lower flight runs 0.0 (at B) to 3.0 (at A). They overlap in x/z but are 2 m apart in y. Neither reaches the other floor.
- `stair_surface_y` (`vertical.rs:121-140`) returns a height only on flight cells (landing = 0.0). Each level uses its own datum.
- `MID_LANDING_RISE`, `GODOT_SWITCHBACK_RUN`, `FLIGHT_RUN` (`vertical.rs:52-64`) are defined and unused by the drawn geometry.

### 1.3 Rendering that hides the missing floor

- `render/stairs.rs:186-201`: pit lining and a "far wall behind the lowest tread". `stairs.rs:207-231`: shaft walls to `wall_h + SHAFT_ABOVE (1.6)` with a plain cap at 230. `stairs.rs:263-324`: `ladder_down` pit and `ladder_up` hatch plate.
- `render/world_mesh.rs:1017-1115` `build_meshes`: every level at floor y = 0 and ceiling y = `wall_h`. The ceiling is skipped over flight-up cells (about 1093-1111). The floor is replaced by treads over flight cells (1067-1072, `flight_cell`).
- `world_mesh.rs:223-238` `is_jump` / `builds_in_one_step`: any level change rebuilds the whole window in one step (the swap-frame hitch). `world_mesh.rs:321-322` `retain(|_, p| p.used + 1 >= builds)` keeps only the last two builds.
- `render/scene.rs:169-210` `draw_scene` draws `g.world.meshes` for one level only.

### 1.4 State and entities held per level

- `Player.height` is relative to the current floor (`game/player.rs:86, 414-415`, eased at `STAIR_FOLLOW_RATE 40`). `feet_y` is clamped `>= 0` (`player.rs:403-411`). `body_fits` forces `h = 0` off a stair (`player.rs:33-48`). There is no absolute y.
- `Facility::body_fits_at` (`world/facility.rs:1098-1127`) treats well cells as holes unless `on_stair_flight` (1083-1089).
- Hunters: `store_*`/`restore_level_hunters` (`endless.rs:1445-1500`), `respawn_outside_window` (1134-1159). Brains are `(f32,f32)` with no y (`hunters/brain.rs:63, 280`). A level the runner is not on is frozen.
- NPC bots: `Npc.level` (`game/npc.rs:210`). `update_npcs_with` freezes bots on another level (1058). `npc_place_beside_runner` (1004-1018) re-places bots beside the runner and overwrites `n.level`.
- Remote players: `Sample { depth, x, y, ... }` (`net/world_sync.rs:70-82`) has no height. `remotes_at(depth, origin)` (1019-1036) filters on equal depth. `net/runner_figure.rs:187, 204` draws at y = 0. A teammate vanishes the moment their level flips.
- Link pairing by data: `world/level_links.rs:16-65` `ladders()` / `well_stairs()` emit separate `Stair` records into two `Facility` objects that agree only by cell arithmetic. `transition.rs:49-51` rejects any `flight_steps != STAIR_FLIGHT + 1`.
- Tests that certify the swap and must be rewritten: `ladder.rs:447` (`climbing_down_to_the_bottom_changes_storey_on_that_frame_with_no_fade`), `ladder_tests.rs`, `endless.rs:2501-2568, 2861-2969, 3026-3027` (including `the_window_built_ahead_is_the_one_swapped_in`), `npc.rs:1334`, `vertical.rs:188` (`a_walked_flight_reaches_the_level_change_height`), `render/stairs.rs:403, 475`.

### 1.5 What is already close

- Catwalks are same-world: `Facility.walk_surface` plus `apply_catwalks` (`facility.rs:1269-1363`) and `route/catwalk_links.rs`. A deck is a surface height inside the same window. This is the template.
- The horizontal generator already aligns the two ends. Lower flight B = A + run*(STAIR_FLIGHT+1) shares the same 2x4 well cells (`infinite.rs:279-307`, `facility.rs:254 stair_well_cells`). Sewer ladder pairs share x/z by construction. Waterfall drop cells line up (`route/sewer/level2.rs:216`).
- `render/waterfall.rs:318` already draws a sheet spanning two storeys.
- Godot: `current_level()` is derived from the player's world y with hysteresis (`streamed_facility_view_3d.gd:360-367`); stairwells are real colliders; neighbour levels stay loaded.

---

## 2. Target model

### 2.1 One vertical frame

- World y of a level's floor is `level * STOREY` (STOREY = 5.0 m, kept; Godot's is 4 m scaled 1.25). Level 0 is y = 0, level -1 (sewer) is y = -5, Inferno (-3) is y = -15. Going down is negative y. The sign is `+level`, not `-level`, because levels below ground are already negative integers. A level's ceiling is `level*STOREY + wall_h`, which coincides with the floor of the level above. A visible slab thickness (`SLAB`, data) separates them.
- `Player` gets an absolute `y` (feet). `height`, `climb_lift` and the `feet_y.max(0.0)` clamp are removed as the source of truth. The camera eases for smoothing only. Gravity runs against `ground_y(x, z, y)`.
- The current level is derived: `current_level = level_from_height(y, previous)` (the existing hysteresis shape, `vertical.rs:144-150`). It is a label for HUD, save, audio bus, `inf.level` bookkeeping and hunter residency. Changing it must have no side effect on the body and no load or fade.

### 2.2 A resident stack, not a replaced window

- `Game.fac` becomes `Game.stack: LevelStack`. It holds the `Facility` for the runner's level plus the adjacent levels that are reachable (see 2.4), all in the same x/z frame (shared window origin; `recentre_window` shifts every resident level together). `fac()` returns the active level's `Facility` so existing code keeps compiling. New code goes through `stack.surface_at(x, z, y)`.
- `inf.prefetched` becomes the live neighbour set. `prefetch_connections` is renamed `keep_stack` and loads the levels the stack needs. `load_window` stops replacing `self.fac` on connections. It runs only for a real teleport such as respawn, checkpoint or new run.
- Residency radius is data (`stairs_links.json`): a neighbour level is resident when any link is within `prefetch_distance` cells, or the runner is inside a shaft or on a flight.

### 2.3 One flight, defined once, in absolute y

A new `world/flight.rs` defines `Flight { upper_level, anchor_cell, run, kind }` and `flight_surface_y(flight, x, z) -> Option<f32>` returning absolute y from the lower floor (`(upper-1)*STOREY`) to the upper floor (`upper*STOREY`). Both levels derive their geometry, openings and collision from it. `well_stairs()` and `ladders()` become thin producers of `Flight` records (the pairing is by construction, not by two matched `Stair` entries).

The flight must rise exactly 5.0 m. See KT decision D1 for the shape. Recommended: a switchback inside the existing 2x4 well (two lanes, mid landing at `MID_LANDING_RISE = 2.5`, 0.625 m rise per cell, comfortable). The constants in `vertical.rs:52-64` already describe it.

Openings (the contract, enforced at generation and by test):
- The upper level has a floor hole over the well cells, with a railing on the open sides. Its landing cell A is open floor.
- The lower level has no ceiling over the well cells. Its foot cell B is open floor with a clear margin.
- Both are the same x/z footprint, computed once from `Flight`.
- Ladders: a 1-cell shaft with a floor hole (or open hatch) at the top level and no ceiling at the bottom. A manhole cover is a model sitting on the hole, not an opaque disc closing it.
- Drops and waterfalls: a hole in the upper channel floor and no ceiling over the pool below. The fall is physical (section 3, phase 7).

### 2.4 Collision

`stack.ground_y(x, z, y, r)` returns the highest walkable surface at or just below the feet across the resident levels: floor, flight tread, deck. `body_fits_at` takes absolute y and tests walls, props, doors and holes of every level whose vertical extent overlaps `[y, y + body_height]`. The `on_stair_flight` hole exception and `h = 0` forcing go away because the well is a real opening. Jump and fall are allowed anywhere; a body over a hole falls to the next level's `ground_y` with normal gravity.

### 2.5 Rendering

- Keep per-level `WorldMeshes` (`PartKey` already carries `level`). Hold the runner's level plus the neighbour(s) that are visible through an opening. Neighbour vertices get a y offset of `level*STOREY` baked at build time (the `reframe()` pattern, `world_mesh.rs:211-220`, extended with `dy`), so lamps, fog and flashlight uniforms stay level-agnostic.
- Every level builds at its true absolute y. The current level is no longer special. `is_jump` stops treating a level change as a jump. The retain rule keeps parts for all resident levels.
- Delete the fake closure geometry in `render/stairs.rs`: pit far wall and lining (186-201), shaft cap (230), ladder hatch plate and pit (263-324). Replace with treads, railings, and the real neighbour floor visible through the hole.
- `scene.rs` draws resident levels in one pass with the existing fog-box cull applied to offset bounds. Lamps from the neighbour are added with their y offset.

### 2.6 Rules the result must satisfy (acceptance)

1. No code path changes the body's x, z or absolute y because a level label changed.
2. No fade, loading, announce-before-arrival or one-frame rebuild when walking a flight, climbing a ladder or dropping.
3. The upper floor is visible from the flight while ascending, and the lower floor is visible from above through the opening.
4. A hunter, bot or remote player uses the same flight with the same rules (section 4).

---

## 3. Phased plan

Each phase leaves the game working and shippable. Hub files are shared and are touched by only one task per phase (the integrator). The 12 hub files requested: `game/mod.rs`, `game/endless.rs`, `render/scene.rs`, `render/world_mesh.rs`, `net/world_sync.rs`, `hunters/brain.rs`, `world/mod.rs`, `config/mod.rs`, `app/mod.rs`, `content/mod.rs`, `game/hunting.rs`, `game/interact.rs`, `ui/menus.rs`, `hunters/mod.rs`. Hubs are marked **[hub]** below.

### Phase 0. Contract test and flight definition (no behaviour change)

- Owns: new `world/flight.rs` (`Flight`, `flight_surface_y`, `well_footprint`), new `world/flight_tests.rs`, new data `data/flights.json` (`storey`, `slab`, `rise_total`, `switchback`, `mid_landing_rise`, `lane_width`, `margin_cells`).
- Reads: `world/vertical.rs`, `world/level_links.rs`, `world/infinite.rs`.
- Hubs touched: `world/mod.rs` **[hub]** (module line only).
- Tests (generation test, mandatory): for every seed in the golden set plus a property sweep (many seeds, all chunks in a 3x3 window, levels 0 to -3), for every stair, ladder and drop assert the top and bottom Facilities agree on: x/z footprint of the hole and the open cells, landing A open on the upper level, foot B open on the lower level, `flight_surface_y` at A equals `upper*STOREY` and at B equals `lower*STOREY` within epsilon, no wall, prop or water in the footprint. Initially marked `#[ignore]` where the current data fails, with a list of failing seeds, then un-ignored as later phases fix them.
- Ships: nothing user-visible. The game works because nothing consumes it yet.

### Phase 1. Flight geometry that spans 5.0 m (still swapping)

- Owns: `world/vertical.rs` (re-derive `tread_top`, `stair_surface_y` from `Flight`), `data/flights.json`, `render/stairs.rs` (treads from the new surface).
- Generation: widen or lengthen the well bay only if D1 requires (`world/infinite.rs`, `world/route/live.rs` `carve_stair_well`).
- Hubs: none beyond Phase 0.
- Tests: `a_flight_reaches_the_storey` (surface is 0 at foot, `STOREY` at top, monotone, no step above a max riser), `vertical.rs` and `render/stairs.rs` mesh-agreement tests rewritten, Phase 0 generation test un-ignored for stairs.
- Ships: the same swap, but the stair now visually rises the full storey (the swap threshold becomes the real midpoint). `LEVEL_CHANGE_HEIGHT` and `RISE_UP`/`RISE_DOWN` retire.

### Phase 2. Absolute y for bodies

- Owns: `game/player.rs` (absolute `y`, `ground_y` provider, camera smoothing), `world/facility.rs` (`walk_height_at(x, z, y)`, `body_fits_at(x, z, y, r)`), new `game/level_stack.rs` (single-level stack at first).
- Hubs: `game/mod.rs` **[hub]** (Player y plumbing), `game/endless.rs` **[hub]** (derive `inf.level` from y through `level_from_height`, keep `change_storey` as the only place the stack changes).
- Tests: jump, crouch, catwalk and stair tests on absolute y; `momentum_carries_across_the_storey_change` kept; footsteps and bob gated by `grounded`, not `feet_y <= 0`.
- Ships: identical gameplay, absolute y everywhere. `feet_y.max(0)` is gone.

### Phase 3. Two-level stack resident, collision across levels

- Owns: `game/level_stack.rs` (adjacent levels resident; `ground_y`, overlap query), `game/preload.rs` (keep set, byte budget now counts meshes), `world/infinite.rs` (shared origin and recentre).
- Hubs: `game/endless.rs` **[hub]** (`prefetch_connections` becomes `keep_stack`; `recentre_window` shifts all levels together; `load_window` only for non-connection loads), `game/mod.rs` **[hub]**.
- Tests: `recentre_shifts_every_resident_level`, `stack_ground_y_picks_the_right_floor`, `body_on_flight_is_blocked_by_lower_level_walls_below_the_landing`, memory cap test.
- Ships: still renders one level; collision already reads two. Walking the flight no longer calls `change_storey`: `walked_level_change` is deleted and the label is derived. The `arrive` closure, `nearest_open_or` snap and `p.height = ...` disappear.

### Phase 4. Render the stack and open the wells

- Owns: `render/world_mesh.rs` (per-level build at absolute y with `dy`, openings cut in floor and ceiling from `well_footprint`, neighbour budget, no level-jump rebuild), `render/stairs.rs` (delete pit, cap, hatch fakes; add railings), `render/scene.rs` (draw resident levels, offset lamps, near-opening cull), `data/render/stream.json` (add `neighbour_chunks_per_frame`, `neighbour_max_parts`, `neighbour_cull_radius`; later `neighbour_near_chunks_per_frame`, the raised per-frame part count while the eye is within `prefetch_distance` of an opening of the active floor, and parts are built nearest-opening first so the well fills first).
- Hubs: `render/world_mesh.rs` **[hub]**, `render/scene.rs` **[hub]**, `game/mod.rs` **[hub]** (`sync_world` builds the stack's meshes).
- Tests: `no_ceiling_over_a_well`, `no_floor_over_a_well_on_the_upper_level`, `neighbour_vertices_are_offset_by_storey`, `a_level_change_is_not_a_jump`, bounds/cull test, lamp cap test (neighbour lamps never evict the 8 nearest own-room lamps).
- Ships: the stairwell is open. The upper floor is visible while going up, the lower floor visible from above. This is the first phase where KT sees the result.

### Phase 5. Interact stairs, links data, take_stairs removal

- Owns: `world/level_links.rs` (emit `Flight`, delete `well_stairs` far-end fake), `game/transition.rs` (remove `flight_steps`, `landing_offset` use and the parse invariant at 49-51; keep `prefetch_distance` as residency radius), `data/stairs_links.json`.
- Hubs: `game/interact.rs` **[hub]** (a flight is not an interact prompt any more; remove `Focus::Stairs` use and `take_flight`), `game/endless.rs` **[hub]** (delete `take_stairs`, 1182-1208), `ui/menus.rs` **[hub]** only if a prompt string or tutorial hint is removed.
- Tests: Phase 0 generation test for all stairs passes; `stairs_cannot_be_used_to_teleport` (no interact path moves the body in x/z); an end-to-end headless walk up and down every flight in the golden seeds with y continuity assert (|dy| per frame under speed*dt + epsilon, no jump over 0.25 m).
- Ships: stairs fully continuous for the player.

### Phase 6. Ladders

- Owns: `game/ladder.rs` (climb moves absolute y along the shaft at `LADDER_CLIMB_SPEED`, mount and dismount lerp x/z to rung line and to the real floor cell on the far end, no `change_storey`, no `place_beside_ladder`), `game/ladder_tests.rs`, `render/stairs.rs` (ladder shaft with rails, open hatch, cover as a model), `world/level_links.rs` (ladder `Flight` kind).
- Hubs: `game/mod.rs` **[hub]** only if the movement dispatch changes.
- Tests: climb is monotone in y, camera y never jumps, dismount stands on a walkable cell, falling off a ladder lands on `ground_y` below, Phase 0 test for ladders.
- Ships: sewer and catwalk ladders continuous. Catwalk links adopt `Flight` for the geometry only.

### Phase 7. Drops and waterfalls

- Owns: `game/swim_effects.rs` (plunge becomes a physical fall from absolute y into the lower pool; remove `change_storey` at 372 and the pos rewrite at 377), `game/swim.rs`, `world/route/sewer/level2.rs` (drop opening in the channel floor, open air above the pool), `render/waterfall.rs` (sheet between true y values).
- Hubs: `game/mod.rs` **[hub]** (remove the "no input, no collision" plunge gate at 776-779 or narrow it to the splash).
- Tests: `the_drop_cells_line_up_between_the_two_levels` extended with floor height, fall time equals `sqrt(2*STOREY/g)` within tolerance, pool surface y equals `-STOREY + water::surface_y`.
- Ships: waterfalls are a real fall.

### Phase 8. Hunters, bots and remote players cross

Detailed in section 4.

- Owns: `hunters/brain.rs` **[hub]** (`y`, `level` derived), `hunters/navigation.rs` (flight edges), `game/hunting.rs` **[hub]** (iterate resident levels), `game/npc.rs`, `net/world_sync.rs` **[hub]** (`Sample.h`), `net/runner_figure.rs`, `net/hunter_events.rs`, `game/guest_apply.rs`.
- Hubs: `hunters/mod.rs` **[hub]**, `app/mod.rs` **[hub]** (sample sender), `content/mod.rs` **[hub]** only if hunter data files are registered, `config/mod.rs` **[hub]** only if new constants are added.
- Tests: `a_hunter_chases_the_runner_up_a_flight`, `a_bot_follows_across_levels`, `remote_on_a_flight_is_drawn_at_its_height`, `old_sample_without_h_still_parses`, determinism test for the host sim.
- Ships: remote and AI bodies share the connection.

### Phase 9. Delete the swap machinery

- Owns: removal of `store_level0_hunters`, `store_floor_hunters`, `restore_level_hunters`, `inf.floor_hunters`, `n.level != level` freeze, `npc_place_beside_runner` re-level, `Prefetch` as a swap cache, unused constants and their tests, and doc updates (`level_links.rs:6-7`, `transition.rs` header).
- Hubs: `game/endless.rs` **[hub]**, `game/mod.rs` **[hub]**.
- Tests: `cargo test` full suites CI runs (`--lib`, `lr_server`, headless), plus a grep test that `change_storey` is only called by respawn, checkpoint and new-run code.

---

## 4. How hunters, bots and remote players cross

- Every body has absolute `y` and a derived `level`. Hunters simulate on the `Facility` of their own level, but the stack is shared, so the simulation iterates the resident levels. Hunters beyond one storey of any runner are not resident and keep the existing store-by-absolute-cell fallback until Phase 9 removes it (open decision D5).
- Navigation: each level keeps its `hunter_open` grid. A `Flight` adds an edge between landing cell A (upper) and foot cell B (lower) with cost equal to the flight's path length. `find_path` (`facility.rs:2085`) runs over the union graph. A hunter walking an edge follows `flight_surface_y(flight, x, z)` at its normal speed. Ladders use the same edge with climb speed. Drops are one-way edges. There is no scripted timer and no final snap, unlike Godot's `LadderTraffic` (`ladder_traffic.gd:29-91`, snap at `infinite_route_test.gd:3383`).
- Stair-aware fit: the hunter footprint test must allow flight cells (the same exception player collision loses in Phase 2 becomes an edge-aware rule here).
- Line of sight: `has_line_of_sight` becomes 3D-aware so a hunter sees through an opening but not through a slab.
- Bots (NPC teammates): remove the freeze at `npc.rs:1058`; bots path over the same edges toward the runner. `npc_place_beside_runner` stays only for new-floor rebuilds.
- Remote players: `Sample` gains optional `h` (absolute y). `depth` is kept for old clients and set to the nearest-floor value from the hysteresis. `remotes_at` returns peers within one storey of the viewer. `RemoteView` gets `h`, and `draw_runners` uses it instead of y = 0.
- Hunter frames (`hunter_events`) gain `level` and `presentation_y` (Godot's `_collect_hunter_report` shape, 6020-6050). Guests render what the host sends; the host sends hunters for every level a guest can see.
- Wire compatibility: new fields are optional and default when absent (`Sample::from_json` already defaults missing flags).

---

## 5. Performance budget and culling

Budget targets (to be measured with the existing stress tests, see [STRESS_TESTS.md](STRESS_TESTS.md)):
- Frame time: no regression above 1.0 ms at the p95 on the reference scene over today's single-level draw.
- Triangles: neighbour level at most +35% of the current level's triangles (see [PERF_TRIANGLES.md](PERF_TRIANGLES.md)).
- Meshing: neighbour parts are built through `StreamBudget` (`stream.json`, 4 ms frame budget, 9 ms max) so no frame exceeds today's budget. There is no one-step build on a level change any more.
- Memory: preload's byte cap counts GPU mesh bytes as well as `Facility` bytes. At most 3 levels resident (current plus one above and one below); only the neighbour on the side of a reachable link is kept.

What to cull:
1. Neighbour level parts only within `neighbour_cull_radius` of an opening or link (1 to 2 `PartKey`s), and only when the eye is within `prefetch_distance` of that link or inside a shaft.
2. Fog-box cull (`scene.rs:983-996`, `WORLD_CULL_RANGE` about 38 m) on offset bounds (recompute `mesh_bounds`).
3. A neighbour part is skipped when the eye cannot see its opening: test the opening's bounding box against the camera frustum and a plane test (eye above or below the slab plane).
4. Doors, props, items, relays, pickups, water and level fx are drawn for the current level only, except hunters, bots and remote players in the opening's view cone.
5. Lamps: neighbour lamps are limited to the stairwell lamp plus 2 nearest. The 16-lamp cap (`shader.rs:13`) keeps the 8 nearest own lamps reserved.
6. z-fighting: a slab of `SLAB` thickness between floor and ceiling, drawn double-sided only at the opening rim; coplanar faces are not emitted.

---

## Decisions (KT, 2026-11-04)

These answer D1 to D8 below and override the recommendations where they differ.

| # | Decision | Effect on the plan |
|---|----------|--------------------|
| D1 | Use all three flight shapes: switchback, longer straight flight and spiral. | `Flight` carries a `shape` (data: `flights.json`); each stairwell picks one from the seed and region. Phase 0 defines all three, and the generation contract test covers every shape. The 2x4 footprint is the switchback; the straight and spiral shapes need their own footprints (`well_footprint` per shape) and generation clearance. |
| D2 | Floors are 5.0 m apart (`y = level * 5.0`). | As designed. |
| D3 | No interact prompt on stairs. **Done (phase 5).** | `Focus::Stairs` and its strings are removed in phase 5. |
| D4 | Ladder top: step onto the real floor cell beside the hatch. | One clear cell per hatch is part of the generation contract. |
| D5 | Simulate hunters on all loaded floors, not only adjacent ones. **Done (phase 8).** | Hunters on every resident floor keep moving. This raises the cost on the old Intel Airs, so the resident-floor count and a per-floor hunter cap are data (`stairs_links.json`), and phase 8 measures frame time on KT-MBA. |
| D6 | Hunters use stairs and ladders at normal speed. | No slow-down or climb pose. |
| D7 | Waterfall: free fall with damage. **Done (phase 7).** | Phase 7 adds a data-driven landing rule (stun or hurt when the player lands outside the pool); inside the pool it stays the splash. Damage numbers go in `data/swim/`. |
| D8 | Ship every phase to main as it passes the tests. | Each phase is its own commit, pushed after the CI-equivalent suites pass. |

## 6. Risks and decisions for KT

Risks:
- **Collision and render must agree on the opening.** A mismatch lets a body fall or clip. The Phase 0 generation test plus a mesh-versus-collision test per well is the guard.
- **Generation independence.** Level 0 (`route/live.rs:402 carve_stair_well`), levels below 0 (`infinite.rs:304-330`), sewer (`sewer::chunk`) and Inferno (`legacy_chunk`) carve their surroundings independently. A flight footprint can hit a room, a water channel, or the cave floor. A property test across many seeds is required, not just the golden seeds.
- **Inferno (-3).** The cave floor is non-flat (`world_mesh.rs:776-815 InfernoFloor::corner`). The flight foot needs a flat pad; otherwise add the corner height into the flight.
- **Hunter behaviour change.** Chases will now follow across levels; difficulty and Night Watch pacing change. Needs playtest.
- **Network.** Guests render host frames only (`guest_gate`). Old clients ignore `h` and filter by `depth`; a mixed session may see players pop at the depth flip.
- **Saves.** The checkpoint stores `depth` (the label) plus optional `y_abs` (absolute feet y). A legacy save loads at `level*STOREY`, resume starts at the level's start cell, and autosave fires on the label change only. `y_abs` decides the label through `level()` and the resume height through `Checkpoint::migrated_y`: a saved y within `standing_tolerance` (`data/app/resume.json`, 0.5 m) of the resumed level's floor is kept (a body that stood or crouched there), any other y is a mid-flight autosave and resumes on the floor, because the save stores no x/z and the start cell is on the floor (a kept mid-flight y would hang the body in the air and drop it with a hard landing). So absolute height is still stored and still used; it is just never used to start a fall.
- **Lighting.** Neighbour lamps through an opening may darken a room if the cap is not respected.
- **Code size.** About 433 stair/ladder/storey hits across 44 files; many tests pin the swap.
- **Memory and meshing.** Double meshing cost is the main performance risk.

Decisions needed from KT:
- **D1 Flight shape.** A straight 5.0 m flight over the current 4 cells is 1.25 m of rise per cell, too steep. Options: (a) switchback with a mid landing inside the 2x4 well (recommended; uses `MID_LANDING_RISE`), (b) a longer straight flight (about 8 cells, changes the generation footprint), (c) a spiral. Godot uses switchback and spiral.
- **D2 Vertical sign and level numbering.** The doc uses floor y = `level * 5.0` (sewer at -5). Confirm.
- **D3 Interact prompt.** A walked flight has no interact. Confirm the stairwell shows no prompt, and whether the Focus::Stairs prompt is removed (saves a few strings in `ui/menus.rs`).
- **D4 Ladder dismount.** Top of a ladder: step onto the real floor next to the hatch (needs 1 clear cell) versus being lifted through. Recommended: step onto the floor cell.
- **D5 Hunter residency.** Do hunters on a level not adjacent to a runner stay frozen (current behaviour, cheapest) or simulate in the background?
- **D6 Hunters on flights.** Hunters walking the same flights at normal speed (recommended) versus a slower or climb animation.
- **D7 Waterfalls.** Full physical fall with damage or splash rules, versus a guided fall that still keeps input off.
- **D8 Staging.** Ship after Phase 4 (visible open wells, collision and ladders still the old swap) or hold the release until Phase 6.

---

## 7. Edit tasks for parallel Sonnet agents

Rules for every task: edit only the listed files; do not touch the hubs unless the task names them; the integrator (task 12) merges hub changes. No overlap in files owned between tasks. Run the focused tests listed, then the full suites CI runs. Phases below are dependencies, not parallel tasks unless noted.

1. **Flight definition (Phase 0).**
   - Files: new `world/flight.rs`, `world/flight_tests.rs`, `data/flights.json`.
   - Work: `Flight`, `flight_surface_y`, `well_footprint` (both levels' hole and open cells), `FlightKind { Stairs, Ladder, Drop }`. Pure functions, absolute y, constants from JSON.
   - Test: unit tests for monotone surface, 0 at foot and `STOREY` at top, footprints identical from both sides, JSON validation.

2. **Generation contract test (Phase 0, runs alongside 1).**
   - Files: new `world/stack_contract_tests.rs`.
   - Work: sweep seeds and chunks, levels 0 to -3; for every stair, ladder and drop compare top and bottom `Facility` x/z footprint, open landing and foot, floor heights, absence of walls, props and water in the footprint. Report failing seeds in the test message. Mark failing categories `#[ignore]` with a tracking comment.
   - Test: the test itself; it must run in under the CI time budget (see [LOCAL_CI.md](LOCAL_CI.md)).

3. **Vertical constants and stair geometry (Phase 1).**
   - Files: `world/vertical.rs`, `render/stairs.rs` (tread and rise only), `data/flights.json` values.
   - Work: re-derive `tread_top` and `stair_surface_y` from `Flight`; remove `RISE_UP`/`RISE_DOWN` and `LEVEL_CHANGE_HEIGHT` as geometry drivers; keep a `level_from_height` with the same hysteresis shape. Update `vertical.rs:188` and `stairs.rs:403, 475` tests.
   - Test: `a_flight_reaches_the_storey`, mesh versus `stair_surface_y` agreement, max riser.

4. **Generation fixes for the contract (Phase 1 to 5).**
   - Files: `world/infinite.rs`, `world/route/live.rs`, `world/route/sewer/*.rs`, `world/inferno*.rs`.
   - Work: make every carve use `well_footprint`: bay, margin, hall, prop clearing, Inferno flat pad, sewer channel avoidance. Drop cell openings.
   - Test: un-ignore the task 2 categories one by one.

5. **Absolute y player and collision (Phase 2).**
   - Files: `game/player.rs`, `world/facility.rs`, new `game/level_stack.rs` (single level at first).
   - Work: `Player.y`, remove `height`, `climb_lift` and the `feet_y` clamp as truth; `ground_y(x, z, y)`, `body_fits_at(x, z, y, r)`; camera smoothing separate; `grounded` for footsteps and bob.
   - Test: existing player tests ported; new jump, crouch, deck, stair tests.

6. **Resident stack and preload (Phase 3).**
   - Files: `game/level_stack.rs` (extend), `game/preload.rs`, `game/endless_stack.rs` (new helper module holding `keep_stack`, recentre-all-levels, byte cap), `world/infinite.rs` window API only if task 4 is not editing the same function (otherwise route through the integrator).
   - Work: adjacent levels resident with shared origin; overlap query; GPU mesh bytes in the cap.
   - Test: `recentre_shifts_every_resident_level`, `stack_ground_y_picks_the_right_floor`, memory cap.

7. **Stack rendering and openings (Phase 4).**
   - Files: `render/world_mesh.rs` (as sole owner, hub), `render/stairs.rs` (closure removal and railings, coordinating with task 3 by sequencing after it), `render/scene.rs` (hub), `data/render/stream.json`.
   - Work: per-level build at absolute y, floor and ceiling openings, no level-jump rebuild, retain all resident levels, neighbour budget, offset lamps, frustum and opening cull.
   - Test: the Phase 4 tests listed above plus a triangle budget test.

8. **Ladders (Phase 6).**
   - Files: `game/ladder.rs`, `game/ladder_tests.rs`, `world/level_links.rs` (ladder `Flight` only), `render/ladder_model.rs` (new; open hatch, rails, cover).
   - Work: real y climb, floor-cell dismount, remove `place_beside_ladder` and `change_storey`; rewrite `ladder.rs:447`.
   - Test: monotone y, no camera jump, falling off lands correctly.

9. **Drops and waterfalls (Phase 7).**
   - Files: `game/swim_effects.rs`, `game/swim.rs`, `world/route/sewer/level2.rs`, `render/waterfall.rs`.
   - Work: physical fall; no `change_storey`; open channel floor and pool air; sheet between true y values.
   - Test: fall time, landing in pool, drop cell and floor height contract.

10. **Hunter and bot crossing (Phase 8, AI side).**
    - Files: `hunters/navigation.rs`, `hunters/brain.rs` (hub), `game/npc.rs`, `game/hunting.rs` (hub), `hunters/mod.rs` (hub), new `hunters/flight_edges.rs`.
    - Work: absolute y and derived level on brains and bots, flight, ladder and drop edges in the path graph, 3D line of sight, remove the NPC freeze.
    - Test: `a_hunter_chases_the_runner_up_a_flight`, `a_bot_follows_across_levels`, determinism.

11. **Remote players and net (Phase 8, network side).**
    - Files: `net/world_sync.rs` (hub), `net/runner_figure.rs`, `net/hunter_events.rs`, `net/wiring.rs`, `game/guest_apply.rs`, `app/mod.rs` (hub, sender only).
    - Work: optional `h` in `Sample`, `remotes_at` within one storey, `RemoteView.h`, draw at height; hunter frames carry `level` and `presentation_y`; backward compatible parse.
    - Test: `remote_on_a_flight_is_drawn_at_its_height`, `old_sample_without_h_still_parses`, round-trip JSON.

12. **Integrator: walk, interact, swap removal, saves, docs (Phases 3, 5, 9).**
    - Files: `game/endless.rs`, `game/mod.rs`, `game/interact.rs`, `game/transition.rs`, `world/level_links.rs` (stairs part; ladder part belongs to task 8), `data/stairs_links.json`, `world/mod.rs`, `config/mod.rs`, `content/mod.rs`, `ui/menus.rs`, save and checkpoint code, `docs/` updates.
    - Work: the only task that edits the hubs listed above. Delete `walked_level_change`, `take_stairs`, `start_stairs`/`take_flight`, `well_stairs` far-end fake, `store_*`/`restore_*` hunters, `respawn_outside_window` level path; switch `change_storey` to respawn, checkpoint and new-run only; derive `inf.level` from y; autosave on label change; save migration for absolute y; fix the stale comments (`level_links.rs:6-7`, `transition.rs` header); update the swap-pinning tests (`endless.rs:2501-2568, 2861-2969, 3026-3027`, `interact.rs:682`, `npc.rs:1334`).
    - Test: end-to-end headless walk of every flight in the golden seeds with y continuity (no jump over 0.25 m per frame), `stairs_cannot_be_used_to_teleport`, grep test that `change_storey` has only respawn, checkpoint and new-run callers, then `cargo test --lib`, `lr_server`, and headless suites.

Ordering: 1 and 2 first (parallel). Then 3 and 4. Then 5. Then 6 and 7 (7 after 3). Then 8, 9, 10, 11 in parallel (after 5 to 7). Task 12 lands the hub changes at each phase boundary, in the order Phase 3, 5, 9, and is the only agent that edits hubs.

---

## Phase 1 as built

- `Stair` carries `shape` and `clockwise`; `run` is the flight's own run on both levels. A `dir < 0` stair is the upper level's landing A, a `dir > 0` stair is the lower level's foot B.
- `Stair::surface_rel(level, x, z)` is the flight's surface relative to the level holding the stair (flight y minus `level * STOREY`), `Some` only on the footprint plus the stair's own end, so swapping levels at the midpoint keeps the absolute y continuous.
- `world/flight_links.rs` is the single place that turns a connection into a `Flight`; both levels call the same pure function, so their footprints cannot disagree.
- Level-0 wells keep Godot's layout boxes: stairs give Switchback, spiral gives Spiral. Straight therefore appears only on the -1/-2 bay links. A spiral falls back to Switchback when its foot is blocked on the sewer side.
- `interact` `take_stairs` still swaps the level (teleport) until phase 5. Pit and shaft closure fakes in the renderer stay until phase 4.
- Contract tests: `world/stack_contract_tests.rs` (landing and foot, footprint clearance, seed 40490470 chunk (3,2) walk). The ladder, catwalk and drop contract tests stay ignored until their phases.

## Phase 5 as built

- A flight has no focus and no prompt. `Focus::Ladder` is for ladders only; `Focus::Stairs` and `take_flight` are gone (D3). The runner just walks the flight.
- `data/stairs_links.json` keeps `prefetch_distance` and the preload keys (`preload_levels_up`, `preload_levels_down`, `preload_max_windows`, `preload_memory_cap_mb`, `preload_builds_per_frame`). `landing_offset` and the `arrival` block are removed.
- The headless runner walks flights like the player does, so the headless suites cover every flight.
- Named tests: `stairs_cannot_be_used_to_teleport` and the end-to-end flight walk with y continuity (no jump over 0.25 m per frame).

## Phase 6 as built

- A climb moves the body in absolute y along the shaft at `LADDER_CLIMB_SPEED`. There is no `change_storey` and no `place_beside_ladder`.
- The step cell (where you step on and off) comes from the `Flight` of kind Ladder: one clear cell per hatch (D4).
- The rails stand on the side opposite the step cell.
- The hatch is a hole in the upper floor, with a propped cover model beside it. The lower level has no ceiling over the shaft.
- `ladder_flight` is the producer of the Ladder `Flight` for both ends, so the two levels cannot disagree.
- Catwalk links use Ladder footprints for cells only; the deck height stays the catwalk model's.
- The ladder contract tests are no longer ignored.

## Phase 7 as built

- Void cells (waterfall drop cells) are open: no floor, no ceiling, so the body falls through them.
- Free fall uses `PLAYER_GRAVITY` 12 (`config/mod.rs`), so one storey (5.0 m) takes 0.913 s. Steering in the air is scaled by `AIR_CONTROL`.
- `Landing` decides the result by what is below: the pool gives the splash and swim, anything else is a hard landing (the pool and ground rule).
- `data/swim/fall.json` holds the fall rule, scaled by drop height (fields and limits in [DATA_FILES.md](DATA_FILES.md)). The game has no hit points, so damage is stun + stamina loss + exposure + noise (KT D7).
- The scripted plunge machinery (`try_begin_plunge`, `advance_plunge`, the input and collision gate) is deleted. `movement.json` `plunge` keeps only the pool-landing numbers.

## Phase 8 as built

Hunters, bots and remote players cross floors on the same geometry as the runner.

- `hunters/flight_edges.rs`: `StackNav` plans over the union of the resident levels' `hunter_open` grids plus flight edges; `EdgeWalk` walks one edge. Flight and Ladder edges are two-way; Drop edges (waterfall, hole) are one-way and off unless `drop_allowed` in `data/hunters/flights.json`. Ladders use `ladder_speed_scale`; a stairs flight is walked at normal speed (D6).
- `game/resident_hunters.rs`: one hunter list per resident floor (D5), capped by `resident_hunter_cap`. Every resident floor keeps simulating.
- Line of sight: `sight_clear_3d` sees through an open well but not through a slab (`see_adjacent`, `hunter_eye_height` in `data/resident_hunters.json`).
- Net: `Sample.h` is optional (absolute y); `depth` is kept for old clients. Hunter frames carry `level` and `presentation_y`.
- `render/stack_actors.rs` draws hunters, bots and remote players at their absolute y.
- Noise: `NoiseEvent` carries a level; a sound one storey away is scaled by `hear_adjacent_scale`.
- Sewer water and valve floods reach hunters (`game/hunter_pipes.rs`).

## Phase 9 as built

The swap machinery is deleted.

- `store_level0_hunters`, `store_floor_hunters` and `restore_level_hunters` are gone. `shelve_active_hunters` and `unshelve_active_hunters` replace them; `stored` and `floor_hunters` are the shelf of non-resident floors only. Resident floors are simulated.
- `relabel_hunters` lost its swap fallback; a recentre fits resident lists back into the window.
- `change_storey` remains for `respawn_on_level` (the admin level jump, which now rebuilds the floor with one label change and one autosave instead of refusing) and the `cfg(test)` `go_to_level`. The label changes at runtime only by `relabel`, which never touches x, y or z.
- `LEVEL_CHANGE_HEIGHT` and the unused vertical constants are gone. `is_jump` is level-blind. `prefetch_distance` keeps its key name; it is the residency radius.
- The `walk_flight` and `teleport` `stairs_down` / `stairs_up` headless commands stay: `tests/*.json` and the bots use them.
- Save format: the checkpoint stores `depth` (the level label) and an optional `y_abs` (absolute feet y). `y_abs` decides the label on resume (`Checkpoint::level`); the body itself resumes standing on the floor of that level (`Checkpoint::migrated_y`), because the save has no x/z and resume starts at the level's start cell. (A y taken mid-flight used to hang the body in the air over the start cell and drop it with a hard landing: fixed with `a_save_made_mid_flight_resumes_on_the_floor_not_in_the_air`.) A legacy save without `y_abs` loads at `level * STOREY`. Autosave fires on the label change only.
- Audit tests (task 5): `change_storey` has only the allowed callers, the retired names are absent, `relabel` and `walk_relabel` never change x, z or y, and one label change autosaves once.
