# Heater stations (retired 2026-11-06)

Archived by owner decision: the temperature hotbar item (slot 7, `src/bin/last_route/game/specialty.rs`) made the
stations obsolete. This folder sits outside `src`, so nothing here is embedded in the game.

## What is here

| File | What it is |
| --- | --- |
| `assets/heater_station.bbmodel` | the prop model the game drew (was `src/bin/last_route/assets/interactables/heater_station.bbmodel`, embedded key `interactables/heater_station.bbmodel`) |
| `source_glb/heater_station.glb` | the Blender export it was converted from (was `tools/props/source_glb/interactables/heater_station.glb`) |
| `removal.patch` | the exact source, test and doc changes that removed the feature (`git apply -R` restores them) |

There was no JSON prop data for the station: `data/props/*.json` never listed it, the game placed it from code.

## What it was

One station per floor, a small machine the runner interacted with to warm the facility, as Wynelle's counterplay.

- Placement: `world/fixtures.rs` `heater_for` / `place_with_box` put it 3 to 7 cells from the floor's start
  (`near_start`, seeded stream `<seed>:HEATER`). Live level 0 chunks (`world/route/live.rs`) got one each, kept off the
  safe room and its ring, stair wells and doorways, and door thresholds; the streamed window used its centre chunk's
  station (`world/infinite.rs`). `Fixtures.heater: Option<Cell>` carried it.
- State: `game::Heater { Frozen, Thawed, Powered }` in `Game.heater`. `[E]` on it (`Focus::Heater`, `game/interact.rs`)
  cycled Frozen -> Thawed -> Powered -> Thawed. Prompts: `[E] THAW HEATER STATION`, `[E] POWER HEATER STATION`,
  `[E] POWER DOWN HEATER`. In the endless route `game/endless.rs` kept each station's state per (level, centre chunk)
  (`stash_heater`, `restore_heater`) so a powered one stayed powered while the runner was far away.
- Effects: `Game::heater_running()` (`game/void_player.rs`) = Powered and not under Void Orange's climate outage. It fed
  `meters::Situation.heater_on` (ambient +3/s instead of the slow drift) and the hunters' `Ctx.heater_on`; `Ctx.heater_thawed`
  was "not Frozen". Wynelle read both: a thawed heater made a materialised Wynelle thaw away, a running one stopped her gathering.
- Render: `render/scene.rs` drew the model tinted by state (frozen blue, thawed white, powered warm) with an orange
  glow strip when powered.
- Headless: `objects.heater`, `teleport {to: "heater"}`, `run.heater`, the `heater` event, the `HEATER_STATION`
  focus and observation kind (`headless/sim.rs`, `headless/protocol.rs`), and the `tests/interactables.json` test
  "the heater thaws, powers and powers down".
- Not synced over the network: the room server only carries a generic `heaterOn` player flag (`lr_server`), which the
  client never sent. It is left in place for protocol compatibility.

## Why it is obsolete

The temperature item is one hotbar item that is either heat or cold. Heat warms the body and counts as a heater for
Wynelle; cold deters Solyth. That covers everything the station did without a fixed prop to walk to, a three-state
interaction or per-chunk state to keep. Wynelle now depends only on the item: `Game::heat_source_for_hunters()` feeds
`Ctx.heater_on` and `Ctx.heater_thawed` (both kept, both set from that one source). Void Orange's climate outage now
switches that source off for its nine seconds, as it used to switch the station off.

One leftover is deliberate: the finite-floor placement (`fixtures::place_with_box`) still computes the old station's
cell and reserves it (nothing is placed there), so the music box, vents and coins keep the cells they had.

## How to restore

1. `git apply -R archive/heater-stations/removal.patch` (from the repo root; the patch covers source, tests and the
   `tests/interactables.json` suite, not the two binary-ish model files).
2. Move the model back: `assets/heater_station.bbmodel` to `src/bin/last_route/assets/interactables/` and
   `source_glb/heater_station.glb` to `tools/props/source_glb/interactables/`.
3. Decide what should happen with the temperature item (it should probably keep working beside the station), and
   re-add `heater_station` to the interactables list in `content/props.rs`'s embedded-model test.
