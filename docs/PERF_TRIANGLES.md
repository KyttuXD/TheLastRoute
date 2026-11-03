# Triangle budget: GLB prop and character models

Measured for the Intel Mac FPS task. Source of truth for the counts is the glTF JSON chunk of each file.

## Method

- Scope: all 430 `.glb` files under `assets/`. Every one of them is parsed at startup by `Assets::load` in `src/shared/assets.rs` (the `props/` and `interactables/` trees, plus `pack:` props from content packs). Nothing else under `assets/` is a GLB.
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
| 1 | `assets/interactables/hiding_spot.glb` | 1,144 | `src/render/scene.rs:244`, once per hiding locker |
| 2 | `assets/interactables/boarded_hallway.glb` | 1,100 | No Rust draw call; loaded into `Assets.glb` only |
| 3 | `assets/interactables/lift.glb` | 732 | `src/render/scene.rs:369`, once per lift |
| 4 | `assets/interactables/revive_kit.glb` | 712 | No Rust draw call; `item_file` has no revive kit |
| 5 | `assets/interactables/boarded_door.glb` | 704 | No Rust draw call; loaded into `Assets.glb` only |
| 6 | `assets/interactables/vent.glb` | 616 | `src/render/scene.rs:342`, once per vent |
| 7 | `assets/interactables/backpack_large.glb` | 572 | No Rust draw call; loaded into `Assets.glb` only |
| 8 | `assets/interactables/locked_relay_barrel.glb` | 544 | No Rust draw call; loaded into `Assets.glb` only |
| 9 | `assets/interactables/music_box.glb` | 536 | `src/render/scene.rs:322`, once per music box |
| 10 | `assets/props/maze/office_crt_security_console.glb` | 520 | Prop `officeCrtSecurityConsole` (`data/props/office.json`), one variant per room via `src/render/world_mesh.rs:336-345` |
| 11 | `assets/props/maze/office_crt_security_console_charcoal.glb` | 520 | Same prop, variant |
| 12 | `assets/props/maze/office_crt_security_console_teal.glb` | 520 | Same prop, variant |
| 13 | `assets/props/sewer/sewer_bridge.glb` | 520 | Prop `sewerBridge` (`data/props/p11.json`) |
| 14 | `assets/props/beach/frozen_beach_hut.glb` | 518 | Prop `frozenBeachHut` (`data/props/p10.json`) |
| 15 | `assets/props/carnival/carousel_centerpiece.glb` | 518 | Prop `carouselCenterpiece` (`data/props/p10.json`) |

Rows 2, 4, 5, 7 and 8 are parsed at startup but no Rust code draws them. Godot may draw them, so confirm there before removing them.

## Recommendation

No single model is expensive: the heaviest is 1,144 triangles and the library median is 440, so the frame cost comes from how many copies are drawn, not from any one file. The per-frame wins are in the models that repeat. The hiding locker (1,144) draws once per locker and the office consoles and beach or carnival props (518 to 520) are placed per room, so decimating those to about 500 triangles or less is the first step. Measure instance counts per level on the Intel Mac before changing any file, and confirm the Godot side for the five unused interactables. Those five are a startup and memory cost, not a frame cost, so lazy parsing would help load time but not FPS.
