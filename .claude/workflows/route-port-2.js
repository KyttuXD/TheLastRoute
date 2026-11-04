export const meta = {
  name: 'route-port-2',
  description: 'Route port stage 2: the room text becomes Godot\'s stored ROOM-<code>-<20 hex> per run, and the chunk room layout (rooms, corridors, suites, edge openings) is ported and checked against the Godot goldens',
  whenToUse: 'Use to continue the infinite-route port after stage 1 (src/bin/last_route/world/route)',
  phases: [
    { title: 'Study', detail: 'map the Godot room-layout closure and the Rust run-start and save paths' },
    { title: 'Port', detail: 'two agents on disjoint files: room text plumbing, and the room layout port with its golden test' },
    { title: 'Verify', detail: 'build, tests, headless suites and the layout parity count' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-2'

const CONTEXT = `Goal: the Rust Infinite route generates exactly what the live Godot Infinite route generates for the same room.

Already done (stage 1): the world seed is str(djb2(room text)) and the region rules match Godot on 588 of 588 golden chunks (src\\bin\\last_route\\world\\route\\regions.rs, src\\bin\\last_route\\world\\route\\mod.rs). The test suites pass 48 of 48.

Room text (Godot, room_state.gd lines 565 and 1342): "ROOM-<code>-<20 hex uppercase>" where the 20 hex come from md5 of a random stamp, and <code> is the room code. run_seed = maxi(1, abs(int(text.hash()))) (room_state.gd run_seed(), line 1353). The Rust port currently DERIVES the text from the run number (src\\bin\\last_route\\world\\route\\mod.rs room_text(run_seed), format "ROOM-{code}-{8 hex}"), so the text is not stored and a room text cannot reproduce Godot's world seed. The agreed change: a run creates a random text in Godot's format, stores it, derives run_seed from it, and carries it through saves and checkpoints (an older save with no stored text falls back to room_text(run_seed)).

Layout target: the Godot function StreamedRouteGenerator._region_layout (streamed_route_generator.gd lines 1722 to about 2650), with edge_openings (1374-1390), stair_links (1399-1402), _carve_edges (1549-1606) and their helpers. Godot file: ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd (5,725 lines). The chunk fields checked are rooms, corridors, suites, edges (west, east, north, south) and stairs. The grid is NOT checked in this stage: grid carving also depends on the region props, which is stage 3.

Goldens (correct, scene harness, zero script errors; do NOT regenerate): ${GOLD}\\golden-parity.json (world seed 1426541270), golden-k7qxm.json (world seed 4157433391), golden-demo.json (world seed 2899547009). Each has levels 0,-1,-2,-3 and radius 3 (196 chunks), and each chunk is {coord:[x,y,z], data:{...}} with the full chunk dictionary. Those dumps already contain rooms, corridors, suites, edges and stairs, so compare against them directly.

Rules: do not edit Godot. Do not commit. Use Node 24 on PATH for JSON. Report every file you change. Keep the final report short: put details in files, not in the message.`

const STUDY = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    layout_closure: { type: 'string', description: 'the functions and constants _region_layout calls, with line numbers' },
    rust_run_paths: { type: 'string', description: 'where the Rust run is created, carries the seed and saves it, with file and line' },
    plan: { type: 'string' },
  },
  required: ['summary'],
}

const ROOM_TEXT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    notes: { type: 'string' },
  },
  required: ['summary'],
}

const LAYOUT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    layout_parity: { type: 'string' },
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

phase('Study')
const study = await agent(`${CONTEXT}

Task: study only, do not edit files. (1) List the functions and constants that StreamedRouteGenerator._region_layout depends on, from line 1722 onward, with line numbers, and the helpers of edge_openings, _carve_edges and stair_links. Keep the list to what a Rust port of the room layout needs. (2) In ${RUST}, find where a run is created with its seed (src\\bin\\last_route\\game\\mod.rs new_infinite and new_infinite_sim, src\\bin\\last_route\\game\\endless.rs, the menus in src\\bin\\last_route\\app\\mod.rs, the checkpoint in src\\bin\\last_route\\app\\checkpoint or src\\bin\\last_route\\shared\\checkpoint.rs, and any save format). Say where the room text would be stored and how an old save without it falls back.`, {
  label: 'study-layout', phase: 'Study', schema: STUDY,
})

phase('Port')
const [roomText, layout] = await parallel([
  () => agent(`${CONTEXT}

Study (JSON): ${JSON.stringify(study)}

Task: the room text change, in ${RUST}. Owner files: src\\bin\\last_route\\game\\mod.rs, src\\bin\\last_route\\game\\endless.rs, src\\bin\\last_route\\shared\\checkpoint.rs (or the checkpoint module the study found), src\\bin\\last_route\\world\\route\\mod.rs (only the helpers), and src\\bin\\last_route\\app\\mod.rs (only the run start and the displayed text). Do not touch src\\bin\\last_route\\world\\route\\layout* or the layout code.
1. A new run creates a random room text "ROOM-<code>-<20 hex uppercase>". Use the time and the random source the Rust code already has (no new crates). The code is 5 characters from the existing room-code alphabet. Keep the text format exactly.
2. run_seed = maxi(1, djb2(text)) from route::run_seed_of_text (already matches Godot). The world seed stays str(run_seed) (route::world_seed).
3. Store the text with the run: Game/InfState keeps the room text, and the checkpoint/save stores it too. An older save with no stored text falls back to route::room_text(run_seed).
4. The menus, the loading screen, the pause menu and the run report show the stored text.
5. Run cargo test --lib and cargo test --bin last_route; fix anything you broke, and update tests that assumed the 8 hex text. Report the files changed in the short report and say what you verified.`, {
    label: 'room-text', phase: 'Port', schema: ROOM_TEXT_REPORT,
  }),
  () => agent(`${CONTEXT}

Study (JSON): ${JSON.stringify(study)}

Task: port the room layout, in ${RUST}. Owner files: src\\bin\\last_route\\world\\route\\layout.rs (new), src\\bin\\last_route\\world\\route\\mod.rs (only to add "pub mod layout;"), tests\\parity\\layout-*.json (new, compact), and src\\bin\\last_route\\world\\route\\layout.rs tests. Do not edit the room text code or the run start.
1. Port Godot's _region_layout, edge_openings, stair_links, _carve_edges and the helpers they need, reproducing the same SeededRandom draws in the same order (src\\bin\\last_route\\world\\rng.rs Rng is already bit-exact with SeededRandom; keep the same seed text per draw, as the Godot source writes it). Keep the Godot names in doc comments with the line numbers.
2. Output a Rust layout struct per chunk with rooms, corridors, suites, edges and stairs in the same shape as the Godot chunk dictionary.
3. Parity test: extract per chunk the rooms, corridors, suites, edges and stairs from the three goldens (for levels 0,-1,-2,-3 and radius 3) into compact tests\\parity\\layout-<tag>.json files (tags: parity, k7qxm, demo), and write a Rust test that compares the Rust layout with each file. Report the number of chunks matching per seed and level, and list the first mismatches with coordinates.
4. Regions Godot does not lay out this way (the inferno level -3, which has no rooms) must be skipped and counted as skipped, not matched. Say which regions and levels you skipped.
5. Run cargo test --lib and report the counts.
Keep the report short; write the detail in the files.`, {
    label: 'layout', phase: 'Port', schema: LAYOUT_REPORT,
  }),
])

phase('Verify')
const verify = await agent(`${CONTEXT}

Port reports: ${JSON.stringify({ roomText, layout })}

Task: verify stage 2 in ${RUST}. Do not change code unless a check fails with a clear cause, and say what and why if you do.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts.
3. Headless suites, writing the report to a Windows path: "${RUST}\\target\\debug\\last_route.exe" --test-suite "tests/*.json" --out "C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-2\\suite.json" (use that exact Windows path form; do not use /c/ paths for --out). Report the RESULT line.
4. Recount the layout parity yourself from the tests\\parity\\layout-*.json files, and report the matching chunks per seed and level and the skipped ones.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-layout', phase: 'Verify', schema: VERIFY,
})

return { study, roomText, layout, verify }
