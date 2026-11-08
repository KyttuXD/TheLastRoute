// edit-pass: the standard Last Route change pass. Pass the task list as args:
//   Workflow({ name: 'edit-pass', args: { label: 'pass 30', extraRules: '...', tasks: [{ n: 1, files: '...', task: '...' }, ...] } })
// Stages: Edit (sonnet, low effort, no cargo) -> Integrate -> QA (three read-only checkers: data and format, compile read,
//   test read; they never build) -> Build (ONE agent compiles and fixes compile errors) -> Test (ONE agent runs the built
//   tests and fixes what the pass broke) -> Review (adversarial, read-only). At most two agents compile, one after the
//   other (the agent build rules (global CLAUDE.md, and AGENTS.md for this repo)).
// Why: edit agents never compile, so nothing fights over the target directory and the PC does not lag; QA catches typos,
// bad JSON and stale tests before the slow stages.
export const meta = {
  name: 'edit-pass',
  description: 'Standard Last Route pass: parallel edit agents (sonnet low, no cargo), read-only QA, one build-and-fix agent, one test-and-fix agent, then a read-only review. Args: { label, extraRules, tasks: [{ n, files, task }] }',
  phases: [{ title: 'Edit' }, { title: 'Integrate' }, { title: 'QA' }, { title: 'Build' }, { title: 'Test' }, { title: 'Review' }],
}

const REPO = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const DOCS = 'C:\\Users\\Kyttu\\Documents\\Projects\\Docs'

const tasks = (args && args.tasks) || []
const label = (args && args.label) || 'pass'
const extra = (args && args.extraRules) || ''
// args.skipNightWatch: leave src/bin/night_watch out of the build and test stages (KT: ignore Night Watch during the stairs pass)
const NW = !(args && args.skipNightWatch)
// One build folder shared by every pass (not the main target/): it stays incremental from pass to pass. A fresh folder per
// pass rebuilt everything from scratch (4-5 minutes per command) and left a full build behind each time.
const TARGET = 'target/pass'

const RULES = `Repo: ${REPO} (git repo; commit nothing, stage nothing, no git add/commit/checkout/stash/reset/push, no deploys, no SSH, no VPS changes).
Other agents edit the tree at the same time. Edit only the files named in your task. For a shared file, make the smallest additive edit and re-read git diff for that file just before you edit it. Never revert changes you did not make.
Do NOT touch src/bin/night_watch/ or packaging/ unless your task names them. src/bin/lr_server/ and .github/ only when named.
EVERYTHING TUNABLE IS DATA: numbers, timings, lists, per-hunter, per-difficulty and per-region settings go in JSON under src/bin/last_route/data/, loaded through the existing content loaders and embedded by build.rs. Code holds behaviour only.
Tests: write a test per feature (a changed data value changes behaviour; Standard values match the game). Update any test your change breaks without weakening what it checks. Tests should check behaviour and ranges, not incidental exact numbers, unless the number is a Godot-parity value.
References: Godot ${GODOT}; offline docs ${DOCS} (INDEX.md first; stack-cheatsheets\\rust.md for Rust and macroquad).
Keep each file's line endings. Never print or log tokens, passwords or download keys.
${extra}`

const NO_CARGO = `Do NOT run cargo (no build, check or test) in this stage. Check by reading and by git diff on your own files.`

// Shared hub files: many features need a line in them. Edit agents never touch them (two agents editing one file at the
// same time is what broke braces in game/hunting.rs). They list each needed change under "hooks"; one Integrate agent
// applies all hooks in a single pass.
const HUBS = 'game/mod.rs, game/hunting.rs, game/endless.rs, game/interact.rs, game/npc.rs (unless it is your named file), net/world_sync.rs, ui/menus.rs, app/mod.rs, content/mod.rs, hunters/brain.rs, hunters/mod.rs, world/mod.rs, render/scene.rs, render/world_mesh.rs, config/mod.rs (all under src/bin/last_route/) unless a hub file is named in your own task'
const HUB_RULE = `HUB FILES: do NOT edit these shared files unless your task names them: ${HUBS}. Put your feature in NEW files (new module, new data JSON, new tests) and list every change you need in a hub file under "hooks", as "file | anchor (the function or line to place it next to) | the exact code to add". An Integrate agent applies all hooks after the edit stage.`

const SCHEMA = {
  type: 'object',
  properties: { done: { type: 'string' }, files: { type: 'array', items: { type: 'string' } }, tests_written: { type: 'string' }, hooks: { type: 'array', items: { type: 'string' } }, open: { type: 'string' } },
  required: ['done', 'files', 'tests_written', 'hooks', 'open'],
}

phase('Edit')
const built = await parallel(tasks.map(t => () => agent(`${RULES}
${NO_CARGO}
${HUB_RULE}

Task ${t.n} of ${tasks.length}. Own these files only: ${t.files}.
${t.task}

Steps: git diff your files; implement; update tests your change breaks; write new tests; do not run cargo. Return what you did, the files and lines, the tests, and what is still open.`, { label: `task ${t.n}`, phase: 'Edit', model: 'sonnet', effort: 'low', schema: SCHEMA })))

phase('Integrate')
const allHooks = built.filter(Boolean).flatMap(b => (b.hooks || []).map(h => `task result: ${h}`))
const integrate = await agent(`${RULES}
${NO_CARGO}
You are the single integrator. The edit agents did not touch the shared hub files (${HUBS}); they listed the changes they need. Apply every hook below to its file in ONE pass, in this order: declarations and module lists first (mod lines, pub use, struct fields and constructors), then function bodies, then tests. Resolve overlaps between hooks (two agents asking for the same line, or the same function). Keep each file's line endings and brace balance: after each file, re-read the edited region and count braces. If a hook is unclear or its anchor is gone, skip it and list it under "skipped". Add nothing that is not in a hook, except the minimal glue to make hooks compile together (imports, a missing field in a constructor).
Return: files edited, hooks applied, hooks skipped with the reason.

Hooks (${allHooks.length}):
${allHooks.join('\n') || '(none)'}

Task reports (for context):
${JSON.stringify(built)}`, { label: 'integrate hooks', phase: 'Integrate', model: 'sonnet', effort: 'low' })

const reports = JSON.stringify(built) + '\nIntegrate: ' + String(integrate)

phase('QA')
const [dataQa, compileQa, testQa] = await parallel([
  () => agent(`${RULES}
${NO_CARGO}
READ-ONLY QA, data and format checker. Changes are in the tree. Do the following with node (not cargo): parse every .json under src/bin/last_route/data and src/bin/last_route/assets/hunters (report each file that fails to parse); for each data file changed in git status, check its keys against the loader that reads it (unknown keys are errors in several loaders); check line endings are consistent per file (no mixed CRLF and LF inside one file in git diff); look for leftover debug prints (println!, dbg!) in changed Rust files. Fix NOTHING. Return a list of problems with file and line.

Task reports:
${reports}`, { label: 'QA: data and format', phase: 'QA', model: 'haiku', effort: 'low' }),
  () => agent(`${RULES}
${NO_CARGO}
READ-ONLY QA, compile reader. Read git diff for every changed .rs file and find what would not compile: missing imports, wrong paths (super:: versus crate::), renamed items with stale call sites, test functions that return a value, borrow errors that are obvious, missing match arms for new enum variants, tests in one module using private items of another. Fix NOTHING. Return each problem with file and line and the one-line fix.

Task reports:
${reports}`, { label: 'QA: compile read', phase: 'QA', model: 'haiku', effort: 'low' }),
  () => agent(`${RULES}
${NO_CARGO}
READ-ONLY QA, test reader. Find tests (in changed files and in tests/*.json) that the changes will break: assertions on old constants, on behaviours that were removed or changed, on exact difficulty lists, on strings that changed. Also flag tests that depend on HashMap order, wall-clock time or an unseeded random. Fix NOTHING. Return each with file and line and what the test should expect now.

Task reports:
${reports}`, { label: 'QA: test read', phase: 'QA', model: 'haiku', effort: 'low' }),
])

phase('Build')
const build = await agent(`${RULES}

You are the ONLY agent in this pass that compiles code before the test stage (the agent build rules (global CLAUDE.md, and AGENTS.md for this repo)). Use CARGO_BUILD_JOBS=4 and CARGO_TARGET_DIR=${TARGET} for every cargo call, one cargo at a time. Loop, at most 6 rounds:
1. cargo build --bin last_route
2. cargo test --no-run --lib --bin last_route
3. cargo check --target wasm32-unknown-unknown --bin last_route
4. cargo build --bin lr_server --features server
On a compile error, fix it in the file the compiler names with the smallest change that keeps the author's intent (the QA reports below often have the fix), then go again. Fix compile errors only: no refactors, no behaviour changes, no new features. Report every error you saw, each fix (file, line, what), and the final state of all four commands.

QA data/format:
${String(dataQa)}
QA compile read:
${String(compileQa)}
QA tests read:
${String(testQa)}`, { label: 'build and fix', phase: 'Build', model: 'sonnet', effort: 'low' })

phase('Test')
const tests = await agent(`${RULES}

You are the ONLY agent that runs tests in this pass, and it starts after the build agent finished (the agent build rules (global CLAUDE.md, and AGENTS.md for this repo)). CARGO_BUILD_JOBS=4, CARGO_TARGET_DIR=${TARGET}. Plain cargo test aborts on this PC.
1. Run the newest ${TARGET}/debug/deps/last_route-*.exe --test-threads=4 (the build agent built it with cargo test --no-run). Note every failure.
2. Run ${TARGET}/debug/last_route.exe --headless --test-suite "tests/*.json" once. Note every failure.
3. For each failure that this pass caused (not one of the known failures in the agent build rules (global CLAUDE.md, and AGENTS.md for this repo); rerun timing-flaky tests alone first): if the test pins behaviour the task deliberately changed, update the test to the new behaviour without weakening it; if the code is wrong, make the smallest fix. Then rebuild only with cargo test --no-run --lib --bin last_route (and cargo build --bin last_route for the headless suites) and rerun the affected tests. At most 4 rounds.
Report: pass, fail and ignored counts before and after, every change you made (file, line, why), and anything still failing with its first assertion.

Build: ${String(build)}`, { label: 'test and fix', phase: 'Test', model: 'sonnet', effort: 'low' })

phase('Review')
const review = await agent(`${RULES}
${NO_CARGO}
READ-ONLY adversarial review of the pass. Read git diff for every changed file. Check: a tunable left in code instead of data; a guest able to change hunter state; bots triggering the danger readout, heartbeat or chase cue; a Standard value changed without a Godot reason; a fixed-seed region output that changed; a lock held across a network call in lr_server; a test weakened just to pass; a hunter or bot that can get stuck; a per-frame allocation or scan added to a hot path; a wasm32 path using std::time, threads or the file system. Return blockers first.

Build: ${String(build)}
Tests: ${String(tests)}
Reports: ${reports}`, { label: 'adversarial review', phase: 'Review', model: 'haiku', effort: 'low' })

return { label, built, qa: { dataQa, compileQa, testQa }, build, tests, review }
