# Catwalks and deck heights

The Godot build lets a hall have raised catwalk decks over part of the maze (the Mezzanine, level +1). This note records
the geometry read from the Godot source and what the Rust port has so far.

## Godot geometry

Read from `game/world/generation/` in the Godot project (`LastRoute Godot/KyttusLabApp`, read only):

- **Cells are 1 m** (`CELL_SIZE := 1.0` in the facility views).
- **Chunks are 45 x 45 cells** (`StreamedRouteGenerator.CHUNK_SIZE`). Every upper level carries a shared three-wide
  cross on rows and columns 21 to 23 (`UpperLevelTools`, `CENTER` 22, `CROSS_HALF` 1), so seams meet.
- **Storeys are 4.0 m** (`UpperLevelTools.LEVEL_HEIGHT`). Level L may not rise above `(L + 1) * 4.0 - 0.1` (the 0.1 m
  `SLAB`). Level +1's floor therefore stands at 4.0 m over level 0.
- **Catwalk grating and solid decks are both walkable.** `GRATE` is see-through catwalk and `DECK` a solid floor
  (`MezzanineRouteGenerator`). `PIT` is the drawn floor at the bottom of the void and is never used for navigation.
- **Balconies are 7 x 7 decks** (`BALCONY_HALF` 3) hung off the cross. Railings are 1.1 m (`RAIL_HEIGHT`) on every edge
  that drops to open air or the pit.
- **Where decks sit:** `DeckPatches` picks deck chunks with smooth value noise (`SCALE` 3.0 chunks, `THRESHOLD` 0.575),
  so patches are 2 to 6 chunks across and about 40% of chunks away from spawn are decks. The spawn chunk and its ring
  (`SPAWN_CLEAR` 1) stay under a ceiling. Level +1 exists only over a deck patch or the Mezzanine void.
- **Way up:** each patch has at least one spiral well from level 0 (`StreamedRouteGenerator._plan_maze_stairwell`).
- **Status in Godot:** the Mezzanine is removed from play (`MEZZANINE_ENABLED` is false, KT 118), so no +1 chunk is
  generated there today. The geometry above is kept for the port.

## What the Rust port has (step 1)

In `src/bin/last_route/world/facility.rs`:

- `DeckKind` (`Floor`, `Catwalk`) and `WalkSurface` (kind plus `height` in metres). The default is `Floor` at 0.0.
- `Facility` keeps a sparse `walk_surface` map, the same style as `door_state`. A cell with no entry is `Floor` at 0.0 m.
- Accessors `walk_surface_at(cell)` and `height_at(cell)`, and the setter `set_walk_surface(cell, kind, height)`, which
  changes that one cell only.
- Nothing sets a catwalk yet. Generation, movement, collision and rendering are unchanged.

## Still to do

- **Generator:** port `DeckPatches` and the cross and balcony layout so deck cells call `set_walk_surface` with the
  Catwalk kind and the storey height.
- **Movement:** onto a deck only by ladder or stairs (`LevelLink` ends), never by walking off the floor.
- **Collision:** use the cell height, so a ground-level body cannot enter a raised deck cell and a deck body cannot
  walk off a deck cell into a lower one.
- **Rendering:** taller halls, the railed deck edges and the catwalk grating.

Related code: [facility.rs](../src/bin/last_route/world/facility.rs).

## Ladders to a deck (continuous levels, phase 6)

Catwalk links use the Ladder `Flight` footprint for cells only: the step cell and the rail side follow the same rule as sewer ladders (see [SEWERS.md](SEWERS.md)). The deck height stays the catwalk's `walk_surface` height. See [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md).

## Deck height in the Rust port

The Rust decks stand 2.5 m over level 0 (`data/regions/catwalk.json` `height`), half our 5 m storey. A deck must stay on
level 0: `level_from_height` flips at the storey midpoint (2.5 m) plus a margin, so a deck at 4 m would read as level 1 and
swap a deck body into another hunter list. The test `deck_height_stays_level_zero` (`hunters/brain.rs`) holds
`level_at(height + 0.1, 0) == 0`.

## Hunters and bots on decks

Ladders and stairs onto a deck are continuous `Flight` footprints, not a swap. Bots and hunters use the same edges as the runner (`hunters/flight_edges.rs`). Nothing is "taken" and nothing fades.

## Climbing to a deck (2026-11-04)

- Every deck link is a ladder the runner climbs by hand (`game/ladder.rs`, `Climb::catwalk`): walk into the deck's front face from the link's ground cell to climb up, or into it from the deck landing to climb down. The shaft spans the deck's height (`deck_height`), not a storey; the top lands on the deck cell and the bottom back on the ground cell, with no jump or fade. `ladder_share` is 1.0 because a link has only two cells of footprint, too small for a stair of that rise.
- Decks are 2.5 m up under the 5 m ceiling (`catwalk.json` `height`, `catwalk_links.json` `deck_height`): 2.5 m of headroom, so the head clears the ceiling. A deck must stay under the level-label midpoint (`world/vertical.rs`, about 2.94 m).
- A body on a deck keeps the deck's absolute y in `height` (like a tread), so the collision and the ladder agree on `y()`; before, the deck left `height` at 0 and the body fell or was refused.
- The deck's rail is left open on the side facing the ladder; the rails and rungs are drawn on the deck's front face (`render/world_mesh.rs`, `catwalk_ladder_boxes`).

Rust design, with no Godot parity: Godot's hunters are 2D and cannot reach a deck at all (its catwalk was a separate level
+1, and the Mezzanine is off). The rules are data, in `data/hunters/decks.json` and `data/hunters/flights.json`.

- **Sight is 2D.** `Facility::has_line_of_sight` tests walls only, which is exact between a deck and the floor: the deck is
  a see-through grating with 1.1 m rails and the walls are 5 m. A floor hunter sees a deck runner and a deck hunter sees
  the floor. What differs is reach: a target on another layer is
  never dashed at (the layer is the walk surface under the target's cell against the hunter's own, so a runner part-way up
  a flight or ladder, or in a fall, over a floor cell is still on the floor layer; `same_layer_eps` = 0.5 m now only
  gates the jump permit and the director), and capture needs the same height (`capture_height` 1.2 m).
- **Layers.** A cell has one walk height, so (level, cell) names a node. `hunter_open` stays ground only (spawns, wander
  and search picks and the director read it); a separate `Facility::deck_open` marks the deck cells a body stands on at the
  deck's height, `nav_open` is either. A grid step never changes height; the only ways across are two edges, built by
  `StackNav` on the same level:
  - `CatwalkLadder`: two-way, at each catwalk link (the ground foot cell to the deck cell it runs onto), climbed at the
    ladder speed. A stairs link is walked the same way (a climb at ladder speed; the hunters do not use its treads).
  - `CatwalkJump`: one-way, from each deck cell beside open ground down to that ground cell (never the ladder's foot): a
    ballistic fall (`jump_gravity`, `jump_arc_reach_m`, `jump_arc_samples`), cost = length plus `jump_extra_cost`, so a
    jump wins only when the ladder is about 4 cells further. No jump from a deck taller than `jump_max_drop_m`.
- **Who may.** `decks.json` rows: `can_climb`, `can_jump_down`, `landing_stun_seconds` (0), `fall_damage` ("none"). By
  default every hunter climbs and may jump down; nyxie, solyth, ysolde, void_blue and void_green are ground-only until
  their gimmicks are deck-aware. A hunter jumps only while chasing or investigating a goal lower than itself; wandering or
  searching it takes the ladder. A hunter that has no edge to a goal on another layer (cannot climb, ladder gone) walks to
  the nearest spot of its own layer and watches from there instead of looping Chase, give up, Search.
- **Footing.** A deck hunter's footprint test is made at its own height (`body_fits_at_y`), so it cannot step off a rail;
  peeking is off while on a deck; `fit_into_window` and the separation push read the hunter's height too.
- **The landing.** `Brain::landed` is set once on the frame a jump ends. The host makes a `hunter_landing` noise event
  (`noise_events.json`): bots hear it and the runner hears a thud; other hunters do not.
- **Bots.** `StackNav::find_path` and `next_hop` give a bot no catwalk edge (`EdgePermit::NONE`), so bots do not use decks
  yet.
- **Remote players** are not hunter targets (the host's hunters target the host runner and the bots only).
