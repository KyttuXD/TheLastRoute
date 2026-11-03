# Credits

Original game: **KYTTU: The Last Route** by Kyttu (KyttuXD, Kyttu's Lab), Godot 4 project at
`C:\Users\Kyttu\Documents\Projects\LR Game Stuff\LastRoute Godot\KyttusLabApp`. This Rust
port is also by Kyttu and uses the files of that Godot project, for private use on this computer.

## Original files used (source path in the Godot project → path in this project)

| Original | Here | Use |
|---|---|---|
| `assets/environment/wall_<region>.png`, `floor_<region>.png` (maze, mansion, hotel, apartment, prison, gothic, beach, carnival, greenhouse, inferno, lab, hospital, mall) | `assets/environment/` | wall and floor pictures |
| `assets/hunters_blocky/<id>/<id>.bbmodel` and `<id>_atlas.png` (24 roster hunters) | `assets/hunters/` | hunter bodies |
| `assets/ui/hunter_portraits/<id>.png` | `assets/portraits/` | run report and dossier |
| `assets/ui/inventory/{battery,crowbar,flashlight_item,rescue_axe,stim_pack,taser_item}.png` | `assets/inventory/` | hotbar icons |
| `assets/props/blocky/<type>_0.glb` (351 types, listed in `game/config/blocky_prop_models.gd`), converted with `tools/prop_convert` | `assets/props/<type>.bbmodel`, `assets/props/blocky_atlas.png` | room props |
| `assets/props/{maze,mansion,beach,nether,prison,carnival,...}/*.glb` — the authored Blender exports listed as each type's `models` in `data/props/*.json` (354 files, 177 types used by the room catalogue) | `assets/props/<set>/` | room props, drawn as the original's detailed models; the blocky `.bbmodel` rebuilds remain only as a fallback |
| `assets/interactables/*.glb` (relay, battery, coin, hiding_spot, lift, safe_zone, vent, music_box, heater_station, wall_light_switch, flashlight_item, taser_item, crowbar, rescue_axe, stim_pack, lockpick_set, revive_kit, backpacks, boarded doors, locked relay crate/barrel, service ladder) | `assets/interactables/` | fixtures, floor pickups and the tool in hand |
| `data/props/*.json` | `data/props/` | prop model variants and lamp lights (lr.prop/1) |
| `assets/branding/logo_{16,32,64,128,256}.png`, `logo.svg` | `assets/branding/` | window and executable icon |
| `data/hunters/*.json` | `data/hunters/` | hunter tuning (lr.hunter/1) |
| `data/regions/*.json` | `data/regions/` | region data (lr.region/1) |
| `data/loot/*.json` | `data/loot/` | loot tables (lr.loot/1) |
| `game/config/room_catalog.gd` (extracted constants) | `data/rooms/catalog.json` | room types, props and footprints |

Rules and numbers were read from `game/config/game_config.gd`, `game/player/*.gd`,
`game/hunters/ai/hunter_brain.gd`, `game/hunters/gimmicks/*.gd`,
`game/world/generation/facility_generator.gd` and `seeded_random.gd`, and are written out in
`MECHANICS.md` and `docs/GIMMICKS.md`; the code here is new.

Ported from the original's code (no files copied; the output is the same pixels and samples):

| Original | Here | Use |
|---|---|---|
| `game/look/ref_textures.gd` (RefTextures painters), `ref_look.gd`, `floor_sets.gd`, `data/themes/**`, `data/regions/*.json` | `src/render/textures/` | the 99 painted wall, floor, ceiling and duct textures and the per-district / per-room picks, hall runners and safe-room pad; byte-identical to Godot's output (see its NOTES.md) |
| `game/autoload/sound.gd` and its cue tables, `music_box_tune.gd` | `src/audio/cues/` | every sound cue rendered to the original's samples (checked against `tests/validate_audio.gd` goldens) |
| `game/world/generation/streamed_route_generator.gd` (constants and stair-bay rules) | `src/world/infinite.rs`, `src/game/endless.rs` | the Infinite route's chunks, districts, seams and stairwells |
| `ui/route_loading.gd`, account panel, `GameState.route_checkpoint` | `src/ui/menus.rs`, `src/app/checkpoint.rs` | BUILDING ROUTE screen, account menu, saved run |
| `game/ui/**` scenes and scripts, `palette.gd` schemes | `src/ui/` | menus, settings, pause, HUD, death moment and run card on the original's 480×270 canvas |
| `game/admin/admin_menu.gd`, `hub_admin_panel.gd` | `src/ui/admin/` | the admin menu (key 0) and admin status panel |
| `game/hunters/ai/hunter_director.gd` | `src/hunters/director.rs` | hunters sharing sightings, flanking, scattering, relay rush |
| `game/hunters/view/hunter_view_3d.gd`, `grab_styles.gd`, `hunter_grab.gd`, `held_item_poses.gd`, `player_controller_3d.gd` tweens | `src/render/anim/` | hunter animation playback, grabs and held-item motion |
| `dev/playtest/playtest_bridge.gd`, `bots/last_route_bot.gd`, `tests/` | `src/headless/`, `tests/` | headless runs, agent protocol, test suites and bot |
| `backend/*.mjs`, `game/autoload/backend_client.gd` | `server/`, `src/net/` | the account/room/save server and its client |

| Original data file | Here | Use |
|---|---|---|
| `assets/hunters_blocky/<id>/<id>.glb` animation clips (extracted with `src/render/anim/extract.py`) | `assets/hunters/anim/<id>.json` | the original's idle/walk/chase/sprint/stun/... clips and rest bones |
| `data/grabs/*.json` | `assets/hunters/grabs/` | each hunter's grab style pool |
| `assets/data/death_lines.json` | `assets/ui/death_lines.json` | the death-moment catcher lines |

Text uses the system fonts the original requests (Courier New Bold, Impact); they are read
from the system at runtime and not copied. The original ships no sound files; its cues are
synthesised at runtime, and so are these.
