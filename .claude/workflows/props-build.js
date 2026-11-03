export const meta = {
  name: 'props-build',
  description: 'Draw Blender GLB props everywhere: port Godot per-type draw scale, decimate heavy originals to 520 triangles, detail blocky-only maze/office/mansion props into GLBs, then re-seat surfaces and verify the build',
  whenToUse: 'Use to carry out the prop plan from the props audit (originals everywhere, per-type scale, triangle budget, detailed blocky-only types)',
  phases: [
    { title: 'Build', detail: 'three agents on disjoint files: draw scale, decimation, blocky-to-GLB' },
    { title: 'Surface', detail: 're-measure the drawn surface tops and set the table heights to match' },
    { title: 'Verify', detail: 'build, test and check every prop path resolves and every GLB is under budget' },
  ],
}

const GODOT = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Godot\\KyttusLabApp'
const RUST = 'C:\\Users\\Kyttu\\Documents\\Projects\\LR Game Stuff\\LastRoute Rust'
const SCRATCH = 'C:\\Users\\Kyttu\\AppData\\Local\\Temp\\claude\\props-build'
const BLENDER = 'C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe'
const BUDGET = 520

const RULES = `You are implementing part of a props change in the Rust port at ${RUST}. Work only on the files your task names. Do not run git commands that change state, do not commit, and do not touch other files.
Scratch and backups go under ${SCRATCH} (create folders as needed). Godot reference (read it, do not edit it in this step): ${GODOT}.
Tools on this laptop: Node 24 on PATH (use it for JSON and GLB work); Blender 5.2 at "${BLENDER}" run headless as: "${BLENDER}" --background --factory-startup --python <script.py> -- <args>. Use "py -3" or the full Python path C:\\Users\\Kyttu\\AppData\\Local\\Programs\\Python314\\python.exe if you need Python; the plain "python" is a Store stub.
Triangle budget: ${BUDGET} triangles per prop model (Godot's MAX_ORIGINAL_TRIS). Count triangles as the number of polygon corners minus two, summed over mesh primitives.
Report: the exact files you changed, counts, and any failure with its cause. Never claim a result you did not measure.`

const DRAW_SCALE = `Task: port Godot's per-type draw scale into the Rust port.
Godot source of truth: ${GODOT}\\game\\config\\prop_model_catalog.gd, the const HUNTER_SCALE table (around line 832) and the function that returns float(HUNTER_SCALE.get(type, 1.0)) (around line 885). Read how room_catalog.gd (around line 665) and streamed_route_generator.gd (around lines 3231 and 3504) use it, so the Rust port draws each prop at the same scale Godot does.
Rust today: every prop model is drawn with the one constant PROP_MODEL_SCALE = 1.5 (src\\config\\mod.rs line 110), used in src\\render\\world_mesh.rs around lines 290-311 for GLB originals and the bbmodel fallback.
Do this:
1. Create a data file ${RUST}\\data\\render\\prop_draw_scale.json holding the Godot table as { "<prop type>": <scale> } with a "default": 1.0 entry. Do NOT place it under data\\props, which the prop loader reads. Check src\\content\\embedded.rs and build.rs to be sure a file under data\\render is embedded and readable at runtime.
2. Load it in Rust (a small module, for example src\\render\\prop_scale.rs, or in src\\content) and use the per-type scale in world_mesh.rs in place of PROP_MODEL_SCALE for both the GLB path and the bbmodel path, including the mount-offset maths that subtracts model.min.y times the scale.
3. Keep PROP_MODEL_SCALE only if something else still needs it; otherwise remove it. Do not change placement, collision or footprints.
4. Add a Rust unit test that loads the table and checks at least one entry that Godot draws at 1.0 and one that it draws at a different value.
5. Run: cd "${RUST}" then cargo test --bin last_route and report the result.
Owner files (only these): src\\render\\world_mesh.rs, src\\config\\mod.rs, src\\render\\ (new prop_scale module if needed), src\\content\\ (only if you need to load the table there), data\\render\\prop_draw_scale.json (new).`

const DECIMATE = `Task: reduce every original prop GLB above the budget, without changing the others.
1. List every .glb under ${RUST}\\assets\\props (recursively). Record each file's triangle count with Node. Only these existing files are yours; do not touch any .glb that does not exist yet.
2. Back up every file you will change to ${SCRATCH}\\backup-originals\\ with the same relative path, before changing it.
3. For each file over ${BUDGET} triangles, write a Blender Python script (under ${SCRATCH}) that imports the GLB with the glTF importer, and for each mesh object applies a Decimate modifier (collapse mode) with a ratio chosen so the file's total triangles land at or just under ${BUDGET}. Keep materials, textures and UVs. Then exports back to the same path as GLB (export_format GLB, texture embedded).
4. Verify each rewritten file with Node: triangle count <= ${BUDGET}, still exactly the same number of textures/images and materials as before, and the file loads (the GLB magic and JSON chunk parse). If a file fails, restore it from the backup and report it.
5. Report: how many files were over the budget, how many were changed, the before and after maximum triangle counts, and any file that failed or was restored.
Owner files: the existing .glb files under assets\\props only.`

const BLOCKY = `Task: give the blocky-only props in the maze, office and mansion sets real geometry as GLB files, and detail the six room-door types.
1. Work out which prop types have no GLB under assets\\props\\<set>\\ but do have a .bbmodel (the top-level assets\\props\\*.bbmodel files, which the Rust fallback draws as blocks), and which of those are placed in maze, office or mansion rooms. Use data\\props\\*.json (the "res://assets/..." model lists) and the room catalogue in data\\rooms\\catalog.json and src\\world\\catalog.rs to see placement. Put the list in your report.
2. For the six room-door types (gothic, luxury and normal room doors, and the double-room doors), check whether a GLB now exists under assets\\props\\mansion\\ (some were copied from Godot). If a door still has only a blocky model, detail it like the others. If it already has a GLB, check its triangle count and report it; leave it alone otherwise.
3. For each type that needs a GLB: convert its .bbmodel to glTF. Write a Node converter that reads the .bbmodel (cube elements with from/to and faces, or mesh vertices/faces) and writes a self-contained GLB with its texture or atlas embedded and correct UVs. Check the textured result with Blender before you trust it.
4. Detail it in Blender: smooth shading with auto-smooth, a small bevel on hard edges (about 1 to 2 cm at scale), and one subdivision level where the triangle count allows. Then decimate so it stays at or under ${BUDGET} triangles. Export to ${RUST}\\assets\\props\\<set>\\<type>.glb. Use the same set folder as the existing originals for that room purpose.
5. Point the prop's data entry at the new file: in data\\props\\*.json, replace that type's model list with the new "res://assets/props/<set>/<type>.glb" path. Change only the entries for the types you converted.
6. Report every type converted, its final triangle count, any type you could not convert and why, and the full list of blocky-only types you did not touch.
Owner files: the NEW .glb files you create under assets\\props\\<set>\\, the entries for converted types in data\\props\\*.json, and your scratch scripts. Do not edit .glb files that already existed.`

const SURFACE = `Task: set the surface heights so a child prop sits on the table it is drawn on.
Background: the placement height of a surface comes from its "surface_heights" value in ${RUST}\\data\\rooms\\catalog.json (used by src\\world\\catalog.rs surface_height, around lines 88-136). The drawn top is measured from the model that the Rust port now draws for that parent type, after the per-type draw scale (data\\render\\prop_draw_scale.json, being added by another agent) and the decimation and new GLB work (other agents). Their results may still be landing as you start, so check the files as they are now and say what you saw.
Do this:
1. For every parent type listed in surface_heights, find the GLB the port draws (data\\props\\*.json, first variant that exists) and measure its drawn top: the area-weighted height of up-facing triangles (normal y > 0.95), after node transforms, times the type's draw scale. Read the scale from data\\render\\prop_draw_scale.json (default 1.0 when the type is not listed).
2. Compare with the current surface_heights value. Where they differ by more than 1 cm, set the surface_heights value to the measured drawn top. Keep the rest of catalog.json unchanged, and keep the file's formatting.
3. Note any parent type with no GLB (a bbmodel fallback): measure the bbmodel element bounds the same way and say so.
4. Report a table: parent type, old value, new value, measured drawn top, and the largest child offset you would expect afterwards.
Owner files: ${RUST}\\data\\rooms\\catalog.json only.`

const VERIFY = `Task: verify the prop build in the Rust port at ${RUST}. Do not change code unless a check fails with a clear cause; if you fix something, say exactly what and why.
1. Run: cd "${RUST}" and cargo build --bins, then cargo build --release --bins. Report the result and any error.
2. Run: cargo test --lib, cargo test --bin last_route, cargo test --bin lr_server. Report counts.
3. Run the headless suites: "${RUST}\\target\\debug\\last_route.exe" --test-suite "tests/*.json" --out <scratch>\\suite.json, and report the RESULT line.
4. Check every prop model path in data\\props\\*.json resolves to a file under assets\\props (a file that exists). List any that do not.
5. Check every .glb under assets\\props is at or under ${BUDGET} triangles (use Node). List any over budget.
6. Check that data\\render\\prop_draw_scale.json is valid JSON, and that every prop type named in data\\props has a scale or uses the default.
7. Report pass or fail for each step, with the numbers. Do not claim anything you did not run.
Owner files: none. Read-only except scratch output.`

const FILE_REPORT = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    files_changed: { type: 'array', items: { type: 'string' } },
    counts: { type: 'string' },
    failures: { type: 'array', items: { type: 'string' } },
    details: { type: 'string' },
  },
  required: ['summary', 'files_changed', 'counts', 'failures', 'details'],
}

phase('Build')
const [drawScale, decimate, blocky] = await parallel([
  () => agent(`${RULES}\n\n${DRAW_SCALE}`, { label: 'draw-scale', phase: 'Build', schema: FILE_REPORT }),
  () => agent(`${RULES}\n\n${DECIMATE}`, { label: 'decimate-originals', phase: 'Build', schema: FILE_REPORT }),
  () => agent(`${RULES}\n\n${BLOCKY}`, { label: 'blocky-to-glb', phase: 'Build', schema: FILE_REPORT }),
])
log(`build: draw-scale ${drawScale ? 'done' : 'failed'}, decimate ${decimate ? 'done' : 'failed'}, blocky ${blocky ? 'done' : 'failed'}`)

phase('Surface')
const surface = await agent(`${RULES}\n\n${SURFACE}\n\nBuild results so far (JSON), for context:\n${JSON.stringify({ drawScale, decimate, blocky })}`, {
  label: 'surface-heights', phase: 'Surface', schema: FILE_REPORT,
})

phase('Verify')
const verify = await agent(`${RULES}\n\n${VERIFY}\n\nWhat the build agents reported (JSON), for context:\n${JSON.stringify({ drawScale, decimate, blocky, surface })}`, {
  label: 'verify', phase: 'Verify', schema: FILE_REPORT,
})

return { drawScale, decimate, blocky, surface, verify }
