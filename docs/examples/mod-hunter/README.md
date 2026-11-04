# Mimic sample pack

A small, loadable mod pack that adds one new hunter, `mimic`, with its own skin atlas and tuned stats.

## Install

Copy this whole folder (the one containing `pack.json`) into a pack root as `mods/mod-hunter`. Roots scanned, in order:

1. `mods/` beside the game executable
2. `mods/` in the working folder
3. `%APPDATA%/KYTTU/TheLastRoute/mods`

Packs load in folder-name order. Warnings (bad JSON, duplicate id) show in the game's content warnings; they never crash the game.

## Files

| File | Purpose |
|---|---|
| `pack.json` | Manifest, schema `lr.pack/1`. Needs a unique `id`; `name`, `version`, `author`, `description` are optional. |
| `hunters/mimic.json` | `lr.hunter/1` stats: tier, category, speed, chase, prog, sight, hearing, spawn, chase_style, taser_immune, melee_immune, plus pack-relative `model` and `atlas`. |
| `hunters/mimic.bbmodel` | Placeholder body: an unmodified copy of `assets/hunters/dovelle.bbmodel`. Replace with your own model. |
| `hunters/mimic_atlas.png` | Generated skin (checker plus stripes). Rebuild with `node tools/make_skin.mjs`. |
| `tools/make_skin.mjs` | Dependency-free PNG writer (node zlib only). |
| `_proposed_rule_rows/` | NOT loaded. Rows for water, peek and grab, written in the base files' formats, for when packs can carry rule tables. |

The atlas is 64x128, not 64x64, because the Dovelle body maps its UVs onto a 64x128 sheet. A custom model sets its own size.

## Works with data only today

- Stats, tier, category (`az` joins the normal draw by tier weight), chase_style label, immunities.
- Custom `.bbmodel` plus atlas PNG. Name cubes `R-arm`, `L-arm`, `R-thigh`, `L-thigh` to get the limb swing.
- Optional `portrait` PNG (pack-relative path); none is set here, so no portrait is drawn.
- If the model or atlas fails to load, the hunter is drawn as Dovelle.

## Needs Rust (not possible from a pack yet)

- Mechanics: `Gimmick::for_id` matches hard-coded ids; an unknown id gets `Gimmick::None` (neutral speed and sight, no special behaviour).
- Rule tables compiled into the exe: water (`data/hunters/water.json`), peek (`data/peek.json`), charge, emotes, death lines. Mimic uses the defaults (wades, no swimming; peek 0.35 / 1.4 / burst; weight 1; no death line).
- Noise events have no per-hunter rows at all (six fixed kinds); only `hearing` scales them.
- Animation rig: embedded per id, so mimic uses the plain stride-swing draw with no chase layer, idle or look animation, or grab pose; `chase_style` has no visible effect there.
- Grab pool (`assets/hunters/grabs/<id>.json`): not read from packs; mimic gets `shoulder_grab` only.
- `spawn` is stored and shown in the admin menu; no gameplay code reads it.
- Multiplayer: guests without the pack draw mimic as Dovelle.
