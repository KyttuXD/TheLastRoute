# The Inferno's secret creature: the white wolf spider (Rust side)

Port of the original's `docs/SECRET_WOLF_SPIDER.md`. One secret hunter, made from KT's `white_wolf_spider.bbmodel`, living on
**level -3, the Inferno**. Once the runner has spent 30 s there, the host brings it (the difficulty sets its knobs). It stays
secret: no roster draw, no dossier row before it is met, no loading tip, story text or player-facing string. Working name,
placeholder dossier and death lines (KT's to replace).

The black wolf spider was removed from the game on 2026-11-07: its model, rig, grab pool, hunter definition and the pair (the
two-creature mode, shared rest, recall and admin pair entry) went with it.

## Where everything is

| Part | File |
| --- | --- |
| Model (the original's bbmodel, palette embedded) | `assets/hunters/white_wolf_spider.bbmodel` |
| Rig and the six clips (`idle walk sprint` loop; `bark attack death` once), stride table | `assets/hunters/anim/white_wolf_spider.json` |
| Rig builder (reads the bbmodel's own Blockbench animations) | `render/anim/extract_creature.py` |
| Hunter definition (stats, category `secret`) | `data/hunters/white_wolf_spider.json` |
| Every tunable: trigger, dwell, spawn distances, lair rules, behaviour numbers, presentation numbers, difficulty knobs, placeholder dossier text | `data/hunters/secret_creatures.json` (`lr.secret_creatures/1`), read by `hunters/secret_creatures.rs` |
| Behaviour (lurk, bark, hunt, retreat) | `hunters/gimmicks/wolf_spider.rs` (`Gimmick::WolfSpider`) |
| Trigger, lair choice, spawn | `game/inferno_secrets.rs` |
| Presentation: clip choice, bark one-shot, attack lunge | `render/anim/hunter.rs` (`update_creature_animation`), `render/anim/clip.rs` (`Rig::metres`) |
| Its catch, the pounce-and-pin grab | `assets/hunters/grabs/white_wolf_spider.json` (pool `pounce_pin` 6), `render/anim/grab.rs` (`pounce_pin` pose, `CREATURE_FALLBACK`) |
| Sound: the tell, the skitter, the lair crackle | `audio/creature.rs` (existing cues `spider_bark`, `spider_hiss`, `spider_skitter`, `inferno_crackle`) |
| Admin summon id | `ui/admin/menu.rs` (`hunter_list`), `ui/admin/actions.rs` (`summon`, `remove_hunter`) |
| Dossier FOUND section, death lines | `ui/menus.rs` (`dossier_rows_found`), `assets/ui/death_lines.json`, `ui/report.rs` (`pick_line_with`) |
| Tests | `hunters/secret_creatures.rs`, `hunters/gimmicks/wolf_spider.rs`, `game/inferno_secrets.rs`, `audio/creature.rs`, `render/anim/creature_tests.rs`, `ui/admin/tests.rs`, `ui/menus.rs`, `ui/report.rs`, `game/hunting.rs` |

Rebuild the rig after editing the model (run from `src/bin/last_route`):

```
python3 -I src/bin/last_route/render/anim/extract_creature.py assets/hunters/white_wolf_spider.bbmodel assets/hunters/anim/white_wolf_spider.json
```

Rust has no path for the original's glb on hunters, so the model is the bbmodel, drawn by the same bone-by-name rig path as every
hunter (`Model::build_posed`); the converter writes the same `lr.rig` json that `extract.py` writes for the humanoids, plus `metres`
(0.035 m per unit: the head about 2 m up, the legs 3.4 m across). `Rig::draw_scale()` is `metres` for a creature and
`MODEL_SCALE / unit` for a humanoid. The stride reference speeds (walk, sprint) are measured from the planted feet of the clips.

## Behaviour

```
LURK -> BARK (stands still, bark clip + cue) -> HUNT -> RETREAT (home, 3.2x) -> LURK (cooldown)
```

| | White |
| --- | --- |
| Chase speed / sight / hearing | 4.9 / 7.0 / 1.9 |
| Notices a runner within | 15 cells with line of sight, or one it hears |
| Tell | bark, 1.25 s standing still |
| Hunt | up to 7 s, 1.0x |
| Ends on | a shelter or a hiding place, a flashlight beam held on it (0 s hold), breath, time |
| Rests after a hunt | 30 s |

* A runner on another level is never marked and ends a hunt (`Target::cross`), and a hidden or sheltered runner is not marked.
* It never leaves level -3 (`secret_creatures::stays_home`): `Brain::hop_to` plans no flight, ladder or drop for it,
  `noise_events::react` skips an adjacent-floor sound for it.
  It is also in `data/peek.json` `never` (no corner peeks).
* The window's shift moves its lair (`WolfSpider::shift`, called from `shift_brain`).

## Spawn (`game/inferno_secrets.rs`)

`InfernoSecrets` lives on `Game`. From the host's `update_hunters` it counts seconds on level -3 (only there; it keeps what it has
when the runner climbs). At 30 s it fires once per run. The lair comes from `pick_lair`: 220 samples 16-23 cells from the runner,
ranked by open ground (a 7 x 7 square: >= 90% wide, >= 60% roomy, then any), reachable from the runner by `find_path`, seeded by
`"<seed>:SECRET-LAIR:<id>"`. The creature appears already telegraphing.

| difficulty | notice / chase / rest / alert |
| --- | --- |
| relaxed | 0.85 / 0.92 / 1.4 / 0.85 |
| standard | 1 / 1 / 1 / 1 |
| hard | 1.1 / 1.05 / 0.85 / 1.1 |
| nightmare | 1.2 / 1.1 / 0.7 / 1.2 |

`trigger: "off"` in the data stops anything spawning by itself (the admin picker still works, on -3 only).

## Not visible before it is met

* `category: "secret"`: not in `IMPLEMENTED_IDS`, the per-floor draw (`pick_for_floor`), Umbriel's pool, the A-Z or VOID dossier
  rows, or the category locks. The dossier gets a FOUND section only after the account has recorded the encounter (`profile::discover`;
  line of sight within 14 cells, `note_sightings`). `secrecy_tests` greps every embedded data file and text asset: only the files
  named above may mention it.
* Death lines are placeholders (caption style) that only a debug build renders (`pick_line_with`); a release build shows the
  generic line.
* The admin picker lists it (summons it on the Inferno, REMOVE takes it out). On any other level the summon is refused ("THIS ONE
  LIVES ON LEVEL -3"); SUMMON ALL leaves it out there.

## Net

A guest runs no hunter simulation. The host's hunter frame carries `phase` and `bark_seq` for the spider (`frame_entry`); the guest
plays the bark clip and cue from the number. A guest makes a missing hunter from its id (`create_guest_hunter`), which finds the
spider's definition like any other.

## Not done, and why

* **Collider.** The data keeps the original's capsule (`body.radius` 0.36, `body.height` 1.2) but the Rust brain has one fixed body
  radius for every hunter, so nothing reads it yet. The legs sprawl wider than the body, as in the original.
* **Death clip.** The `death` clip is in the rig; nothing kills a spider yet, as in the original.
* **Bark noise on the meter.** The original's doc says the bark raises the runner's noise meter, but no code of it does; not added.
* **Dossier and death text** are placeholders for KT. **Name** is a working name.
* **Lair and cages.** The Inferno's cages and the spider lair of TODO.md are not ported; lairs come from the open ground
  of the caves that exist (`world/inferno*.rs`), which is how the original derives them too.
* **Save.** A spider on a saved route is kept like any hunter if hunters are saved; the trigger counts an existing one as fired.
  A run saved before it fired and resumed starts the 30 s clock again.
* **Not run in the real window.** The checks are the tests above (model, rig, clips, the grab's camera, the state machine, the
  spawn on a real Inferno window). No screenshot was taken.
