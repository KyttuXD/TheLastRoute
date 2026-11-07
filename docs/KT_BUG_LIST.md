# KT's bug and feature list (2026-11-07)

Everything KT reported or asked for, in KT's words where possible. The open list started from main `5bfc405` (web build `e7442c5e` live on https://lastrouterust.games.kyttuslab.org). Items 1, 3, 4, 5 and 14 are fixed on `cursor/audit-open-bugs-d6c4` and are not deployed. Hand this to another agent as the starting brief. Platform priority: the WEB build (wasm32, WebGL1), then desktop. Reference browser: Firefox on a 2015 dual-core Intel MacBook Air.

Legend: DONE = merged and deployed; MERGED = in main, not all of it live; BRANCH = finished on a branch, not merged; OPEN = not fixed yet; NEEDS KT = waiting on a decision or a server change.

## Fixed on branch `cursor/audit-open-bugs-d6c4` (not merged, not deployed)

1. **Small gray pipes have no collision for the player.** A standing runner and a standing bot stop at a crawl-under pipe; a crouched runner passes, and releasing crouch under the pipe does not stand them back up. Hunters stay fully closed (`Facility::crawl_stops`, `Player::fits`). The floor footprint stays open.
3. **Hunters must not spawn in sewer water.** Runtime spots (`Game::spawn_spot`, resident hunters) require `hunter_may_spawn` (open and dry). The chunk generator already required a dry neighbourhood.
4. **Collectables and props must not spawn on sewer water.** Pickups, relays, batteries, coins, vents and hiding spots skip water. Pipes, bridges, grate decks, floor grates, overflow grates, sluice gates and the outflow pipe may stand in it (`may_stand_in_water` in `data/sewer_water.json`). Lamps, pumps and barrels may not.
5. **Sewer water glows.** The surface is lit (`shader::LIT_MODE`) and the shipped `glow_mix` is 0, so the dark sewer is not washed with the water tint.
14. **Status bars.** One row of thin outlined bars above the hotbar, no icons and no panel: BATTERY (amber, `BATTERY 97% // F`), STAMINA (light blue), FADE (crouch amount), EXPOSURE (orange-red, middle zone `LIT`), NOISE, and TEMP as the sixth. The break-free box sits above that row.
17. **Standing-runner blocking of crawl pipes** is item 1. The hard-difficulty chase speed is still open (KT may want a small buff; Hard currently gets none).

## Open bugs (not fixed yet)

2. **Hunter pathfinding is not smart enough.** "fix hunter pathfinding to be more smarter". Includes: hunters dither about 10 s near the foot of the switchback stairs, no stuck recovery, poor search after losing the runner. Branch `fix-smart-pathing` (partial work), plan: simulation harness measuring catch time, path efficiency, stuck time; then fix.
6. **Stair railings with collision.** "add railings going down stairs + collisions". Spiral guardrails exist (merged); generalise to all flight shapes (straight, switchback, spiral) and top landing edges, with collision. Branch `fix-stair-railings`.
7. **Stuck prevention on stairs, corners and so on.** Player and bots get wedged. Needs sliding fixes, stuck detection and recovery, and a sweep test. Branch `fix-unstuck`. (Earlier report: "can easily get stuck behind props going down the spiral stairs"; the prop clearing fix is merged, the stuck report itself was not reproduced.)
8. **Route sync after the host dies and restarts.** "route sync when host dies and hits Restart route causes host to get a new room but other players still see old room". Branch `fix-restart-route-sync`.
9. **Spectator system for co-op (feature).** Caught players follow a living teammate. Branch `fix-spectator`.
10. **CREATE ROOM says BUILD MISMATCH on the web version.** Root cause found (NEEDS KT): vps2's region agent starts the old Godot dedicated room host, with another protocol (`8645f524` vs the Rust `09923496`). The client-side message and tests are merged. The real fix needs a Linux build of the Rust game on vps2 plus `docs/patches/node-hosted-build.patch` applied to the Node backend and `LASTROUTE_RUST_BIN` in `/etc/lastroute-agent/agent.env` (restart drops open rooms). Details in `docs/SERVERS.md`.
11. **Separate the room systems of the Godot and Rust games.** "i need you to separate the room systems for godot and rust versions". The Rust web site shares vps2's Node backend (`lastroute-godot-test.service`, port 8205) with the Godot site. Plan: a second backend instance for Rust, own port, data dir, relay and secrets. Branch `feat-separate-rooms` (mapping done, kit not built). Nothing on servers touched yet; needs KT's approval.
12. **About 120 other prop models have dropped triangles (see-through holes).** The sewer outflow pipe, broken stub and bridge were restored from Godot (merged). The rest: branch `fix-cut-props` (3 commits). Restore from Godot GLBs within the web triangle budget or decimate properly; closed-solid tests for all props.
13. **`glBindTexture called with an already deleted texture ID 17!` on the web build** (logged on the menu, also ID 567 on the Airs). Root cause not found yet. Branch `fix-gl-deleted-texture` (partial work).
15. **Major web performance pass for the dual-core Intel MacBook Airs (and other low-end devices).** Use Firefox on the Airs (KT-MBA, AK-MBA, XR-MBA, JV-MBA, ZL-MBA; all KT's). Bench harness first, then GPU, CPU, streaming/memory/load and governor/low-end preset work, each measured. Branch `perf-bench` and the `perf*` branches (only the harness started).
16. **Flat, unshaded world textures to match the hunters.** Plan in `docs/OCD_TEXTURE_PLAN.md`, decisions in memory (2-3 tone patterns, 3-6 colours, lift the palette a little, no UI, lr_story in scope, Godot parity retired, the re-OCD pack is a style reference only). Branches `feat-flat-textures`, `feat-flat-props` (floors done, the rest pending).
17. **Hard difficulty chase speed:** Hard currently gets no chase buff (some void hunters already out-dash the sprint there); KT may want a small buff. Left unchanged.
18. **Inventory port (not started).** Godot backpacks, left and right hunter bars, key B, Godot stack rules (memory: `inventory-port-decisions`).
19. **Map overhaul (waiting on KT).** Concept images and colour key are in memory (`map-overhaul-concept`) and `docs/MAP_OVERHAUL_AUDIT.md`. Waiting for KT's final compact region image and the scale question.
20. **Native launcher (merged, needs KT).** Needs a signing key generated and trusted before it works; see `docs/LAUNCHER.md`. KT has since chosen Firefox over the Mac app, so this is low priority.
21. **Admin password reset (merged).** The new `lr_server` is not deployed to vps3, so the offline command and the in-game button do not exist on the live server yet (`docs/SERVERS.md`, "Lost password").
22. **Public route: server-side host.** The handover fallback is live. The server-side host (`last_route --dedicated-host PUBLC` on vps2) is written but not deployed (`docs/SERVERS.md`).

## Done (checked against the code, 2026-11-07)

These claims match the tree. They were not re-deployed.

- Co-op black screen: `render_play_with` calls `pipeline::reset()` before `set_default_camera` and the window blit (`render/pipeline.rs`, test `the_frame_resets_the_pipeline_before_it_draws_to_the_window`).
- Stamina lasts 1.5x longer: `data/player/stamina.json` `sprint_seconds` 4.6875 (Godot 3.125 s) and `weak_below` 0.45. The HUD FIT zone starts at that line.
- Guests show as `GUEST_####`: `lr_server/accounts/guest_names.rs` accepts `GUEST_` plus four digits and refuses that shape as an account name.
- Danger overlay removed: the crimson edge pulse is gone (`ui/hud.rs`). "DANGER n%" is the route danger label, not that overlay. Frost and heat haze at the temperature extremes remain.
- Catwalks removed: no catwalk code in the game; the data lives in `archive/catwalks/` (`docs/SOURCE_LAYOUT.md`).

## Done (for context)

- Co-op black screen with an empty hotbar slot and another player: a leaked world shader (never reset after drawing remote runners). Fixed and deployed (`render/pipeline.rs`). The earlier skin, flashlight and ladder black-screen reports were very likely the same bug (the ladder one was also a separate render panic, fixed).
- Skinless guest not seeing KYTTU's skin: web never downloaded remote skins; Node upload answer not recognised; fixed.
- Guests show as `GUEST_####`; AYTTU's temporary password set on both account stores (vps2 Node backend and vps3 Rust server); copy and paste in login and code fields; GUEST_ account names reserved.
- Hunters: slightly faster chase (Nightmare-only above sprint speed), per-hunter animation profiles and 44 mechanic animations (Ysolde battery steal and so on), hunters keep moving and attacking while a player has the pause menu open online, pause menu closes on an online grab, Solyth cold deterrence, Nyxie on a dead flashlight (including level 0), full A-Z hunter names, no hunters stuck inside stair wells or crawl pipes.
- Player: stamina lasts 1.5x longer, weaker below 45 percent; temperature slot on every level; flashlight reveals the maze fog; danger overlay removed; catwalks removed; heater stations archived; spiral guardrails and no LEVEL lettering; ladder black screen (render panic); sewer water on the map; discover-as-you-go on level 0.
- Web: stuck keys, pointer lock and Escape, music box timing and Glyra-only play, join code field, specialty slots, update screen, skin picker and mods import, fps work (culling, tiling, governor).
- Mac app "KYTTU The Last Route" (WKWebView) installed on all Macs. KT concluded Firefox is better; do not invest further.

## Rules for the agent

- Do not edit `main` directly while other agents work: use a git worktree per task, run the tests with the direct test executable (plain `cargo test` aborts on KT's PC), and rerun the known flaky timing tests (`headless::dedicated`, `net::http`, `shared::skins`) alone.
- KT's PC runs out of memory with many builds: at most two agents and one cargo build at a time (`CARGO_BUILD_JOBS=4`).
- Never change the live servers (vps1, vps2, vps3) without KT's explicit approval of the exact steps. The Godot site `lastroutegodot.games.kyttuslab.org` and its backend must stay untouched.
- Never commit credentials. Deploy the web build with `sh packaging/web/deploy-vps2.sh dist/web` after building the wasm.
