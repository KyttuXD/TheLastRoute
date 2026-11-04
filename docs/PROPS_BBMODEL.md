# Props as bbmodel (GLB to .bbmodel)

Standard: the Rust port uses Blockbench `.bbmodel` for world models. Godot's GLBs are converted with
`tools/props/glb_to_bbmodel.mjs` (Node built-ins only, no install step).

Mod packs use the same format: a pack prop model is a `.bbmodel` (texture embedded as a data URI, or a pack `.png` it names), read by the same `PropModel::parse` as the built-in props. `.glb` in a pack is legacy: it still loads through `PropModel::parse_glb`, and the server's pack path rule still accepts it, but new packs should ship `.bbmodel` (see MODDING_SPEC.md section 2.6).

## How to run

```sh
# one file
node tools/props/glb_to_bbmodel.mjs <in.glb> <out.bbmodel> [--name NAME] [--atlas-name FILE.png]
# a folder (add --recursive to descend)
node tools/props/glb_to_bbmodel.mjs --dir <glb_dir> --out <out_dir> [--recursive] [--quiet]
# regenerate every number in this page
node tools/props/fidelity_report.mjs [path/to/Godot/KyttusLabApp/assets]
```

Each conversion prints its cube and mesh counts, the bounding box, and a round-trip result. The round trip
re-reads the written bbmodel the way `src/bin/last_route/render/bbmodel.rs` does (a JS replica of the cube and mesh branches)
and checks that every source triangle is covered with the same position and UV, and that the total surface area
matches. The exit code is non-zero if any file fails.

## Conventions (copied from `tools/prop_convert` and the existing `src/bin/last_route/assets/props/*.bbmodel`)

- 16 bbmodel units per metre, Y up, no axis flip. glTF node transforms are baked in; every element has origin `[0,0,0]`.
- UVs are atlas pixels (glTF u times width, v times height), `resolution` is the atlas size.
- The GLB's embedded PNG is the texture (data URI in `source`, `relative_path` = `<name>_atlas.png`; the loader
  falls back to the embedded copy when no file of that name is found).
- An axis-aligned box whose faces each cover one whole side and have rectangular UVs is written as a `cube`
  (`from`/`to`, per-face `uv`, `rotation` 0/90/180/270 as the loader reads it). Missing sides get `texture: null`.
- Everything else is a `mesh` element (vertices plus faces); triangle pairs are merged back into quads.

## The five Godot-only models

| Model | Written to | GLB bbox (m) | bbmodel bbox (m) | GLB tris | bbmodel tris | Cubes / mesh elements | Texture | Round trip |
|---|---|---|---|---|---|---|---|---|
| door_frame | `src/bin/last_route/assets/environment/door_frame.bbmodel` | 0.92 x 2 x 0.17 | 0.92 x 2 x 0.17 | 120 | 120 | 4 / 2 | 256x256 PNG, identical | OK |
| floor_tile | `src/bin/last_route/assets/environment/floor_tile.bbmodel` | 1 x 0.1 x 1 | 1 x 0.1 x 1 | 72 | 72 | 0 / 2 | 256x256 PNG, identical | OK |
| wall_block | `src/bin/last_route/assets/environment/wall_block.bbmodel` | 1 x 2.4 x 0.23 | 1 x 2.4 x 0.23 | 120 | 120 | 10 / 0 | 256x256 PNG, identical | OK |
| kathy_scythe | `src/bin/last_route/assets/props/hunter_items/kathy_scythe.bbmodel` | 0.08 x 1.81 x 0.718 | 0.08 x 1.81 x 0.718 | 144 | 144 | 4 / 1 | 64x128 PNG, identical | OK |
| sewer_outflow_pipe_broken | `src/bin/last_route/assets/props/sewer/sewer_outflow_pipe_broken.bbmodel` | 1.9 x 1.998 x 1.57 | 1.9 x 1.998 x 1.57 | 516 | 516 | 14 / 13 | 128x16 PNG, identical | OK |

Nothing in the Rust source loads these five yet. They are data only; wiring them in is a separate step.

### Scale against existing props

Running the converter on the Godot `_0` GLB of props the port already ships gives the same min/max corners (in bbmodel
units) as the existing `src/bin/last_route/assets/props/<name>.bbmodel`:

| Prop | Min / max (units) | Same as existing |
|---|---|---|
| arcadeCabinet | -5,0,-5 / 5,29,7 | yes |
| diningChair | -4,0,-4 / 4,15,3 | yes |
| bedroomRug | -14,0,-9 / 14,1,9 | yes |
| labBench | -16,0,-6 / 16,23,6 | yes |
| gothicRoomDoor | -10,0,-13.73 / 10,46,2 | yes |
| ivStand | -4,0,-4 / 4,29,4 | yes |

So the five new models sit in the same space: 1 m = 16 units, floor at y = 0.

### Variants (arcadeCabinet_0 / _1 / _2)

Converted to `tools/props/out/variants/arcadeCabinet_0.bbmodel`, `_1`, `_2` (temp output, not shipped). Each is
0.625 x 1.813 x 0.75 m, the same as the existing single `src/bin/last_route/assets/props/arcadeCabinet.bbmodel`, uses the same atlas PNG
(`blocky_atlas.png`), and round-trips OK. Each variant comes out as 1 cube plus 1 mesh element (56 faces), so the
existing file (all mesh) and the variants differ only in how the box parts are stored, not in size.

## What is not representable (or is lossy)

- Skinning, bones, morph targets and animations in the GLB: ignored. Only static triangle geometry is read.
- Vertex colours, normals and tangents: dropped (the loader derives normals from winding). Smooth shading becomes flat.
- Materials: only the first embedded PNG is kept. Multiple images, non-embedded images, PBR channels, emissive and
  alpha modes are not carried over; a GLB with a non-PNG or external image is rejected.
- Sparse accessors and non-triangle primitives (lines, points, strips, fans) are rejected.
- A cube can only be axis-aligned, outward-facing, one face per side, with a rectangular UV that is the atlas
  rectangle optionally mirrored or turned in 90 degree steps. Anything else (bevels, chamfers, tilted boxes,
  sheared UVs, flat single quads, open shells) goes to a mesh element.
- Cubes in this loader use the per-face UV rotation the Rust loader implements; Blockbench's own viewer may
  rotate a UV the other way round. Check a rotated face in Blockbench before relying on it there.
- Zero-area sliver triangles are dropped (they draw nothing). Vertices are rounded to 1e-4 units.
- Each bbmodel embeds its own copy of the atlas (about 15 to 50 KB), so converting hundreds of colour variants
  duplicates the texture per file.

## Sizing up a bulk conversion (the 430 GLBs still in `assets/`)

All 430 GLBs convert with no failures and no round-trip mismatch.

| Folder | GLBs | Pure cubes | Cubes plus mesh | Mesh only | Cubes | Mesh faces |
|---|---|---|---|---|---|---|
| interactables | 25 | 0 | 9 | 16 | 53 | 6450 |
| props/beach | 8 | 0 | 4 | 4 | 8 | 1907 |
| props/carnival | 6 | 0 | 6 | 0 | 70 | 588 |
| props/mansion | 256 | 0 | 42 | 214 | 150 | 54530 |
| props/maze | 102 | 0 | 69 | 33 | 330 | 21591 |
| props/nether | 9 | 0 | 1 | 8 | 1 | 2012 |
| props/prison | 11 | 0 | 1 | 10 | 2 | 2854 |
| props/sewer | 13 | 0 | 11 | 2 | 146 | 1930 |
| Total | 430 | 0 | 143 | 287 | 760 | 91862 |

No shipped GLB is a pure-cube model. About 5 percent of faces end up in cubes; the rest are mesh faces. The shipped
GLBs are bevelled and rounded rather than strict blocks (a mansion body is about 44 triangles where a plain box is
12), which is why boxes do not survive as cubes there. The maze, carnival and sewer folders are the blockiest.

Sample (every 30th GLB, all round-trip OK): backpack_large 0 cubes / 13 mesh; sand_castle 1 / 32; bedroom_nightstand 0 / 8;
bedside_lamp 0 / 5; gothic_bedroom_bench_crimson 0 / 11; gothic_highback_chair_forest 0 / 11; gothic_wardrobe_forest 0 / 14;
lounge_couch_ivory_wide 0 / 12; mansion_tea_cart 0 / 13; single_bed_burgundy 0 / 12; arcade_cabinet_teal 10 / 9;
office_cubicle_partition_charcoal 0 / 6; retro_computer_compact_teal 9 / 49; wooden_dresser_sideboard_ash 3 / 12.

Recommendation: bulk convert, as mesh elements. Geometry, UVs and scale survive exactly (same round trip as the
existing 351 mesh props), and the loader already reads mesh elements, so there is no visual loss and no
reason to keep GLB as a second format. Do not expect cubes to make the files hand-editable: for mansion props
the result is a mesh, editable in Blockbench's mesh tools only. The one cost is size: about 10 percent larger (maze folder: 5.0 MB bbmodel vs 4.5 MB GLB, since the
atlas is embedded per file), so for the 256 mansion colour variants consider a shared atlas file
via `--atlas-name` so `relative_path` resolves to one PNG. Keep GLB only for anything that needs skinning or
animation, which none of these 430 use (checked: 0 have animations, 0 have skins).

## Stage 1 status: every prop model has a bbmodel (2026-11-03)

`node tools/props/fidelity_report.mjs` (exit 0) prints, for every model, the GLB bbox, the bbmodel bbox (read the way the
Rust loader reads it), triangle counts and a match flag. A match needs the same bbox corners (3 mm), surface area (0.2 percent)
and area-weighted centroid (3 mm), so a model that is turned, mirrored or moved does not match.

| Folder | GLB models | With a bbmodel beside them |
|---|---|---|
| interactables | 25 | 25 |
| props/beach | 8 | 8 |
| props/blocky | 220 | 220 (new) |
| props/carnival | 6 | 6 |
| props/mansion | 256 | 256 |
| props/maze | 102 | 102 |
| props/nether | 9 | 9 |
| props/prison | 11 | 11 |
| props/sewer | 13 | 13 |
| Total | 650 | 650, all match |

- The prop data (`data/props/*.json`, 337 prop ids) names 628 models; every one has a bbmodel, as do the 25 interactables
  (relay, locker, lift, doors and the rest). `environment/` and `hunter_items/` were converted earlier (the five models above).
- The 220 `props/blocky/*.glb` were the only models missing from the Rust assets. They were copied from the Godot
  `assets/props/blocky` folder and converted with `--atlas-name blocky_atlas.png` (all 935 Godot blocky GLBs share the same 128x64
  atlas, byte for byte the same as `assets/props/blocky_atlas.png`). They total 44,140 triangles, at most 862 per model.
- Converter fix: a GLB whose materials use more than one image (`frozen_tideline_pool`, `office_crt_security_console`) used to
  read every UV against image 0, so the second material drew the wrong colours. The images are now stacked into one atlas
  (`encodePng` / `decodePng` in the converter) and each primitive's UVs move down by its image's offset. Checked by rendering
  `office_crt_security_console.bbmodel` (wood desk and grey console both coloured). The other 428 files regenerate byte for byte.
- Orientation, scale and pivot: no model is turned, rescaled or re-pivoted (the converter bakes node transforms and keeps
  Y up and the floor at y = 0, the same space the GLB loader draws in). The 319 one-per-prop `assets/props/<id>.bbmodel` files
  also match their Godot blocky `_0` GLB (same bbox, area and centroid), so the old complaints (floating computers, turned
  props, blocky arcade cabinets) do not come from conversion; they come from which model the loader picks (see below).

### Unresolved and notes for the loader stage

- Unresolved conversions: none. Every model converts and round-trips.
- `assets.rs` draws every prop bbmodel with the shared `props/blocky_atlas.png` and ignores the embedded texture. That is right
  for `props/blocky/*` and wrong for the maze, mansion, beach, carnival, nether, prison and sewer models (their atlases are
  per model). The loader must use the bbmodel's own texture for those.
- `assets.rs` keys the props map by file name under `props/`, so the new files appear as `blocky/<name>` and the others as
  `maze/<name>` and so on. They do not collide with the one-per-prop keys but they are parsed at startup (headless suite still
  passes 52/52, run time about 8.5 s).
- Not carried over (the Rust GLB loader does not draw them either): `sewer_lamp`'s emissive glow material and PBR factors.
- Not copied: 715 Godot blocky GLBs that no prop data names (other `_N` variants of props the data draws from authored models),
  and the 12 `hunters_blocky/void_*` GLBs named by `data/hunters/void_*.json` (hunters, not props; Godot has a ready
  `void_*.bbmodel` for each beside the GLB).
- The embedded assets grew by about 14 MB (220 GLBs at 6 MB plus their bbmodels at 8 MB). Dropping the GLBs at runtime (next stage)
  removes the GLB half.

## Stage 2 status: the game loads bbmodels only (2026-11-03)

The runtime no longer reads a `.glb` from its own assets. Every prop, fixture and hand-tool model is a `.bbmodel`.

### What the loader does now

- `data/props/*.json` name bbmodels (`res://assets/props/maze/locker.bbmodel`). A prop type draws one of its listed models, picked by a
  hash of the room id (`PropModelCatalog._variant_index`), for every type: the old split (authored GLB if it parsed, else the
  one-per-prop blocky bbmodel) is gone, so a blocky prop now draws the `_N` variant the data names, as Godot does.
- `render/prop_model.rs` (was `glb.rs`) reads a bbmodel into `PropModel`: the faces come from `bbmodel::Model::from_json`, one part per
  model (a model has one texture). Models that name the same atlas file share one texture and one draw batch (all of `props/blocky/*`
  share `props/blocky_atlas.png`, found beside the model or one folder up; every other model uses its embedded copy).
  Models in `props/blocky/` get the rounded normals (`soften`) the blocky rebuilds always had; the authored models keep flat normals,
  which is what their GLBs held (checked: 0 percent of the vertices of all 650 GLBs differ from their face normal).
- `Assets.models` (path -> `PropModel`) replaces `Assets.props` and `Assets.glb`. It holds every `interactables/*.bbmodel` and every model the prop
  data names (650 in all, 88 of 88 props of the self-test floor draw from them), and a mod pack's props (`pack:<id>:<path>`).
- Outliner groups carry what the GLB node tree carried (the converter writes them for these seven models only):
  - `DoorHinge` / `DoorHingeRight`: the leaf elements of a room door. The group origin is the hinge pivot, the elements stay in the open
    pose; `render/door.rs` closes, swings and fits them as before.
  - `DoorFrameTop`, `DoorFrameL`, `DoorFrameR`: measured for the raise and the widen.
  - `LockerDoor` (in `interactables/hiding_spot.bbmodel`): the swinging door leaf; the origin is the hinge line.
  Door measures and the leaf fit come out the same as from the GLBs (the door tests keep their numbers; they now read the bbmodels).

### What still reads a GLB

- Mod packs: `PropModel::parse_glb` (the `gltf` crate) loads a `pack:<id>:...glb` prop, because the pack rules (`docs/MODDING_SPEC.md`, the
  server's file-path check) only allow `.glb`, `.png` and `.ogg` assets. A pack may also bring a `.bbmodel`, read like the game's own.
  Nothing in the game's own assets is a GLB.
- Hunters were never GLB at runtime (bbmodel plus `hunters/anim/<id>.json` extracted once by `render/anim/extract.py`).

### Where the GLB sources are

`tools/props/source_glb/<path under assets/>.glb` (650 files, 22.9 MB), moved from `assets/` with `git mv`. They are not embedded or loaded.

```sh
node tools/props/glb_to_bbmodel.mjs --check   # every GLB converts to exactly the bbmodel on disk (650 models, 0 problems)
node tools/props/glb_to_bbmodel.mjs --all     # regenerate every bbmodel from source_glb (props/blocky uses blocky_atlas.png)
```

### Embedded assets, before and after (`build.rs`, `ASSET_FILES` keys)

| | png | bbmodel | json | glb | total |
|---|---|---|---|---|---|
| before | 86 | 1030 | 50 | 650 | 1816 |
| after | 86 | 679 | 50 | 0 | 815 |

- 650 fewer: every `.glb` (nothing loads one from the embedded assets).
- 351 fewer bbmodels: the one-per-prop `assets/props/<id>.bbmodel` rebuilds. Nothing loads them any more (the prop data names models under
  `props/<set>/` and `props/blocky/`); the files stay on disk.
- Still embedded and not loaded (by design, wiring them in is separate): `environment/{door_frame,floor_tile,wall_block}.bbmodel`,
  `props/hunter_items/kathy_scythe.bbmodel`, `props/sewer/sewer_outflow_pipe_broken.bbmodel`, `props/manifest.json`.
- Everything the game loads is embedded: the 650 models named by the prop data plus the interactables, checked by
  `content::props::tests::every_prop_model_is_an_embedded_bbmodel`.

Sizes (ci profile, `last_route.exe`): 73,553,920 bytes before, 39,353,344 after (34.2 MB smaller). The assets folder went from 64,919,528
bytes to 42,046,624 (the 22,872,904 bytes of GLBs moved out). Each bbmodel still carries its own copy of its atlas as a data URI
(the blocky ones are not read, the shared PNG is); stripping those would shrink the executable and the folder further.
