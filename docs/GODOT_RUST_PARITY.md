# Godot to Rust Parity Report

**Date of run:** 2026-11-03

**Sides compared:** Godot at `LastRoute Godot\KyttusLabApp\` (paths below are relative to that folder). Rust at `LastRoute Rust\` (paths below are relative to that folder).

**Method:** Read-only audit by the `godot-rust-parity` workflow. No builds, tests, game runs, or file or git changes were made.

## Summary

The run compared 377 items. 292 matched and 85 did not. Verification was capped at 25 of the 85 non-matching items. The confirmed set holds 17 entries: 11 with status `mismatch`, 5 `rust_missing`, and 1 `unverified`. One entry (crouch eye transition) is listed under two domains, so there are 16 distinct confirmed items. The `unverified` item (the unit of Godot's mouse `relative` value) cannot be closed from the workspace. The other non-matching items were not verified. The run's `unverifiedCount` (352) does not reconcile with 377 minus 292 = 85; it is reported here as returned and not corrected.

## Confirmed mismatches

### mouse-camera

| Name | Godot value (file:line) | Rust value (file:line) | Status | Explanation |
|---|---|---|---|---|
| Effective radians per physical pixel at default settings | 0.0075 rad per unit of `relative` (`game/autoload/settings.gd:27`; applied at `game/player/player_controller_3d.gd:1115-1116`) | 0.004375 rad per logical px = 0.0025 rad per physical px (`src/bin/last_route/config/mod.rs:22, 34-36`; `src/bin/last_route/app/mod.rs:115-120, 184-190`) | unverified | If Godot's `relative` is in physical px, Rust turns one third as far per physical px. If it is in window px (physical / 3), both give 0.0025 rad per physical px. Godot engine source is not in the workspace. Close it by running one identical physical swipe in both builds at the same sensitivity. |
| MOUSE_LOOK_MULTIPLIER vs Godot window scale k | Windowed k = max(1, min(floor(0.85·w/480), floor(0.85·h/270))) (`game/autoload/settings.gd:63, 654-668`) | `MOUSE_LOOK_DISPLAY_SCALE` 1.75 and `MOUSE_LOOK_GODOT_WINDOW_SCALE` 3.0, so multiplier 0.5833 (`src/bin/last_route/config/mod.rs:34-36`, hardcoded, used at `src/bin/last_route/app/mod.rs:189`) | mismatch | Rust turn per physical px = sens·M/s, Godot = sens/k. They match only when M = s/k. This is exact on the reference machine (s = 1.75, k = 3) and wrong elsewhere: a 100% display with k = 3 gives 1.75x, and 4K at 100% with k = 6 gives 3.5x. Fullscreen is estimated at about 1.57x and was not run. |
| Mouse-look pitch clamp | ±1.4 rad (±80.2°) (`game/player/player_controller_3d.gd:1116`; the same 1.4 also at lines 1074 and 1093) | ±1.45 rad (±83.1°) (`src/bin/last_route/game/player.rs:143`, called from `src/bin/last_route/game/mod.rs:653`) | mismatch | Rust allows 0.05 rad (2.9°) more pitch each way. The same clamp is shared with keyboard look and the headless protocol. |
| Stick and touch look pitch clamp | ±1.4 rad (`game/player/player_controller_3d.gd:1073-1074` stick; `:1093` touch) | ±1.45 rad (`src/bin/last_route/game/player.rs:143`, shared `look()`) | mismatch (touch part: missing in Rust) | Rust "stick" is keyboard-only (`src/bin/last_route/app/mod.rs:195-208`). No gamepad or right-stick input was found. No touch-look path exists in Rust, so the touch row should read "missing" rather than "mismatch". |
| Crouch eye transition | Linear slew toward target at 3.2 m/s via `move_toward` (`game/player/player_controller_3d.gd:1017-1018`) | Exponential blend, rate 14/s: `crouch_t += (target - crouch_t) * (1 - exp(-14 dt))` (`src/bin/last_route/game/player.rs:218-219`) | mismatch | Different curves. Rust starts faster (about 8.9 m/s initially versus 3.2 m/s) and is ahead of Godot at every instant. |
| Horizontal camera sway | `target_x = cos(phase) · amplitude · 0.42` applied to the pivot (`game/player/player_controller_3d.gd:1014, 1019-1021`). Peak offsets: walk about 10.8 mm, sprint about 17.0 mm, crouch about 5.4 mm | `eye()` applies vertical bob only, no horizontal term (`src/bin/last_route/game/player.rs:134-138`) | rust_missing | Godot's lateral sway has no Rust counterpart. |
| Camera near plane | 0.06 m first-person (`game/main_3d.tscn:50`; `game/player/player_controller_3d.gd:382`). Model-eye 0.02 m on ladders and hunter grabs (`player_controller_3d.gd:383`, applied at `:1557` and `hunters/view/hunter_grab.gd:1123`) | `z_near: 0.04` m (`src/bin/last_route/render/scene.rs:43`) | mismatch | Rust's 0.04 m matches neither Godot value. It is 0.02 m nearer than the 0.06 m default and 0.02 m farther than the 0.02 m model-eye value. |
| Camera far plane | 160 m scene (`game/main_3d.tscn:51`). Tiers: HIGH keeps 160, MEDIUM 140, LOW 90, Quest 60 (`game/look/quality_tier.gd:119-135`). Applied at `game/player/player_controller_3d.gd:895-897`. Live degrade ×0.7 at `game/look/quality_governor.gd:142-143` | `z_far: 80.0` m, one fixed value (`src/bin/last_route/render/scene.rs:44`) | mismatch | Rust is 60 m shorter than Godot's desktop default of 140 m (AUTO resolves to MEDIUM on a common desktop) and 80 m shorter than the scene's 160 m. The inventory cited `game/autoload/settings.gd:108`, which is a comment and holds no far value. The conclusion is unchanged. |
| Field of view maximum | 105° (`game/autoload/settings.gd:38`; slider max at `game/menus/settings_overlay.tscn:250-251`) | 110° `FOV_MAX_DEG` (`src/bin/last_route/config/mod.rs:39`), used only by the load clamp at `src/bin/last_route/app/settings.rs:84` | mismatch | The UI range matches on both sides (60–105), so the UI cannot reach 110. A hand-edited settings file with FOV between 105 and 110 loads unchanged in Rust and is clamped to 105 in Godot. The 110 value is attributed to native loader FUN_14008a590 and was not re-verified. |
| Window scale and size | Windowed k integer multiple of 480×270, k = max(1, min(floor(0.85·w/480), floor(0.85·h/270))) (`game/autoload/settings.gd:59, 63, 654-669`). Target machine: 1440×810 physical (k = 3) | `fit = min(0.8·sw/1280, 0.8·sh/720, 1.0)`, window 1280×720 × fit in logical px, floor 640×360 (`src/bin/last_route/app/platform.rs:41-62`), `high_dpi: false` (`src/bin/last_route/main.rs:30`) | mismatch | Different share (0.85 vs 0.8) and different sizing rule: Godot uses an uncapped integer multiple in physical px, Rust a capped continuous fit in logical px. On a 1920×1080 100% display, Godot gives 1440×810 and Rust 1280×720 (derived, not run). The inventory said `project.godot` encodes k; it does not (`project.godot:57-62` sets a 960×540 override, which the runtime overwrites in windowed mode). |
| Rest, sit and lay look clamps | Sit yaw 75°, lay yaw 55° relative to furniture facing; sit pitch (-0.95, 0.6) rad; lay pitch (-0.35, 1.05) rad (`game/player/player_controller_3d.gd:396-399`, applied in `_clamp_rest_look` at `:597-603`) | None. No rest pose, yaw limit or rest pitch (`src/bin/last_route/game/player.rs:12-48, 141-144`). Pitch is ±1.45 rad in every state | rust_missing | Inventory status was `unverified` (name search only). The verdict confirms the feature is absent, not renamed. `src/bin/last_route/world/route/furnished.rs:77-105` returns sit and lay as prop data, but nothing in `src/bin/last_route/game` or `src/bin` reads it to change the camera. |

### movement-meters

| Name | Godot value (file:line) | Rust value (file:line) | Status | Explanation |
|---|---|---|---|---|
| Walk, sprint and crouch multipliers, Nightmare | player 0.94, crouch 0.94, sprint 0.96 (`game/config/game_config.gd:51`; the key is at line 50, so the inventory's 50-52 range is off by one) | Not implemented. `src/bin/last_route/config/difficulty.rs:7-12` has only Relaxed, Standard, Hard | rust_missing | Rust omits Nightmare on purpose (`difficulty.rs:2-3`). A room with the setting `nightmare` still exists: `difficulty_from_name` (`src/bin/last_route/net/world_sync.rs:537-539`) maps it to Standard, so those rooms run at multiplier 1.0. |
| Wading speed scale (sewer flood) | `WADE_SPEED_SCALE` 0.45 at knee depth, lerped over `SewerFloods.DEEP_DEPTH` 0.9 m (`game/player/player_controller_3d.gd:368, 1892-1893`) | Not implemented. Speed pipeline is `src/bin/last_route/game/player.rs:222-236` and reads no water or depth value | rust_missing | No wading or flood-depth model in Rust. Searches for flood, wade and DEEP_DEPTH hit only sewer breach fill, a loot table and a texture. |
| Crouch capsule height and headroom gate | Crouch capsule 1.08 × 1.067 = 1.1524 m; standing 1.7 × 1.067 = 1.8139 m (`player_controller_3d.gd:40-41`). Stand-up blocked by a headroom box query (`:962-992`) | Radius-only collision via `body_fits(x, y, PLAYER_RADIUS)` (`src/bin/last_route/game/player.rs:158-173`; `body_fits` at `facility.rs:950`, path not confirmed in the run). No height or headroom check | rust_missing | `CROUCH_COLLISION_HEIGHT` exists in Rust at `src/bin/last_route/world/route/sewer/core.rs:88`, but only for sewer geometry, not player collision. |
| Crouch eye height (resulting value) | `min(standing × 0.69, 1.1524 − 0.08)` = 1.0724 m at the 1.7072 m fallback standing height (`player_controller_3d.gd:1001-1003`; constants at `:41, 46, 47`) | 1.7072 + (1.1780 − 1.7072) × crouch_t, reaching 1.1780 m (`src/bin/last_route/game/player.rs:99-101`; `src/bin/last_route/config/mod.rs:15-17`). No clamp | mismatch | Rust's crouched eye is 0.106 m higher. The Godot cap binds whenever standing height is at least 1.554 m. Godot's runtime standing height is measured from the model (`:946-959`) and was not verified. |
| Crouch eye transition | Linear 3.2 m/s `move_toward` (`game/player/player_controller_3d.gd:1017-1018`) | Exponential, rate 14/s (`src/bin/last_route/game/player.rs:218-219`) | mismatch | Duplicate of the mouse-camera row. Godot's drop is 0.635 m and reaches target at about 0.198 s. Rust's drop is 0.529 m, and it is still about 0.03 m from target at 0.2 s. |
| Camera bob cadence (phase rate) | PI / stride rad per metre, stride walk 1.34 m, sprint 1.28 m, crouch 1.15 m (`game/autoload/sound.gd:411, 425`; `player_controller_3d.gd:1007-1009, 1991`). Walk 2.344, sprint 2.454, crouch 2.732 rad/m | `bob_phase += dt × ground × 1.9` (`src/bin/last_route/game/player.rs:300-301`): 1.9 rad/m for every gait (0.605 cycles/m) | mismatch | Rust bobs 19–30% slower per metre and ignores gait. Godot falls back to a time-based rate below 0.3 m/s, and Rust does not advance the bob below 0.3 m/s. That behaviour difference is not in the claim. |

## unit_differs items and conversions

None. No item in this run was classed `unit_differs`. Every verdict found a difference beyond a unit conversion.

One relationship is a conversion in form. For mouse look, Rust turn per physical px = sens·M/s and Godot turn per physical px = sens/k, so the two agree only when M = s/k. This is a mismatch, not `unit_differs`, because the constants do not equal s/k on any machine except the reference one.

## Gaps and unverified items

**mouse-camera**
- Godot's `relative` unit (physical px or physical/k) is not confirmed. No Godot engine source or docs are in the workspace. `docs/PARITY.md:63` flags the same premise as unverified. This decides the effective-rate item and the window-scale item.
- Fullscreen scale was not computed. Godot's fullscreen branch (`settings.gd:660-662`) skips the window fit, so fullscreen turn rates were not compared.
- Rust's display scale (1.75) and k (3) are hardcoded in `src/bin/last_route/config/mod.rs:34-35` and were not re-measured.
- Godot's InputMap in `project.godot` was not read, so look key bindings were not compared. Rust bindings were read at `src/bin/last_route/app/mod.rs:196-207`.
- Touch look was not compared on the Rust side. Godot touch clamp is at `player_controller_3d.gd:1093`.
- Rest, sit and lay look was checked by name search only (see the table).
- Godot runtime standing eye height depends on model eye attachments, which were not read. The Rust model eye heights were not read.
- Godot desktop far plane depends on the tier AUTO selects. The MEDIUM mapping comes from comments and `quality_tier.gd:119-121`, not a run.
- Godot's `allow_hidpi` and `keep_aspect` defaults were taken from engine defaults, not source. The macroquad `fovy` convention (vertical) was assumed, not checked in macroquad source.
- Rust `src/bin/last_route/app/mod.rs:964-975` self-test checks the Rust model against its own assumption. It is not independent evidence.

**movement-meters**
- Paths are relative to the two project roots (see the header).
- `player_controller_3d.gd` is 4067 lines. The run read movement, camera, stamina, lamp, battery, vertical motion and the constants block. Ladder, plunge, XR, vent, hide and admin paths were not audited.
- Standing camera height at runtime comes from the GLB eye attachments and was not read. The fallback 1.7072 m matches Rust.
- `stamina_drain` (Godot 13.5 Standard, 12.0 Hard; `game_config.gd:35, 45`) is used for hunter stamina, not the player. Rust uses it at `src/bin/last_route/hunters/brain.rs:448`. Out of player scope, not compared.
- Rust collision is grid-based with a radius only, so the crouch-height items are `rust_missing` rather than mismatched.
- Rust `PLAYER_HEIGHT` (`src/bin/last_route/config/mod.rs:14`, 1.90 m) is never read. It is dead config. Godot's standing capsule is 1.8139 m.

**hunters-difficulty**
- Units: speeds and chase are cells per second (1 cell = 1 m); sight, hearing reach and capture reach are cells; timers are seconds; multipliers, probabilities and noise scales are dimensionless. The hunter `hearing` key is a multiplier on noise. `spawn` has no confirmed unit.
- Nightmare is absent from the Rust `Difficulty` enum (`src/bin/last_route/config/difficulty.rs:8-12`) and from the Rust director profile.
- Confirmed gameplay divergence: taser stun. Godot stuns a non-immune hunter for 2.8/2.7/2.4 s (`taser_stun`). Rust uses `escape_stun` at 3.0/2.9/2.6 s. No built-in hunter is `taser_immune: false`, so it affects pack content only today.
- Latent divergences: the Rust tier draw does not renormalise weights, and Rust does not clamp depth in the lineup seed. Both are no-ops for current content and default paths.
- Data-driven gap: Godot requires core hunter fields and supports per-hunter `taser_slow` overrides. Rust defaults missing fields silently and has no `taser_slow`. Pack hunters are affected; built-ins are not.
- Not ported in Rust: hard abilities (pounce, burst, slam, intercept, stumble), hard speed scale, troll speed factor, City daytime sight, LightField exposure branch (dormant by default), patrol anchor wandering, emotes, anti-idle/AFK timers, secret spiders. `relay_alert` and `spawn_distance` fields are defined in Rust but never read.
- Not compared or only partly compared: per-gimmick logic for Briselle, Ceryth, Ravelle, Ysolde, Verrine, Nyxie, Umbriel and the void gimmicks; Godot escape call site; Rust director `open_near` and `exit_point` fallbacks; `glyra_gimmick.gd`; `secret_creatures.gd`; callers of depth and tier draws outside `src/bin/last_route/hunters/mod.rs`; music box cue playback; the hunter_peek, inspect and naps brains.
- Dead Godot constants, unused in `game/`: `HUNTER_SEARCH_TIME` (`game_config.gd:183`), `HUNTER_STAMINA_CHASE_DRAIN` (187), `HUNTER_SPAWN_DISTANCE_MIN` (191), `HUNTER_LOST_SIGHT_GRACE` (192). Rust defines none of them.
- Method: JSON values for the 36 hunter files were compared with a node script. All keys matched. No builds or tests were run.

**route-generation**
- Sealed-wing gating by level (`streamed_route_generator.gd:1032-1038`, SealRules) was not read. Rust assumes sealed cells are empty on level 0, so the level-0 bonus relay match depends on that.
- Office Void hunter roll at levels -2 and below (`streamed_route_generator.gd:5689`, 0.015) was not compared.
- Sewer links between -1/-2 and -2/-3 use sewer ramps and ladders in Godot (`sewer_route_generator.gd`; `ramp_between` at `streamed_route_generator.gd:578`). Rust applies the shared 0.3 stair_link rule. Not compared in detail.
- Inferno (level -3) is `InfernoCaverns` in Godot (`streamed_route_generator.gd:933, 953, 958`), not the streamed layout. Not compared.
- Godot hunter residency and stream radii (`infinite_route_test.gd:14, 24`) were not compared with Rust `residency.rs`.
- Rust chunk cache retention (`infinite.rs:320-322`) was not compared with Godot's trim logic.
- Godot documentation contradicts its code: the header at `streamed_route_generator.gd:14-16` and `:1397-1398` say something different from `stairwells.gd:51` and `:1399-1400`. The code was compared.
- `mechanics.json` entries marked estimated (infinite stair chance 0.3; infinite relays and batteries per chunk 3 and 4) do not match the Godot-derived values and should be replaced. The standard hunter population step of 2 relays is correct.
- No builds, tests or runs. The unit convention cell = 1 m comes from `streamed_facility_view_3d.gd:17`.

**items-audio**
- The requested Godot `game/items/` directory does not exist. Item constants are in `game/player/player_controller_3d.gd` and `game/config/game_config.gd`; loot tables in `data/loot/*.json` and `game/world/interactables/loot_tables.gd`.
- Several Godot entries (pink fake steps, troll step, climbing, scythe_walk) cite a prior port note rather than a fresh read. They are marked unverified or `rust_missing` with that caveat.
- Units not stated in source: taser half angle, melee half angle, axe noise level (18.0). Radians and game noise units are assumptions.
- Not compared: the Godot nightmare difficulty (Rust does not port it), the Godot pack loot override path (Rust reads no packs), `route_interactable_3d.gd` TOOL_PICKUP_MODELS, and render goldens.
- Pickups: sealed-wing relay logic and revive or backpack behaviour on the Rust in-hand side were checked for absence only, not against the full Godot ruleset.
- Surface footstep profiles were compared value for value across all 8 surfaces, but the `surface_sounds.gd` line range came from a port note (around lines 30-40), not a fresh grep.
- Cue render primitives (tone, thump, noise, envelope, LCG, 16-bit quantise) were sampled in `render.rs` against `sound.gd:1084-1161`, not diffed exhaustively.

**Run-level**
- Verification was capped at 25 of 85 non-matching items. The confirmed array holds 17 entries. The run does not list verdicts for the rest.
- `unverifiedCount` (352) does not reconcile with 377 − 292 = 85. Reported as returned.
- Inventory citation errors found during verification: far-plane `settings.gd:108` (comment only); window scale claim that `project.godot` encodes k; `game_config.gd:50-52` should read 51-53; Rust window sizing is in `platform.rs`, not `main.rs:29`.
- Item statuses corrected by verdict: "Rest, sit and lay look clamps" is `rust_missing`, not `unverified`. "Stick and touch" touch half is `missing`, not `mismatch`. "Crouch eye transition" appears twice (mouse-camera and movement-meters); the duplicate should be merged.

## How to re-run

From the project folder, run the workflow `godot-rust-parity`.
