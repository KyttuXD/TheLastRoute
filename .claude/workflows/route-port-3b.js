export const meta = {
  name: 'route-port-3b',
  description: 'Route port stage 3b-i: the prop data tables (collider sizes and offsets, template sizes, room-size profiles, region prop sets) generated from the Godot source into Rust, with a regeneration check. Furniture placement comes in 3b-ii and 3b-iii',
  whenToUse: 'Use to continue stage 3b of the infinite-route port (src/bin/last_route/world/route)',
  phases: [
    { title: 'Port', detail: 'one agent writes a generator script and the generated Rust table module, with tests' },
    { title: 'Verify', detail: 'regenerate the tables, build, run the tests and the suites' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const STUDY = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b'

const CONTEXT = `Goal: the Rust Infinite route places the same furniture as Godot (stage 3b). This sub-stage (3b-i) ports the DATA TABLES the placement reads, generated from the Godot source, not typed by hand.

Godot sources (read-only, do not edit):
- ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd: REGION_PROP_SETS (lines 134-248), TEMPLATE_PROP_SIZES (299-494), the STREAMED_HUNTER_IDS list (499), and other constants the prop code reads (ROOM_SIZES 126-127, LARGE_ROOM_* 109-116).
- PropColliderSizes and PropColliderBoxes (find them under game\\config or game\\world\\collision: grep for class_name PropColliderSizes and PropColliderBoxes). SIZES (about 190 entries), OFFSETS (about 73), and the EXTRA and VARIANT boxes.
- RoomSizeProfiles (game\\config\\room_size_profiles.gd, 136 lines, PROFILES table of 12 entries, and within() at line 128).
- The region prop tables used by _region_props: REGION_PROP_SETS already, plus any per-region table it reads. Check the study notes first.
- PropModelCatalog.is_wall_mounted (line 568) and hide_capacity (line 504), if the placement uses them (game\\config\\prop_model_catalog.gd). HUNTER_SCALE is already ported (src\\bin\\last_route\\render\\prop_scale.rs, draw only; do not duplicate).

The study notes from stage 3 are at ${STUDY}\\region-props-notes.txt. Read them first.

Rust today: src\\bin\\last_route\\world\\route\\ (mod.rs, regions.rs, layout.rs, grid.rs). src\\bin\\last_route\\world\\facility.rs has an older, different prop model that the live route uses; do not change it in this sub-stage.

Goldens (correct; do NOT regenerate): ${GOLD}\\golden-parity.json, golden-k7qxm.json, golden-demo.json (levels 0,-1,-2,-3, radius 3), and golden-mansion1.json, golden-apt1.json, golden-beach1.json, golden-gothic1.json (level 0, radius 3). Each chunk has data.region_props (a list of props with type, cell, yaw, id, occupied_cells, doorway_cells and template fields). The prop tables feed those.

Rules: do not edit Godot. Do not commit. Node 24 on PATH. Report every file you change.`

const PORT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    table_counts: { type: 'string' },
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

Task: stage 3b-i in ${RUST}. Owner files: tools\\parity\\gen_prop_tables.mjs (new generator, Node, reads the Godot sources and writes the Rust file), src\\bin\\last_route\\world\\route\\prop_tables.rs (generated), src\\bin\\last_route\\world\\route\\mod.rs (only "pub mod prop_tables;"), and tests in prop_tables.rs.
1. Write tools\\parity\\gen_prop_tables.mjs. It reads the Godot files listed above with plain text parsing (the Godot tables are GDScript const dictionaries and arrays; write a small, careful parser for the forms they use, and fail loudly on any form it does not understand). It writes src\\bin\\last_route\\world\\route\\prop_tables.rs with each table as a Rust const or static, with the same names and the same values, and a header saying it is generated and by which script.
2. Run the generator. Then check the counts against the Godot sources (the number of REGION_PROP_SETS entries, TEMPLATE_PROP_SIZES entries, SIZES, OFFSETS, EXTRA and VARIANT boxes, and the 12 room-size profiles).
3. Add a Rust test per table group that checks a few specific entries against values you read from the Godot source with line numbers in the test comments. Add a check that the generated file is up to date: the generator must produce the same bytes as the file in the repo.
4. Do not port any placement code in this sub-stage.
5. Run cargo test --lib and report the counts.
Keep the report short.`, {
  label: 'port-prop-tables', phase: 'Port', schema: PORT_REPORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port report: ${JSON.stringify(port)}

Task: verify stage 3b-i in ${RUST}. Do not change code unless a check fails with a clear cause; say what and why if you do.
1. Run node tools\\parity\\gen_prop_tables.mjs into a scratch copy under ${SCRATCH}, and diff it against src\\bin\\last_route\\world\\route\\prop_tables.rs. Report whether they are identical.
2. cargo build --bins and cargo build --release --bins: pass or fail.
3. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts.
4. Headless suites, with --out given as the Windows path C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b\\suite.json. Report the RESULT line.
5. Spot-check three table entries against the Godot source yourself (pick them at random from different tables), and report the result.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-prop-tables', phase: 'Verify', schema: VERIFY,
})

return { port, verify }
