# render/textures: port of the original's painted surfaces

Source (read-only original): `game/look/ref_textures.gd` (`RefTextures`), chosen by `game/look/ref_look.gd`,
`game/look/floor_sets.gd`, `game/look/theme_data.gd`, `data/themes/regions/*.json`, `data/themes/rooms/*.json`,
`data/regions/{hospital,mall}.json`, and drawn by `game/look/ref_surface_mesh.gd` +
`game/world/streaming/streamed_facility_view_3d.gd`.

Not wired in yet: add `pub mod textures;` to `src/render/mod.rs` (this folder was written without touching other files).
Only `std` is used; no macroquad types.

## Files

| file | contents |
|---|---|
| `mod.rs` | painter tables (original order = texture-array layer order), `paint`, `paint_or_first`, `kinds`, `variant_of`, `wall_tiers`, `wall_v`, `surface_params`, `ARRAY_TINT` |
| `rng.rs` | Godot `RandomNumberGenerator` (PCG32) and `String::hash()` |
| `canvas.rs` | Godot RGBA8 `Image` / `Color` semantics, brushes (`_dot`, `_noise`, `_rivet`, `_crack`, `_mosaic`, `_flower`, `_star`, `_leaf_cluster`), wall bands (`_wall_base`, `_cornice`, `_skirt`, `_rail`, `_grime`, `_sea_cornice`, `_sea_skirt`), `rotate_90(COUNTERCLOCKWISE)` |
| `floors.rs` | metal, floral, star, parquet, stone, check, carpet (red/teal/green/plum), carpet_tile, vinyl, concrete, hazard, terrazzo, epoxy, terracotta, runner, runner parts (l/c/r, z-rotated), runnerz, safe / safe_plain / safe_mark, grate, rock floor |
| `walls.rs` | panel, wallpaper, brick, stone, hedge, office, hospital, lab, glasshouse, mall, corridor, inferno, rockwall |
| `ceilings.rs` | ceil (coffered), vault, glass, rockceil |
| `beach.rs` | sand / wetsand, driftwood, seawall, whitewash, tealpanel, skylight |
| `ducts.rs` | duct sheet, flange, grille (group `"duct"`) |
| `sets.rs` | district / purpose tables, `pick`, `region_set`, `TextureChoice`, `runner_kind`, `safe_pad_kind`, finite-route mapping |

## Exactness

All 99 painted kinds are **byte-identical** to Godot 4.7.2's. To check: a scratch Godot project (in %TEMP%) held copies
of `ref_textures.gd`, `material_look.gd` and `ref_surface.gdshader`, plus stubs (`ContentRegistry.pack_defs -> []`,
`watch -> true`; `RefLook` with only METAL/STONE/WOOD; no-op `RefDecor.cheap_dark`, `ShaderLights`). A `SceneTree`
script called `RefTextures._paint(group, kind, entry)` for every table entry (and the duct painters) and wrote
`img.get_data()` to `<group>_<kind>.rgba`. The test `textures::tests::matches_godot_dump` compares against such a
folder when `LR_TEX_DUMP` is set (it does nothing otherwise). `rng::tests::matches_godot` holds Godot's printed values
for `hash()`, `randi`, `randf`, `randi_range`, `randf_range`.

The Godot behaviour the port relies on (all confirmed by the dumps):
- `hash(String)` = `String::hash()`: djb2 over UTF-32 code points, `h = h * 33 + c`, start 5381, u32 wrapping.
  (Only ASCII keys are used; code points above U+FFFF were not tested.)
- `rng.seed = s`: `pcg32_srandom_r(state = s, initseq = 1442695040888963407)` (the default increment is kept).
- `randi()` = `pcg32_random_r`. `randi_range(a, b)` = `pcg32_boundedrand_r(|a - b| + 1) + min(a, b)` (rejection below `-bound % bound`).
- `randf()` = Godot's float32 `randf`: two draws, `ldexpf((float)(r2 | 0x80000001), -32 - clz(r1))`, 0 if r1 == 0.
  It is compared against double literals (`randf() < .35`), so the port widens it to f64 before comparing.
- `randf_range(a, b)` is computed in float32 (`real_t`): `randf() * (b - a) + a`.
- `Color("rrggbb")` channels are `n / 255.0f` (float32). `_mul` multiplies in double, clamps 0..1, stores float32.
- `Image.set_pixel` on RGBA8 **truncates**: `uint8_t(CLAMP(v * 255.0, 0, 255))` (rounding gives different bytes).
- `Vector2(x, y).length()` is float32; `cos`, `sin`, `atan2`, `fmod`, `pow` are double; `roundi` rounds half away from 0.
- GDScript evaluates call arguments left to right, so RNG draws inside one `_dot(...)` happen in argument order (Rust does the same).

Not ported: content-pack replacement images (`_pack_images`: a pack's PNG resized to 32x32 / 32x128). Mipmaps
(`generate_mipmaps`) are not produced; the original samples with `NEAREST_WITH_MIPMAPS_ANISOTROPIC`.

## API

```rust
pub fn paint(group: &str, kind: &str, variant: i32) -> Option<(u16, u16, Vec<u8>)>
```
`group`: `"floor"`, `"wall"`, `"ceiling"`, `"duct"`. `kind`: a full key (`"metal3"`, `"paper0"`, `"carpet_red"`,
`"runner_l"`, `"wetsand0"`; `variant` is then ignored) or a family prefix that `variant` completes (`"paper"` + 0).
Returns RGBA8, row-major from the top-left texel; alpha is always 255. `paint_or_first` gives the original's fallback
(an unknown kind paints the table's first kind). Painting is cheap (microseconds), but cache the results anyway.

```rust
pub fn region_set(region: &str, room_type: &str, is_hall: bool) -> TextureChoice
pub struct TexRef { group, kind, variant }          // TexRef::paint()
pub struct TextureChoice { district, purpose, wall, floor, ceiling: Option<TexRef>,
                           wall_kinds, floor_kinds, per_cell, runner, is_hall }
impl TextureChoice { fn wall_at(gx, gz) -> TexRef; fn floor_at(x, z) -> TexRef }
pub fn pick(cell_x: i64, cell_z: i64, salt: i64, count: usize) -> usize   // RefLook.pick
pub fn runner_kind(open: impl Fn(i32, i32) -> bool, x: i32, z: i32, width: i32) -> Option<&'static str>
pub fn safe_pad_kind(dx: i32, dz: i32) -> Option<&'static str>
```

## How the original picks (the infinite route: StreamedFacilityView3D)

- **District style** = `RefLook.REGION_STYLE[region]`, falling back to `"maze"` for an unknown id (also for "office"
  and "industrial", whose themes only set an accent). Wall set and ceiling come from `data/themes/regions/<id>.json`.
- **Wall kind** of a face (`RefLook.wall_kind`) comes from the space the face looks into. A room uses its purpose's
  `data/themes/rooms/<purpose>.json` wall list if there is one, else the district's. A hall (room_type "") uses the
  district's. The index is `pick(gx / 4, gz / 4, 2, n)` (4 x 4 cell patches, GDScript integer division truncating
  toward 0), or `pick(gx, gz, 2, n)` per cell in the inferno.
- **Room floor**: `FloorSets.room_kind` = `ROOMS[purpose]`, else `ROOM_DEFAULT[district]`, one kind per room:
  `pick(room_origin_x, room_origin_z, 7, n)` in world cells.
- **Hall floor**: one kind per 45-cell chunk, `pick(chunk_x, chunk_z, 1, n)` from `HALLS[district]`, unless the
  district has `runner` (mansion, apartment, gothic) and the cell is on the runner (below). Doorway cells and hall
  junctions take the hall floor; a room's floor starts inside the door line.
- **Inferno**: every cell (rooms included) is cave rock, `"rock" + posmod(pick(gx, gz, 7, 3), 3)`. Only the chain
  bridge and the ramp walkways are `"hazard"`. `region_set` returns `INFERNO_ROCK` with `per_cell = true`.
- **Ceiling** = the district's ceiling kind, for rooms and halls alike. It is sampled from the floor array (32x32).
- Sewer crawl cells use "hazard", bridge decks "grate", stair wells "concrete1". These are infinite-route features.

## Finite route mapping (this port's choice)

In the original, the finite route (`main_3d.tscn` -> `FacilityView3D`) **does not use RefTextures**. It draws one
baked floor PNG and one wall PNG (`assets/environment/{floor,wall}_texture.png`) on GridMap boxes. Only custom maps
tint them per region (`REGION_FLOOR_TINTS` / `REGION_WALL_TINTS`), with walls repeated `WALL_HEIGHT` (3.6) times
vertically. So `region_set` maps finite regions and room types onto the original's districts and purposes
(`FINITE_DISTRICT`, `FINITE_PURPOSE` in `sets.rs`), then applies the rules above unchanged:

| finite region | district | | finite region | district |
|---|---|---|---|---|
| "" (halls, lobby) | maze | | mansion, bedroom | mansion |
| arcade, circus | carnival | | nether | inferno |
| maintenance | prison | | beach | beach |
| office | maze | | garden | greenhouse |

Room types: stage -> carnival_stage, arcade -> arcade_room, prizeGallery -> game_stall, arcadeHallway -> midway,
serviceBay -> backstage, relayControl -> control_room, grandStairHall -> stairwell, diningRoom -> dining_room,
kitchen / luxuryKitchen / gothicKitchen (gothic district) -> kitchen, bathroomMale/Female -> luxury_bathroom,
publicShowersMale/Female -> public_restroom, houseBathroom -> house_bathroom, netherChamber -> thermal_chamber,
beachCove -> cove, cargoDepot -> storage, ballroom -> (mansion default), garden -> overgrown_room,
library -> archive, circusTent -> big_top, masterBedroom / bedroom -> bedroom, gothicBedroom -> mourning_room (gothic
district), office -> cubicle_room, maintenance -> (prison default), storage -> storage, server -> server_hall,
breakroom -> family_dining, lounge -> living_room, lobby -> (maze default). Hospital, lab, greenhouse, mall and prison
types map to their namesakes. An id that is already an original purpose passes through. Change the two tables to retune.

`region_set(...).wall` / `.floor` are the lists' first entries. Use `wall_at` / `floor_at` for the original's
variation. For finite-route halls, pass any stable per-floor value as the "chunk" to `floor_at`.

## UV and tiling conventions (RefSurfaceMesh)

- **Floor / ceiling**: 32 x 32 px = **1 m x 1 m** (one texture per grid cell, 32 px/m). `u = world x`,
  `v = world z` (fractional; merged quads use u = 0..w, v = 0..h with repeat). Image row 0 lies along the cell's -z
  edge, column 0 along its -x edge. Ceilings use the same (x, z) mapping on a downward-facing quad.
- **Wall**: 32 x 128 px = 1 m wide x **one storey**. `u` runs 0..1 across the 1 m face, left to right as seen from
  the room. `v = tiers * (1 - (y - room_floor) / (room_height - room_floor))` (`wall_v`), with v = 0 (the cornice) at
  the ceiling and repeat on. `tiers = max(1, round(H / 4.0))` (`wall_tiers`, `RefLook.WALL_TEXTURE_SPAN = 4.0`), so
  128 px nominally spans 4 m (32 px/m). In practice the whole painted storey is stretched so the cornice meets the
  ceiling and the skirting meets the floor. Room heights 3.7-4.4 m and corridors 4.0-4.6 m get 1 tier; tall halls
  4.8-6.0 m get 1, or 2 at 6.0. At the finite route's 3.6 m: 1 tier, 35.6 px/m vertically, cornice 0.28 m, chair rail
  1.63 m above the floor (row 70 of 128), skirting 0.225 m. A face that covers only part of the storey (a lintel over
  a lower doorway) keeps the room's v, so the paint lines up.
- Bands in the 128 px wall (from the top): cornice rows 0-9 (highlight row 7, shadow row 10), chair rail rows 68-71
  (`RAIL_Y` = 70), skirting rows 120-127 (shadow row 119).
- Sampling: nearest, repeat. Colour: the chunk shader multiplies texels by `ARRAY_TINT` = (1.25, 1.22, 1.3), because
  the painted values are deliberately dark. Single-material path: 1.4/1.4/1.45, metal floors 1.2/1.2/1.25, ceilings
  .8/.78/.85, glass roof 1.2/1.2/1.3. The texture is marked `source_color` (sRGB). Emission = texel x
  `surface_params().2` (beach kinds .6, skylight .9, sewer vault .16).

## Safe-room floor marker

In a chunk flagged `safe_room`, every **open** cell within Chebyshev distance 3 of the chunk's centre cell
(`CHUNK_SIZE / 2` = 22) has its floor kind replaced (walls are untouched): distance 0 gets `safe_mark` (a big teal
cross), 1-2 get `safe_plain` (bare metal0, so the pad reads as one slab), and 3 gets `safe` (metal0 with a teal inlay
line at rows/columns 1 and 29, a frame round the pad's outer ring). The result is a 7 x 7 pad. All three are metal0
underneath with the metal surface response. `safe_pad_kind(dx, dz)` gives the kind from the offset to the pad centre.
For the finite route, centre it on the safe room.

## Hall runner (left / centre / right)

Only in districts with `hall_runner` (mansion, apartment, gothic). `runner_kind` ports `_runner_kind`: for an open
hall cell it measures the open run across the hall along each axis (up to 6 cells each way). If a run is exactly the
corridor width (3), the cell's offset from the middle picks the strip. With the hall along x (cells across it vary in
z) these are `runner_l` (-z side; brass edge lines at image rows 3 and 5, near v = 0, the outer edge), `runner_c`
(centre; brass diamond, gold 3x3 centre) and `runner_r` (+z side; lines at rows 28 and 26). With the hall along z they
are the `runnerz_*` kinds, the same strips rotated 90 degrees counter-clockwise, so `l` is at -x with its lines at
columns 3 / 5. This puts the brass borders on the outer edges of the 3-cell runner. A junction cell (both runs >= 3)
within 2 cells of an exactly-one-hall-wide cell gets `runner_c`, so the carpet runs unbroken through openings. Halls
that are not exactly 3 wide get no runner. The single-cell `runner` / `runnerz` kinds exist in the tables but the
streamed view does not lay them.
