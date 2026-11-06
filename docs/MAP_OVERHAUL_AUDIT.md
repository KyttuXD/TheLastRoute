# Map Overhaul Audit (concept 2)

Audited at commit 4be106f, read-only. Six subsystem audits were each checked by a skeptic. "Verified" means someone read it in code at that commit. "Inferred" means it was not read or measured. Day counts are for one developer with a coding assistant.

## 1. Bottom line

The overhaul is feasible and needs no engine rewrite. Region identity comes from one pure function, `regions::region_for`, over 45x45-cell chunks, and there are only three real callers (`layout.rs:1563`, `infinite.rs:54`, `maze_regions.rs:192`). The server never calls it. The cost is everything around that seam:

- void and empty chunks,
- the sewer, which is built from the level-0 layout,
- about 250 hard-coded region-name literals across layout, props, render and ambient code,
- two genuinely new geometries (tunnels and stalls),
- the A0A0A0 overlay pass,
- Wynelle's cold system.

After removing overlap between the six audits, I estimate **55-85 developer-days (roughly 12-17 weeks)**, excluding the 503F7F first-concept block, which is not defined enough to estimate. A walkable map where every colour is aliased to an existing region should be reachable in **about 17-22 days**.

The single biggest risk is the **pixel-to-chunk scale, which only KT can answer**. The image is 318x254 with about 22,262 non-black pixels. At 1 px = 1 chunk, that is a 13.4 x 10.5 km world and about 22k chunks. It is also a 46-minute walk from spawn to the far east tunnels (29 minutes sprinting). The 3-pixel-wide tunnels would be 135 m wide, which is not thin. Everything else is parameterised by this one number. The biggest technical risk is the existence and void coupling between level 0, the sewer, stairs and the 3x3 window.

## 2. Region-by-region needs

The table covers the 11 colours, the overlay and black. "Exists" is what is already in the Rust port.

| Colour / region | Generator | Props / assets | Gameplay hooks | Effort | Exists today |
|---|---|---|---|---|---|
| FFFFFF maze (spawn) | Warren (55% of chunks) plus dense banks; unchanged | All present | Spawn chunk (0,0) must be here: forced safe room (`layout.rs:1569`), forced stair link, origin sewer ladder. Maze variants (`maze_regions.rs`) need an instance id | S | Full |
| A17FFF apartments | `mixed_apartment` (four 17x17 suites); unchanged | All present; beds are hide spots | None new | S | Full |
| 7F3300 prison + cafeteria | Dense layout. Cell rooms and a 21x21 cafeteria hall exist. The `prison_mess` template (`furnished.rs:678, 1434`) uses fixed offsets and is gated to rooms of 15x15 or larger. Six tables is a one-line grid change | Tables, bunks, toilets, sinks present | Warren share must be 0 here. Locked cells, key cards and lockpicks do not exist and are out of scope | M | Mostly |
| FFB27F laboratory | Dense layout; unchanged | Bench, fume hood, tank present | Pod room must be chosen per connected spot, not per 4x4 block (`is_pod_room_chunk`) | S-M | Full |
| 7F3F5B offices | Dense layout plus a cubicle-grid recipe. Only a 17x17 maze office suite exists (`layout.rs:2001`) | Desk, cubicle partition, CRT, printer present; 0 new | New region id, texture, ambient row and catalogue row | M | Partial stub |
| 007F0E greenhouse | Dense layout; recipes written but never run in a real chunk | 1-2 new (glass pane, hedge) | The `regions.rs:78-82` gate rejects it everywhere (`HIGHEST_LEVEL` = 0), so it must be lifted. Needs a bright ambient row | M | Dead content |
| 007F7F ice (Wynelle) | Reskin of dense or maze geometry | About 8 new (crystal, ice pillar, icicles, snowdrift, frosted furniture) | Per-cell climate, ambient-creep clamp, Wynelle roster, forced safe room, cold in the multiplayer sample | L | Hospital's teal ambient row is the nearest look. `climate_for` is stale |
| 3F7F62 stalls | New narrow-slot generator; the dense widths 11 and 21 are literals | Stall, toilet, urinal and sink exist. Needs a single-stall template (the current recipe needs 7x7 or larger) | Warren share 0 | M | `public_restroom` role only |
| 7F006E mineshaft tunnels | New non-dense branch in `chunk_layout`. The default seam logic already joins any region at cell 22. `carve_edges` needs a width parameter (its `reach` is fixed at 1, or 2 for maze). Must opt out of stairwell planning | About 8-10 new (cart, rail, timber arch, ore pile, and so on). Reuse `sewer_props` and `inferno_caves` patterns | Roomless chunks: pickup, hunter and music-box tolerance is unverified. Hunter radius vs 1-2 wide tunnels is unverified | L | Seams only |
| 503F7F first-concept | Undefined. Interim: alias to a mansion-style dense layout | The 52-type finite catalogue exists | Objective unknown (the finite generator has a lift; infinite windows do not) | XL if real; S-M as an alias | Finite generator only |
| 7F3F3F undecided | Alias to maze | None | None | S | n/a |
| A0A0A0 overlay | Not a region. A second bitmap channel plus a new prop pass | See section 3 | Overlay props in halls and tunnels must be non-solid or wall/ceiling mounted | L | No |
| Black | Solid void chunk, built as a trivial shell | None | `edge_openings` must return no openings toward it | M | No |

Hospital, beach, carnival, hotel, gothic and mall have no slot in the key. Leave them dormant, or fold them into 7F3F3F. Dropping them from the map costs nothing.

## 3. Work breakdown by subsystem (deduplicated)

| Subsystem | Effort | Risk | What it is |
|---|---|---|---|
| A. Region source and parity pin | 7-9d | Low-medium | Details below the table |
| B. Existence, void and bounds | 5-7d | Medium-high | Details below the table |
| C. Catalogue plumbing | 4-6d | Medium | Details below the table |
| D. New layout generators | 10-16d | Medium | Tunnels 3-5d, stalls 2-3d, cafeteria 2-3d, offices 2-3d, reskins and greenhouse 2-3d |
| E. Props and overlay | 15-25d | Medium | Overlay engine 4-6d; recipe scaling 3-6d; ice and mine assets 3-6d; prop-table decoupling 1-2d; rows and QA 2-3d; web atlas work 2-3d |
| F. Hunters and cold | 6-9d | Medium | Roster and uniqueness 2-3d; per-cell climate, creep clamp and Wynelle tuning 1-1.5d; `spawn_rules` generalised 1d; ambient/fog rows and safe-room forcing 1-2d; multiplayer temperature 1d |
| G. Net, save, versioning | 1-2d | Low | Add a map id and checksum to `fingerprint()` (`protocol.rs:372`), which currently hashes only the wire version and the RPC table. Add `"world": 2` to the checkpoint (`checkpoint.rs:9`); a save or room code without it resumes on the legacy generator |
| H. Tooling and tests | 5d | Low | Converter 1d, viewer and teleport 1-2d, validators 1d, test conversion 1-2d |
| I. Scaling for long sessions | 4-6d | Medium | `DiscoveredFloor`, waypoint replan and full-map drawing |
| **Total** | **55-85d** | | Excludes 503F7F |

**A. Region source and parity pin (7-9d)**
- A `RegionSource` seam selected by a map tag in the world-seed text, 2d.
- The image baked to a small indexed `.bin` at build time, with a data file for scale, gap and spawn, 1-2d.
- Spacing, 2d.
- Region instance ids for the 4x4-block users, 2-3d. These are music boxes (`music_boxes.rs:60`), maze variants, the pod room, and the BIOME and SPAWN-REGION draws.

**B. Existence, void and bounds (5-7d)**
- Replace the level-only `allowed()` with an exists check in `edge_openings`, `wide_seam`, `large_plan` and `hall_trimmed`.
- `window()` must skip empty chunks, and the hard-coded `parts[4]` centre (`infinite.rs:540`) must be fixed.
- `chunk()` must short-circuit void before `live::chunk`.
- Gate `stair_link`, safe rooms, music boxes and hunter spawns on the level-0 chunk existing.
- Sewer, ladder and Inferno columns need a rule for chunks under void.

**C. Catalogue plumbing (4-6d)**
- Names and tables: `NAMES` (`regions.rs:31`, 14 entries, and `region_index` panics on unknown ids), `TEXTURE_SETS`, `sets.rs` DISTRICTS, `lamp_flicker`, `prop_tables`, `catalog.json`, and the `BUILTIN_FILES` array of 16 with its parity test.
- Layout class lists and spawn rules: `DENSE_DISTRICTS` and `WARREN_REGIONS` move to catalogue fields; `spawn_rules` is generalised into every region file.
- Note: unknown region names currently fall silently to `legacy_chunk` (`chunk_layout` returns None). In map mode this must be a hard error.

## 4. Disagreements resolved

- **Mode switch.** The regions auditor suggested a global like `legacy_level0()`. That is a process-wide `OnceLock` from an environment variable. It cannot let hash-mode goldens and map-mode tests share one test process, and an environment variable is always false on wasm (`infinite.rs:150`). Four auditors converge on a tag in the world-seed text, for example `"123456|m2"`. The seed already flows as a `&str` through every `chunk_layout`, `region_for` and sewer call. A tag leaves all 43 golden files and their test code untouched, and also isolates the memo caches keyed by seed string. The layout auditor's "plumb a mode through callers" is over-engineered.
- **Sewer coupling.** The stairs auditor's skeptic says the `?` at `sewer/core.rs:1725` is a small fix, because a tunnel or stalls generator can return `Some(layout)` with empty stairwells. The regions, layout and streaming skeptics say void and non-dense chunks break it. Both are right. Any class implemented inside `chunk_layout` is cheap (about 1d). Void columns need an explicit solid rule on levels -1 to -3, which is part of B, along with the decision on which regions may host wells. The sewer goldens at level -1 do depend on level-0 layouts (`core.rs:1726`), so the pin protects them.
- **`region_for` cost.** The skeptic benchmarked about 740 ns per call, roughly 100-150 calls per chunk. That is small (about 0.1 ms native), so the swap is not a performance win. Memoise per chunk.
- **Chunk shell cost.** The auditors disagree on how much of the 6-19 ms chunk build is the throwaway `Facility::generate` shell (`live.rs:75-83`). Estimates are 40% (layout skeptic, measured) to about 80% (inferred). I use at least 40%. Void and tunnel chunks should skip it from day one. Chunk builds on web are already roughly 110-230 ms per window-boundary crossing (inferred, 2-4x native).
- **Effort totals.** The audits' sums (roughly 90-120d) double-count the region source and the 4x4-block users. The totals above count each piece once. For props I side with the skeptic's 20-30d and add layout dependencies elsewhere.
- **lr_story.** The regions skeptic found it reads only `last_route_core::shared`, so I drop it from scope. Only the `BUILTIN_FILES` parity test in the main binary matters.
- **Layout languages.** The regions skeptic is right that `dense()` is one reusable template (four halls, 11x9 rooms). Ice, offices, prison, lab and greenhouse can reuse it. Only tunnels, stalls and 503F7F are genuinely new geometry. The layout skeptic is also right that furnishing and prop tables per new region add real time, which is counted in E.
- **Audio beds.** Dropped from the hunters estimate. `ambience()` is silent today and the original never played region beds on this route.
- **Scaling fixes (I).** The pre-existing `DiscoveredFloor` rebuild, waypoint replan and full-map draw degrade past about 150 discovered chunks. A finite sparse map does not need them to ship, but long sessions do. The incremental `DiscoveredFloor` fix is about 1-2d; hierarchical waypoints are the real cost.
- **Stair sweep.** The footprint sweep (`clear_props_reaching`, commit dcc110b) is not in the audited commit 4be106f. It is in main. Re-check props findings against main HEAD after the merge.
- **Warren share.** `WARREN_REGIONS` (`layout.rs:59`) turns 55% of chunks in listed regions into warrens whatever the room roles. That would overwrite stalls, cafeteria and ice in about half their chunks. It must become per-region data, 0 for those regions.

## 5. Recommended build order (flag-gated by seed tag, web-first)

1. **Foundations, no gameplay change (4-6d).** `RegionSource` and the seed-tag pin, image converter that errors on unknown colours with the pixel position, expanded-map dump, and the `world_format`/map checksum in the fingerprint. Record web timings for chunk build and window refresh as the baseline. *KT can:* run the converter on the image, see the expanded map as an image or text dump, and confirm the goldens are still green.
2. **Aliased map goes live (10-14d).** Every colour aliased to an existing region (A17FFF to apartment, 7F3300 to prison, FFFFFF to maze, FFB27F to lab, the rest to maze). Void chunks, bounds, spawn at (0,0) in white maze, sewer/ladder gating, block-keyed users moved to instance ids. *KT can:* walk the macro layout, check scale and walking time, and see whether the image connects the way KT drew it. **Scale must be locked before this step.**
3. **Scaling fixes (4-6d), interleaved when exploration passes about 150 chunks.** *KT can:* explore long routes without hitches.
4. **Tunnels (5-7d).** Tunnel generator, carve width, stairwell opt-out, roomless-chunk checks. *KT can:* walk mineshaft links between regions.
5. **Catalogue rework (8-12d).** New region ids and rows, greenhouse enabled, offices, cafeteria with six tables, warren share as data, spacing as data. *KT can:* see each coloured region with its own look.
6. **Ice and Wynelle (7-10d).** Per-cell climate keyed on absolute coordinates, creep clamp, roster, forced safe room, ice props, cold in the multiplayer sample. *KT can:* meet Wynelle in the ice block.
7. **Stalls and the overlay (8-12d).** Single-stall template, overlay pass, hall/tunnel placement mode. *KT can:* see props layered over regions.
8. **Cutover.** Flip the default, decide whether to retire the legacy path and goldens. Concept-1 (503F7F) comes after, once defined.

## 6. Open questions for KT

1. **What is one pixel?** One chunk (45 m)? The walking-time budget decides: 1 px = 1 chunk gives 46 min to the far end; 2 px per chunk about 23 min; 3 px per chunk about 15 min (but the tunnel bands do not align to multiples of 3 and would blur). 1 px = 2x2 chunks (92 min) should be rejected.
2. **Tunnels:** are the 3-pixel-wide magenta bands meant to be 1 chunk wide? May they be thinned to a centreline at load, or redrawn 1 px wide?
3. **Black inside blocks.** The ice block is 48% black inside its bounding box. Are these internal walls (a macro-maze) or padding?
4. **Outside the image:** void, or Godot-style infinite filler?
5. **Below ground:** may the sewer and Inferno sit only under non-black level-0 chunks, with solid rock elsewhere?
6. **Spawn:** is the white maze at roughly (55,128) the spawn? The origin must host a safe room, stair well and sewer ladder.
7. **Overlay A0A0A0:** will you supply it as a separate layer? Of its 769 pixels, 508 sit in apartments, 95 in greenhouse, 85 in offices and 78 in prison. Inferring the base from neighbours loses that.
8. **503F7F:** what is it, and what is the objective on an infinite window (there is no lift)?
9. **Wynelle:** only in ice? Is cold a fixed floor or an offset? She is currently slowest at the coldest (flat 1.0 per second below 30 degrees, `wynelle.rs:46-50`): keep or fix?
10. **Hunters in tunnels:** wide enough for spawns, or hunter-free?
11. **Seed variety:** the same macro map every seed, or a seed-keyed rotation/mirror (8 variants)?
12. **Goldens:** confirm the legacy hash generator is kept as a pinned oracle, rather than deleted.
13. **Old regions:** hospital, beach, carnival, hotel, gothic and mall: dormant, or folded into 7F3F3F?
14. **Music boxes** (2 per block today): fine if the box count becomes map-dependent?
15. **Palette:** can the PNG be exact-colour indexed, with no anti-aliasing?

## 7. What not to do

- Do not resize `CHUNK = 45` (293 literal 45s, about 400 `CHUNK` uses) or the 3x3 window (135 m is hard-coded in several places).
- Do not use an environment variable or global for the map mode.
- Do not decode a PNG at runtime or add a `png` dependency. Bake to bytes.
- Do not hand-edit the generated `prop_tables.rs`.
- Do not regenerate the parity goldens (WORKFLOW_RULES forbids agents from doing so). Pin them instead.
- Do not let an unknown region fall silently to `legacy_chunk`.
- Do not add a new texture set per region without need: each is downloaded and uploaded to VRAM (`shared/assets.rs:211, 314`). Reuse sets with tints.
- Do not give new props their own atlas. Use the shared 128x64 palette atlas, or one new power-of-two atlas.
- Do not exempt "permanent" props from the stair sweep, and do not make hall or tunnel overlay props solid.
- Do not put regions below level 0.
- Do not build region audio beds.
- Do not start on 503F7F before it is defined.
- Do not resample the image with blur.

VERDICT: feasible-with-work: one swappable region function and reusable seam logic, but void and sewer coupling, string-keyed region tables, tunnels, stalls, overlay and cold add 55-85 days, and the pixel scale is undecided.

