# Agent workflow

How a standard change pass runs on this project. The script is `.claude/workflows/edit-pass.js`.
See also `docs/WORKFLOW_RULES.md` (running several workflows at once, gates).

## Stages

1. **Edit**: one agent per task, in parallel. Model sonnet, effort low. Each agent owns the files named in its task.
2. **Integrate**: one agent applies every hook (see below) to the hub files in a single pass.
3. **QA**: three read-only checkers (haiku, low): data and format, compile read, test read. They fix nothing.
4. **Build**: one agent runs the one cargo build (plus `cargo test --no-run`, `lr_server`, `night_watch`).
5. **Test**: at most three instances (haiku, low): unit tests, headless suites, `lr_server` and `night_watch` tests.
6. **Review**: one adversarial read-only reviewer.

## Rules for edit agents

- Sonnet low edits, haiku low checks. Nothing else runs on a bigger model unless the lead says so.
- **No cargo during edits.** Edit agents check by reading and `git diff` on their own files. This keeps agents from
  fighting over the target directory. One build, then at most three test instances, after every edit is in.
- Commit nothing, stage nothing, no deploys, no SSH.
- Everything tunable is data (JSON under `src/bin/last_route/data/`, embedded by `build.rs`). Code holds behaviour only.
- A test per feature: a changed data value changes behaviour, and Standard values match the game.

## Hub files and hooks

Hub files are shared by many features, so edit agents do not touch them unless their task names them:
`game/mod.rs`, `game/hunting.rs`, `game/endless.rs`, `game/interact.rs`, `game/npc.rs`, `net/world_sync.rs`,
`ui/menus.rs`, `app/mod.rs`, `content/mod.rs`, `hunters/brain.rs`, `hunters/mod.rs`, `world/mod.rs`,
`render/scene.rs`, `render/world_mesh.rs`, `config/mod.rs` (all under `src/bin/last_route/`).

Put the feature in NEW files (module, data JSON, tests). Every needed change in a hub file goes in the agent's
`hooks` list as:

```
file | anchor (the function or line to place it next to) | the exact code to add
```

The Integrate agent applies all hooks in one pass: declarations first, then function bodies, then tests. It resolves
overlaps, keeps line endings, and lists skipped hooks with the reason. `src/bin/night_watch/`, `packaging/`,
`src/bin/lr_server/` and `.github/` are off limits unless a task names them.

## Launching a pass

Call the Workflow tool with the script path and args:

```
Workflow({
  scriptPath: '.claude/workflows/edit-pass.js',
  args: {
    label: 'pass 30',
    extraRules: 'extra text added to every agent prompt',
    tasks: [ { n: 1, files: 'docs/X.md (new)', task: 'what to do' } ]
  }
})
```

Keep task file sets disjoint. Every edit agent returns what it did, files and lines, tests, hooks and what is open.

## Lesson: committing mid-pass

Other agents edit the tree while a pass runs. Do not commit from a file version you did not just read: a commit
made mid-pass from a stale or half-edited file captures someone else's partial work or drops their change. If a
commit is needed mid-pass, re-read the file and its `git diff` right before committing, commit only files whose
state you know, and never revert changes you did not make. Normally the lead commits once, after Review.
