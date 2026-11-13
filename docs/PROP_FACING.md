# Prop facing, 45 degree turns and hall props

A Rust-only pass over a level-0 chunk's props. It runs once when a chunk is built, in `world::infinite::chunk`,
after the Godot-ported generator (`world/route/**`) and its adapter (`route::live::from_layout`). The generator and
its golden files are untouched: the goldens read the generator's raw output, and this pass works on the built
`Facility` afterwards.

Code: `world/prop_facing.rs` (facing, diagonals, the flood-fill check, `dress`) and `world/hall_props.rs` (hall props).
Data: `data/world/prop_facing.json` and `data/world/hall_props.json` (see [DATA_FILES.md](DATA_FILES.md)).

Scope: level 0 of the Infinite route. The sewers, the Inferno and `KYTTU_LEGACY_CHUNKS=1` chunks are not touched.

## North and yaw

- **North is the map's -z**: cell `y - 1`, the top of the map and the admin "north".
- A prop's turn is `Prop::rotation`, in quarter turns. 0 faces north, 1 west, 2 south, 3 east
  (`prop_facing::FACING`). Its forward is the model's gameplay -z, as in Godot.
- A 45 degree turn is stored as `rotation = 4 + quarter turn` (`facility::DIAGONAL`). Code that reads the rotation
  modulo 2 or 4, or multiplies it by a quarter turn, still sees the quarter turn. `Prop::yaw()` gives the drawn turn
  (quarter turns plus an eighth) and `Prop::quarter()` / `Prop::is_diagonal()` the parts. `render/world_mesh.rs`
  bakes the model and places a prop's light with `Prop::yaw()`.
- A diagonal with quarter turn `q` faces between `FACING[q]` and `FACING[q + 1]`: 4 is north-west, 7 north-east.

## Facing

For each solid floor prop that is not exempt:

1. The four cells beside the prop's cell are read as wall or open, and the open run in each direction is counted
   (to `run_cap` cells).
2. **A wall beside it:** the prop turns its back to a wall and faces open floor. With walls on several sides the
   facing with the longest open run wins; a tie keeps the present facing, then north, east, south, west. In a corner
   that is the longer open run.
3. **No wall beside it:** the prop faces north.
4. A deep prop moves out so its back stays `wall_gap` off the wall face (Godot's 5 cm rule).

The collider turns with the prop: a quarter turn swaps the half extents. A turn is made only if the new footprint is
on open floor, clear of every other solid prop, of pickups, lockers, vents and stairs, and leaves the doors' approach
cells to the hunters (`door_clearance::Placement`). Otherwise the next best facing is tried, and failing all the prop
stays as it was.

### What is left alone

- Room doors and arches, wall-mounted props, props standing on a surface, and the table or desk under them.
- Props a room template placed on purpose. The generator marks them: `oriented` (a seat turned to its table or
  screen), `wall_mounted`, a template prop with a `backing_wall`, and the prop of a hiding fixture
  (`prop_facing::Locks::of_layout`).
- Other template props (`room_template`) are only ever turned to put their back to a wall beside them: no north
  default and no diagonal.
- `keep_types`: beds, lockers, wardrobes, stalls, plumbing, signs and the like. `symmetric_types` (barrels, plants,
  lamps, bins) have no front, so they are not re-faced, but they may take a diagonal.
- Sewer props, crawl pipes and non-solid props (rugs, frames).

Type lists match when the lower-case type contains the entry.

## 45 degree turns

- **Free-standing:** a `diagonal.free_types` prop (crates, barrels, plants, carts, small tables) with no wall beside
  it and outside room templates turns 45 degrees on a hashed share, `free_chance`. It faces north-west or north-east.
- **Corners:** a `diagonal.corner_types` prop with walls behind two sides faces diagonally out of the corner
  (`corner_chance`). It moves out of the corner by up to `max_corner_shift` so the turned model stays out of the
  walls; a prop too large for that keeps a plain facing.
- **Never:** `diagonal.never_types`. Seats are here because sitting looks out along the quarter turn
  (`game/bed.rs`), so chairs are not turned 45 degrees. Beds, lockers, hiding props and doors are exempt from the
  whole pass.
- **Collider:** a non-square box becomes the bounding box of the turned box (`prop_facing::diagonal_half`); a square
  or round one keeps its box.

## Hall props

Solid props along hall walls, from types that already exist in `data/props`, listed by region in
`data/world/hall_props.json`. They are ordinary `Facility` props (ids `HALL:<cx>:<level>:<cy>:PROP:<n>`), added after
the generator's, so they collide, feed the hunters' grid through `Facility::rebuild_grids`, and are drawn by the
mesh builder like any other prop. Their collider is the type's row in `route::prop_tables::PROP_COLLIDER_SIZES`.

A prop may stand on a cell when:

- the cell is hall (open floor outside every room), with a wall behind it and behind the cell either side of it, so
  it is never at or beside an opening;
- the hall is at least `min_width` (3) cells wide there, and the cells in front are clear: two, or all that is left
  in a three-wide hall, so props there hug one wall. The lane left is at least `min_lane` metres
  (`min_lane_narrow` in a three-wide hall);
- it is outside the gaps round doorways (`door_gap`), stairs and ladders (`stair_gap`), lockers, vents and hiding
  props (`hide_gap`), pickups, other props and the start, and `edge_margin` cells from the chunk border;
- it is `spacing` cells from the next hall prop, and the chunk has fewer than `max_per_chunk`.

The chance per wall-side cell is by region (`chance`), times `long_bonus` on a straight wall run of `long_run` (8)
cells or more. Safe-room chunks get none. The sewer and Inferno regions are set to zero.

## Determinism and multiplayer

Every draw is `prop_facing::hash(seed text, [absolute cell x, absolute cell y, level, salt])`. The host and every
guest build chunks from the seed, so they get the same props, in the same order. Hall props come after the
generator's props, so no existing prop index moves.

## The no-seal check

After the pass the chunk is flood-filled with a hunter's body (`HUNTER_RADIUS`) before and after
(`prop_facing::body_groups`, `still_joined`). If any group of cells that was joined is now split, the chunk keeps its
original props and gets no hall props.

## Cost

Nothing per frame. Per built chunk (cached with the chunk): one clone of the prop list, one pass over the 45 x 45
cells for the hall mask and the keep-out mask, the facing loop, and, only when something changed, one extra
`rebuild_grids` and two flood fills.

## Tests

In `world/prop_facing.rs` and `world/hall_props.rs`: a prop beside a wall turns its back to it; a sideways prop's
collider turns with it; a corner takes the longer run; open floor faces north; exempt and locked props are unchanged;
only listed types turn 45 degrees and the turns repeat for a seed; a corner prop faces out of the corner; hall props
stand back to a wall and collide; they never cut a hall (flood fill before and after, widths 3 to 6); none stands at
an opening, by a locker, on a pickup or in a hall under three cells wide.

## Not done

- Hall arches and pilasters (`render/hall_arch`) are placed by the mesh builder and are not known when a chunk is
  built, so a hall prop can stand next to a pilaster.
- A playable machine type (arcade cabinet) placed in a hall is playable like any other.
