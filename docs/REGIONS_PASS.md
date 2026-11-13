# Regions pass: offices, seasonal regions, more props, colour variants (2026-11-12)

KT asked for new regions, so level 0 of the game may now differ from Godot. The Godot draw is kept whole beside it
and is what every parity golden reads. The last pass could not do this: see [PROPS_PASS.md](PROPS_PASS.md).

## What is new

| Thing | Where |
|---|---|
| Seven more regions on level 0: `offices`, `ice`, `stalls`, `inferno0` (switched on), `festive`, `halloween`, `harvest` (new) | [data/world/region_mix.json](../src/bin/last_route/data/world/region_mix.json) |
| 45 prop types, 46 models | [data/props/regions_pass.json](../src/bin/last_route/data/props/regions_pass.json), `assets/props/{maze,mansion,festive,halloween,harvest}/` |
| More props scattered in existing regions (`extra_prop_set`) | `data/regions/<id>.json` |
| Colour variants (`palettes`) for 11 regions | `data/regions/<id>.json` |
| Light, lamp tint and texture set of the new regions | `data/render/ambient.json`, `data/render/lamps.json`, `data/region_textures.json` |

## How a region is chosen, and how parity is kept

- `world/route/regions.rs` `godot_region_for` is Godot's draw, unchanged. `GODOT_NAMES` is the parity set (14 names,
  the first 14 of `NAMES`). Every golden test builds through `layout::with_godot_regions` (`chunk_layout_godot`),
  which reads only that draw.
- The game's `region_for` asks `extra_region_for` first, on level 0 only. It has its own seed text
  (`<world seed>:EXTRA-REGION:<block x>:<block z>`): one float against `extra_share`, then one weighted pick. It takes
  nothing from Godot's random streams. The spawn block is never an extra region.
- A block that is not extra keeps Godot's draw, so the Godot regions keep their weights against each other. With
  `extra_share` 0.24 each Godot region appears 0.76 times as often as before.
- The draw depends on the world seed only. Host and guests get the same world.

| Region | Weight | Share of all level 0 blocks |
|---|---|---|
| offices | 6 | 9.6% |
| ice | 3 | 4.8% |
| stalls | 2 | 3.2% |
| inferno0 | 1 | 1.6% |
| festive | 1 | 1.6% |
| halloween | 1 | 1.6% |
| harvest | 1 | 1.6% |
| tunnels | 0 | off: its corridors are carved by the concept map, which is switched off |
| utility | 0 | off: the region is still undecided (`"undecided": true`) |

To tune: change a `weight`, or `extra_share` for all of them. `extra_share` 0 gives Godot's level 0 again.
To see a seasonal region often while testing, set its weight to 20.

**No month setting.** A boost by calendar month would have to come from the local clock, and the host and a guest
can be in different months (or have wrong clocks), which would build two different worlds from one seed. A safe
version needs the host to send its season with the room seed (`net/`), which this pass did not own. Until then it
is a data weight.

## Generator changes outside the region list (all behind the parity switch)

| File | Change | Why parity holds |
|---|---|---|
| `world/route/layout.rs` `is_dense` | the extra regions are laid out as banks | Godot's draw never returns an extra region |
| `world/route/props.rs` `prop_set` | appends `extra_prop_set` | skipped when `godot_regions()` is on |
| `world/route/furnished.rs` `data_only_size` | a recipe prop with no Godot model key and no row in the Godot size tables takes its size from `data/region_rooms/<id>.json` `props` | returns `None` when `godot_regions()` is on, and for every Godot type |
| `render/textures/sets.rs` `finite_district` | a region with no district of its own takes the set named in `data/region_textures.json` | only regions that had no district change (they were drawn as the maze) |

Before this, a region that was neither dense nor the apartment had no layout at all, which is why the six
map-concept regions could not appear without the concept map.

## Regions

Each seasonal region has four data rooms (`data/region_rooms/<id>.json`) and three palettes.

| Region | Rooms | Own props | Looks like |
|---|---|---|---|
| `festive` | tree_parlour, gift_workshop, hearth_lounge, snow_court | christmasTree, wrappedPresents, festiveWreath, snowman, candyCanePost, stockingMantel, festiveSleigh, nutcracker, stringLightPost | hotel set, warm light |
| `halloween` | pumpkin_patch, graveyard_room, witch_kitchen, haunted_parlour | jackOLantern, witchCauldron, coffin, tombstone, scarecrow, cobwebCorner, skeletonProp, witchBroomRack, candleCluster | gothic set, dim orange light |
| `harvest` | hay_barn, feast_hall, harvest_market, corn_store | hayBale, cornucopiaTable, pumpkinPile, cornStalkBundle, harvestFeastTable, pieStand, turkeyCentrepiece, leafPile, woodenCart | mansion set, amber light |
| `offices` | the nine it had, plus server_room and training_room | officeWhiteboard, officeCoffeeStation (new); serverRack, cableCoil (existing) | lab set, grey blue |

The office already had desks, chairs, cubicle partitions, printers, a photocopier, a water cooler, filing cabinets
and a break room; it was only never placed.

## Props in existing regions

`extra_prop_set` in a region file: the game scatters these as well as the Godot set; a room with a furnished
template is not scattered.

| Region | Added |
|---|---|
| mansion | New: mansionPortraitFrame (two paintings), mansionCandelabra, mansionArmoire, pianoStool, taxidermyPedestal, suitOfArmour, mansionBookshelfTall, mansionBookshelfLow. Existing types that no region placed: mansionFireplace (now lit), mansionGrandfatherClock, mansionChaiseLounge, mansionUprightPiano, mansionBustPedestal, mansionConsoleTable, mansionDisplayCabinet, mansionCoatRack, mansionTeaCart, mansionRockingChair, diningChandelier |
| maze | cardboardBoxes, floorPuddle, fallenCeilingTile, wetFloorSign, bucketAndMop |
| offices | papersStack, cardboardBoxes, wetFloorSign, cableCoil |
| hotel, apartment, hospital, mall, lab, prison, stalls, ice, utility | two or three clutter types each (papersStack, brokenSeat, trafficCone, and the ones above) |

Clutter that lies flat (papersStack, floorPuddle, fallenCeilingTile, leafPile, cableCoil) is not solid. A prop with
no wall mount stands on the floor: the portrait, the wreath, the whiteboard and the cobweb are on stands or frames,
because wall-mounted types are a code list (`prop_tables.rs` `WALL_MOUNTED_TYPES`).

A new type's name must not hold `chair`, `couch`, `bench`, `stool`, `sofa`, `bunk`, `mirror` or end in `bed`,
unless it is one: `furnished.rs` `interaction_for` reads the name. `pianoStool` is a seat on purpose.

## Models

`tools/props/props_pass_models.py` (second half) writes the 46 models: cubes only, 60 to 404 triangles, the same
256 x 8 palette picture as the props pass, so one texture and one batch. Sizes are in metres, drawn at scale 1; the
region files hold each type's box. The script refuses to write over a model it did not make. The files add about
0.42 MB to the executable; a model is parsed when a room first draws it.

Lights (`data/props/*.json` `light`): christmasTree, stockingMantel, jackOLantern, witchCauldron are glows
(`"glow": true`, they share the four glow places of a floor); candleCluster, stringLightPost, mansionCandelabra and
mansionFireplace are lamps. Energy 0.3 to 0.5.

`GOLDEN_ALL` in `render/bbmodel.rs` must be re-recorded with `print_golden_digests`: the count goes from 704 to 750.

## Colour variants

`palettes` in a region file: `name`, `weight`, `textures` (a texture set), `wall`, `trim`, `floor`, `lamp` tints and
`ambient`. `regions::palette_at(world, cx, cz, level)` returns the palette of the chunk's region block, by a stable
hash of the seed and the block (`packs/regions.rs` `stable_hash`), so a whole patch has one palette and every player
sees the same one. Regions with palettes: offices (grey_blue, beige, green), mansion, maze, hotel, apartment,
hospital, mall, lab, festive, halloween, harvest.

The data and the pick are in. Drawing them needs the hooks below; until then a region shows its base look
(`ambient.json` `region.<id>`, `lamps.json` `room_tint`, `region_textures.json`).

## Hooks in files this pass did not own

1. `render/world_mesh.rs`, where a chunk's `TextureChoice` is made (`textures::region_set(&key.0, &key.1, key.2)`):
   take `regions::palette_at(seed, cx, cz, 0)` for the chunk; when it has `textures`, pass that name as the region to
   `region_set`; multiply the wall, floor and trim vertex tint by `wall`, `floor`, `trim` (bytes / 255).
2. `render/world_mesh.rs` `lamps()`: multiply a lamp's colour by the palette's `lamp` after `room_tint`.
3. `render/scene.rs` `lighting()`: when the runner's chunk has a palette with `ambient`, use it for `ambient_color`.
4. `content/props.rs` `ceiling_top` and `furnished.rs` `is_ceiling_mounted` are code lists, so no new ceiling prop was
   added; the mansion scatters the existing `diningChandelier`.
5. `render/prop_model.rs` `prop_model_key`: add the prop id to the key for a pick per prop, so two portraits in one
   room can differ (the same follow-up as the props pass).

## Tests

`world/route/regions.rs` (the parity set is Godot's list, the Godot draw stays inside it, the extra draw is
repeatable and follows the weights, the shipped mix, palettes per patch), `packs/regions.rs` (palettes and
`extra_prop_set` parse, bad files name the key), `content/regions.rs` (every region prop names a type whose models
parse, one size per type, the 46 models, the seasonal sets and lights), `world/route/furnished.rs` (the seasonal
rooms are data rooms; data-only types never reach the parity layout).

Not run in this pass (edit agents do not build). Two things to watch in the first test run: tests that sample live
level 0 chunks of a fixed seed without `godot_regions_for_this_test` may now land in an extra region, and the
frame-time check of the stress run is still to do (nothing was added to the frame loop; `region_for` does one more
seeded draw per call, at chunk build).
