# Night Watch modes: Godot to Rust mapping

This page maps the nine Night Watch modes onto the Godot 3D project at `LastRoute Godot/KyttusLabApp/game/night_watch` and records where each one stands in the Rust binary at `src/bin/night_watch`. The Godot tree is the read-only reference. All Night Watch code goes under `src/bin/night_watch`, and shared core items come from `last_route_core` only.

State on 2026-11-04: all nine modes have their rules ported as pure logic and all nine are on the hub, and all nine have a playable driver with a picture, controls and a call box: `classic_play.rs` and `modes_play/{masked_hours,dead_air,bedtime,facility,descent,salvage_run,open_air,mixed_shift}.rs`. Classic also has the monitor with the building map. `main.rs` opens each driver from the hub (Open Air through its own `extra` arm, the rest through Night Select), and `StartCustom` is wired for Classic, Masked Hours, Dead Air, Bedtime and The Facility; Open Air chains its Custom Night inside its own driver. `modes::start` and `Session::tick` remain only as the fallback for an id with no driver (none today). Sound is `nw_sound.rs` with the cues in `data/sounds.json` (see the shared framework section). The numbers and the content live in a data layer (see below), not in the Rust files.

Mode ids follow the hub (`shell.rs`), and each night type opens through the Night Select screen (`select.rs`). The Godot mode id and the Rust module name are shown where they differ.

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

## Data layer

Every tunable number, cast list, room graph and map is JSON under `src/bin/night_watch/data`, embedded in the executable and parsed once. Loading panics with the mode and key when a key is missing, so a content break shows up in the tests. `data.rs` is the reader (`rules`, `cast`, `building`, with dotted-path helpers `num`, `int`, `text`, `list`).

| Folder | Content | Modes |
| --- | --- | --- |
| `data/rules/<mode>.json` | Timings, level tables, scoring, pacing | all nine |
| `data/cast/<mode>.json` | Hunters, phantoms and their per-night levels | classic, bedtime, masked_hours, dead_air, facility, open_air, mixed_shift |
| `data/buildings/<mode>.json` | Room cameras, labels and the hunters' room graph | classic, bedtime, masked_hours, dead_air, facility, mixed_shift |
| `data/maps/<mode>.json` (schema `nw.map/1`) | The whole building as a floor plan: rooms with outlines in metres, links, cameras, offices, spawns | all nine |
| `data/calls_ui.json` (schema `nw.calls_ui/1`) | Typewriter speed, punctuation pauses, blip voices | shared |
| `calls/<id>.json` (schema `nw.calls/1`) | The headset call scripts, copied from Godot | all except mixed (its call is the `call` block of `rules/mixed_shift.json`) |

Rules for the data layer:

- A mode file has no number a data file also holds. Open Air's listed levels come from `levels.listed` in `rules/open_air.json` (`Levels::listed()`), and its test checks the table names the four hunters in Godot's order.
- Classic's hunter graph is derived from the building map's walkable links (`building_map::neighbours`, minus the office); a test holds it equal to the old table. `buildings/classic.json` still lists the graph as the cross-check used by the map tests.
- The Godot rules and cast for Classic are also kept, unembedded, in `src/bin/night_watch/assets` as a reference copy.
- Code: `typewriter.rs` (call text reveal), `building_map.rs` (map queries and drawing, used by Classic's monitor), `calls_view.rs` and `blip.rs` (call box and sound).

## Mapping table

| Hub id | Name | Godot id and folder | Main Godot scene script | Rules script | Rust module | Rust state |
| --- | --- | --- | --- | --- | --- | --- |
| classic | Classic | `classic`, root | `night_watch_classic.gd`; shift scene `night_watch.tscn` | `night_watch_classic.gd`, `rules/classic.json` | `modes/classic.rs`, `classic_play.rs` | Rules and play: doors, lights, monitor with building map, power, call box, Kathy night |
| mask | Masked Hours | `mask`, `mask/` | `mask/mask_game.gd` | `mask/mask_logic.gd` | `modes/masked_hours.rs`, `mask/vent_lights.rs`, `modes_play/masked_hours.rs` | Playable: mask, monitor, music box, flashlight, vents, call box, six nights |
| lure | Dead Air | `lure`, `lure/` | `lure/lure_game.gd` | `lure/lure_logic.gd` | `modes/dead_air.rs` (rules), `modes_play/dead_air.rs` (play) | Rules and play: booth seals, heater and fan, monitor with the station map, lure, reboots, phantoms, climate, call box, endings, save on a win |
| bedroom | Bedtime | `bedroom`, `bedroom/` | `bedroom/bedroom_game.gd` | `bedroom/bedroom_logic.gd` | `modes/bedtime.rs`, `modes_play/bedtime.rs` | Rules and play |
| story | The Facility | `story`, `story/` | `story/story_game.gd` | `story/story_logic.gd` | `modes/facility.rs`, `modes_play/facility.rs` | Rules and play: rooms, task list, station puzzles, noise, beacon, lift, call box, sign-off and catch screens |
| descent | The Descent | `descent`, `descent/` | `descent/descent_game.gd` | `descent/descent_logic.gd` | `modes/descent.rs`, `modes_play/descent.rs` | Playable |
| dayshift | Salvage Run | `dayshift`, `dayshift/` | `dayshift/dayshift_game.gd` | `dayshift/dayshift_logic.gd` | `modes/salvage_run.rs`, `modes_play/salvage_run.rs` | Playable: day shop, rack, window, quota, call, endings |
| extra | Open Air | `extra`, `openair/` (scene root `night_watch_extra.tscn`) | `openair/openair_game.gd` | `openair/openair_logic.gd` | `modes/open_air.rs`, `modes_play/open_air.rs` | Rules and play: three watches, torch, lantern oil, hatch and winches, campfire, call box, endings, Custom Night chain |
| mixed | Mixed Shift | `mixed`, `mixed/` | `mixed/mixed_game.gd` | `mixed/mixed_logic.gd` | `modes/mixed_shift.rs`, `modes_play/mixed_shift.rs` | Playable: shift select (five shifts, ENDLESS last), threat roster and director, vents, doors, monitor with the building map, call box, endings, save |

Hub order (Godot and `shell.rs`, nine entries): 1 Classic, 2 Masked Hours, 3 Dead Air, 4 Bedtime, 5 The Facility, 6 The Descent, 7 Salvage Run, 8 Open Air, 9 Mixed Shift.

The one-line files `modes/{bedroom,dayshift,extra,lure,mask,mixed,story}.rs` are old stubs that `modes/mod.rs` no longer declares; they are not compiled.

## Per-mode notes

### classic (Classic)

- Godot: `night_watch_classic.gd` holds the rules. Its `step(dt_ms, input)` takes door, light and monitor state and returns events (`moved`, `arrived`, `repelled`, `pushed`, `flinch`, `attack`, `out`, `caught`, `win`).
- Rules and scoring: seven nights, two steel doors and two lights, eight cameras, one power meter, 6 AM win, power loss opens doors and kills lights. Points per level are 10 (`POINTS_PER_LEVEL` in `night_watch_modes_data.gd`). Custom Night sliders run 0 to 20 per hunter.
- Hunters: Dovelle, Nyxie, Ravelle, Ilyra, Aki (cast). Night 7 has Kathy alone (TODO).
- Assets and sounds: `nw_door_left` and `nw_door_right`, `nw_light_left` and `nw_light_right`, `nw_lamp`, `nw_name`, plus the Night Watch cue family. Building data is in `data/night_watch/buildings/classic.json`.
- Rust: rules and play are done (see the table above). The call script and the save clear rule exist.

### bedtime (Bedtime)

- Godot: `bedroom/bedroom_game.gd` is the shift scene, `bedroom_logic.gd` the rules. The walkers are `NightWatchWalker` for Javelle and a creeping path for Haelith. `bedroom_map.gd`, `bedroom_paths.gd` and `bedroom_wing.gd` give the layout.
- Rules: no cameras. Hold A to shut the closet door (Loxelle breathes behind it), D to light the hall (Javelle waits there), F to light the foot of the bed (Haelith creeps in while the room is dark). One hand, so one action at a time. The flashlight battery never recharges. Holding the door uses both hands. Presets: Closet Door, Hall Watch, Lights Out, All Twenty.
- Scoring: the shared 10 points per level and the rating table.
- Assets and sounds: `bedroom_sounds.gd` (`BedroomSounds.breath`, `step_in`), a dark bedroom, the pillow-view look controls, the shared Sound autoload and the walker.
- Rust: rules and scoring are in `modes/bedtime.rs`. Play is in `modes_play/bedtime.rs`: hold A (closet door, no light while held), hold D (hall light) or F (bed light), TAB for the floor plan (the house by sound: a marker shows the room a hunter's sound comes from), the call box from `calls/bedroom.json`, the battery, the win at 6 AM (saved as `bedroom` in the progress file) and the caught endings. The Custom Night starts from Night Select (`StartCustom`, three sliders). Not done in this file: the pillow view and look controls (the plan view and bed art are used instead), the score and rating on the result screen, and the breathing audio.

### salvage_run (Salvage Run)

- Godot: `dayshift/dayshift_game.gd` runs the day shop (`dayshift_shop.gd`) and the night in the Classic security room with a workbench, a four-slot rack and a window. `dayshift_logic.gd` holds the rules, and `dayshift_yard.gd`, `cargo_bay.gd` and `dayshift_map.gd` give the layout.
- Rules: spend the week's budget on upgrades by day, then hold USE to salvage pieces off the rack at night. Salvaging is loud and makes the visitors keener. Doors and door lights are as in Classic. Ysolde watches the window. Quota and `short {money, quota}` events decide the night. Power is tuned for an eight-minute night (TIME_SCALE comes from `data/night_watch/rules/classic.json`).
- Hunters: Ravelle, Ceryth and Ysolde (cast). Presets: Left Door, Right Door, After Hours, All Twenty.
- Scoring: the rack value per salvage event (`salvage {slot, value}`), the quota, and the shared 10 points per level.
- Assets and sounds: `theme_nightwatch.tres`, `night_watch_player.tscn`, the shared art, and the nw cues. The rules use the same Mulberry stream as the 2D game.
- Rust: the rules and scoring are in `modes/salvage_run.rs`; the playable driver is `modes_play/salvage_run.rs` (hub id `dayshift`, in `shell::PLAYABLE`). Shop: 1 to 5 or a click buys and drops an upgrade, ENTER starts the shift. Night: A and D shut the side doors, Q and E hold the lights, 1 to 4 pick a rack slot and SPACE (held) or a held click salvages it, M mutes the call. Each week's call is `calls/dayshift.json`. A cleared week saves `cleared_salvage_run`, a short or caught week is tried again with the same carry, and week 5 ends the schedule. The room is `assets/offices/dayshift.png` with `dayshift-fg.png` over it, and every number and label is in `data/rules/salvage_run.json` (`ui` holds the timings of the screen). Test: `cargo test --bin night_watch salvage` (a scripted campaign reaches an ending, the shop keeps to the budget, the call plays to its end). Screenshot: `night_watch --mode dayshift --screenshot PATH --week N --frames N` (add `--shop` for the shop). Still missing: the Custom Night from the hub, and the window's own picture (Ysolde is a figure at the glass).

### descent (The Descent)

- Godot: `descent/descent_game.gd` is a fixed view from inside an elevator, driven by `descent_logic.gd`. The panel is `descent_panel.gd`, the shaft is `descent_world.gd` and `descent_office.gd`. The cursor is free, with no player controller.
- Rules: three puzzle types. Wires light in turn, and a wrong button re-deals them. Memory is a four-pad signal sequence. Pipes need one line from the left edge to the right edge, and Elise closes in at half speed. Hold SPACE to keep the door shut on the last stage. Elise walks the sill and reaches the glass as the closeness bar fills.
- Hunter: Elise only. Presets: Slow Ride (5), Service Run (10), Express (15), Free Fall (20).
- Scoring: stage clears and the 10 points per level.
- Assets and sounds: `descent_data.gd`, `descent_world.gd`, the shared Sound autoload and the HUD call box and end card.
- Rust: playable. The driver is `modes_play/descent.rs` (menu, car and window with Elise, the service panel, HUD, call box, result). Controls: keys 1 to 6 or a click press a wire button or a signal pad, arrows and ENTER (or a click) turn pipe pieces, SPACE (or the door button) holds the door. The menu lists Descents 1 to 5 (each opens once the one before is cleared) and the four presets from `presets` in `data/rules/descent.json` (Descent 5 at the preset's Elise level, no call, opens once Descent 5 is cleared). A win saves `cleared_descent` (campaign) or the best and star (preset). Labels, presets and the menu levels are in the rules file. The bot in the driver's tests solves the puzzles to check each run reaches an end. `--descent-shot PATH [--descent N] [--solve K] [--frames F]` writes a picture (`--descent 0` for the menu). Art: the Godot project has no `classic.png` or `classic-fg.png`, so the car is drawn with primitives.

### masked_hours (Masked Hours)

- Godot: `mask/mask_game.gd` is the shift scene (Classic security room, no doors, dark, with a wing of seven rooms on the monitor). `mask_logic.gd` is a port of the 2D mask logic and matches the 2D game event for event for the same seed. `mask_controls.gd`, `mask_world.gd` and `mask_map.gd` give controls and layout.
- Rules: wear the mask (exclusive with the monitor) when hall hunters walk in. Watch cameras 1 to 8 for who is coming. Hold R to wind the music box, which jumps the monitor to CAM 1. Hold G or H for the vent lights and X for the hall light. Keep the flashlight alive, because when it dies someone comes. Presets: Hall Pass, Vent Duty, Wound Tight, All Twenty.
- Hunters: Verrine, Thessine, Glyra, Nyxie, Umbriel, Haelith, Ilyra (cast). The events include `room`, `hall`, `step`, `leave`, `rush`, `flashHit`, `duct`, `vent`, `fetch`, `creep` and `dark`.
- Scoring: the shared 10 points per level and the rating table.
- Assets and sounds: `night_watch_mask.tscn`, `night_watch_player.tscn`, the music box cue and the vent cues. The validator `validate_night_watch_mask.gd` and `tests/fixtures/gen_night_watch_mask_golden.mjs` give golden output.
- Rust: rules and scoring are ported in `modes/masked_hours.rs`, and the vent lights in `mask/vent_lights.rs`. The driver in `modes_play/masked_hours.rs` puts it on screen: hold S for the mask (not with the monitor up), F the flashlight, L the desk lamp, R the music box, Q and E the vent lights, SPACE or W the monitor, arrows and 1-8 the cameras. Its tunables are in `data/rules/masked_hours.json` (including the `ui` block). A win is saved; a loss retries the night. Open: the vent lights work from the desk, since this wing has no panels to clear.

### dead_air (Dead Air)

- Godot: `lure/lure_game.gd` is the broadcast booth with two door buttons (vent seals), two light buttons (heater and fan) and a monitor onto the radio station. `lure_logic.gd` holds the rules. The hall walk for Xivvy uses `lure_paths.gd` and `NightWatchWalker`. `lure_world.gd` and `lure_map.gd` give the layout.
- Rules: one real hunter, Xivvy, crawls the building toward the office vents from night 2. Three phantoms (Ferryn, Briselle, Mavrine) knock out systems. Climate is driven by Solyth and Wynelle. Below 60 F Wynelle freezes the office, and from 95 F Solyth cooks it. The heater and fan fix the number but strain the air. Keys in the monitor: 1 to 8 for cameras, L to lure Xivvy to the room on screen, Q, W, E, R and T to reboot audio, cameras, vents, heater and fan, N to mute Zellia. One more hunter each night, never more than five at once. Presets: Maintenance Only, The Real One, Climate Control, All Twenty.
- Assets and sounds: `lure_sounds.gd` (`LureSounds.moved`, `at_vent`), `night_watch_lure.tscn`, the heater and fan cues, and the shared player.
- Rust: rules in `modes/dead_air.rs`; the driver `modes_play/dead_air.rs` is playable. Booth: A and D seal the vents, H heater, G fan (art from `assets/offices/lure.png` with `lure-fg.png`, vent quads from `data/rules/dead_air.json` `art`). Monitor (SPACE): 1-8 cameras, L lure, Q, W, E, R, T reboot audio, cameras, vents, heater, fan; M or N mutes the call. Every label and number is in `data/rules/dead_air.json` (`play`). The Custom Night starts from Night Select (`StartCustom`, night 5 unlocks it). Still missing: the Xivvy walk on the booth floor (she is only shown on the map), and sound beyond the shared cues. Dead Air hallucinations are left out by decision.

### facility (The Facility)

- Godot: `story/story_game.gd` is a walkable scene of five blocky rooms with a task station in each. `story_logic.gd` holds the rules. `story_world.gd`, `story_map.gd`, `story_office.gd` and `story_panel.gd` cover layout and UI. `story_admin.gd` holds admin switches.
- Rules: finish the night's task list in order at each room's station. Moving loudly makes noise, crawling is slow and quiet, and each action is a little noisy. Walking through a door takes 1.8 s (3.6 s crawling, hold C), and the rules count it as the 2D `move`. Hold F for the beacon. Hold H to hide under the desk in Control. Hold E on the lift's call button. Presets: Quiet Gallery, Behind the Steel, Lift Schedule, All Twenty.
- Hunters: Opalie (gallery, closes in on noise), Porcelle (workshop and auditorium, closes in while dark, held back by the beacon), Quenelle (rides the lift and opens at Control).
- Assets and sounds: `story_sounds.gd` (`StorySounds.hunter_close`, `hunter_back`), `night_watch_story.tscn`, and the shared player.
- Rust: the rules are in `modes/facility.rs` and the playable driver is `modes_play/facility.rs` (hub id `story`, in `shell::PLAYABLE`). Controls: A, D and W walk the left, right and lift doors (C held to crawl), F holds the beacon, H hides in Control, E holds the lift call button. Digits (the code) and 1 to 9 (dials, parts, breakers) press the station, and SPACE turns the crank; a mouse works on every button. The note on the station shows the code, the dial marks and the breaker order, and the list on the left ticks off each task. The room pictures are Godot's `story` art (`assets/offices/story*.png`). A signed-off night is saved under `story` with 10 points per task. Test: `cargo test --bin night_watch facility` (the bot signs off all six nights in god mode, and a shift without god mode always ends). Screenshot: `night_watch --mode facility --screenshot PATH --night N --frames N`. The Custom Night starts from Night Select (`StartCustom`, night 6 unlocks it). Still missing: the walk-through sounds and the lift's own room picture for Quenelle's ride.

### open_air (Open Air)

- Godot: `openair/openair_game.gd` is the shift scene, and the root `night_watch_extra.tscn` points to it. `openair_logic.gd` holds the rules for three watches. `openair_world.gd` builds the walkable sets, and `openair_pacer.gd`, `openair_data.gd` and `openair_paths.gd` give pacing, data and walking routes.
- Rules: the watch is the level. 1 ROOFTOP has Verrine climbing by the stair door or the hatch, a torch keeps her still, and the lantern oil runs the winches (barriers swing open when it is out). 2 BUS STOP and 3 CAMPFIRE are the other two watches, each with its own hunter and way of keeping the dark off. Custom Night chains the watches whose hunters are on. A held watch offers NEXT WATCH, and only the last win is recorded (`record_win`). The wind blows the lantern out. Presets: High Ground, Cold Road, Dusk to Dawn, All Twenty.
- Hunters: Ysolde, Verrine, Wynelle, Quenelle (cast).
- Scoring: the watch win and the shared 10 points per level, with the last win recorded.
- Assets and sounds: `openair_data.gd`, the shared art and the Sound autoload. The Godot check is `validate_night_watch_extra.gd`.
- Rust: rules and scoring are in `modes/open_air.rs`; the driver `modes_play/open_air.rs` (about 1100 lines) plays the three watches and the Custom Night chain (no calls in the chain), with the call box, the endings and a save of the cleared watch or the Custom score. It opens from the hub through the `extra` arm in `main.rs`, not through Night Select. Test: `cargo test --bin night_watch open_air` (each watch and the chain reach an end under `run_script`). Still missing: the Custom Night sliders on a select screen (the chain is picked inside the driver) and sound for the watches.

### mixed_shift (Mixed Shift)

- Godot: `mixed/mixed_game.gd` is the shift scene, a transit hub (`mixed_map.gd`) around the security room with three permanent vents and a vent run above the building (CAM 9). `mixed_logic.gd` (1283 lines) holds the threat roster and the pacing director. `mixed_skin.gd` covers the look. Hunters walk on `NightWatchPaths` and `NightWatchWalker`.
- Rules: three to seven threats are drawn each night from a pool of eight. Each has a tell before it can kill and an answer (`a` and `beat`). Threat types are counter, watched, monitor, path, vent and winder. The director paces the beats, spikes and moods. There are no captions, only sounds, lights, figures and pulsing camera buttons. The five shifts are QUIET, STEADY, RESTLESS, CHAOS (random events) and ENDLESS (no 6 AM, best hour saved by `NightWatchSave.record_best`). Controls: Z and X toggle doors, V, B and N hold to seal the left, top and right vents, G, Y and H tap the vent light, R winds the music box, U is the monitor, 1 to 9 pick cameras.
- Assets and sounds: `night_watch_player.tscn`, `night_watch_art.gd`, the shared cue family, and the vent and door cues. The golden fixture is `tests/fixtures/gen_night_watch_mixed_golden.mjs`.
- Rust: rules in `modes/mixed_shift.rs`; the driver is `modes_play/mixed_shift.rs` (hub id `mixed`, in `shell::PLAYABLE`). The shift select lists the five shifts from `shifts` in `data/rules/mixed_shift.json`, ENDLESS last. Night: Z and X shut the left and right doors (a door pops open when its hold runs out), V, N and B hold the left, right and top vent seals, G, H and Y tap the vent lights, R winds the box, U raises the monitor, and 1 to 9 or the arrows pick a camera. The picture is `art/mixed` (copied from the KyttusLab offices folder: `mixed.png` with `mixed-fg.png` over it, and the vent overlays). The call is the `call` block of the rules file: two lines, marked as pitch until KT approves them, since Godot's mixed shift has no call. Tests: `cargo test --bin night_watch mixed` (every shift reaches an end under a scripted presser and a reactive bot, and the call plays). Screenshot: `night_watch --mixed-screenshot PATH --shift N --frames N [--cam N]`. Still missing: the Custom Night, sound for the tells, and a scripted win. The bot loses every shift it plays, so the win and save path is not yet run live.

## Night Select (`select.rs`)

Choosing a night type on the hub opens its night list: Classic, Masked Hours, Dead Air, Bedtime, The Facility, The Descent, Salvage Run and Mixed Shift. Open Air keeps its own watch select.

- Nights: the count comes from each mode's rules JSON, through the path in `data/select.json` (`nights`). Names come from the rules too where they have them (Descent's levels, Mixed Shift's shifts).
- Locked and cleared: night 1 is open, and a night opens once the one before it is cleared (Classic keeps its own unlock). The best score per night is `night_best_<mode>` in the progress file, written when a campaign night is won (Bedtime does not write one yet).
- Custom Night: the C key or the last row. Its sliders, max and points per level are in `data/select.json`; each slider clamps to `max`, and the score is the sum times `points_per_level`. It unlocks once `unlock_night` is cleared. The slider values are saved per mode (`custom_<mode>`). A Custom Night writes no clear and no score.
- Custom Night exists for Classic (sliders on night 7), Masked Hours (night 6), Dead Air (night 5), Bedtime (night 5) and The Facility (night 6). Not wired yet: Salvage Run and Open Air (their Godot sliders exist, the Rust drivers do not take them), The Descent (its Godot slider is Elise, the Rust driver starts from presets) and Mixed Shift (no sliders).
- Screenshot: `night_watch --select-shot PATH --select-mode classic [--custom]`. Tests: `cargo test --bin night_watch select::`.

## Shared framework

These items serve every mode and sit in `src/bin/night_watch` (framework, data, save, shell).

- Hub and modes: `shell.rs` lists the nine entries; `modes/mod.rs` holds `Session`, one variant per mode, started by hub id.
- Calls: `data.rs` loads `calls/*.json` (eight files; Mixed Shift's call sits in its rules file). The call box is `typewriter.rs` plus `calls_view.rs` and `blip.rs`. In Classic's play screen the call box sits under the HUD bar, above the monitor panels.
- RNG: `framework.rs` has the Mulberry and stream code that matches the 2D game. Each mode keeps the same draw order to stay in step with the 2D and Godot goldens.
- Scoring: the 10 points per level and the rating table are shared; per-mode scoring is in each rules file.
- Sound: `nw_sound.rs` renders the game's `nw_` cues as a small procedural WAV renderer (Night Watch cannot import the game binary's cue table) and plays them with macroquad audio: `cue(name, pan, k)`, `event(mode, event, pan)` and `room_tone(name, on)`. The cue layers and the per-mode event-to-cue map are in `data/sounds.json`; at this writing the `events` map holds Classic only, so the other modes add their block as their drivers call `event`. The module is not declared in `main.rs` yet (hook for the integrator: `mod nw_sound;`). The Godot `sounds/` and `scares/` folders are empty in this checkout.
- Save: `save.rs` keeps `cleared_<mode>` and `NIGHTS`. Mixed Shift saves a cleared shift as `cleared_mixed_shift` and its best points under `mixed_shift`; the ENDLESS best hour is kept under `mixed_shift_endless`.

## Verification

`cargo test --locked --bin night_watch` runs the mode and data tests (the count changes with each pass; run it to see it). `night_watch --screenshot PATH --call-chars 20` renders a Classic frame with a call open (add `--cam 1` for the monitor view). The Godot line counts and mode values were taken from the files listed above on 2026-11-03.
