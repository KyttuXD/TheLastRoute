# Hall lights: fairy-light strings and wall lamps

Halls above the sewers carry two kinds of light fitting that are not props: strings of fairy lights hung across the
hall, and small wall lamps (sconces). The code is `src/bin/last_route/render/hall_lights.rs` (placement and shapes,
pure and unit-tested) and `render/world_mesh.rs` (`add_hall_lights` for the mesh, `window_hall_lights` and
`lamps_with` for the lights). Every number is in `src/bin/last_route/data/render/hall_lights.json`
(`lr.hall_lights/1`). See also [DATA_FILES.md](DATA_FILES.md) and [PERFORMANCE.md](PERFORMANCE.md).

## What is drawn

- **Fairy lights.** Godot hangs short strands (0.7 to 1.2 m, three to five bulbs) under the ceiling of the maze and
  the carnival (`game/world/streaming/ceiling_decor.gd`). Here a string runs the whole width of its hall, from one
  wall face to the other, whatever the width (up to `max_width_cells`), sagging in the middle. A hall is cut into
  stretches along its length; a strung stretch gets a string every `string_every_cells` cells. The bulbs use Godot's
  five colours and are drawn unlit, so they glow in the dark; the wire is thin and dark.
- **Wall lamps.** A dark back plate with a glowing shade on a hall wall, between the pilasters. They are not in Godot.

Neither is solid, neither is in a room, and nothing hangs where a cell has no ceiling. The sewers, the Inferno and the
finite floor get none.

## How they are placed

By a hash of the floor's seed text, the chunk's place in the world and the cell. The route generator's random
numbers are not used, so prop placement (checked against the Godot golden files) does not change. A chunk's lights
depend only on that chunk's own grid, so they are the same in every window that holds the chunk.

## Cost

- **Mesh:** built once with the chunk part that owns the hall cell, into the part's plain mesh. A string is 4 wire
  quads and 2 quads a bulb: 28 triangles in a hall 3 cells wide, at most 48 in one 6 wide. A wall lamp is 2 boxes
  (24 triangles). Nothing is built, sorted or chosen per frame.
- **Lights:** every second string and every wall lamp gives one light. They stand in the floor's lit-prop queue
  (`data/render/lamps.json`: `prop_budget`, 10 on level 0): a string's light as a glow (at most `prop_glow_budget`,
  shared with the screens), a wall lamp's as a lamp, the nearest to the runner first. No place is added to the budget
  and the shader's lamp count is unchanged. A window's list of hall lights is made once per window and kept
  (`window_hall_lights`), so choosing the lamps again as the runner walks does not walk the floor.

## The data file

`chance` blocks hold `default` and one number per region id, from 0 to 1; 0 turns a region off.

| Key | Meaning |
|---|---|
| `fairy.chance` | Share of hall stretches that are strung. Ships as the maze 0.16, the carnival 0.35, everything else 0 (Godot's two regions). |
| `fairy.stretch_cells` | Length of a stretch, cells. A stretch is strung whole or not at all. |
| `fairy.string_every_cells`, `fairy.string_phase` | A string hangs on each cell where (cell along the hall) mod every == phase. 3 and 1 never fall on a hall arch. |
| `fairy.max_width_cells` | The widest hall that is strung (1 to 12). |
| `fairy.drop_m` | How far under the ceiling the string's ends are fixed. |
| `fairy.sag_per_m`, `fairy.sag_max_m` | Sag in the middle per metre of span, and its cap. |
| `fairy.wire_thickness_m`, `fairy.wire_segments`, `fairy.wire_color` | The wire: its height, how many straight pieces draw the curve, its colour. |
| `fairy.bulb_spacing_m`, `fairy.bulb_size_m`, `fairy.bulb_colors` | A bulb every so many metres (at least 0.2), its size, and the colours it is picked from. |
| `fairy.light_every_strings`, `fairy.light` | Which strings carry a light, and the light: `color`, `energy`, `range`, `flicker_hz`, `flicker_depth`. |
| `sconce.chance` | Share of candidate wall faces that get a lamp. |
| `sconce.every_cells`, `sconce.phase` | The candidate faces: (cell along the wall) mod every == phase. 6 and 0 sit between the pilasters. |
| `sconce.height_m` | The lamp's middle above the floor (lower under a low ceiling). |
| `sconce.plate_m`, `sconce.shade_m` | Width along the wall, height and depth of the back plate and of the shade. |
| `sconce.plate_color`, `sconce.shade_color` | Their colours; the shade is drawn unlit. |
| `sconce.light_out_m`, `sconce.light` | How far from the wall the light stands, and the light. |

A bad file is reported on stderr and the built-in copy is used.
