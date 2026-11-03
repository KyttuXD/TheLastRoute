export const meta = {
  name: 'route-port-3b4',
  description: 'Route port stage 3b-iv: the post-passes that remove props (GeometryResolve.push_out and PlacementClearance.clear), and the data-backed hospital and mall prop sets. Then the golden prop test is un-ignored if every level-0 chunk passes',
  whenToUse: 'Use to finish stage 3b of the infinite-route port after 3b-iii',
  phases: [
    { title: 'Port', detail: 'one agent ports the two post-passes and the data-backed prop sets, and removes the golden test ignore if it passes' },
    { title: 'Verify', detail: 'build, tests, suites and the full prop parity count' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b4'
const COMPACT = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3\\compact_all'

const CONTEXT = `Goal: the Rust Infinite route places exactly the props that Godot places on level-0 chunks (stage 3b).

Godot (read-only): ${GODOT}
- Post-passes in _build_chunk, streamed_route_generator.gd lines 1020-1031: GeometryResolve.push_out(grid, region_props) (game\\world\\collision\\geometry_resolve.gd, 51 lines) and scale_surface_mounts, then PlacementClearance.clear(grid, layout.rooms, region_props) when clear_walkways is true (game\\world\\collision\\placement_clearance.gd, 248 lines; clear_walkways is a static var at line 123 of streamed_route_generator.gd, true and never reassigned). Read the real order and the real flags there.
- Data-backed prop sets: data\\regions\\hospital.json and data\\regions\\mall.json, the prop_set field. The Godot code reads them through ContentRegistry. Find the code path that reads the prop_set for a region and what region_props does when it is present (streamed_route_generator.gd around _region_props, 2824-2980).

Rust (verified): src\\world\\route\\props.rs (the non-template driver, returns None for data-backed regions at around lines 22, 93, 266), src\\world\\route\\furnished.rs (templates, verified on every template prop it makes), src\\world\\route\\grid.rs (door props and grid passes), src\\world\\route\\prop_tables.rs (generated tables).

Known gap (from the stage 3b-iii verify): the Rust port makes 153 extra template props across six seeds. The sampled extras are arcade front cabinets, office_cubicles and office_job pieces, so they are the props that the two post-passes remove in Godot. The Rust port also produces no props for the 26 data-backed hospital and mall chunks (12 hospital in parity, 14 mall in beach1).

Goldens (correct, do NOT regenerate): ${GOLD}\\golden-*.json (parity, k7qxm, demo at levels 0..-3, and mansion1, apt1, beach1, gothic1 at level 0). The compact golden files built for the template check are at ${COMPACT}, and the test reads them with the environment variables PARITY_DIR and PARITY_TEMPLATES=1 (see props::tests::region_props_match_godot_goldens in src\\world\\route\\props.rs). The test is #[ignore]. Read the test before you change it.

Rules: do not edit Godot. Do not commit. Node 24 on PATH. Report every file you change. Keep reports short; put detail in files.`

const PORT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    extras_left: { type: 'string' },
    data_backed: { type: 'string' },
    ignore_removed: { type: 'string' },
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

Task: stage 3b-iv in ${RUST}. Owner files: src\\world\\route\\clearance.rs (new: push_out and PlacementClearance.clear), src\\world\\route\\props.rs (the call to the post-passes, in the Godot order, and the data-backed sets), src\\world\\route\\mod.rs (only "pub mod clearance;"), and the props test only to remove the #[ignore] when every level-0 chunk passes.
1. Port GeometryResolve.push_out and PlacementClearance.clear with the Godot names and line numbers in doc comments. Keep the order from _build_chunk. Reproduce any draw in order (check whether either one draws at all).
2. Port the data-backed prop sets for the hospital and mall regions: read data\\regions\\hospital.json and mall.json, and make region_props produce the same props the Godot code makes for those regions.
3. Run the golden test with PARITY_DIR set to ${COMPACT} and PARITY_TEMPLATES=1, and with --ignored. Report the counts per seed: matched chunks, props matched, extras, not made.
4. If every level-0 chunk passes on all seven seeds, remove the #[ignore] and its message. If not, keep the ignore and report the first mismatches by seed, chunk and room type.
5. Run cargo test --lib and report counts.
Keep the report short.`, {
  label: 'port-clearance', phase: 'Port', schema: PORT_REPORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port report: ${JSON.stringify(port)}

Task: verify stage 3b-iv in ${RUST}. Do not change code unless a check fails with a clear cause; say what and why if you do.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts. Also run cargo test --lib region_props_match_godot_goldens -- --ignored with PARITY_DIR=${COMPACT} PARITY_TEMPLATES=1 and report the counts per seed.
3. Headless suites, with --out as the Windows path C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b4\\suite.json. Report the RESULT line.
4. Recount the level-0 props yourself against the raw goldens in ${GOLD} for each of the seven seeds: matched chunks, matched props, extras, not made. Report the totals.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-clearance', phase: 'Verify', schema: VERIFY,
})

return { port, verify }
