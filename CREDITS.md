# Credits

Original game: **KYTTU - The Last Route** by Kyttu (KyttuXD, Kyttu's Lab), Godot 4 project at
`C:\Users\Kyttu\Documents\Projects\LR Game Stuff\LastRoute Godot\KyttusLabApp`. This Rust
port is also by Kyttu and uses the files of that Godot project, for private use on this computer.

## Original files used (source path in the Godot project → path in this project)

| Original | Here | Use |
|---|---|---|
| `assets/environment/wall_<region>.png`, `floor_<region>.png` (maze, mansion, hotel, apartment, prison, gothic, beach, carnival, greenhouse, inferno, lab, hospital, mall) | `src/bin/last_route/assets/environment/` | wall and floor pictures |
| `assets/hunters_blocky/<id>/<id>.bbmodel` and `<id>_atlas.png` (24 roster hunters) | `src/bin/last_route/assets/hunters/` | hunter bodies |
| `assets/ui/hunter_portraits/<id>.png` | `src/bin/last_route/assets/portraits/` | run report and dossier |
| `assets/ui/inventory/{battery,crowbar,flashlight_item,rescue_axe,stim_pack,taser_item}.png` | `src/bin/last_route/assets/inventory/` | hotbar icons |
| `assets/props/blocky/<type>_N.glb` (220 models the prop data names, shared 128x64 atlas `blocky_atlas.png`), converted with `tools/props/glb_to_bbmodel.mjs`; the older one-per-prop rebuilds from `<type>_0.glb` (`tools/prop_convert`) stay in `src/bin/last_route/assets/props/<type>.bbmodel` but nothing loads them | `src/bin/last_route/assets/props/blocky/<type>_N.bbmodel`, `src/bin/last_route/assets/props/blocky_atlas.png` | room props |
| `assets/props/{maze,mansion,beach,nether,prison,carnival,...}/*.glb` — the authored Blender exports listed as each type's `models` in `data/props/*.json` (354 files, 177 types used by the room catalogue), converted with `tools/props/glb_to_bbmodel.mjs` | `src/bin/last_route/assets/props/<set>/*.bbmodel` (the GLB sources are kept in `tools/props/source_glb/props/<set>/`) | room props, drawn as the original's detailed models |
| `assets/interactables/*.glb` (relay, battery, coin, hiding_spot, lift, safe_zone, vent, music_box, wall_light_switch, flashlight_item, taser_item, crowbar, rescue_axe, stim_pack, lockpick_set, revive_kit, backpacks, boarded doors, locked relay crate/barrel, service ladder), converted the same way | `src/bin/last_route/assets/interactables/*.bbmodel` (sources in `tools/props/source_glb/interactables/`) | fixtures, floor pickups and the tool in hand |
| `data/props/*.json` | `src/bin/last_route/data/props/` | prop model variants and lamp lights (lr.prop/1) |
| `assets/branding/logo_{16,32,64,128,256}.png`, `logo.svg` | `src/bin/last_route/assets/branding/` | window and executable icon |
| `docs/player_skin_steve_reference.png`, `docs/player_skin_alex_reference.png` (the Godot2 project's reference skins, 64x64) | `src/bin/last_route/assets/skins/ref_classic.png`, `ref_slim.png` | the skin catalogue (FROM CATALOG) |
| `data/hunters/*.json` | `src/bin/last_route/data/hunters/` | hunter tuning (lr.hunter/1) |
| `src/bin/last_route/data/regions/*.json` | `src/bin/last_route/data/regions/` | region data (lr.region/1) |
| `data/loot/*.json` | `src/bin/last_route/data/loot/` | loot tables (lr.loot/1) |
| `game/config/room_catalog.gd` (extracted constants) | `src/bin/last_route/data/rooms/catalog.json` | room types, props and footprints |

Rules and numbers were read from `game/config/game_config.gd`, `game/player/*.gd`,
`game/hunters/ai/hunter_brain.gd`, `game/hunters/gimmicks/*.gd`,
`game/world/generation/facility_generator.gd` and `seeded_random.gd`, and are written out in
`MECHANICS.md` and `docs/GIMMICKS.md`; the code here is new.

Ported from the original's code (no files copied; the output is the same pixels and samples):

| Original | Here | Use |
|---|---|---|
| `game/look/ref_textures.gd` (RefTextures painters), `ref_look.gd`, `floor_sets.gd`, `data/themes/**`, `src/bin/last_route/data/regions/*.json` | `src/bin/last_route/render/textures/` | the 99 painted wall, floor, ceiling and duct textures and the per-district / per-room picks, hall runners and safe-room pad; byte-identical to Godot's output (see its NOTES.md) |
| `game/autoload/sound.gd` and its cue tables, `music_box_tune.gd` | `src/bin/last_route/audio/cues/` | every sound cue rendered to the original's samples (checked against `tests/validate_audio.gd` goldens) |
| `game/world/generation/streamed_route_generator.gd` (constants and stair-bay rules) | `src/bin/last_route/world/infinite.rs`, `src/bin/last_route/game/endless.rs` | the Infinite route's chunks, districts, seams and stairwells |
| `ui/route_loading.gd`, account panel, `GameState.route_checkpoint` | `src/bin/last_route/ui/menus.rs`, `src/bin/last_route/shared/checkpoint.rs` | BUILDING ROUTE screen, account menu, saved run |
| `game/ui/**` scenes and scripts, `palette.gd` schemes | `src/bin/last_route/ui/` | menus, settings, pause, HUD, death moment and run card on the original's 480×270 canvas |
| `game/admin/admin_menu.gd`, `hub_admin_panel.gd` | `src/bin/last_route/ui/admin/` | the admin menu (key 0) and admin status panel |
| `game/hunters/ai/hunter_director.gd` | `src/bin/last_route/hunters/director.rs` | hunters sharing sightings, flanking, scattering, relay rush |
| `game/hunters/view/hunter_view_3d.gd`, `grab_styles.gd`, `hunter_grab.gd`, `held_item_poses.gd`, `player_controller_3d.gd` tweens | `src/bin/last_route/render/anim/` | hunter animation playback, grabs and held-item motion |
| `dev/playtest/playtest_bridge.gd`, `bots/last_route_bot.gd`, `tests/` | `src/bin/last_route/headless/`, `tests/` | headless runs, agent protocol, test suites and bot |
| `backend/*.mjs`, `game/autoload/backend_client.gd` | `server/`, `src/bin/last_route/net/` | the account/room/save server and its client |

| Original data file | Here | Use |
|---|---|---|
| `assets/hunters_blocky/<id>/<id>.glb` animation clips (extracted with `src/bin/last_route/render/anim/extract.py`) | `src/bin/last_route/assets/hunters/anim/<id>.json` | the original's idle/walk/chase/sprint/stun/... clips and rest bones |
| `data/grabs/*.json` | `src/bin/last_route/assets/hunters/grabs/` | each hunter's grab style pool |
| `assets/data/death_lines.json` | `src/bin/last_route/assets/ui/death_lines.json` | the death-moment catcher lines |

Text uses the system fonts the original requests (Courier New Bold, Impact); they are read
from the system at runtime and not copied. The original ships no sound files; its cues are
synthesised at runtime, and so are these.
