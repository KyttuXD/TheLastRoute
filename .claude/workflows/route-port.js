export const meta = {
  name: 'route-port',
  description: 'Port the Godot Infinite route generator into Rust, one stage at a time, checked chunk by chunk against Godot dumps. Stage 1: seed text and region selection',
  whenToUse: 'Use to continue the infinite-route port (src/bin/last_route/world/route) or to check it against the Godot parity dumps',
  phases: [
    { title: 'Port', detail: 'one agent ports the region stage in Rust, rebuilds the region golden files and tests them' },
    { title: 'Verify', detail: 'build, run the region parity test and the existing test suites' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GOLD = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\parity-golden'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\route-port'

const CONTEXT = `Goal: the Rust Infinite route must generate exactly what the live Godot Infinite route generates for the same room seed text.

Seed rules (verified):
- The room seed text is "ROOM-<code>-<20 hex uppercase>" (room_state.gd lines 565 and 1342).
- run_seed = maxi(1, abs(int(text.hash()))). Godot's String.hash() is djb2 (h = h*33 + c, starting at 5381, 32-bit) (room_state.gd run_seed(), line 1353).
- The live Infinite route's world seed is str(run_seed) (infinite_route_test.gd line 280: facility.begin(str(RoomState.run_seed()), ...)). Not the raw text.
- Inside the generator, every draw is SeededRandom.new("<world_seed>:<TAG>...") (seeded_random.gd: FNV-1a 32 hash, mulberry32). src/bin/last_route/world/rng.rs already matches this bit for bit.
- Region rule (stage 1): StreamedRouteGenerator.region_for (streamed_route_generator.gd lines 1210-1264), biome_for (1207-1208, an alias), _pick_region (1269-1277), region_allowed_at (1279), is_chunk_allowed (1436-1442), spawn region (1239), SPAWN_MAZE_SHARE (68). Void regions and the mezzanine are disabled in Godot (VoidRegion.ENABLED false, MEZZANINE_ENABLED false). The dual-level spec is dead (_dual_level_pair_active returns false).

Godot generator: ${GODOT}\\game\\world\\generation\\streamed_route_generator.gd (5,725 lines).
Rust today: src\\bin\\last_route\\world\\infinite.rs (353 lines). It uses different seed strings and rules, and it is fed the decimal run number (src\\bin\\last_route\\game\\endless.rs InfState::new, src\\bin\\last_route\\game\\mod.rs new_infinite_sim). It is not Godot's layout.

Golden dumps (correct, made by the scene-based harness, zero script errors; do NOT regenerate):
- ${GOLD}\\golden-parity.json  (room text ROOM-PARITY-0000000000000001, world seed 1426541270)
- ${GOLD}\\golden-k7qxm.json   (room text ROOM-K7QXM-3A9F1C0D2B4E5F607182, world seed 4157433391)
- ${GOLD}\\golden-demo.json    (room text ROOM-DEMO-00000000000000000000, world seed 2899547009)
Each has levels 0,-1,-2,-3 and radius 3 (196 chunks). Each chunk is {coord:[x,y,z], data:{...}} where data is the full chunk dictionary; data.region is the region name.
The harness is ${GODOT}\\tests\\parity\\dump.tscn with dump_node.gd (run: Godot_v4.7.2-stable_win64_console.exe --headless --path <KyttusLabApp> res://tests/parity/dump.tscn -- ROOM_TEXT OUT.json LEVELS RADIUS).

Stale files to REPLACE (made from the wrong seed in an earlier run): ${RUST}\\src\\bin\\last_route\\world\\route\\ (mod.rs, regions.rs), and ${RUST}\\tests\\parity\\regions-demo.json, regions-k7qxm.json, regions-parity.json. Rebuild them from the golden dumps above.

Rules: do not edit Godot except nothing (the harness already exists). Do not commit. Use Node 24 on PATH for JSON work. Report every file you change.`

const PORT = {
  type: 'object',
  properties: {
    summary: { type: 'string', description: 'what was ported and how it is wired, in a few sentences' },
    files_changed: { type: 'string', description: 'the files changed, one per line, separated by semicolons' },
    region_parity: { type: 'string', description: 'number of chunks matching the Godot region field, per seed and level' },
    open_issues: { type: 'string', description: 'anything not matching or not done, with coordinates; say none if none' },
  },
  required: ['summary', 'files_changed', 'region_parity', 'open_issues'],
}

const VERIFY = {
  type: 'object',
  properties: {
    checks: { type: 'string', description: 'each check you ran, one per line, with its numbers' },
    passed: { type: 'boolean' },
    notes: { type: 'string' },
  },
  required: ['checks', 'passed', 'notes'],
}

phase('Port')
const port = await agent(`${CONTEXT}

Task: stage 1 port in the Rust project at ${RUST}.
1. Seed plumbing: the infinite route's world seed must be str(run_seed) with run_seed = maxi(1, abs(djb2(room text))), using the same djb2 as Godot (32-bit). Add a helper for it (src\\bin\\last_route\\world\\rng.rs or src\\bin\\last_route\\world\\route\\mod.rs). Thread the room text through the run start so the infinite route can get it. Find where the run starts (src\\bin\\last_route\\app\\mod.rs, src\\bin\\last_route\\game\\mod.rs new_infinite and new_infinite_sim, src\\bin\\last_route\\game\\endless.rs). Keep the decimal run seed for everything else (saves, hunters). The room text the Rust port shows is currently "ROOM-{code}-{8 hex}"; change it to the Godot format "ROOM-<code>-<20 hex>" only if you can do it without breaking the menus, otherwise keep the room text and say so.
2. Region module: rebuild src\\bin\\last_route\\world\\route\\regions.rs (and mod.rs) as a Rust port of the Godot region rules above, reproducing the same RNG draws in the same order. Route the infinite route's region choice through it.
3. Golden test data: from each golden dump, extract a small file tests\\parity\\regions-<tag>.json with the list of {coord, region} for levels 0,-1,-2,-3 and radius 3 (tags: parity, k7qxm, demo). Keep each file small. Overwrite the stale ones.
4. Parity test: a Rust test reads each regions-<tag>.json and checks the Rust region for each coord, using the world seed for that tag (the decimal of djb2 of its room text). Report matches per seed and level, and list any mismatch with coordinates.
5. Run cargo test --lib and report the counts.
Owner files: src\\bin\\last_route\\world\\route\\* , src\\bin\\last_route\\world\\infinite.rs, src\\bin\\last_route\\world\\mod.rs, src\\bin\\last_route\\world\\rng.rs (only to add the seed helper), src\\bin\\last_route\\game\\endless.rs and src\\bin\\last_route\\game\\mod.rs where the seed plumbing needs it, src\\bin\\last_route\\app\\mod.rs where the run start passes the room text, tests\\parity\\*.`, {
  label: 'port-regions', phase: 'Port', schema: PORT,
})

phase('Verify')
const verify = await agent(`${CONTEXT}

Port report (JSON): ${JSON.stringify(port)}

Task: verify stage 1 in ${RUST}. Do not change code unless a check fails with a clear cause; if you change code, say exactly what and why.
1. cargo build --bins and cargo build --release --bins: pass or fail.
2. cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server: counts.
3. Headless suites: "${RUST}\\target\\debug\\last_route.exe" --test-suite "tests/*.json" --out ${SCRATCH}\\suite.json, and the RESULT line.
4. Recount region parity yourself: for each golden dump in ${GOLD}, compare data.region with the Rust region. If you cannot run the Rust side outside tests, use the regions-<tag>.json files and say so. Report the chunks checked and the number matching, per seed and level.
Report each check with its numbers. Do not claim anything you did not run.`, {
  label: 'verify-regions', phase: 'Verify', schema: VERIFY,
})

return { port, verify }
