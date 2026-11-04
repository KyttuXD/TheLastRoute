export const meta = {
  name: 'route-port-3b3',
  description: 'Route port stage 3b-iii: the furnished-room templates. Part A ports the furnished driver, the template helpers and the room types that the goldens contain, and checks the props with room_template. Part B ports the remaining template types, which no golden covers',
  whenToUse: 'Use to continue stage 3b-iii of the infinite-route port after 3b-ii',
  phases: [
    { title: 'Port A', detail: 'driver, template helpers and the golden-covered room types, with the props check' },
    { title: 'Port B', detail: 'the remaining template types, without golden coverage' },
    { title: 'Verify', detail: 'build, tests, suites and the props parity count across all seven seeds' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const STUDY = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b3'

const CONTEXT = `Goal: the Rust Infinite route places the same furnished-room props as Godot on level-0 chunks. Stage 3b-iii ports the furnished-room path. It is the seam left by stage 3b-ii.

Godot (read-only): ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd. The furnished path:
- _furnished_room_props 3058-3227 (the per-room driver), with the VARIETY (3165) and TEMPLATE (3156) RNG streams.
- _room_template_basis 4849 (27 lines), _template_prop_entry 4878-4956 (79).
- _room_template_specs 3630-4291 (662 lines, 38 room-type arms; about 400 lines are reachable). Arms by name: bedroom 3638, kitchen 3692, mansion_dining 3708, family_dining 3730, restaurant_dining 3746, restaurant_hall 3765, dining_hall 3781, house/hotel bathroom 3862, luxury_bathroom 3883, public_restroom 3900, arcade 3916, maze_storage 4024, apartment_living 4065, lounge 4088, office_reception 4111, office_cubicles 4129, office_job 4190, private_office 4238.
- Helpers: _vary_specs 3607, _table_grid_specs 3582, _claim_order 3509, _nudge_seat_clear 3532, _collider_clear 3554, _collider_rects 3569, _fallback_bed 3255, _keep_entrance_clear 4297, even_ceiling_fixtures 3285, one_ceiling_family 3479, orient_props 3357 with ORIENT_PAIRS 3331, seat_in_front_of_screens 3386, nearest_anchor 3423, back_to_wall 3440, back_extent 3461, wall_behind 3469, scale_surface_mounts 3233, _frame_type_for_region 4825, _rug_type_for_region 4835, _cardinal_toward 4959, _collider_size 5007, _prop_hiding_fixtures 5307.
The study notes are at ${STUDY}\\region-props-notes.txt.

Rust (verified): src\\bin\\last_route\\world\\route\\props.rs has the non-template driver (the seam is furnished_room_props, which returns Vec::new()). src\\bin\\last_route\\world\\route\\prop_tables.rs has the generated tables (TEMPLATE_PROP_SIZES, ROOM_SIZE_PROFILES, ORIENT_PAIRS, CEILING_FIXTURE_FAMILY, BED_TEMPLATES, FALLBACK_BEDS, HIDING_PROP_STYLES, EMPTY_FRAME_TYPES, WALL_MOUNTED_TYPES, PROP_COLLIDER_*). Circulation is in layout.rs.

Goldens (correct, do NOT regenerate): ${GOLD}\\golden-parity.json (world seed 1426541270), golden-k7qxm.json (4157433391), golden-demo.json (2899547009), golden-mansion1.json (3833121725), golden-apt1.json (2477555712), golden-beach1.json (223482559), golden-gothic1.json (938954303). The first three cover levels 0..-3 and the other four are level 0. Each chunk has data.region_props. Props with a room_template field are this stage's scope.

The existing golden test props::tests::region_props_match_godot_goldens is #[ignore] until this stage is done. When the furnished path is ported, un-ignore it, and it should pass for every level-0 chunk.

Rules: do not edit Godot. Do not commit. Node 24 on PATH. Report every file you change. Keep reports short; put detail in files.`

const PORT_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'string' },
    arms_ported: { type: 'string' },
    arms_left: { type: 'string' },
    prop_parity: { type: 'string' },
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

phase('Port A')
const portA = await agent(`${CONTEXT}

Task: stage 3b-iii part A in ${RUST}. Owner files: src\\bin\\last_route\\world\\route\\furnished.rs (new), src\\bin\\last_route\\world\\route\\props.rs (only to replace the seam with a call into furnished.rs and to share types), src\\bin\\last_route\\world\\route\\mod.rs (only "pub mod furnished;").
1. Port the furnished driver _furnished_room_props, _room_template_basis, _template_prop_entry, and every helper listed above that the driver reaches. Keep the Godot names in doc comments with line numbers. Reproduce the TEMPLATE and VARIETY streams and each draw in Godot order.
2. Port the template arms for the room types that appear in the goldens. Find them first: count the room_template values on the props in each golden (the template name) and list them in your report. Port those arms fully.
3. Un-ignore props::tests::region_props_match_godot_goldens only if the golden props with room_template now match. Otherwise keep it ignored and report the mismatches by room type and by seed.
4. Run cargo test --lib and report counts.
Keep the report short. Put the counts of room types found in the goldens in the arms_ported field.`, {
  label: 'port-furnished-a', phase: 'Port A', schema: PORT_REPORT,
})

phase('Port B')
const portB = await agent(`${CONTEXT}

Part A report (JSON): ${JSON.stringify(portA)}

Task: stage 3b-iii part B in ${RUST}. Owner files: src\\bin\\last_route\\world\\route\\furnished.rs (the remaining arms only) and its tests.
1. Port the template arms that part A did not port (see arms_left in the report), in the Godot order, with doc comments and line numbers.
2. These arms have no golden coverage. Check each one by reading it against the Godot source line by line, and say in the report which arms you checked that way.
3. Run cargo test --lib and report the counts. Do not un-ignore the golden test unless it passes.
Keep the report short.`, {
  label: 'port-furnished-b', phase: 'Port B', schema: PORT_REPORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port reports: ${JSON.stringify({ portA, portB })}

Task: verify stage 3b-iii in ${RUST}. Do not change code unless a check fails with a clear cause; say what and why if you do.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts. Run the golden prop test with --ignored to print its counts and report them.
3. Headless suites, with --out as the Windows path C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port-3b3\\suite.json. Report the RESULT line.
4. Recount the props with room_template yourself from the Rust output against the goldens in ${GOLD}, for each of the seven seeds: matched chunks, matched props, mismatches, extras. Say which room types are covered by golden data and which are not.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-furnished', phase: 'Verify', schema: VERIFY,
})

return { portA, portB, verify }
