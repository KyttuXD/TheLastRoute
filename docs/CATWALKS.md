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

## Hunters and bots on decks

Ladders and stairs onto a deck are continuous `Flight` footprints, not a swap. Bots and hunters use the same edges as the runner (`hunters/flight_edges.rs`). Nothing is "taken" and nothing fades.
