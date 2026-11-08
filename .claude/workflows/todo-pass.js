// todo-pass: triage the whole TODO.md, plan one edit group of non-overlapping tasks (plus pinned tasks from args), run
// edit-pass on it (edit agents never compile; one build-and-fix, one test-and-fix), then tick finished items in TODO.md.
// Args: { label, extraRules, pinned: [{ files, task }], ranges: [[from, to], ...], maxTasks }
export const meta = {
  name: 'todo-pass',
  description: 'Triage TODO.md, plan one non-overlapping edit group (plus pinned tasks), run edit-pass on it, then tick finished TODO items',
  phases: [{ title: 'Triage' }, { title: 'Plan' }, { title: 'Pass' }, { title: 'TODO' }],
}

const REPO = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const label = (args && args.label) || 'todo pass'
const extra = (args && args.extraRules) || ''
const pinned = (args && args.pinned) || []
const ranges = (args && args.ranges) || [[1, 140], [141, 280], [281, 420], [421, 560], [561, 700], [701, 840], [841, 980], [981, 1200]]
const maxTasks = (args && args.maxTasks) || 24

const READ_ONLY = `Repo: ${REPO}. READ ONLY: edit nothing, run no cargo, no git writes. You may read files, grep and git log/diff.`

const TRIAGE = {
  type: 'object',
  properties: {
    items: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          line: { type: 'integer' },
          text: { type: 'string' },
          verdict: { type: 'string', enum: ['doable', 'already_done', 'blocked'] },
          why: { type: 'string' },
          files: { type: 'string' },
          plan: { type: 'string' },
        },
        required: ['line', 'text', 'verdict', 'why', 'files', 'plan'],
      },
    },
  },
  required: ['items'],
}

phase('Triage')
const triaged = await parallel(ranges.map(([a, b]) => () => agent(`${READ_ONLY}
Triage the OPEN items ("- [ ]", including indented sub-items) on lines ${a}-${b} of TODO.md. For each, decide:
- already_done: the code already does it (check the code; cite file:line in why).
- blocked: needs something an agent cannot do in code alone: KT's decision or design, a person at a device (Mac, MBA, phone, headset, second machine), a live server or VPS change, deploys, secrets or signing keys, GitHub billing, measuring on real hardware, or it lives in an off-limits area (src/bin/lr_story/, src/bin/lr_story_server/, src/bin/night_watch/ (Night Watch belongs to another agent), packaging/story-*, Cargo.toml, Cargo.lock, .gitignore, docs/LR_STORY.md). Say which in why.
- doable: an edit agent without cargo can implement it now in this repo (code, data, tests, docs, .github workflows). Give the files it would touch (repo-relative, specific) and a 1-3 sentence plan. Big items are fine if they can be done in one focused task; otherwise give the doable slice and say what stays open.
${extra}`, { label: `triage ${a}-${b}`, phase: 'Triage', effort: 'low', schema: TRIAGE })))

const items = triaged.filter(Boolean).flatMap(r => r.items)
const doable = items.filter(i => i.verdict === 'doable')
const already = items.filter(i => i.verdict === 'already_done')
log(`triage: ${items.length} items, ${doable.length} doable, ${already.length} already done, ${items.length - doable.length - already.length} blocked`)

const PLAN = {
  type: 'object',
  properties: {
    tasks: { type: 'array', items: { type: 'object', properties: { n: { type: 'integer' }, files: { type: 'string' }, task: { type: 'string' }, todo_lines: { type: 'array', items: { type: 'integer' } } }, required: ['n', 'files', 'task', 'todo_lines'] } },
    deferred: { type: 'array', items: { type: 'object', properties: { line: { type: 'integer' }, why: { type: 'string' } }, required: ['line', 'why'] } },
  },
  required: ['tasks', 'deferred'],
}

phase('Plan')
const plan = await agent(`${READ_ONLY}
Plan ONE edit group for the Last Route Rust repo. Edit agents run in parallel without compiling, so NO TWO TASKS MAY OWN THE SAME FILE. Shared hub files (game/mod.rs, game/hunting.rs, game/endless.rs, game/interact.rs, net/world_sync.rs, ui/menus.rs, app/mod.rs, content/mod.rs, hunters/brain.rs, hunters/mod.rs, world/mod.rs, render/scene.rs, render/world_mesh.rs, config/mod.rs under src/bin/last_route/) are edited by an Integrate agent from hooks, so tasks do not own them unless a pinned task names one.

PINNED tasks (keep them as tasks 1..${pinned.length}, word for word, with their files; no other task may own their files):
${JSON.stringify(pinned, null, 1)}

DOABLE TODO items (from triage; line numbers are TODO.md lines):
${JSON.stringify(doable, null, 1)}

Group the doable items into at most ${maxTasks - pinned.length} more tasks, each a coherent area with its own disjoint file set (use NEW files for new features). Each task text must be self-contained: list each TODO item it covers (quote the item, with its TODO.md line), what done looks like, and the tests to write. Put an item in 'deferred' only if it cannot fit without file overlap or is too large for one task (say why). Number tasks from 1 (pinned first). todo_lines: the TODO.md lines the task finishes.`, { label: 'plan', phase: 'Plan', schema: PLAN })

// Overlap guard: a file named by two tasks is left with the first task only, and the clash is logged.
const own = new Map()
for (const t of plan.tasks) {
  const files = t.files.split(',').map(s => s.trim()).filter(Boolean)
  const keep = []
  for (const f of files) {
    const k = f.toLowerCase().replace(/\\/g, '/').replace(/^.*lastroute rust\//, '')
    if (own.has(k)) log(`file clash: ${f} in task ${t.n} already owned by task ${own.get(k)}; dropped from task ${t.n}`)
    else { own.set(k, t.n); keep.push(f) }
  }
  t.files = keep.join(', ')
}
log(`plan: ${plan.tasks.length} tasks, ${plan.deferred.length} items deferred`)

phase('Pass')
const pass = await workflow({ scriptPath: `${REPO}\\.claude\\workflows\\edit-pass.js` }, {
  label,
  skipNightWatch: true,
  extraRules: extra + '\nWhen a task covers TODO.md items, do NOT edit TODO.md (a final agent ticks them); name each finished item (its text and line) in done, and say in open what is left of any item you only part-finished.',
  tasks: plan.tasks.map(t => ({ n: t.n, files: t.files, task: t.task })),
})

phase('TODO')
const ticked = await agent(`Repo: ${REPO}. Edit ONLY TODO.md (no cargo, no git writes, no other file).
Tick finished items in TODO.md: change "- [ ]" to "- [x]" and append " (done 2026-11-08)" for each item below that the pass finished, and for each item triage found already done (append " (already done: <file:line>)"). For a part-finished item, leave it open and append a short "(2026-11-08: <what is done, what is left>)". Match items by text, not only by line (lines may have shifted). Do not tick anything the build, test or review reports say was broken or reverted.
Planned tasks and their TODO lines: ${JSON.stringify(plan.tasks.map(t => ({ n: t.n, todo_lines: t.todo_lines })))}
Already done per triage: ${JSON.stringify(already.map(i => ({ line: i.line, text: i.text, why: i.why })))}
Edit-pass results (what each task finished, plus build/tests/review): ${JSON.stringify(pass).slice(0, 120000)}
Return a short list of what you ticked, part-ticked and left.`, { label: 'tick TODO', phase: 'TODO', effort: 'low' })

return { label, triage: { total: items.length, doable: doable.length, already: already.length, blocked: items.filter(i => i.verdict === 'blocked').map(i => ({ line: i.line, why: i.why })) }, plan, deferred: plan.deferred, pass, ticked }
