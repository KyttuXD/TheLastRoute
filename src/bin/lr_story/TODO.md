# lr_story TODO

Working list for the lr_story game (`src/bin/lr_story`). Agents: read this first, tick items as they land, and add
new ones at the bottom of their section with the date. Source notes: the 2026-11-05 survey (story and texture
reports), and the code paths cited.

## Done
- [x] Pack loader with core + mod packs, `lr.pack/1` manifests, `<pack id>_` id prefix, `overrides` (`load.rs`)
- [x] KyttusLab-LORE YAML subset parser (`yaml.rs`) and chapter importer (`lore.rs`): one scene per chapter, chained
- [x] First-person body ported from last_route: walk, sprint, crouch, jump, stamina, head bob, mouse look (`body.rs`)
- [x] 3D renderer: map PNG as floor, walls and doors as boxes, NPCs, items, exits (`main.rs`)
- [x] Mouse up/down (and left/right) inversion fixed: macroquad's delta is last minus current, so it is negated

## Story: semi-choice
- [ ] Decide the LORE choice format: an optional `Choices:` list per chapter, with `Text`, `Sets` (flags), `Needs`
      (flags) and `Goto`. Survey: no choice markers exist in any LORE story today, so this is an authoring decision
      for KT, not a data fix. Owner: KT.
      2026-11-05: built as written here (`lore.rs` `parse_choice`, `Goto` is `Chapter N`); KT still to confirm it.
- [x] Map `Choices:` in `lore.rs` to `choices: [{text, if, do: [set_flag], next}]`. Each choice's `next` must be set,
      or `play.rs` ends the scene on it and breaks the chain (`play.rs` `choose`).
      2026-11-05: done. Default `next` is the next chapter; the last chapter's choice has none. Tests in `lore.rs`.
- [x] Persist flags. They are session-only today (`play.rs` `flags`), so a semi-choice story cannot carry across a
      restart. Needs a save file in the user data folder (`last_route_core::shared::paths::user_file`).
      2026-11-05: done. `save.rs` writes `lr_story_flags.json` (keyed by the core pack id); `main.rs` loads at start and
      saves on each pick. Round-trip tests in `save.rs`.
- [ ] Decide whether Godot's `infinite_route_stories.json` schema (node graph, no flags) ports to `lr.story.scenes/1`
      or stays separate. Godot caps choices at 6 (`infinite_story_director.gd` line 14). Owner: KT.
- [ ] Test: a LORE file with one choice imports, and the two branches rejoin at the next chapter.
      2026-11-05: partly covered. `lore.rs` tests a file with choices and their `next`, and the end-to-end test
      plays one choice into chapter two. No test yet has two branches that rejoin.

## Maps and textures: labs
> **Superseded 2026-11-05:** the decision is reuse by copy, not move. Every "move into the lib" line below means "copy into lr_story". Ticked items are copied and tested.
- [x] The lab texture painter is in the last_route bin (`src/bin/last_route/render/textures/`), not the lib. A separate
      bin cannot use it without moving it into `last_route_core` or copying it. Decide which. Owner: KT.
- [ ] Lab floor kinds are `epoxy0` and `epoxy1` (plus `grate`, `hazard`, `metal1` in some rooms). Lab walls are `lab0`
      and `lab1`; the ceiling is `ceil0`. These are generated RGBA buffers (floors 32x32, walls 32x128), not PNG files.
- [ ] `lr_story` maps need their own lab textures as PNG, or a port of the painter. Until then the floor is the map
      art PNG (`maps/*.png`), which is the intended "custom PNG drawn map" path.
- [ ] Ambient light: the last_route ambient file (`data/render/ambient.json`) has no `region.lab` row, so labs use the
      default purple-grey. Add a lab row in lr_story's own data, not last_route's.
- [x] DECIDED 2026-11-05: region "office" uses the office/maze set (`sets.rs` line 420), because the office props
      (desks, chairs, cabinets, server racks) ship in `props/maze`. "lab" uses the lab set. `world/mod.rs` line 33
      (office -> lab) is treated as wrong for lr_story and is not copied.
- [x] DECIDED 2026-11-05: reuse by copy, never by move. Last_route files are not edited. Assets are copied into
      `src/bin/lr_story/assets/` (11 MB: props/blocky 220, props/maze 102, player mannequin and rigs, environment lab and
      maze textures, prop_draw_scale.json, player models.json and anim.json).
- [ ] The wall swatch `assets/environment/wall_lab.png` is 32x32, not the 32x128 the wall tiling expects. Only a
      fallback, but it shows in the 3D view if a map has no art.

## Props and characters (3D, from last_route)
Survey 2026-11-05, workflow `wf_61fbcc13-95d` (journal in `subagents/workflows/wf_61fbcc13-95d/journal.jsonl`).
- [x] Props: reuse the data, port only the parser. Lab set is `assets/props/blocky/` (220 models), office and server
      set is `assets/props/maze/` (102). Skip the 351 root-level `props/*.bbmodel` (orphans, `build.rs:95`).
      Move `render/bbmodel.rs` (`Model::parse`, line 350) into the lib. Replace `shared::assets::try_tex` (line 94)
      with an injected texture lookup. Do not move `prop_model.rs`, `world_mesh.rs`, `furnished.rs` or `facility.rs`.
- [ ] Prop draw scales (from `data/render/prop_draw_scale.json`): labStool 1.5, labCoolingUnit 1.5, fumeHood 1.2,
      serverRack 1.5, officeDesk 1.5. Copy into a lr_story pack file, not hardcoded.
- [x] Lab textures: port the painter (`render/textures/` mod, sets, floors, walls, ceilings). It is pure and
      deterministic. Floors 32x32 (epoxy0, epoxy1, grate, hazard, metal1), walls 32x128 (lab0, lab1), ceiling ceil0.
      Move the lab rows into a lib module. Keep the 32x32 `environment/floor_lab.png` and `wall_lab.png` as fallbacks.
- [x] Characters: mannequin only, never hunters. Use `assets/player/player_mannequin.bbmodel` and `_slim.bbmodel`,
      rigs in `assets/player/anim/`, wiring at `data/player/models.json:4-26`. Move `render/anim/clip.rs` and the
      runner parts of `render/anim/runner.rs` (`clip_for` 335, `player_rig` 402, `pose` 524). Do not move
      `render/anim/hunter.rs` (it imports game state: brain, wolf_spider, secret_creatures).
- [ ] Staff (Felix, Renata, Owen, Priya, Dia): no art or rigs exist. Placeholder is the mannequin with five atlases.
- [x] Guard: a test that fails if a moved module imports `crate::app`, `crate::world`, `crate::hunters` or
      `embedded`. Add it before any port, so the whole game cannot creep into the lib.
      2026-11-05: guard is in `guard.rs` (code lines only; comments skipped). It currently FAILS on
      `textures/sets.rs:761` (a test function name containing "embedded"). Not edited, as the copied module is not
      to be changed here. Needs a decision: rename that test, or allow it. Not ticked until then.

## Organization
- [x] `docs/LR_STORY.md`: the pack format, the commands, and the override rules. Not written yet.
      2026-11-05: done at `LastRoute Rust/docs/LR_STORY.md` (149 lines): layout, pack.json, overrides, scenes, LORE
      Choices, the four commands with examples.
- [x] CLI: `init <id>`, `check`, `pack <dir>` (sha256 fingerprint), `import-lore <files> --id <id>`. The merge and the
      importer exist; the subcommands in `main.rs` do not yet.
      2026-11-05: done in `main.rs` (`--check` and the no-argument window still work). Tests for init and the hash in
      `main.rs`.
- [x] Clear the 19 build warnings from the pack rewrite (unused code in `load.rs` and `model.rs`).
      2026-11-05: lr_story has 0 warnings. Removed dead `Pack.dir`, `MOVE_REPEAT`; test-only `Game` helpers gated with
      `#[cfg(test)]`; the copied textures and anim modules keep `#![allow(dead_code)]` with a reason. The 2 warnings
      left in the build come from `vendor/miniquad`, not lr_story.

## Build order (from the survey plan)
Each step ends with a passing test. Do them in this order.
1. [x] Choice runtime: two-chapter test pack, one choice; the condition hides it, `SetFlag` runs, `next` chains.
   2026-11-05: done (`play.rs` tests `a_choice_whose_condition...` and `picking_the_choice...`).
2. [x] Flag persistence: round-trip flags through a save file under the pack id.
   2026-11-05: done (`save.rs`, wired in `main.rs`).
3. Pack ids: enforce the `<pack_id>_` prefix, mods/ root order, reject duplicates.
4. [x] Import guard test (see Props section). Runs before any port.
   2026-11-05: guard in place (`guard.rs`), failing on `textures/sets.rs:761`; see the Props section.
5. [x] LORE importer: parse `Choices:`. The six `last-route-*.yml` files import unchanged when no `Choices:` exists.
   2026-11-05: done (`yaml.rs` multi-key items, `lore.rs` `Choices:`). The six-file test passes against the repo.
6. Lab textures: move the painter into the lib; test determinism and the 32x32 and 32x128 sizes. Add `region.lab` to
   `data/render/ambient.json`, and a lab arm to `world/mod.rs` 29-45. Resolve "office" (maze vs lab).
7. Bbmodel parser: parse one lab prop and one maze prop; assert bone and cube counts.
8. [x] Prop placement: one labStool at scale 1.5, finite bounds.
   2026-11-05: done as a CPU-side test (`props/bbmodel.rs` `lab_stool_at_its_draw_scale...`). `PropModel::parse` needs a
   GL context for its texture, so the test takes the parsed corners, scales them by 1/16 and the table's 1.5, and checks
   finite, non-empty and under 5 m per axis.
9. Mannequin clips: a walk clip at 30 fps, finite transforms.
10. [x] Staff skins: five atlases on the mannequin.
   2026-11-05: done. Five placeholder copies of `player/player_mannequin_atlas.png` in `assets/player/staff/`
   (felix, renata, owen, priya, dia `_atlas.png`). Test in `anim/mod.rs`. Not wired into the renderer: NPCs still draw
   the mannequin.
11. [x] Semi-choice slice: two chapters, one choice; picking it sets a flag that changes a gated line.
   2026-11-05: done as an end-to-end test (`lore.rs` `a_choice_that_sets_a_flag...`). The "gated line" is a gated
   choice in chapter two: the runtime has no per-line condition, so a line cannot be gated yet.

## Notes for agents
- The sample pack is `data/lr_story` (core, id `lr_story`). Mods go in `mods/<id>/` (folder name must equal id).
- Ids a mod adds must start with `<id>_`. Replacing another pack's id needs an `overrides` entry.
- Run `cargo test --bin lr_story` and `./target/debug/lr_story.exe --check` before reporting done.
