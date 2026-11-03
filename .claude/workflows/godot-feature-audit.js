export const meta = {
  name: 'godot-feature-audit',
  description: 'List every feature of the Godot original, mark what the Rust port has (ported, partial, missing, not applicable), verify the claims, and write docs/GODOT_FEATURE_AUDIT.md with TODO-ready items',
  whenToUse: 'Use to see how far the Rust port is from the Godot original, or to build the TODO list of unported features',
  phases: [
    { title: 'Inventory', detail: 'ten area auditors list Godot features and find their Rust counterpart' },
    { title: 'Verify', detail: 'one skeptic per ported or partial claim, capped, checks the Rust evidence' },
    { title: 'Report', detail: 'write docs/GODOT_FEATURE_AUDIT.md and return the TODO-ready items per area' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\feature-audit'
const STATUS = ['ported', 'partial', 'missing', 'not_applicable']

const RULES = `You are a READ-ONLY auditor. Do not edit or create files in ${GODOT} or ${RUST}, except the single report file named in your task. Scratch files go under ${SCRATCH}. Do not run cargo or Godot. Read the code, and the Godot docs under ${GODOT}\\docs\\*.md where they describe a feature. Cite file paths (and line numbers where you can) for every claim.

Definitions for rust_status:
- ported: the Rust port has the same player-facing behaviour in the same game path (search ${RUST}\\src, ${RUST}\\data and ${RUST}\\docs).
- partial: some of the behaviour exists (for example a constant, a simpler version, or the function without the UI), and you can say what is missing.
- missing: no Rust counterpart that you could find after searching by name and by behaviour.
- not_applicable: a Godot-only or platform-only thing the port does not need (for example XR headset support, editor tooling, Godot import pipeline, deploy scripts).

Be concrete: one feature per player-facing capability or system, not one per file. Give each feature a stable id made from its area and a short slug.`

const AREAS = [
  { key: 'night-watch-a', prompt: `Area: Night Watch, part A. Godot folders: ${GODOT}\\game\\night_watch\\ root files and the subfolders admin, bedroom, data, dayshift and descent. This is a whole separate game mode; the Rust port has no playable Night Watch (only a placeholder in ${RUST}\\src\\bin\\night_watch\\main.rs and a cue table in src\\audio\\cues\\nw.rs). Expect most rust_status to be missing; still list each mode, menu, room, mechanic, hunter pose or sound family as its own feature.` },
  { key: 'night-watch-b', prompt: `Area: Night Watch, part B. Godot folders: ${GODOT}\\game\\night_watch\\ subfolders lure, mask, mixed, office, openair and story. This is a whole separate game mode; the Rust port has no playable Night Watch. List each mode, mechanic, mask, office, story beat and sound family as its own feature.` },
  { key: 'hunters', prompt: `Area: hunters. Godot folder: ${GODOT}\\game\\hunters\\ (ai, gimmicks, view and the root files). Rust: ${RUST}\\src\\hunters, src\\game\\hunting.rs, data\\hunters. List each hunter's behaviour and abilities (one feature per hunter gimmick, per AI behaviour, per view and presentation piece), and the dossier or roster features.` },
  { key: 'world', prompt: `Area: world and generation. Godot folder: ${GODOT}\\game\\world\\ (collision, generation, interactables, regions, streaming). Rust: ${RUST}\\src\\world, src\\game, src\\render\\world_mesh.rs, data\\regions and data\\rooms. Cover room generation, the regions and their rules, interactables, collision, and streaming.` },
  { key: 'infinite-player-main', prompt: `Area: the Infinite route, player and main loop. Godot folders: ${GODOT}\\game\\infinite_route\\, game\\player\\ (14 files), game\\look\\ (17 files) and game\\main_3d.gd. Rust: ${RUST}\\src\\game (player.rs, endless.rs, interact.rs), src\\bin\\last_route\\app. Cover movement, stamina, meters, look and camera, crouch, hiding, the Infinite route's streaming and stairs, and the finite route if any remains.` },
  { key: 'hud-menus', prompt: `Area: HUD, menus and UI. Godot folders: ${GODOT}\\game\\hud\\, game\\ui_common\\, game\\menus\\. Rust: ${RUST}\\src\\bin\\last_route\\ui. Cover each screen (title, hub, room lobby, pause, settings tabs, account, run report, death screens), each HUD element, the map, the dossier, and the accessibility or scheme options.` },
  { key: 'network', prompt: `Area: network and XR. Godot folders: ${GODOT}\\game\\network\\ (18 files) and game\\xr\\ (8 files). Rust: ${RUST}\\src\\bin\\last_route\\net (orphaned, not compiled) and src\\bin\\lr_server (the server). Cover accounts, rooms, saves, the relay, chat, voice, the online room flow, and the server features. XR is probably not_applicable; say so with reasons.` },
  { key: 'config-autoload-boot', prompt: `Area: configuration, autoload systems, boot and mods. Godot folders: ${GODOT}\\game\\config\\ (32 files), game\\autoload\\ (14), game\\boot\\ (3), game\\mods\\ and game\\story\\. Rust: ${RUST}\\src\\config, src\\content, src\\shared, src\\bin\\last_route\\app\\settings.rs, data\\. Cover settings persistence, the difficulty profiles, the data-driven content, mod packs, the story data, the save and checkpoint system, and the boot flow.` },
  { key: 'admin-audio', prompt: `Area: admin tools and audio. Godot folders: ${GODOT}\\game\\admin\\ and game\\audio\\, plus docs\\ADMIN_TOOLS.md and docs\\AMBIENCE.md. Rust: ${RUST}\\src\\bin\\last_route\\ui\\admin, src\\audio. Cover each admin tab and tool, the audio buses, the cue families, ambience beds, music and the music box.` },
  { key: 'docs', prompt: `Area: the Godot design docs. Read every file under ${GODOT}\\docs\\*.md (about 81 files) and list the features they describe that are not already covered by the code areas above (the Godot docs for NPC teammates, blocky hunters, rest and sleep, breach, the carnival booths, the death scenes, the flythrough and so on). For each, check the Rust port. Use feature ids that start with "doc-". Do not duplicate features that another area is clearly covering; cross-reference by name instead.` },
]

const FEATURE = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    area: { type: 'string' },
    name: { type: 'string' },
    description: { type: 'string' },
    godot_files: { type: 'array', items: { type: 'string' } },
    rust_status: { type: 'string', enum: STATUS },
    rust_evidence: { type: 'string' },
    priority: { type: 'string', enum: ['core', 'important', 'polish', 'skip'] },
    notes: { type: 'string' },
  },
  required: ['id', 'area', 'name', 'description', 'godot_files', 'rust_status', 'rust_evidence', 'priority', 'notes'],
}

const INVENTORY = {
  type: 'object',
  properties: {
    features: { type: 'array', items: FEATURE },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['features', 'gaps'],
}

const VERDICT = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    confirmed_status: { type: 'string', enum: STATUS },
    explanation: { type: 'string' },
  },
  required: ['id', 'confirmed_status', 'explanation'],
}

const REPORT = {
  type: 'object',
  properties: {
    report_path: { type: 'string' },
    totals: {
      type: 'object',
      properties: {
        features: { type: 'number' },
        ported: { type: 'number' },
        partial: { type: 'number' },
        missing: { type: 'number' },
        not_applicable: { type: 'number' },
      },
      required: ['features', 'ported', 'partial', 'missing', 'not_applicable'],
    },
    todo_items: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          area: { type: 'string' },
          items: { type: 'array', items: { type: 'string' } },
        },
        required: ['area', 'items'],
      },
    },
  },
  required: ['report_path', 'totals', 'todo_items'],
}

phase('Inventory')
const results = await parallel(AREAS.map(a => () =>
  agent(`${RULES}\n\n${a.prompt}\n\nReturn one entry per feature. Use priority core for things the game cannot be played without, important for things a player would notice missing, polish for presentation, skip for things that do not matter to the port.`, {
    label: `inventory:${a.key}`,
    phase: 'Inventory',
    schema: INVENTORY,
  }),
))

const features = []
const gaps = []
results.forEach((r, i) => {
  if (!r) { gaps.push(`${AREAS[i].key}: agent failed, no features`); return }
  r.features.forEach(f => features.push(f))
  r.gaps.forEach(g => gaps.push(`${AREAS[i].key}: ${g}`))
})
const count = s => features.filter(f => f.rust_status === s).length
log(`${features.length} features: ${count('ported')} ported, ${count('partial')} partial, ${count('missing')} missing, ${count('not_applicable')} not applicable`)

phase('Verify')
const toVerify = features.filter(f => f.rust_status === 'ported' || f.rust_status === 'partial')
const CAP = 60
const checked = toVerify.slice(0, CAP)
if (toVerify.length > CAP) {
  gaps.push(`verification capped at ${CAP} of ${toVerify.length} ported or partial claims; the rest are unverified`)
  log(`verification capped: ${toVerify.length - CAP} claims not verified`)
}
const verdicts = await parallel(checked.map(f => () =>
  agent(`${RULES}

Adversarially verify this claim that the Rust port has the feature. Default to refuting it. Find the Rust code yourself and check that it does what the Godot code does, not just that a name matches.

Feature: ${f.name} (${f.id})
Godot: ${f.godot_files.join(', ')}
Description: ${f.description}
Claimed Rust status: ${f.rust_status}
Claimed evidence: ${f.rust_evidence}

Return the confirmed status: ported, partial or missing, with a short explanation.`, {
    label: `verify:${f.id}`.slice(0, 80), phase: 'Verify', schema: VERDICT,
  }).then(v => ({ feature: f, verdict: v })),
))
const confirmedById = {}
verdicts.filter(Boolean).forEach(v => { if (v.verdict) confirmedById[v.feature.id] = v.verdict })
features.forEach(f => {
  const v = confirmedById[f.id]
  if (v) f.verified_status = v.confirmed_status
})

phase('Report')
const report = await agent(`You are writing the Godot feature audit for the Rust port. Write ONE file: ${RUST}\\docs\\GODOT_FEATURE_AUDIT.md. Nothing else.

Structure:
- Title and a summary of totals (features, ported, partial, missing, not applicable), using verified_status where it exists and rust_status otherwise, and say which claims were verified.
- One section per area: a table of the features with status, priority and the Godot file.
- A "Gaps and unverified" section listing the gaps and the claims that were not verified.
- A "How to re-run" section: "Workflow godot-feature-audit" from the project folder.

Then return the TODO items: for each area, a list of short TODO lines (one per missing or partial feature with priority core or important), each as "Port <feature>: <one-line what is missing>". Skip not_applicable and skip-priority items. Keep the lines short.

Data (JSON): ${JSON.stringify({ features, gaps })}`, { label: 'report', phase: 'Report', schema: REPORT })

return {
  totals: report && report.totals,
  todo_items: report && report.todo_items,
  report: report && report.report_path,
  gaps,
}
