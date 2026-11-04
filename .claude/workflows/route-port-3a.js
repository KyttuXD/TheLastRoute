export const meta = {
  name: 'route-port-3a',
  description: 'Route port stage 3a: the level-0 grid. Port the door planner, the threshold restores, the wall-hump pass and the safe-room carve, then check the grid of every level-0 golden chunk. Furniture is not ported here',
  whenToUse: 'Use to continue stage 3 of the infinite-route port after the stage-3 study',
  phases: [
    { title: 'Port', detail: 'one agent ports the grid passes and writes the grid golden test' },
    { title: 'Verify', detail: 'build, tests, headless suites and the grid parity count' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const STUDY = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3a'

const CONTEXT = `Goal: the Rust Infinite route makes the same grid as the live Godot route for level-0 chunks (grid[z][x], 45x45, 1 solid and 0 open).

Godot generator: ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd (5,725 lines). The grid steps of _build_chunk are at lines 916-1074. The study notes are in ${STUDY}\\grid-carve-study.txt and ${STUDY}\\region-props-notes.txt. Read them first.

Already in Rust (stage 1 and 2): src\\bin\\last_route\\world\\route\\regions.rs (region), src\\bin\\last_route\\world\\route\\layout.rs (rooms, corridors, suites, edges, the level-0 stairwell carve). layout.rs chunk_layout returns the grid after the edge carve and the stairwell carve.

Stage 3a scope: the grid-writing passes that read no furniture:
1. _room_door_props (streamed_route_generator.gd 5047-5143) with its helpers _find_room_door_threshold (5190-5282), _door_faces_hall_end (5153), _is_major_room (5144), _stair_door_exclusion (5177), and the region door tables (_CODE_REGION_ROOM_DOORS and _CODE_REGION_DOUBLE_ROOM_DOORS, lines 249-266, and whatever they reference). It writes shoulder cells to 1 and doorway cells to 0. It uses no RNG.
2. _restore_single_room_thresholds (2652) and _restore_authored_room_door_thresholds (2728).
3. _remove_wall_humps (2676), REMOVE_HUMPS true (line 121). It protects room perimeters and door doorway cells.
4. _carve_safe_room (1712), when is_safe_room(coord) is true (the origin chunk and its neighbours by the rule in is_safe_room, 1403-1413).
The order in _build_chunk matters: the layout and stairwell carve, then the threshold restore, then the safe-room carve, then the door planner, then the authored-threshold restore, then the hump pass (see 1003-1031 of Godot; read the real order there).
NOT in scope: furniture, fixtures, pickups, hunter spawns, push_out and PlacementClearance (they never write the grid). Do not port them.

Goldens (correct, do NOT regenerate): ${GOLD}\\golden-parity.json (world seed 1426541270), golden-k7qxm.json (world seed 4157433391), golden-demo.json (world seed 2899547009). Each has levels 0,-1,-2,-3 and radius 3, and each chunk is {coord:[x,y,z], data:{grid:[[...45 ints]...], ...}}. Compare level 0 only (49 chunks per file, 147 in total).

Rules: do not edit Godot. Do not commit. Node 24 on PATH for JSON. Report every file you change. Keep the reports short; put detail in files.`

const PORT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    grid_parity: { type: 'string' },
    notes: { type: 'string' },
  },
  required: ['summary'],
}

const VERIFY = {
  type: 'object',
  properties: {
    checks: { type: 'string' },
    passed: { type: 'boolean' },
    notes: { type: 'string' },
  },
  required: ['checks'],
}

phase('Port')
const port = await agent(`${CONTEXT}

Task: stage 3a in ${RUST}. Owner files: src\\bin\\last_route\\world\\route\\grid.rs (new), src\\bin\\last_route\\world\\route\\mod.rs (only "pub mod grid;"), src\\bin\\last_route\\world\\route\\layout.rs (only to hand the grid and the rooms to grid.rs and to call it from chunk_layout for level 0), tests\\parity\\grid-*.json (new, compact: per seed, level 0, the 49 grids as strings of 45 characters per row), and the test in grid.rs.
1. Port the four passes as Rust functions with the Godot names in doc comments and the Godot line numbers. Keep the RNG out of these passes (they have none); if you find a draw, stop and report it.
2. Port the region door tables exactly as data (a Rust const table is fine).
3. Wire them in the Godot order. The chunk_layout for level 0 should return the grid that the golden shows.
4. Extract the level-0 grids from the three goldens into compact tests\\parity\\grid-<tag>.json (tags parity, k7qxm, demo). Write a Rust test that compares each Rust grid with the golden, and report matches per seed and per chunk, with the first mismatch coordinates.
5. Run cargo test --lib and report the counts. If a grid does not match, do not weaken the test: find the cause, and report it when you cannot fix it.
Keep the report short.`, {
  label: 'port-grid', phase: 'Port', schema: PORT_REPORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port report: ${JSON.stringify(port)}

Task: verify stage 3a in ${RUST}. Do not change code unless a check fails with a clear cause, and say what and why if you do.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts.
3. Headless suites, with the report written to the Windows path form C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3a\\suite.json (do not pass /c/ paths to --out). Report the RESULT line.
4. Recount the grid parity from the tests\\parity\\grid-*.json files against the golden dumps in ${GOLD}, per seed and level 0: matched and total.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-grid', phase: 'Verify', schema: VERIFY,
})

return { port, verify }
