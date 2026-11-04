export const meta = {
  name: 'route-port-3',
  description: 'Route port stage 3, study first: map the Godot closure for the region props and the grid carving on level 0, and the sizes of sewers, Inferno, pickups and hunter spawns',
  whenToUse: 'Use before porting stage 3 of the infinite-route generator (src/bin/last_route/world/route)',
  phases: [
    { title: 'Study', detail: 'four read-only studies in parallel, each sized and listed for a later port' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'

const RULES = `READ-ONLY study. Do not edit or create files in ${GODOT} or ${RUST}; scratch notes only under C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3. Cite file and line numbers. Godot generator: ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd (5,725 lines). Rust: ${RUST}\\src\\bin\\last_route\\world\\route\\ (layout.rs, regions.rs, mod.rs) and src\\bin\\last_route\\world\\infinite.rs. Goldens with the full chunk data: ${GOLD}\\golden-parity.json, golden-k7qxm.json, golden-demo.json (each: levels 0,-1,-2,-3, radius 3, chunks [{coord, data}]).
Keep the report short: a summary and a sized list. Put long lists into a notes file under the scratch folder and name its path.`

const STUDY = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    detail: { type: 'string' },
  },
  required: ['summary'],
}

phase('Study')
const [props, grid, pickups, sewer] = await parallel([
  () => agent(`${RULES}

Study: the region props of a level-0 chunk. Map StreamedRouteGenerator._region_props (around line 2824 to the end of that function) and every function it calls for level-0 maze, mansion, hotel and apartment chunks: prop tables per district, prop placement draws in order, the RoomSizeProfiles and PropColliderSizes inputs, mount and surface logic, scale_surface_mounts, and the hiding and fixture helpers it uses. Count the functions and the lines. Say what Rust already has (src\\bin\\last_route\\world\\route and src\\bin\\last_route\\world\\facility.rs) and what the port needs. Name the golden field that would check it (region_props in each golden chunk).`, { label: 'study-props', phase: 'Study', schema: STUDY }),
  () => agent(`${RULES}

Study: the grid carving that runs after the layout on level 0, in _build_chunk (lines 916-1074): _carve_stairwell effects on the grid, _carve_safe_room, _restore_single_room_thresholds (around 2652), _remove_wall_humps (2676), _restore_authored_room_door_thresholds, GeometryResolve.push_out, PlacementClearance.clear (only when clear_walkways is true: check that flag), and _carve_stair_access. For each, say what it changes in the grid, which inputs it reads (region_props or rooms), and its size. Name the golden field that checks the grid (grid in each golden chunk) and state whether the grid can be matched before the props are ported (it cannot if these functions read region_props).`, { label: 'study-grid', phase: 'Study', schema: STUDY }),
  () => agent(`${RULES}

Study: pickups, fixtures, music boxes and hunter spawns for level 0 (_pickups, _fixtures, _music_boxes, _hunter_spawns, _prop_hiding_fixtures, _ensure_pickups_reachable, pickup_id and hunter_id around line 1528-1537, and the sealed_cells input). Map each function with its line range, its RNG tags and its inputs, and size them. Name the golden fields they fill (pickups, fixtures, music_boxes, hunter_spawns) and note any draw that depends on props or on the grid.`, { label: 'study-pickups', phase: 'Study', schema: STUDY }),
  () => agent(`${RULES}

Study: the sewer levels -1 and -2. chunk() routes these to SewerRouteGenerator (streamed_route_generator.gd line 796). Read game\\world\\generation\\sewer_route_generator.gd (1,300 lines) and list its public entry points (chunk, ramps_at, ramp_between, falls_between, and whatever else the streamed generator calls), its region and room tables, its RNG tags, and its output fields. Size the port. Check whether the goldens hold sewer chunks at levels -1 and -2 with rooms, pump and valve rooms, and report what they contain. Also list the Inferno (level -3) entry points: InfernoCaverns.carve, SealPlanner.plan_cave and SealPlanner.specs, and their sizes (game\\world\\generation or game\\world\\regions).`, { label: 'study-sewer', phase: 'Study', schema: STUDY }),
])

return { props, grid, pickups, sewer }
