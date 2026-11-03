# Props as bbmodel (GLB to .bbmodel)

Standard: the Rust port uses Blockbench `.bbmodel` for world models. Godot's GLBs are converted with
`tools/props/glb_to_bbmodel.mjs` (Node built-ins only, no install step).

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
re-reads the written bbmodel the way `src/render/bbmodel.rs` does (a JS replica of the cube and mesh branches)
and checks that every source triangle is covered with the same position and UV, and that the total surface area
matches. The exit code is non-zero if any file fails.

## Conventions (copied from `tools/prop_convert` and the existing `assets/props/*.bbmodel`)

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
| door_frame | `assets/environment/door_frame.bbmodel` | 0.92 x 2 x 0.17 | 0.92 x 2 x 0.17 | 120 | 120 | 4 / 2 | 256x256 PNG, identical | OK |
| floor_tile | `assets/environment/floor_tile.bbmodel` | 1 x 0.1 x 1 | 1 x 0.1 x 1 | 72 | 72 | 0 / 2 | 256x256 PNG, identical | OK |
| wall_block | `assets/environment/wall_block.bbmodel` | 1 x 2.4 x 0.23 | 1 x 2.4 x 0.23 | 120 | 120 | 10 / 0 | 256x256 PNG, identical | OK |
| kathy_scythe | `assets/props/hunter_items/kathy_scythe.bbmodel` | 0.08 x 1.81 x 0.718 | 0.08 x 1.81 x 0.718 | 144 | 144 | 4 / 1 | 64x128 PNG, identical | OK |
| sewer_outflow_pipe_broken | `assets/props/sewer/sewer_outflow_pipe_broken.bbmodel` | 1.9 x 1.998 x 1.57 | 1.9 x 1.998 x 1.57 | 516 | 516 | 14 / 13 | 128x16 PNG, identical | OK |

Nothing in the Rust source loads these five yet. They are data only; wiring them in is a separate step.

### Scale against existing props

Running the converter on the Godot `_0` GLB of props the port already ships gives the same min/max corners (in bbmodel
units) as the existing `assets/props/<name>.bbmodel`:

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
0.625 x 1.813 x 0.75 m, the same as the existing single `assets/props/arcadeCabinet.bbmodel`, uses the same atlas PNG
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
