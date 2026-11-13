# Props pass: arcade, carnival, sizes and playable booths (2026-11-12)

What this pass added, where it lives, and what it could not do. Nothing here changes which props the level-0
generator places, their order or its random draws: `world/route/**` and the region `prop_set` lists are untouched.

## How new things reach the game without moving the goldens

| Way | Used for |
|---|---|
| More model variants on an existing prop type (`data/props/*.json` `models`) | New cabinets, claw machines, booths. The generator never reads the model list of these types, so the placement is the same; only the drawing differs. |
| A new interaction on an existing prop type | The playable machines and booths ([data/amusements.json](../src/bin/last_route/data/amusements.json)). |
| A draw-only stretch per axis | The wrongly sized couches and chairs ([data/render/prop_axis_scale.json](../src/bin/last_route/data/render/prop_axis_scale.json)). |

A variant is picked by `render/prop_model.rs` `pick_variant`: the stable 31-hash of the room's variant key
(`render/world_mesh.rs` `variant_key`), so every cabinet of one room shows the same variant and rooms differ.
A pick per prop instead of per room is one line in `prop_model_key` (add the prop id to the key); that file was
another agent's in this pass, so it is left as a follow-up.

## New models (20)

All are written by `tools/props/props_pass_models.py` (cubes only, at most 520 triangles, faces that can never be
seen left untextured). They share one 256 x 8 palette picture, so the loader makes one texture and one batch for
all of them. Each fits inside the model it stands beside, so no collider changes.

| Prop type | New variants |
|---|---|
| `arcadeCabinet` (was 5) | `arcade_cabinet_lime` (invaders), `_sunset` (falling blocks), `_mono` (pong, trackball), `_crimson` (maze), `_racer` (road, wheel, shifter, pedals), `_blaster` (crosshair, two light guns) |
| `arcadeCabinetCyanMagenta` (was 1) | `arcade_cabinet_magenta_gold` (hearts), `arcade_cabinet_cyan_lime` (level bars, trackball) |
| `prizeClawMachine` (was 1) | `prize_claw_machine_pink`, `prize_claw_machine_blue` (open case, claw, plush prizes) |
| `arcadeTokenChanger` (was 1) | `arcade_ticket_redeemer` |
| `prizeCounter` (was 1) | `prize_plush_shelf` |
| `carnivalTicketBooth` | `carnival_ticket_booth_blue` |
| `popcornCart` | `cotton_candy_cart` |
| `ringTossStall` | `balloon_dart_stall` |
| `strengthTester` | `strength_tester_blue` (high striker) |
| `circusPedestal` | `circus_pedestal_blue` |
| `shootingGallery` | its own `shooting_gallery` (was the ring toss model) |
| `whackAMole` | its own `whack_a_mole` (was the ring toss model) |
| `duckPond` | its own `duck_pond` (was the circus pedestal model) |

The embedded files add about 0.43 MB to the executable; a model is parsed only when a room first draws it.

## Playable machines and booths

`game/instruments/amusements.rs`, data in `data/amusements.json` (`lr.amusements/1`). Interact plays the press cue
and makes the runner's noise burst (hunters hear it); after `result_seconds` a win or lose cue plays with a line
of text. The win roll is a hash of the prop id and the press count, so no run random stream is used. Sounds are
the existing `carnival_*` cues. Local only: nothing is sent to other players.

| Prop type | Prompt | Result |
|---|---|---|
| `arcadeCabinet`, `arcadeCabinetCyanMagenta` | PLAY A ROUND | HIGH SCORE / GAME OVER |
| `prizeClawMachine` | DROP THE CLAW | grip (1 coin, once per machine) / IT SLIPPED |
| `arcadeTokenChanger` | PULL THE LEVER | a token (1 coin, once per machine) / EMPTY |
| `arcadeBasketballHoop` | SHOOT A HOOP | SWISH / MISSED |
| `prizeCounter` | RING THE BELL | - |
| `strengthTester` | SWING THE MALLET | the bell / a thud |
| `whackAMole` | WHACK A MOLE | GOT ONE / TOO SLOW |
| `duckPond` | HOOK A DUCK | splash, then HOOKED ONE / IT SWAM OFF |
| `shootingGallery` | TAKE A SHOT | clang / MISSED |
| `ringTossStall` | PLAY THE STALL | A WINNER / NO LUCK |
| `popcornCart` | TURN THE CRANK | - |
| `carnivalTicketBooth` | RING FOR A TICKET | NOBODY HOME |
| `carnivalCarousel`, `carouselCenterpiece` | WIND THE CAROUSEL | the carousel tune (12 s cooldown) |

A run pays at most `prize.max_per_run` (5) coins this way. The coin goes through the same counter as a floor coin.

Not done: the screen light does not flicker on a press. The prop lights are built in `render/world_mesh.rs` and
`render/lamp_flicker.rs`, which this pass did not own.

## Sizes

`data/render/prop_axis_scale.json` (`lr.prop_axis_scale/1`), read by `render/prop_scale.rs` and baked into the
model once when it loads (`render/prop_model.rs`), so the draw loop does no extra work. Colliders, footprints and
the generator's seat heights (which read `prop_draw_scale.json`) are unchanged.

| Type | x, y, z | Why |
|---|---|---|
| `loungeCouch` | 1.75, 1.0, 1.55 | drawn 1.53 m wide in a 2.67 m collider |
| `loungeCouchWide` | 1.35, 1.0, 1.65 | drawn 2.73 m wide in a 3.70 m collider |
| `beachChair` | 1.0, 0.6, 2.0 | drawn 1.95 m tall and 0.77 m deep; the collider is a 1.13 m tall, 1.53 m long lounger |
| `gothicHighbackChair` | 0.75, 0.62, 1.0 | drawn 3.17 m tall; the collider is 1.97 m |

Cause: Godot draws its blocky model for these types (its `ORIGINAL_DENY` list) and derives the collider from it;
the port draws the converted original.

## Audit of all 339 prop types

- No model: none. Every type names at least one model and every named file exists.
- Shared model, fixed: `shootingGallery`, `whackAMole`, `duckPond`.
- Shared model, intended: `sewerLamp`, `sewerEmergencyRed`, `sewerEmergencyAmber` (they differ by light).
- Drawn size against the collider, still to look at in a windowed run (not changed here): `restaurantPartyTable`
  (2.2 m tall, collider 0.85), `prisonServingCounter` (3.0 m, 1.65), `concessionCounter` (2.5 m, 1.5),
  `bathroomVanity` (2.5 m with its mirror, 1.4), `diningChandelier` (0.7 m tall, 1.5), `hangingVines` (0.6 m wide,
  1.2), `surgicalLight` (0.4 m deep, 1.5), and the wall pieces drawn about 1.5 times their box (`studioMonitors`,
  `acousticPanel`, `onAirSign`, `diveHeadsetShelf`). `beachUmbrella` and `carouselCenterpiece` overhang a small
  collider on purpose (a pole, a hub).

## Regions: not possible in this pass

An arcade-hall or carnival-midway region, or a re-dressing of the carnival, cannot be added by data alone:

1. A region id must be in `NAMES` and in the region draw of `world/route/regions.rs` (golden-checked), and its
   file in `BUILTIN_FILES` of `src/packs/regions.rs`; a test refuses a region file the placement does not know.
2. A region's `prop_set` is the list the generator's draw indexes, in file order, so adding a type to
   `data/regions/carnival.json` or `maze.json` changes the seed's output.
3. Room types (`arcade_room`, the carnival templates) are code in `world/route/furnished.rs` and `layout.rs`.
4. The one Rust-only prop layer, `world/sewer_emergency.rs`, is wired into the sewer build in `world/infinite.rs`.
   A level-0 layer like it (new booth types placed after the generator) is possible but needs that file, door
   and walkway clearance, and the hunters' grid; it was outside this pass.

So the arcade room, the maze and the carnival get the new look through the variants above.

## Tests

`render/prop_model.rs` (every variant is reached, the new models' shape, size, triangles and shared picture, the
stretch), `render/prop_scale.rs` (the axis table), `game/instruments/amusements.rs` (the data, every cue exists,
the roll, the prompt, the delayed result, the prize caps).

`GOLDEN_ALL` in `render/bbmodel.rs` counts and hashes every embedded model: it must be re-recorded with
`print_golden_digests` (the count goes from 684 to 704).
