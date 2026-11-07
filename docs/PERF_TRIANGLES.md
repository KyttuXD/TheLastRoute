# Triangle budget: GLB prop and character models

Measured for the Intel Mac FPS task. Source of truth for the counts is the glTF JSON chunk of each file.

## Method

- Scope: all 430 `.glb` files under `assets/` (now `tools/props/source_glb/`). Every one of them is parsed at startup by `Assets::load` in `src/bin/last_route/shared/assets.rs` (the `props/` and `interactables/` trees, plus `pack:` props from content packs). Nothing else under `assets/` is a GLB.
- Triangles per primitive: mode 4 (or unset) is index count / 3, mode 5 and 6 are count - 2. The count is the sum over the file's meshes, taken once per mesh.
- Each heavy file has one mesh root, so the drawn count per instance equals the file count. Per-level instance counts were not measured.
- Script: `C:\Users\Kyttu\AppData\Local\Temp\claude\C--Users-Kyttu-Documents-Projects-LR-Game-Stuff\a43ede59-bd7c-4884-aa74-987bb6181833\scratchpad\glb_tris.mjs` (output in the same folder as `glb_tris.json`).
- Character models are not GLB. The hunters are `.bbmodel` (blocky) and are not in this count.

## Library summary

- 430 files, 167,880 triangles in total (unique meshes).
- Median 440 triangles per file, maximum 1,144, six files above 600.

## The 15 heaviest files

| # | File | Triangles | Where it is drawn |
|---|------|----------:|-------------------|
| 1 | `assets/interactables/hiding_spot.glb` | 1,144 | `src/bin/last_route/render/scene.rs:244`, once per hiding locker |
| 2 | `assets/interactables/boarded_hallway.glb` | 1,100 | No Rust draw call; loaded into `Assets.glb` only |
| 3 | `assets/interactables/lift.glb` | 732 | `src/bin/last_route/render/scene.rs:369`, once per lift |
| 4 | `assets/interactables/revive_kit.glb` | 712 | No Rust draw call; `item_file` has no revive kit |
| 5 | `assets/interactables/boarded_door.glb` | 704 | No Rust draw call; loaded into `Assets.glb` only |
| 6 | `assets/interactables/vent.glb` | 616 | `src/bin/last_route/render/scene.rs:342`, once per vent |
| 7 | `assets/interactables/backpack_large.glb` | 572 | No Rust draw call; loaded into `Assets.glb` only |
| 8 | `assets/interactables/locked_relay_barrel.glb` | 544 | No Rust draw call; loaded into `Assets.glb` only |
| 9 | `assets/interactables/music_box.glb` | 536 | `src/bin/last_route/render/scene.rs:322`, once per music box |
| 10 | `assets/props/maze/office_crt_security_console.glb` | 520 | Prop `officeCrtSecurityConsole` (`data/props/office.json`), one variant per room via `src/bin/last_route/render/world_mesh.rs:336-345` |
| 11 | `assets/props/maze/office_crt_security_console_charcoal.glb` | 520 | Same prop, variant |
| 12 | `assets/props/maze/office_crt_security_console_teal.glb` | 520 | Same prop, variant |
| 13 | `assets/props/sewer/sewer_bridge.glb` | 520 (832 since 2026-11-06, see the end) | Prop `sewerBridge` (`data/props/p11.json`) |
| 14 | `assets/props/beach/frozen_beach_hut.glb` | 518 | Prop `frozenBeachHut` (`data/props/p10.json`) |
| 15 | `assets/props/carnival/carousel_centerpiece.glb` | 518 | Prop `carouselCenterpiece` (`data/props/p10.json`) |

Rows 2, 4, 5, 7 and 8 are parsed at startup but no Rust code draws them. Godot may draw them, so confirm there before removing them.

## Recommendation

No single model is expensive: the heaviest is 1,144 triangles and the library median is 440, so the frame cost comes from how many copies are drawn, not from any one file. The per-frame wins are in the models that repeat. The hiding locker (1,144) draws once per locker and the office consoles and beach or carnival props (518 to 520) are placed per room, so decimating those to about 500 triangles or less is the first step. Measure instance counts per level on the Intel Mac before changing any file, and confirm the Godot side for the five unused interactables. Those five are a startup and memory cost, not a frame cost, so lazy parsing would help load time but not FPS.

## Runner figures (the player mannequin)

The runner mannequin (`assets/player/player_mannequin.bbmodel`, 43 cubes, about 516 triangles; the slim-arm twin is the same size) is drawn once per other runner, bot and locker occupant by `net/runner_figure.rs`. A full room of 8 runners and 6 bots is 14 figures, about 7,200 triangles, one `draw_mesh` each, built into a reused scratch buffer (no per-frame allocation). `Model::build_posed` stops at 64,000 vertices (u16 indices), far above one figure. A crouching figure draws in the lit translucent shader mode (`GHOST_MODE`) at 45% opacity, as Godot fades it to 0.55 transparency. The local runner has no body (Godot hides it in first person and has no shadow or mirror of it), so first person draws only the hands.

## Cut-down GLBs and see-through props (2026-11-06)

Many source GLBs in `tools/props/source_glb/` were cut to about 500 triangles by dropping triangles, not by simplifying: a round
barrel that should have 76 triangles (a 20-sided tube with caps) kept 23, so a third of its wall remained and the rest of the
room showed through it. The sewer outflow pipe was the worst (every tube, ring and strap of it open: 500 of the Godot file's 1,732
triangles), the sewer bridge lost 312 of 832. Both were re-taken from the Godot GLBs (`KyttusLabApp/assets/props/sewer/`) and
converted again (`node tools/props/glb_to_bbmodel.mjs --all`); the stub `sewer_outflow_pipe_broken.glb` was added beside them. The
bounding boxes are unchanged, so footprints and placement are too. The cost is 1,232 more triangles per outflow pipe (one at most
per sewer chunk), in a prop that is batched once and cached while it stays on screen.

The `render::bbmodel` tests now hold the sewer pipework (outflow pipe, its stub, pipe run, bridge) to closed, outward-wound solids:
every welded edge shared by exactly two faces, mesh elements wound counter-clockwise from outside. About 120 other GLBs still carry
fewer triangles than their Godot twin (the palm tree, the grand staircases, the retro computers, the bunks and so on; compare the
triangle counts with `LastRoute Godot/KyttusLabApp/assets/props`); they are the same kind of cut and are not re-taken yet.
