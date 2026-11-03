# Night Watch modes: Godot to Rust mapping

This page maps the nine Night Watch modes from the TODO "Night Watch port" section onto the Godot 3D project at `LastRoute Godot/KyttusLabApp/game/night_watch`, and records what the Rust binary at `src/bin/night_watch` already has and what it still lacks. The Godot tree is the read-only reference. All new Rust code goes under `src/bin/night_watch`, and shared core items come from `last_route_core` only.

Mode ids follow the TODO and the brief. The Godot mode id and the current Rust module name are shown where they differ.

## Folder inventory

| Godot folder or file | Role | Maps to |
| --- | --- | --- |
| `night_watch_classic.gd`, `night_watch.tscn`, `admin/classic_admin.gd` | Classic rules, shift scene and admin hooks. There is no `classic/` folder. | classic |
| `bedroom/` | Bedtime shift scene, rules, map, paths, sounds | bedtime |
| `dayshift/` | Salvage Run: day shop, night rack, rules | salvage_run |
| `descent/` | The Descent elevator puzzles and rules | descent |
| `mask/` | Masked Hours: mask, monitor, music box, vents | masked_hours |
| `lure/` | Dead Air: vents, reboots, climate, Xivvy | dead_air |
| `story/` | The Facility: five rooms, tasks, Opalie, Porcelle, Quenelle | facility |
| `openair/` plus root `night_watch_extra.tscn` | Open Air: three outdoor watches. The mode id in the registry is `extra`. | open_air |
| `mixed/` | Mixed Shift and Endless: drawn threats and a pacing director. Registry id `mixed`. | mixed_shift |
| `office/` | Shared office base, map registry, palette, signature layouts | all modes that walk a building |
| `admin/night_watch_admin.gd` | Shared admin hooks for every shift scene | all modes |
| `data/nw_schemas.gd` | Schema helpers for the night watch data files | all modes |
| Root `night_watch_*.gd` | Shared framework: mode base, session, mulberry and stream RNG, pacer, walker, paths, stalker, poses, art, sounds, HUD, cam map, building, save, menu, data, mode table and registry, call script | shared framework |

Mode tables and data live in `LastRoute Godot/KyttusLabApp/data/night_watch/`:

- `modes/<id>.json` for each mode (menu entry, cast, presets, Custom Night sliders, blurb, levels)
- `rules/classic.json` and `buildings/classic.json`
- `calls/` (Zellia's call scripts)
- `casts/`, `offices/`, `scares/` and `sounds/` (the last three are empty in this checkout)

The Godot validators that cover the modes are `tests/validate_night_watch_<mode>.gd`, plus `validate_night_watch_modes.gd`, `validate_night_watch_calls.gd`, `validate_night_watch_sounds.gd` and `validate_night_watch_menus.gd`. Their headless replays are the best oracle for porting each rules file.

## Mapping table

| Mode id | TODO name | Godot id and folder | Main Godot scene script | Rules script | Rust today | Rust gap |
| --- | --- | --- | --- | --- | --- | --- |
| classic | Classic | `classic`, root | `night_watch_classic.gd` (rules); shift scene `night_watch.tscn` | `night_watch_classic.gd` (212 lines), `data/night_watch/rules/classic.json` | Hub entry, `calls/classic.json`, `modes/classic.rs` stub, save and call framework | No rules, no scene, no monitor, doors, lights or power port |
| bedtime | Bedtime | `bedroom`, `bedroom/` | `bedroom/bedroom_game.gd` (580 lines) | `bedroom/bedroom_logic.gd` (216 lines) | Hub entry, `calls/bedroom.json`, `modes/bedroom.rs` stub | No rules, no scene, no bed, closet or hall logic |
| salvage_run | Salvage Run | `dayshift`, `dayshift/` | `dayshift/dayshift_game.gd` (728 lines) | `dayshift/dayshift_logic.gd` (357 lines), `dayshift_shop.gd`, `dayshift_data.gd` | Hub entry, `calls/dayshift.json`, `modes/dayshift.rs` stub | No day shop, budget, rack or night rules |
| descent | The Descent | `descent`, `descent/` | `descent/descent_game.gd` (485 lines), `descent_world.gd`, `descent_panel.gd` | `descent/descent_logic.gd` (476 lines) | Hub entry, `calls/descent.json`, `modes/descent.rs` stub | No wires, signal-pad or pipe puzzles, no elevator view |
| masked_hours | Masked Hours | `mask`, `mask/` | `mask/mask_game.gd` (947 lines), `mask_world.gd`, `mask_controls.gd` | `mask/mask_logic.gd` (679 lines) | Hub entry, `calls/mask.json`, `mask/vent_lights.rs` (299 lines), `modes/mask.rs` stub | Vent-light part exists; no mask, monitor, music box, flashlight or hall-hunter logic |
| dead_air | Dead Air | `lure`, `lure/` | `lure/lure_game.gd` (864 lines), `lure_world.gd`, `lure_panel.gd` | `lure/lure_logic.gd` (434 lines) | Hub entry, `calls/lure.json`, `modes/lure.rs` stub | No Xivvy walk, phantoms, climate, reboots or lure monitor |
| facility | The Facility | `story`, `story/` | `story/story_game.gd` (695 lines), `story_world.gd`, `story_panel.gd` | `story/story_logic.gd` (441 lines) | Hub entry, `calls/story.json`, `modes/story.rs` stub | No rooms, task list, noise, beacon, lift or the three hunters |
| open_air | Open Air | `extra`, `openair/` (scene root `night_watch_extra.tscn`) | `openair/openair_game.gd` (668 lines), `openair_world.gd` | `openair/openair_logic.gd` (668 lines), `openair_pacer.gd`, `openair_data.gd`, `openair_paths.gd` | Hub entry, `calls/extra.json`, `modes/extra.rs` stub | No three watches, torch, lantern oil, hatch or campfire logic |
| mixed_shift | Mixed Shift | `mixed`, `mixed/` | `mixed/mixed_game.gd` (872 lines), `mixed_world.gd`, `mixed_map.gd`, `mixed_skin.gd` | `mixed/mixed_logic.gd` (1283 lines) | `modes/mixed.rs` stub only. Not in `HUB_ENTRIES` and no `calls/mixed.json`. | Hub entry, call table, threat roster, director and five shifts all missing |

Hub order in Godot is 1 Classic, 2 Masked Hours, 3 Dead Air, 4 Bedtime, 5 The Facility, 6 The Descent, 7 Salvage Run, 8 Open Air, 9 Mixed Shift. The Rust hub in `shell.rs` lists only eight entries and omits Mixed Shift.

## Per-mode notes

### classic (Classic)

- Godot: `night_watch_classic.gd` holds the rules. Its `step(dt_ms, input)` takes door, light and monitor state and returns events (`moved`, `arrived`, `repelled`, `pushed`, `flinch`, `attack`, `out`, `caught`, `win`).
- Rules and scoring: seven nights, two steel doors and two lights, eight cameras, one power meter, 6 AM win, power loss opens doors and kills lights. Points per level are 10 (`POINTS_PER_LEVEL` in `night_watch_modes_data.gd`). Custom Night sliders run 0 to 20 per hunter.
- Hunters: Dovelle, Nyxie, Ravelle, Ilyra, Aki (cast). Night 7 has Kathy alone (TODO).
- Assets and sounds: `nw_door_left` and `nw_door_right`, `nw_light_left` and `nw_light_right`, `nw_lamp`, `nw_name`, plus the Night Watch cue family. Building data is in `data/night_watch/buildings/classic.json`.
- Rust cannot port yet: the classic rules, the power budget, camera feeds, the walkable eight-room building, the security room and the Night 7 scene. The call script `calls/classic.json` and the save clear rule (`NIGHTS` in `save.rs`) already exist.

### bedtime (Bedtime)

- Godot: `bedroom/bedroom_game.gd` is the shift scene, `bedroom_logic.gd` the rules. The walkers are `NightWatchWalker` for Javelle and a creeping path for Haelith. `bedroom_map.gd`, `bedroom_paths.gd` and `bedroom_wing.gd` give the layout.
- Rules: no cameras. Hold A to shut the closet door (Loxelle breathes behind it), D to light the hall (Javelle waits there), F to light the foot of the bed (Haelith creeps in while the room is dark). One hand, so one action at a time. The flashlight battery never recharges. Holding the door uses both hands. Presets: Closet Door, Hall Watch, Lights Out, All Twenty.
- Scoring: the shared 10 points per level and the rating table.
- Assets and sounds: `bedroom_sounds.gd` (`BedroomSounds.breath`, `step_in`), a dark bedroom, the pillow-view look controls, the shared Sound autoload and the walker.
- Rust cannot port yet: the pillow view and look controls, the closet, hall and bed hunters, the battery, and the breathing check. The call script `calls/bedroom.json` exists.

### salvage_run (Salvage Run)

- Godot: `dayshift/dayshift_game.gd` runs the day shop (`dayshift_shop.gd`) and the night in the Classic security room with a workbench, a four-slot rack and a window. `dayshift_logic.gd` holds the rules, and `dayshift_yard.gd`, `cargo_bay.gd` and `dayshift_map.gd` give the layout.
- Rules: spend the week's budget on upgrades by day, then hold USE to salvage pieces off the rack at night. Salvaging is loud and makes the visitors keener. Doors and door lights are as in Classic. Ysolde watches the window. Quota and `short {money, quota}` events decide the night. Power is tuned for an eight-minute night (TIME_SCALE comes from `data/night_watch/rules/classic.json`).
- Hunters: Ravelle, Ceryth and Ysolde (cast). Presets: Left Door, Right Door, After Hours, All Twenty.
- Scoring: the rack value per salvage event (`salvage {slot, value}`), the quota, and the shared 10 points per level.
- Assets and sounds: `theme_nightwatch.tres`, `night_watch_player.tscn`, the shared art, and the nw cues. The rules use the same Mulberry stream as the 2D game.
- Rust cannot port yet: the day shop, the upgrade budget, the rack and slot logic, the salvage loop, the window and the quota. Mulberry and the call script are shared and ready.

### descent (The Descent)

- Godot: `descent/descent_game.gd` is a fixed view from inside an elevator, driven by `descent_logic.gd`. The panel is `descent_panel.gd`, the shaft is `descent_world.gd` and `descent_office.gd`. The cursor is free, with no player controller.
- Rules: three puzzle types. Wires light in turn, and a wrong button re-deals them. Memory is a four-pad signal sequence. Pipes need one line from the left edge to the right edge, and Elise closes in at half speed. Hold SPACE to keep the door shut on the last stage. Elise walks the sill and reaches the glass as the closeness bar fills.
- Hunter: Elise only. Presets: Slow Ride (5), Service Run (10), Express (15), Free Fall (20).
- Scoring: stage clears and the 10 points per level.
- Assets and sounds: `descent_data.gd`, `descent_world.gd`, the shared Sound autoload and the HUD call box and end card.
- Rust cannot port yet: the three puzzles, the closeness bar, the elevator view and the Elise walk. The call script `calls/descent.json` exists.

### masked_hours (Masked Hours)

- Godot: `mask/mask_game.gd` is the shift scene (Classic security room, no doors, dark, with a wing of seven rooms on the monitor). `mask_logic.gd` is a port of the 2D mask logic and matches the 2D game event for event for the same seed. `mask_controls.gd`, `mask_world.gd` and `mask_map.gd` give controls and layout.
- Rules: wear the mask (exclusive with the monitor) when hall hunters walk in. Watch cameras 1 to 8 for who is coming. Hold R to wind the music box, which jumps the monitor to CAM 1. Hold G or H for the vent lights and X for the hall light. Keep the flashlight alive, because when it dies someone comes. Presets: Hall Pass, Vent Duty, Wound Tight, All Twenty.
- Hunters: Verrine, Thessine, Glyra, Nyxie, Umbriel, Haelith, Ilyra (cast). The events include `room`, `hall`, `step`, `leave`, `rush`, `flashHit`, `duct`, `vent`, `fetch`, `creep` and `dark`.
- Scoring: the shared 10 points per level and the rating table.
- Assets and sounds: `night_watch_mask.tscn`, `night_watch_player.tscn`, the music box cue and the vent cues. The validator `validate_night_watch_mask.gd` and `tests/fixtures/gen_night_watch_mask_golden.mjs` give golden output.
- Rust cannot port yet: the mask, monitor, music box, flashlight, the hall and duct hunters, and the rules in `mask_logic.gd`. Rust already has `mask/vent_lights.rs` (299 lines), which covers the vent-light part.

### dead_air (Dead Air)

- Godot: `lure/lure_game.gd` is the broadcast booth with two door buttons (vent seals), two light buttons (heater and fan) and a monitor onto the radio station. `lure_logic.gd` holds the rules. The hall walk for Xivvy uses `lure_paths.gd` and `NightWatchWalker`. `lure_world.gd` and `lure_map.gd` give the layout.
- Rules: one real hunter, Xivvy, crawls the building toward the office vents from night 2. Three phantoms (Ferryn, Briselle, Mavrine) knock out systems. Climate is driven by Solyth and Wynelle. Below 60 F Wynelle freezes the office, and from 95 F Solyth cooks it. The heater and fan fix the number but strain the air. Keys in the monitor: 1 to 8 for cameras, L to lure Xivvy to the room on screen, Q, W, E, R and T to reboot audio, cameras, vents, heater and fan, N to mute Zellia. One more hunter each night, never more than five at once. Presets: Maintenance Only, The Real One, Climate Control, All Twenty.
- Assets and sounds: `lure_sounds.gd` (`LureSounds.moved`, `at_vent`), `night_watch_lure.tscn`, the heater and fan cues, and the shared player.
- Rust cannot port yet: the lure monitor, reboots, climate model, phantoms, the Xivvy walk and the vent seals. The call script `calls/lure.json` exists.

### facility (The Facility)

- Godot: `story/story_game.gd` is a walkable scene of five blocky rooms with a task station in each. `story_logic.gd` holds the rules. `story_world.gd`, `story_map.gd`, `story_office.gd` and `story_panel.gd` cover layout and UI. `story_admin.gd` holds admin switches.
- Rules: finish the night's task list in order at each room's station. Moving loudly makes noise, crawling is slow and quiet, and each action is a little noisy. Walking through a door takes 1.8 s (3.6 s crawling, hold C), and the rules count it as the 2D `move`. Hold F for the beacon. Hold H to hide under the desk in Control. Hold E on the lift's call button. Presets: Quiet Gallery, Behind the Steel, Lift Schedule, All Twenty.
- Hunters: Opalie (gallery, closes in on noise), Porcelle (workshop and auditorium, closes in while dark, held back by the beacon), Quenelle (rides the lift and opens at Control).
- Assets and sounds: `story_sounds.gd` (`StorySounds.hunter_close`, `hunter_back`), `night_watch_story.tscn`, and the shared player.
- Rust cannot port yet: the five rooms, the task list, the noise model, the beacon, the lift, the three hunters and the walk-through door timing. The call script `calls/story.json` exists.

### open_air (Open Air)

- Godot: `openair/openair_game.gd` is the shift scene, and the root `night_watch_extra.tscn` points to it. `openair_logic.gd` holds the rules for three watches. `openair_world.gd` builds the walkable sets, and `openair_pacer.gd`, `openair_data.gd` and `openair_paths.gd` give pacing, data and walking routes.
- Rules: the watch is the level. 1 ROOFTOP has Verrine climbing by the stair door or the hatch, a torch keeps her still, and the lantern oil runs the winches (barriers swing open when it is out). 2 BUS STOP and 3 CAMPFIRE are the other two watches, each with its own hunter and way of keeping the dark off. Custom Night chains the watches whose hunters are on. A held watch offers NEXT WATCH, and only the last win is recorded (`record_win`). The wind blows the lantern out. Presets: High Ground, Cold Road, Dusk to Dawn, All Twenty.
- Hunters: Ysolde, Verrine, Wynelle, Quenelle (cast).
- Scoring: the watch win and the shared 10 points per level, with the last win recorded.
- Assets and sounds: `openair_data.gd`, the shared art and the Sound autoload. The Godot check is `validate_night_watch_extra.gd`.
- Rust cannot port yet: the three watches, the torch, the lantern oil, the hatch and winches, the campfire, and the Custom Night chain. The call script `calls/extra.json` exists. The Rust id is `extra`, the TODO name is Open Air.

### mixed_shift (Mixed Shift)

- Godot: `mixed/mixed_game.gd` is the shift scene, a transit hub (`mixed_map.gd`) around the security room with three permanent vents and a vent run above the building (CAM 9). `mixed_logic.gd` (1283 lines) holds the threat roster and the pacing director. `mixed_skin.gd` covers the look. Hunters walk on `NightWatchPaths` and `NightWatchWalker`.
- Rules: three to seven threats are drawn each night from a pool of eight. Each has a tell before it can kill and an answer (`a` and `beat`). Threat types are counter, watched, monitor, path, vent and winder. The director paces the beats, spikes and moods. There are no captions, only sounds, lights, figures and pulsing camera buttons. The five shifts are QUIET, STEADY, RESTLESS, CHAOS (random events) and ENDLESS (no 6 AM, best hour saved by `NightWatchSave.record_best`). Controls: Z and X toggle doors, V, B and N hold to seal the left, top and right vents, G, Y and H tap the vent light, R winds the music box, U is the monitor, 1 to 9 pick cameras.
- Assets and sounds: `night_watch_player.tscn`, `night_watch_art.gd`, the shared cue family, and the vent and door cues. The golden fixture is `tests/fixtures/gen_night_watch_mixed_golden.mjs`.
- Rust cannot port yet: nothing beyond the `modes/mixed.rs` stub. The threat roster, the director, the five shifts, the transit map, the call table and the hub entry are all missing. The Rust hub lists eight modes and omits this one.

## Shared framework work

These items serve every mode and sit in `src/bin/night_watch` (framework, data, save, shell). They do not belong to any one mode.

- Mode table and registry: the Godot `night_watch_modes_data.gd` and `night_watch_mode_registry.gd` read `data/night_watch/modes/`. Rust needs the same nine entries in `shell.rs` and a mode trait (the Godot `night_watch_mode.gd` base).
- Calls: Rust already loads `calls/*.json` through `data.rs`, which lacks `mixed.json`.
- RNG: `framework.rs` has the Mulberry and stream code that matches the 2D game. Each mode port must use the same draw order to stay in step with the 2D and Godot goldens.
- Scoring: `POINTS_PER_LEVEL` 10 and the rating table in `night_watch_modes_data.gd` are shared. The per-mode scoring is in each `*_logic.gd`.
- Assets: the nw_ cue family, `night_watch_art.gd` and `night_watch_hunter_scale.gd` are the shared art and scale path. The Godot `sounds/` and `scares/` folders are empty in this checkout, so the audio source for the cues is the Godot Sound autoload and must be confirmed before porting.
- Save: `save.rs` already keeps `cleared_<mode>` and `NIGHTS`. Mixed Shift needs the endless best-hour record.

## Verification

This page is documentation only. No Rust code changed, so no build was run. The Godot line counts and mode values were taken from the files listed above on 2026-11-03.
