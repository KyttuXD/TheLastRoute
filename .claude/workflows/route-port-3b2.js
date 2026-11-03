export const meta = {
  name: 'route-port-3b2',
  description: 'Route port stage 3b-ii: the region-props placement driver and its helpers (placement, wall mounts, orientation, seats, ceiling families, colliders, hiding props, surface scaling), checked against the golden props that have no room template',
  whenToUse: 'Use to continue stage 3b of the infinite-route port (src/world/route), after the 3b-i tables',
  phases: [
    { title: 'Port', detail: 'one agent ports the driver and its non-template helpers and writes the golden check' },
    { title: 'Verify', detail: 'build, tests, suites and the prop parity count' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const STUDY = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b2'

const CONTEXT = `Goal: the Rust Infinite route places the same region props as Godot for level-0 chunks (StreamedRouteGenerator._region_props, streamed_route_generator.gd 2824-2980, and the helpers it calls).

Already in Rust (verified): region (route/regions.rs), rooms, corridors, suites, edges and the stairwell (route/layout.rs), the level-0 grid and the door props planned by the grid pass (route/grid.rs DoorProp, finish_level0_grid), and the generated data tables (route/prop_tables.rs: REGION_PROP_SETS, TEMPLATE_PROP_SIZES, PROP_COLLIDER_SIZES and offsets and boxes, ROOM_SIZE_PROFILES, ORIENT_PAIRS, WALL_MOUNTED_TYPES, EMPTY_FRAME_TYPES, CEILING_FIXTURE_FAMILY, HIDING_PROP_STYLES, FALLBACK_BEDS, BED_TEMPLATES, and more).

Stage 3b-ii scope (this step): the driver _region_props and every helper it calls that is NOT part of a furnished-room template. The study notes are at ${STUDY}\\region-props-notes.txt; read them first. The helpers in scope include _solid_prop_placement (4969), _wall_mount_for_room (5028), _prop_occupied_cells (5011), _room_door_props and _find_room_door_threshold (door props, check how grid.rs already did it and reuse it), _is_major_room (5144), _door_faces_hall_end (5153), _stair_door_exclusion (5177), _room_facing_yaw (5290), _wall_count (5607), _backing_wall_direction (5618), _exclude_neighbourhood (5600), _cardinal_toward (4959), _collider_size (5007), _prop_hiding_fixtures (5307), _frame_type_for_region (4825), _rug_type_for_region (4835), scale_surface_mounts (3233), orient_props (3357), seat_in_front_of_screens (3386), nearest_anchor (3423), back_to_wall (3440), back_extent (3461), wall_behind (3469), even_ceiling_fixtures (3285), one_ceiling_family (3479), claim_order (3509), nudge_seat_clear (3532), collider_clear (3554), collider_rects (3569), vary_specs (3607) and table_grid_specs (3582) only if the driver reaches them for non-template props, and the RNG streams the driver uses (DRESSING at 2832 and VARIETY at 3165 as read). Furnished-room templates (_furnished_room_props 3058, _room_template_specs 3630-4291, _template_prop_entry 4878, _room_template_basis 4849, _keep_entrance_clear 4297, _fallback_bed 3255) are stage 3b-iii, NOT this step. Where the driver calls a template function, leave a clearly marked seam that returns an empty list for now, and say so.

Goldens (correct, do NOT regenerate): ${GOLD}\\golden-parity.json (world seed 1426541270), golden-k7qxm.json (4157433391), golden-demo.json (2899547009) at levels 0..-3, plus golden-mansion1.json (3833121725), golden-apt1.json (2477555712), golden-beach1.json (223482559), golden-gothic1.json (938954303) at level 0. Each chunk has data.region_props (a list). A prop without a room_template field is in this step's scope; props with room_template are stage 3b-iii.

Check: for each level-0 chunk, compare the Rust props with no room_template against the golden props with no room_template, keyed by id. Two props match when type, cell, yaw, occupied_cells and doorway_cells all match. Report matched chunks and matched props per seed, and the first mismatches with ids.

Rules: do not edit Godot. Do not commit. Node 24 on PATH. Report every file you change. Keep the report short.`

const PORT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    prop_parity: { type: 'string' },
    seams: { type: 'string' },
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

Task: stage 3b-ii in ${RUST}. Owner files: src\\world\\route\\props.rs (new), src\\world\\route\\mod.rs (only "pub mod props;"), tests\\parity\\props-*.json (new, compact: for each level-0 chunk, the props without room_template, with id, type, cell, yaw, occupied_cells, doorway_cells), and tests in props.rs. Edit layout.rs or grid.rs only where the driver needs a value they do not expose yet, and say so.
1. Port the driver and the helpers listed above with their Godot names and line numbers in the doc comments. Reproduce the RNG draws in order (the DRESSING and VARIETY streams and the per-helper streams the study notes name). If you find a draw you cannot place, stop and report it.
2. Write the extraction of the compact golden files, then a Rust test that compares the non-template props per chunk.
3. Run cargo test --lib and report the counts. Report the prop parity per seed, the matched chunks, and the first mismatches with ids. Do not weaken the test.
Keep the report short.`, {
  label: 'port-props-driver', phase: 'Port', schema: PORT_REPORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port report: ${JSON.stringify(port)}

Task: verify stage 3b-ii in ${RUST}. Do not change code unless a check fails with a clear cause; say what and why if you do.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts.
3. Headless suites, with --out given as the Windows path C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b2\\suite.json. Report the RESULT line.
4. Recount the non-template prop parity yourself from the tests\\parity\\props-*.json files against the goldens in ${GOLD}, per seed: matched chunks, matched props, mismatches.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-props-driver', phase: 'Verify', schema: VERIFY,
})

return { port, verify }
