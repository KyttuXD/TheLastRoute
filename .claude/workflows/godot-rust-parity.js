export const meta = {
  name: 'godot-rust-parity',
  description: 'Compare the Rust port against the Godot original domain by domain, adversarially verify every mismatch, and write docs/GODOT_RUST_PARITY.md',
  whenToUse: 'Run after gameplay, feel or data changes to see where the Rust port still differs from the Godot original (read-only)',
  phases: [
    { title: 'Inventory', detail: 'five domain agents list Godot and Rust values side by side' },
    { title: 'Verify', detail: 'one skeptic per non-matching item re-reads both sides' },
    { title: 'Report', detail: 'write docs/GODOT_RUST_PARITY.md and return the confirmed list' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'

const STATUS = ['match', 'mismatch', 'unit_differs', 'rust_missing', 'godot_missing', 'unverified']

const INVENTORY = {
  type: 'object',
  properties: {
    items: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          name: { type: 'string' },
          godot_file: { type: 'string' },
          godot_value: { type: 'string' },
          rust_file: { type: 'string' },
          rust_value: { type: 'string' },
          status: { type: 'string', enum: STATUS },
          note: { type: 'string' },
        },
        required: ['name', 'godot_file', 'godot_value', 'rust_file', 'rust_value', 'status', 'note'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['items', 'gaps'],
}

const VERDICT = {
  type: 'object',
  properties: {
    name: { type: 'string' },
    confirmed: { type: 'boolean' },
    final_status: { type: 'string', enum: STATUS },
    explanation: { type: 'string' },
  },
  required: ['name', 'confirmed', 'final_status', 'explanation'],
}

const REPORT = {
  type: 'object',
  properties: {
    report_path: { type: 'string' },
    matches: { type: 'number' },
    confirmed_mismatches: { type: 'number' },
    summary: { type: 'string' },
  },
  required: ['report_path', 'matches', 'confirmed_mismatches', 'summary'],
}

const RULES = `You are a READ-ONLY auditor. Do not edit, create or delete files, and do not run git commands that change state. Do not run builds. Compare VALUES, and always state the unit (radians or degrees, pixels or metres, per second or per frame, seconds, cells, etc.) on both sides. A value that matches only after a unit conversion is "unit_differs", not "match". Cite file and line for every value.

Godot original (read-only reference): ${GODOT}
Rust port: ${RUST}`

const DOMAINS = [
  {
    key: 'mouse-camera',
    prompt: `Domain: mouse look and camera. Godot: game/player/player_controller_3d.gd (mouse look, _unhandled_input, _update_controller_look), game/autoload/settings.gd (DEFAULT_MOUSE_SENSITIVITY, MIN/MAX). Rust: src/app/mod.rs (look_radians, read_controls, MOUSE_LOOK_MULTIPLIER use), src/config/mod.rs (MOUSE_SENSITIVITY, SENSITIVITY_MIN/MAX, MOUSE_LOOK_MULTIPLIER, FIELD_OF_VIEW_DEG, CAMERA_HEIGHT), src/game/player.rs (look, pitch clamp). Note that Godot's relative mouse motion is in physical pixels and macroquad's is in logical pixels (physical divided by the display scale). Compare the effective radians per physical pixel at default settings on both sides, and the pitch clamp limits.`,
  },
  {
    key: 'movement-meters',
    prompt: `Domain: player movement, stamina, meters, lamp and crouch. Godot: game/player/player_controller_3d.gd, game/player/player_meters.gd. Rust: src/config/mod.rs, src/game/player.rs, src/game/meters.rs. Compare walk, sprint and crouch speeds, stamina drain and recovery, the lamp and battery numbers, and the camera bob amounts.`,
  },
  {
    key: 'hunters-difficulty',
    prompt: `Domain: hunters and difficulty profiles. Godot: game/hunters/ (brains, speeds, sight and hearing ranges), the difficulty profile definitions (search for relaxed, standard, hard, nightmare). Rust: src/config/difficulty.rs, src/hunters/brain.rs, src/hunters/director.rs, data/hunters/*.json, src/content/hunters.rs. Compare hunter speeds, sight and hearing radii, capture distances, and difficulty multipliers.`,
  },
  {
    key: 'route-generation',
    prompt: `Domain: the Infinite route generation and layout. Godot: game/infinite_route/streamed_route_generator.gd (CHUNK_SIZE, REGION_SIZE_CHUNKS, SPAWN_MAZE_SHARE, stair chances, relay and battery counts per chunk, hunter population steps). Rust: src/world/infinite.rs, src/world/facility.rs, src/game/endless.rs, src/world/fixtures.rs, mechanics.json (cites Godot constants). Compare each constant, plus the chunk window size (3x3 chunks of 45 cells) and the stair rules.`,
  },
  {
    key: 'items-audio',
    prompt: `Domain: items, loot weights and audio cue tuning. Godot: game/items/ (or wherever item definitions live), game/autoload/sound.gd, the audio cue tables. Rust: src/game/items.rs, src/content/loot.rs, data/loot/*.json, src/audio/sound.rs, src/audio/cues/table.rs. Compare item counts and loot weights, cue volumes and durations for the shared cues (footsteps, heartbeat, breath).`,
  },
]

phase('Inventory')
const inventories = await parallel(DOMAINS.map(d => () =>
  agent(`${RULES}\n\n${d.prompt}\n\nReturn one item per value you compared. Use status "unverified" when you could not read a value confidently.`, {
    label: `inventory:${d.key}`,
    phase: 'Inventory',
    schema: INVENTORY,
  }),
))

const items = []
const gaps = []
inventories.forEach((r, i) => {
  if (!r) { gaps.push(`${DOMAINS[i].key}: agent failed, no items`); return }
  r.items.forEach(it => items.push({ ...it, domain: DOMAINS[i].key }))
  r.gaps.forEach(g => gaps.push(`${DOMAINS[i].key}: ${g}`))
})
log(`${items.length} values inventoried, ${items.filter(i => i.status === 'match').length} matching at first look`)

const suspicious = items.filter(i => i.status !== 'match')
const CAP = 25
const toVerify = suspicious.slice(0, CAP)
if (suspicious.length > CAP) {
  gaps.push(`verification capped at ${CAP} of ${suspicious.length} non-matching items; the rest are unverified`)
  log(`verification capped: ${suspicious.length - CAP} items not verified`)
}

phase('Verify')
const verdicts = await parallel(toVerify.map(it => () =>
  agent(`${RULES}

Adversarially verify this claim. Default to refuting it unless both sides clearly support the stated status. Re-read the cited lines yourself; do not trust the inventory.

Item: ${it.name} (domain ${it.domain})
Godot: ${it.godot_file} = ${it.godot_value}
Rust: ${it.rust_file} = ${it.rust_value}
Claimed status: ${it.status}
Note from the inventory agent: ${it.note}

Return whether the claim is confirmed, the final status, and a short explanation with units.`, {
    label: `verify:${it.domain}:${it.name}`.slice(0, 80),
    phase: 'Verify',
    schema: VERDICT,
  }).then(v => ({ item: it, verdict: v })),
))

const checked = verdicts.filter(Boolean)
const confirmed = checked.filter(c => c.verdict && c.verdict.confirmed && c.verdict.final_status !== 'match')
const unverifiedCount = items.length - checked.length
log(`${confirmed.length} confirmed mismatches, ${checked.length} verified, ${unverifiedCount} unverified`)

phase('Report')
const report = await agent(`You are writing the parity report. Write it to ${RUST}\\docs\\GODOT_RUST_PARITY.md (create or overwrite that one file, nothing else). Use this structure:
- A title and the date of the run (today's date from your environment).
- A one-paragraph summary: how many values matched, how many confirmed mismatches, what was not verified.
- A table of CONFIRMED mismatches grouped by domain: name, Godot value (file:line), Rust value (file:line), status, explanation.
- A list of unit_differs items with the conversion that makes them agree.
- A list of gaps and unverified items.
- A "How to re-run" section: "Workflow godot-rust-parity" from the project folder.

Data (JSON): ${JSON.stringify({ matches: items.filter(i => i.status === 'match').length, confirmed, gaps, unverifiedCount, totalItems: items.length })}

Return the report path, the match count, the confirmed mismatch count and a one-sentence summary.`, { label: 'report', phase: 'Report', schema: REPORT })

return {
  totals: { items: items.length, matches: items.filter(i => i.status === 'match').length, confirmed: confirmed.length, unverified: unverifiedCount },
  confirmed: confirmed.map(c => ({ domain: c.item.domain, name: c.item.name, godot: `${c.item.godot_file} = ${c.item.godot_value}`, rust: `${c.item.rust_file} = ${c.item.rust_value}`, status: c.verdict.final_status, explanation: c.verdict.explanation })),
  gaps,
  report,
}
