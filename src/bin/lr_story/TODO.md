# KYTTU - LRS TODO (lr_story)

Working list for the lr_story game (`src/bin/lr_story`; binary, pack id and save keys keep the name `lr_story`). Agents:
read this first, tick items as they land, and add new ones at the bottom of their section with the date.
Audited 2026-11-08 against the code, `docs/LR_STORY.md` and `packaging/story-samples/make-samples.mjs`. Files of the
medieval pass (`textures/medieval.rs`, `textures/mod.rs`, `model.rs`, `scenery.rs`, `floor.rs`, `main.rs`,
`make-samples.mjs`) are off limits to the "Next (small, safe)" items below.

## Decided by KT
Newest first.
- [x] 2026-11-08: medieval old town. `sample_city` (id unchanged, title "Sample: The Old Town") becomes a medieval town:
      buildings, roads and old-style props. New wall styles `timber` and `stone`, floors `cobble` and `dirt`, ceiling
      `beams` for indoor maps with timber or stone walls. Existing styles (lab, brick, concrete, plaster; blocks, asphalt,
      paving, tiles) stay for other packs. Flat textures mode gets a 2-3 tone version of each. Land state: verify against
      `textures/` and `docs/LR_STORY.md` (the `walls`/`floor` tables) when the pass is merged.
- [x] 2026-11-08: title is "KYTTU - LRS". Binary, folder, pack id and save keys keep `lr_story`.
- [x] 2026-11-08: a spawn's `facing` is optional (compass name or degrees); a spawn without it keeps the body's current
      look (`load.rs` `parse_facing`, test `spawn_facing_reads_names_and_degrees`).
- [x] 2026-11-07: combat is action only (real time). No turn-based mode. Fixes in `rpg.rs`: tile-path chase, fitting
      spawn, no melee through walls, telegraphs equal hit shapes, white-tint flash, HP 0 never saved.
- [x] 2026-11-07: NO XP and NO levels. Removed from code (`rpg.rs` says so), docs (`LR_STORY.md` lines 128, 166, 178:
      old `xp` op is a load error, old save fields ignored). Kept: HP and stats, items with `use`, quests, journal, inventory.
- [x] 2026-11-07: indoor maps get a ceiling, outdoor maps a sky (`MapDef`: `outdoor`, `ceiling {height,color}`, `sky`).
- [x] 2026-11-07: per-map `walls` and `floor` styles; unknown names are a load error.
- [x] 2026-11-07: sample pack `sample_city`, made by `make-samples.mjs`.
- [x] 2026-11-05: reuse last_route lab assets by copy, never by move; last_route is not edited.
- [x] 2026-11-05: LORE `Choices:` format (`Text`, `Sets`, `Needs`, `Goto: Chapter N`) built as written; KT to confirm.

## Next (small, safe)
Each is under an hour for one agent. Proof is `cargo test --bin lr_story` (run by the test agent, not the edit agent).
1. [ ] Ambient light row for the lab. `assets/data/regions/` exists but there is no `region.lab` ambient row, so labs use
       the default purple-grey. Files: `assets/data/regions/` (new row), the reader (`props/` or `look.rs`).
       Test: new `ambient_lab_row_is_read_and_unknown_region_gets_default`.
2. [x] 2026-11-08 (test `lobby_code_is_trimmed_uppercased_and_clamped`; `multiplayer::clean_room_code`, used by `menu.rs`) Lobby code and name trimming/clamping is untested (item was only half done). Files: `multiplayer.rs` (pure helper
       plus tests; menu code in `menu.rs` calls it). Test: `lobby_code_is_trimmed_uppercased_and_clamped`.
3. [ ] `ui::wrap` / text fitting has no test (needs a macroquad context). Split the width measure behind a closure so a
       fixed-width fake can drive it. Files: `ui.rs` is off limits, so add a thin pure wrapper in `hud.rs` and test it
       there. Test: `wrap_breaks_on_spaces_and_never_exceeds_the_width`.
4. [x] 2026-11-08 (test `a_pack_with_a_start_scene_but_no_scenes_is_refused`; `validate` already refused it, only the test was missing) Missing top-level `scenes`/`items`/`legend` keys read as empty (`load.rs` line ~609 "A missing or null one is
       empty"). Make a missing `scenes` a load error for a pack that declares a start scene; keep items/legend lenient.
       Files: `load.rs`, `docs/LR_STORY.md` (note). Test: `a_pack_with_a_start_scene_but_no_scenes_is_refused`.
5. [ ] Per-line conditions: the runtime only gates choices. Add an optional `needs` on a talk line and skip the line when
       it fails. Files: `model.rs` is off limits, so first check whether `Line` already carries it; if not, put this under
       Next (bigger). Test: `a_line_with_a_failed_need_is_skipped`.
6. [x] 2026-11-08 (already covered by `no_path_for_four_seconds_moves_the_enemy_next_to_the_player`) Enemy no-path rule (`rpg.rs` `NO_PATH_SECS` 4.0, teleport next to the player) has a constant but no test over a
       sealed map. Files: `rpg.rs` (tests module). Test: `an_enemy_with_no_path_is_moved_next_to_the_player_after_the_timeout`.
7. [x] 2026-11-08 (test `a_closed_door_blocks_the_chase_path_and_an_open_one_does_not`; doc line not added, LR_STORY.md is off limits) Closed doors in enemy pathing: unclear whether `find_path` treats a closed door as a wall. Pin it with a test and a
       doc line. Files: `rpg.rs` (tests), `docs/LR_STORY.md`. Test: `a_closed_door_blocks_the_chase_path_and_an_open_one_does_not`.
8. [ ] 2026-11-08 half done: `hud::clip_tag` and test `a_long_npc_name_is_clipped_with_an_ellipsis_and_an_empty_one_draws_nothing` exist; wiring into `main.rs` tag drawing still to do. NPC name tags (`main.rs` `name tag` size/opacity maths is tested) are not checked for an NPC with an empty or very
       long name. Move the clamp into a pure fn in `hud.rs` and test it. Files: `hud.rs`. Test:
       `a_long_npc_name_is_clipped_with_an_ellipsis_and_an_empty_one_draws_nothing`.
9. [ ] Save edge cases: a save whose flags name a scene that no longer exists (pack updated). Check `play.rs` falls back to
       the start scene with a toast instead of panicking. Files: `play.rs`/`save.rs` (tests). Test:
       `a_save_pointing_at_a_removed_scene_restarts_at_the_start_scene`.
10. [ ] Spawn facing only applies on spawns that set it; an exit that arrives at an unset spawn keeps the old look, which
       can face a wall. Decide the default (face the nearest open tile) in `load.rs` `parse_facing` callers. Files:
       `load.rs`. Test: `a_spawn_without_facing_faces_open_floor_when_the_old_look_faces_a_wall`.

## Next (bigger)
- [ ] Medieval content follow-up: real timber/stone props (market stall, well, cart, barrels, signs, stocks) as bbmodel
      props in `sample_city`. Files: `make-samples.mjs`, `assets/props/`. Today the sample props are simple cubes.
- [ ] Sample skins are recolours: the five atlases in `assets/player/staff/` (Felix, Renata, Owen, Priya, Dia) are
      recoloured copies of the mannequin atlas. Per-NPC `skin` selection works (`mannequin.rs` `Figures::draw_npc`);
      what is missing is real art.
- [ ] Combat pathing caveats: repath rate (`REPATH_SECS` 0.3) on big maps, moving props, doors, the path visit cap. Needs a
      headless test over a maze map and over `sample_city`.
- [ ] Wall/floor painters: `concrete`, `plaster`, `asphalt`, `paving`, `tiles` are checked only as "exist"; confirm each has
      its own painter rather than a recolour (`textures/walls.rs`, `floors.rs`).
- [ ] Ceilings for non-flat map heights (stairs, mixed wall heights) and light falloff under a ceiling.
- [ ] Per-region ambient/lighting as pack data (Next item 1 is the first step).
- [ ] Browser build: a Playwright playtest (`scripts/playtest-*.mjs` style) for lr_story web; none exists.
- [ ] Sound: no audio in lr_story (hits, steps, music). `KyttusLab-MIDI` is the generator.
- [ ] Godot `infinite_route_stories.json` node graph schema: port or keep separate (see Needs KT).
- [ ] Doc/code sync check for the `walls`/`floor`/`ceiling` style tables in `docs/LR_STORY.md` (line ~205 lists only the
      four old walls): add a test that every style name the loader accepts is in the doc.

## Needs KT
- [ ] Mouse look: yaw sign and scale (`body.rs`, `look.rs`, `main.rs` line ~879). Reported left-right inverted and 2x too
      fast; the up/down fix was made by feel. Needs a hands-on check in the game window with a mouse, then one change.
- [ ] Confirm the LORE `Choices:` format.
- [ ] Godot node-graph schema: port to `lr.story.scenes/1` or keep separate.
- [ ] Enemy with no path: teleport next to the player after 4 s (current) or a timeout win?
- [ ] Real art for staff characters, sample skins and sample props.
- [ ] Which pack mix ships on the site (lore packs single-player; samples and the Old Town multiplayer?).
- [ ] Medieval pass: confirm the look in the window (timber, stone, cobble, dirt, beams) and flat textures mode.

## Done
Pack, loader and tools
- [x] Pack loader: core + mod packs, `lr.pack/1`, `<pack id>_` prefix, `overrides`, one id in two folders refused (`load.rs`)
- [x] YAML subset parser plus tab, CRLF, BOM and empty-file tests (`yaml.rs`); LORE importer (`lore.rs`), rejoining branches test
- [x] CLI: `init`, `check`, `pack`, `import-lore` (`main.rs`)
- [x] `docs/LR_STORY.md`; a test keeps every loader op in it and `xp` out (`lore.rs`)
- [x] Import guard (`guard.rs`) passes; the banned-word test name in `textures/sets.rs` was renamed
- [x] Spawn `facing` (optional) with parser and test (`load.rs`)
- [x] Prop draw scales live in `assets/data/render/prop_draw_scale.json` with a test (`props/prop_scale.rs`)
- [x] Wall lab PNG fallback is 32x128, with a test (`textures/mod.rs`)
- [x] Net: malformed and empty reply tests (`net.rs`); guest lease token tests (`multiplayer.rs`)
- [x] Colour helper tests (`ui.rs`)

Play and saves
- [x] First-person body (`body.rs`), 3D renderer (`main.rs`, `floor.rs`), choice runtime and atomic saves (`play.rs`, `save.rs`)
- [x] Action combat, quests, journal, inventory with `use` items (`rpg.rs`, `battle_ui.rs`)
- [x] NPC name tags that shrink and fade with distance (`main.rs`, test `name_tags_shrink_and_fade_with_distance`)
- [x] Per-NPC skins path in `mannequin.rs` (art still placeholder, see Next bigger)
- [x] Server and lobby client (`net.rs`, `multiplayer.rs`)

Look and content
- [x] UI overhaul (`ui.rs`, `menu.rs`, `hud.rs`)
- [x] Lab textures painter (`textures/`; floors 32x32, walls 32x128), deterministic, `--flat-textures` honoured
- [x] bbmodel props and mannequin clips (`props/`, `anim/`, `mannequin.rs`)
- [x] Sample packs greenhouse, lighthouse, archive, city (`make-samples.mjs`); simulora pilot tests

## Superseded / removed
- XP, levels, `op: xp`, `battle.xp`: removed by KT 2026-11-07.
- Turn-based combat plan: removed.
- Old "sample_city is a modern city" scope: replaced by the medieval old town (2026-11-08).

## Notes for agents
- The core pack is `src/bin/last_route/data/lr_story` (id `lr_story`). Mods go in `mods/<id>/` (folder = id).
- Ids a mod adds start with `<id>_`. Replacing another pack's id needs an `overrides` entry.
- Browser build: no threads, `std::fs` or `std::time::Instant` in game code; read files through `crate::assets_vfs`.
- Edit agents do not run cargo; the build and test agents run `cargo test --bin lr_story` and `lr_story --check`.
