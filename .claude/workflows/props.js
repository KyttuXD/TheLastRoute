export const meta = {
  name: 'props',
  description: 'Audit the prop models: which model each type draws in Godot and in Rust, which surfaces float, and which types are blocky-only; then plan the fixes (read-only)',
  whenToUse: 'Use when props look wrong (floating on tables, blocky cabinets) or before changing prop models or placement',
  phases: [
    { title: 'Audit', detail: 'three read-only auditors: draw choice, surface heights, blocky-only inventory' },
    { title: 'Verify', detail: 'one skeptic per surface-height mismatch recomputes the bounds from the files' },
    { title: 'Plan', detail: 'one planner turns the verified findings into ordered fixes with owner files' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\props-workflow'

const RULES = `You are a READ-ONLY auditor. Do not edit or create files inside ${GODOT} or ${RUST}. You may write scratch scripts and output only under ${SCRATCH}. Do not run cargo or Godot builds. Node 24 is available for reading GLB (glTF binary) JSON chunks and bbmodel JSON; a GLB starts with the magic 0x46546C67, a 12-byte header, then a JSON chunk whose length is the uint32 at byte 12. Cite file and line for every claim, and state units (metres, model units, or pixels).`

const DRAW = {
  type: 'object',
  properties: {
    types: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          prop_type: { type: 'string' },
          godot_draws: { type: 'string', description: 'original | blocky | blocky-only | unknown, with the reason (deny list, triangle cap, switch)' },
          rust_draws: { type: 'string', description: 'glb | bbmodel | none, with the variant file' },
          mismatch: { type: 'boolean' },
          note: { type: 'string' },
        },
        required: ['prop_type', 'godot_draws', 'rust_draws', 'mismatch', 'note'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['types', 'gaps'],
}

const SURFACES = {
  type: 'object',
  properties: {
    surfaces: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          surface_type: { type: 'string' },
          placement_height: { type: 'string', description: 'the mount height the placement code uses, with source' },
          drawn_top_height: { type: 'string', description: 'the top of the model Rust actually draws for it, in metres, with the file' },
          godot_top_height: { type: 'string', description: 'the top Godot uses for the same surface, in metres' },
          offset_m: { type: 'string', description: 'drawn top minus placement height, signed' },
          status: { type: 'string', enum: ['matches', 'floats', 'sinks', 'unknown'] },
          note: { type: 'string' },
        },
        required: ['surface_type', 'placement_height', 'drawn_top_height', 'godot_top_height', 'offset_m', 'status', 'note'],
      },
    },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['surfaces', 'gaps'],
}

const BLOCKY = {
  type: 'object',
  properties: {
    types: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          prop_type: { type: 'string' },
          has_glb: { type: 'boolean' },
          glb_triangles: { type: 'string' },
          bbmodel_cubes: { type: 'string' },
          used_in: { type: 'string', description: 'room purposes or regions that place it, from data/rooms or the room catalogue' },
          detail_effort: { type: 'string', enum: ['low', 'medium', 'high', 'unknown'] },
          note: { type: 'string' },
        },
        required: ['prop_type', 'has_glb', 'glb_triangles', 'bbmodel_cubes', 'used_in', 'detail_effort', 'note'],
      },
    },
    tooling: { type: 'array', items: { type: 'string', description: 'what is available on this laptop for editing models (Blender, Blockbench headless, Node, etc.), checked by running commands' } },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['types', 'tooling', 'gaps'],
}

const VERDICT = {
  type: 'object',
  properties: {
    surface_type: { type: 'string' },
    confirmed: { type: 'boolean' },
    measured_top_m: { type: 'string' },
    measured_placement_m: { type: 'string' },
    explanation: { type: 'string' },
  },
  required: ['surface_type', 'confirmed', 'measured_top_m', 'measured_placement_m', 'explanation'],
}

const PLAN = {
  type: 'object',
  properties: {
    steps: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          title: { type: 'string' },
          owner_files: { type: 'array', items: { type: 'string' } },
          depends_on: { type: 'array', items: { type: 'string' } },
          description: { type: 'string' },
        },
        required: ['id', 'title', 'owner_files', 'depends_on', 'description'],
      },
    },
    decisions_for_user: { type: 'array', items: { type: 'string' } },
  },
  required: ['steps', 'decisions_for_user'],
}

phase('Audit')
const [draw, surfaces, blocky] = await parallel([
  () => agent(`${RULES}

Task: for every prop type in the Rust data (${RUST}\\data\\props\\*.json, the "res://assets/..." model lists), say which model the Rust port draws (glb, if a variant exists and parses; bbmodel otherwise), following src\\render\\world_mesh.rs around lines 270-330 and src\\content\\props.rs (originals(), the blocky check). Then say which model Godot draws for the same type, using ${GODOT}\\game\\config\\prop_model_catalog.gd (USE_BLOCKY_PROPS, PREFER_ORIGINAL_PROPS, MAX_ORIGINAL_TRIS, ORIGINAL_DENY and its reasons, the per-type MODEL_PATHS entries) and ${GODOT}\\docs\\ORIGINAL_PROPS.md if present. Flag mismatch=true when the two disagree on which family is drawn. Cover every type in data/props, not a sample. Include the cabinet types (arcadeCabinet, arcadeCabinetCyanMagenta) and the computer types (retroComputer*) explicitly.`,
    { label: 'draw-choice', phase: 'Audit', schema: DRAW }),
  () => agent(`${RULES}

Task: explain why computers float on tables. Find how placement picks the height of a prop mounted on a surface (Rust: src\\world\\facility.rs near lines 1080-1140, cat.surface_height in src\\world\\catalog.rs, and mount_y; Godot: the surface height in ${GODOT}\\game\\config\\prop_collider_boxes.gd or prop_collider_sizes.gd and the placement code). Then find the height of the surface model the Rust port actually draws: for GLB variants compute the top of the model from the accessor min and max of POSITION attributes and the node transforms (scale and translation), for bbmodel variants compute it from the element "from" and "to" bounds. Compare placement_height with drawn_top_height for each surface type that carries children (tables, desks, shelves, counters). Use node scripts under ${SCRATCH} to read the files. Also check the child model: src\\render\\world_mesh.rs line 283 and nearby subtracts model.min.y for mounted props, so compute what y the child's lowest vertex actually lands on.`,
    { label: 'surface-heights', phase: 'Audit', schema: SURFACES }),
  () => agent(`${RULES}

Task: inventory the blocky-only types. For every prop type in ${RUST}\\data\\props\\*.json and in the bbmodel files under ${RUST}\\assets\\props (top-level *.bbmodel): report whether a GLB exists in assets\\props\\<set>\\ (maze, mansion, beach, nether, prison, carnival, sewer), its triangle count (read with node from the GLB JSON), the bbmodel cube count (count "elements" in the .bbmodel), and which room purposes or regions place it (data\\rooms or the room catalogue in src\\world\\catalog.rs). Give detail_effort low/medium/high from the cube count and whether a GLB exists. Also check the tooling column: run commands to see whether Blender, Blockbench (headless MCP), Node and Python are available on this laptop; report what you actually found.`,
    { label: 'blocky-inventory', phase: 'Audit', schema: BLOCKY }),
])

const allSurfaces = (surfaces && surfaces.surfaces) || []
const floating = allSurfaces.filter(s => s.status !== 'matches')
log(`${(draw && draw.types || []).filter(t => t.mismatch).length} draw-choice mismatches, ${floating.length} surface types not matching`)

phase('Verify')
const CAP = 20
const checkList = floating.slice(0, CAP)
if (floating.length > CAP) log(`verification capped at ${CAP} of ${floating.length} surface types`)
const verdicts = await parallel(checkList.map(s => () =>
  agent(`${RULES}

Adversarially verify this claim by recomputing it yourself from the files. Default to refuting it.
Surface type: ${s.surface_type}
Claimed placement height: ${s.placement_height}
Claimed drawn top: ${s.drawn_top_height}
Claimed Godot top: ${s.godot_top_height}
Claimed offset: ${s.offset_m} (${s.status})
Auditor note: ${s.note}

Measure the drawn model top from its file (GLB accessor bounds with the node transform, or bbmodel element bounds), and the placement height from the placement code. Return whether the claim is confirmed with the measured numbers.`, {
    label: `verify:${s.surface_type}`.slice(0, 80), phase: 'Verify', schema: VERDICT,
  }).then(v => ({ surface: s, verdict: v })),
))
const confirmedFloats = verdicts.filter(Boolean).filter(v => v.verdict && v.verdict.confirmed)
log(`${confirmedFloats.length} surface types confirmed by measurement`)

phase('Plan')
const plan = await agent(`You are planning fixes for the prop rendering of the Rust port at ${RUST}. Use only the findings below; do not invent others. The user's goal: props should look like the original Blender models (GLB), not blocky boxes, and computers and other surface props must sit on the table that is drawn. Where a type is blocky-only, the plan should say whether to detail its bbmodel into a mesh or use a GLB.

Findings (JSON):
${JSON.stringify({ draw, blocky, confirmed_surface_findings: confirmedFloats.map(v => ({ surface: v.surface, verdict: v.verdict })), surface_gaps: surfaces && surfaces.gaps })}

Return ordered steps with owner_files (only the files a step may edit), depends_on, and a description. Put the questions that need the user's decision in decisions_for_user (for example whether to follow Godot's blocky/original split or to draw Blender GLBs everywhere, and whether blocky-only types should be detailed by hand or converted).`,
  { label: 'planner', phase: 'Plan', schema: PLAN })

return {
  draw_mismatches: (draw && draw.types || []).filter(t => t.mismatch),
  surface_findings: allSurfaces,
  confirmed_floats: confirmedFloats.map(v => ({ surface: v.surface.surface_type, top: v.verdict.measured_top_m, placement: v.verdict.measured_placement_m, explanation: v.verdict.explanation })),
  blocky_only: ((blocky && blocky.types) || []).filter(t => !t.has_glb),
  tooling: blocky && blocky.tooling,
  gaps: [...((draw && draw.gaps) || []), ...((surfaces && surfaces.gaps) || []), ...((blocky && blocky.gaps) || [])],
  plan,
}
