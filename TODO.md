# TODO

The person's wishlist for the port, with status. `[x]` done, `[~]` in progress, `[ ]` not started.
Updated as work lands. The port's current implementation is now treated as the
completed scope for this checklist; future expansions should be tracked as new
items rather than left in an ambiguous in-progress state.

## Status 2026-11-04

- Stairs linking audit (2026-11-04): real-entry walk tests, residency, twin-record and resume tests, the `stairs_report` protocol command and a faster neighbour build near a well are in; a resume from a mid-flight save no longer drops the body. Open: the stairs-vs-ramps question and the "stairs do not link" repro data (docs/OPEN_QUESTIONS.md 27), and the partial floor right after a relabel (below).
- Shipped: the stairs and ladders project (phases 0-9, one continuous space), build 42 published for windows, linux and macos, lr_server on VPS3, all suites green at ae38f79.
- Blocked: the GitHub Actions budget is used up (every job fails to start); KT must raise the spending limit. Publishing is local until then.
- Next: wire the locker body and nearby-player TTS into the game, cleanup Tier B (needs KT), mod packs with mechanics, the Godot parity batches.
- Waiting on KT: hallucinations design for Dead Air, Descent sounds, Standard-visible hunter changes, the Mac Mini GitHub login.

## Godot parity gaps (audit 2026-11-03)

From a read-only audit of every open and partial item against the Rust code (539 distinct gaps; 38 remain here, because gaps that also appear in the topical audit sections below were merged into those sections and carry an (effort S/M/L) tag). [~] partial, [ ] open. Effort S is under half a day, M a day or two, L more. 47 audit items were marked implemented and are not listed here; their old TODO entries may be stale. Where the audits disagreed on status, the less complete one is kept with a note.

### Effort S (16)

- [~] Camera bob with step-locked cadence and lateral sway (Rust bobs vertically at a fixed 1.9 rad/m, player.rs 301; Godot player_controller_3d.gd 997-1020)
- [~] Crowbar and axe STUNNED callout still printed
- [~] Dossier: black square instead of name for undiscovered hunters
- [~] Floor-surface footsteps: surface kind passed
- [~] Hide glide animating into the locker
- [~] Hold drop key to drop a whole stack (Rust drops one item per press, src/bin/last_route/game/items.rs 231-255)
- [~] Locker hide check still prints callouts
- [~] Movement, camera and look tuning values
- [~] Player list shows the room's runners
- [~] Port Hiding occupancy (capacity, front and beside slots)
- [~] Port Music boxes (two per 4x4 region block, shared tune)
- [~] Quality governor stepping quality down on slow frames
- [~] Quenelle lock time and speed ramp constants
- [~] Relay rush timings and targets need verifying
- [~] Sweep and waypoint look-ahead parity unconfirmed
- [~] Title menu: PUBLIC ROUTE button and admin visibility for non-KYTTU admins

### Effort M (17)

- [~] Automatic run save: backups and more save points
- [~] Crowbar and axe: breach boards and doors, cut grabs, rescue teammates
- [~] Flood valves
- [ ] Moderation client (online only)
- [~] Night Watch cue family: scenes call them
- [~] NPC teammates, built as player bots and not as in-game NPCs (`game/player/npc_teammates.gd`; normal runs seed them from src/bin/last_route/data/npcs.json, they wait, join and flee chasing hunters; still no catch/revive, chat, storey crossing, saves, net sync) (session 2026-11-03: first slice done: player bots spawn in runs; they obey the runner's door rules: a bot stops at a shut door, waits about 0.3 s and presses interact through `press_door`, never opens one by walking up, and counts as a body in the doorway; only hunters auto-open. Hunters ignore bots, and pickups and interact focus are the runner's alone) (update 2026-11-04: storey crossing is done: bots cross flights and ladders)
- [~] Online backend client compiled and wired
- [~] Patrol-anchor branch in wander and search picks
- [~] Port Breakables (sealed wings, loose boards, pipes; no interaction)
- [~] Port Carnival booths (playable stalls and carousel missing)
- [~] Port Chunk seams (centre-cross openings, wide hall openings)
- [x] Port Stairwells (spiral and switchback flights, placement, access halls) (done 2026-11-04: flights are switchback, straight and spiral, see docs/STAIRS_CONTINUOUS_DESIGN.md)
- [~] Port Vent network (ducts and crawl movement missing, grates only)
- [ ] Public route featured hunters and lineup
- [~] Room climate offsets: per-room profiles instead of per-region estimates
- [~] Text-to-speech chat for nearby players (update 2026-11-04: audio/tts.rs exists, no caller yet)
- [ ] Void Red hall runner: hallway and room speed multipliers

### Effort L (5)

- [~] Fix Infinite map to match Godot layout, districts, stairs, population
- [~] Port Inferno caverns (cave carving, lava pits, cages, spider lair)
- [~] Port Level stack (sewer -1, lower sewer -2, Inferno -3, floor at -3) (update 2026-11-04: sewer water, second sewer and inferno caves are in; check the depth floor and the remaining parity)
- [~] Port NPC teammates with dialogue, text chat with TTS, full online mode (session 2026-11-03: NPCs are player bots, not in-game NPCs; the first slice spawns them in runs, dialogue, chat and online are still open)
- [~] Port Sewer layer (separate generator, ramps, falls, ladders, floods) (update 2026-11-04: water, swim, pipes, a second sewer, ladders and drops are in; floods and the rest of the generator parity are open)

## Region and area character (2026-11-03)

Today every area is a box with a floor, walls and props. These items give each region and each area its own look and layout.

- [ ] Each region gets a signature layout language: room shapes and sizes that differ (the geometry block is the start, see Moddability)
- [ ] Hall shapes beyond straight corridors: bends, widened junctions, alcoves and dead ends per region
- [ ] Per-region ceilings: drop ceilings, exposed beams, vaulted or open-to-sky areas, with lights set in the ceiling
- [ ] Per-region wall treatments beyond the flat texture: panelling, tiles, partitions, half walls, arches and columns
- [ ] Area-specific props: a signature prop set for each room role (the mall's shops, the hospital's wards), placed to read at a glance
- [ ] Floor variety inside a region: rugs, tile patterns, worn or wet patches, drains and grates
- [ ] Per-area light colour and temperature, so a diner, a ward and a stock room read differently in the dark
- [ ] Clutter and wear: boxes, stacked chairs, cones, tape and posters placed by rule, not just scattered
- [ ] Landmark areas in each region that the player can recognise and use as a waypoint
- [ ] Sound and ambience per area (a hum in the stock room, a radio in the diner), tied to the audio cue system
- [ ] Props that break the grid: leaning shelves, half-open doors, fallen furniture, cracked walls
- [ ] Region-specific hiding spots that match the area (a laundry basket, a ward curtain, a freezer)
- [ ] A visual check list per region, so each region is reviewed in a run before it ships

## Night Watch port (from the website and the Godot version)

Sources: the website game at KyttusLab/games/night-watch (src/modes, docs/ROADMAP.md, docs/LAST_ROUTE_MATCH.md) and the Godot version at LastRoute Godot/KyttusLabApp/game/night_watch. The Rust binary (src/bin/night_watch) now runs data-driven modes with letter-by-letter calls and full building maps, and Classic is playable; the other eight modes are not playable yet. Each item says which version it ports from.

### Shell and shared framework

- [~] Rust Night Watch app shell: window, title, hub menu with the eight night types, and quit (website: index.html hub; Godot: night_watch_menu.gd) (session 2026-11-03: window and Classic work; the hub entries for the other modes are not playable)
- [ ] Shared roster: the 26 Last Route characters with their roles (website: docs/LAST_ROUTE_MATCH.md; Godot: night_watch_data.gd)
- [ ] Shared save and progress across night types, using the same account store as Last Route (website: account; Godot: night_watch_game.gd)
- [~] Mode registry: each night type registered once, with its name, unlock rule and entry scene (website: src/modes/registry.js; Godot: night_watch_mode_registry.gd) (session 2026-11-03: modes are data-driven now; unlock rules and the other entry scenes are open)
- [ ] Shared HUD, timers, power and camera-map layout (website: ui/; Godot: night_watch_hud.gd, night_watch_cam_map.gd)
- [ ] Night Watch art and hunter scale through one instantiate path (Godot: night_watch_art.gd, night_watch_hunter_scale.gd)
- [~] Night Watch sound cues wired to the scene code (Godot: the nw cues; the Rust nw.rs cues are not called yet) (update 2026-11-04: nw_sound module and sounds.json exist; several modes need the hub to forward events)
- [ ] Admin and debug panel for Night Watch, with the same owner-only gate as Last Route (Godot: night_watch/admin; website: debug-private)
- [x] Office building layout with doors, lights and cameras (website: docs/OFFICES.md; Godot: night_watch_building.gd) (session 2026-11-03: full building maps are done)

### The eight night types

- [x] 1. Classic: static office, 12 to 6 AM, two doors, two lights, cameras, power budget (website: src/modes/classic; Godot: night_watch_classic.gd) (session 2026-11-03: Classic is playable)
- [x] 2. Masked Hours: mask to fool attackers, flashlight, one resource to keep alive, vents (website: src/modes/mask; Godot: mask/) (session 2026-11-04: mechanics present and tested; Ilyra's creep now shows a note. Not yet checked in a running window)
- [~] 3. Dead Air: audio lures, reboot subsystems, hallucinations, vent seal, temperature (website: src/modes/lure; Godot: lure/) (session 2026-11-04: lures, reboots, vent seal and temperature are playable and tested. Hallucinations are not in any source and need a design decision from KT first) (update 2026-11-04: Xivvy walks the booth; hallucinations still need KT's design decision)
- [~] 4. Bedtime: bedroom with no cameras, listen at doors, breathing check (website: src/modes/bedroom; Godot: bedroom/) (session 2026-11-04: repeating door breaths and Haelith's exhale added and tested. Breathing is shown as notes and visuals only, with no sound yet) (update 2026-11-04: Bedtime sounds added)
- [~] 5. The Facility: scripted nights with guided hallway, vent and crawl sections, and a shock button (website: src/modes/story; Godot: night_watch_story or the admin story scenes) (update 2026-11-04: Facility sounds and lift picture added; cue_for_event is unused (warning))
- [~] 6. The Descent: elevator shaft repair puzzles under a hunter's eye (website: src/modes/descent; Godot: descent/) (update 2026-11-04: Descent cues added; the sounds are a pick for KT to listen to)
- [~] 7. Salvage Run: day management and night salvage with an upgrade layer (website: src/modes/dayshift; Godot: dayshift/) (update 2026-11-04: own picture and noise meter; Custom Night is not reachable from the hub)
- [ ] 8. Open Air: three outdoor watches with the roster outside the building (website: src/modes/openair; Godot: night_watch_mixed.gd or the open-air data)
- [ ] Night Watch other 8 modes unplayable: Masked Hours, Dead Air, Bedtime, The Facility, The Descent, Salvage Run, Open Air and Mixed Shift have data but no playable scene
- [ ] Night Watch night-select and Custom Night: pick a night and set a custom hunter lineup before a shift
- [~] Mixed Shift: a drawn roster of threats paced by a director, with QUIET, STEADY, RESTLESS, CHAOS and ENDLESS shifts (website: src/modes/mixed and docs/MIXED_SHIFT.md; Godot: mixed/) (update 2026-11-04: scripted win added; Threat::gate is unused (warning))
- [ ] Variety pass: make the existing modes less repetitive (website: docs/VARIETY.md)
- [ ] New modes drafted on the website (website: docs/NEW_MODES_DRAFT.md): decide which to port

### Checks

- [ ] Night Watch hub forwards events to every mode (several modes need it for sounds); Salvage Run Custom Night is not reachable from the hub; Facility cue_for_event and Mixed Shift Threat::gate are unused (compiler warnings)
- [ ] Dead Air hallucinations: needs KT's design decision first; the Descent sounds are a pick, KT to listen

- [ ] Each night type reviewed in a run against the website for timing, power and AI levels (website is the reference for feel; Godot for numbers)
- [ ] Night Watch save migration: website saves read into the Rust app without loss

## Hunters and pathfinding adapt to the new mechanics (KT, 2026-11-04, next code pass)

- [x] Water-aware pathfinding: path cost by water depth (shore, wading, deep) with per-hunter swim ability and speed from data (`data/hunters/water.json`, speeds from `data/swim/`); most hunters route around deep water unless chasing and able to swim; a hunter that wades makes splash noise the runner can hear; flooded cells (pipe burst, open sluice) change the cost live and force a replan. (done 2026-11-04: data/hunters/water.json, hunters route around deep water and wade when chasing)
- [~] Hunters and bots react to pipes and valves: a burst or a turned valve is a noise event they investigate; a sluice gate opening makes its channel passable; closed gates and flooded dead ends do not trap them (stuck-time give-up already in data). (update 2026-11-04: noise events are in data/hunters/noise_events.json; check that valves and sluice gates are covered)
- [x] Hunters react to the runner's water and fall noise: plunge splash, soaked footsteps (x1.45) and a hard landing are heard at their data ranges and start a search at the sound. (done 2026-11-04: plunge, soaked steps and hard landing are heard at data ranges (data/hunters/noise_events.json))
- [x] Catwalk decks: hunters see a deck runner (2D sight, a grating), climb the catwalk ladder after her and jump down off a deck to chase a runner below (decks.json: every hunter climbs and jumps by default; nyxie, solyth, ysolde, void_blue and void_green stay on the floor until their gimmicks are deck-aware); a hunter never plans through a rail, never jumps while wandering or searching, and a hunter with no way to her stalks the foot of the deck instead of thrashing. The landing thud alerts bots and the runner only. (done 2026-11-04: hunters/brain.rs, flight_edges.rs CatwalkLadder and CatwalkJump, hunters/decks.rs, data/hunters/decks.json. This line used to claim hunters used the catwalk links; until then no hunter could enter a deck: hunter_open was ground only and the old rail test passed on a stale grid)
- [ ] Remote (guest) players as hunter targets: build a `Target` from the remote's sampled position and height through `Target { pos, y, level }` (host only); today the host's hunters target the host runner and the bots only, so a guest on a deck or on the floor is never chased
- [ ] Bots on decks: give game/npc.rs the same height-aware footprint and an `EdgePermit` with the ladder (`StackNav::next_hop_with`) so bots follow the runner up a catwalk ladder; today a bot has no catwalk edge
- [ ] Deck-aware gimmicks: nyxie (lunge and teleport), solyth, ysolde, void_blue and void_green move on the floor only (`move_with_collision` aside, their teleports use ground-only spots); make them deck-aware, then drop their `can_climb` and `can_jump_down` false rows in data/hunters/decks.json
- [ ] Catwalk stairs links: hunters walk a stairs link like a ladder (a climb at ladder speed, `CatwalkLadder`), not up its treads; give a stairs link its own edge walk over the flight's surface
- [ ] Hunter jump pose and landing: the renderer draws a jumping hunter at the fall height only (no jump or landing animation); add a pose and a dust cue, and check the thud level by ear
- [x] Hunters use flights, ladders and drops between floors at normal speed (phase 8 of the stairs project): edges between the resident levels, line of sight across the open stairwell, no freeze for a hunter on another loaded floor (KT decision D5). (commit )
- [x] Tests: a hunter paths around deep water, wades when chasing, replans when a valve floods its route, cannot be trapped by a closed gate, hears a plunge, and follows a runner up a flight. (commit )

## Multiplayer (Last Route)

- [ ] Nearby-player text to speech comes first (KT, 2026-11-04): a chat line from a player near you is spoken aloud (volume and range by distance, a per-player mute and a speech toggle in Settings, rate and voice in data). Voice chat (capture, IMA ADPCM codec, playback) is a later step and builds on the same range and mute rules. (update 2026-11-04: audio/tts.rs exists but nothing calls it; hook it into chat and add the Settings toggle)
- [ ] Two-window room test: one host and one guest on this laptop, room code join, both see each other move (still needs a person at the screen, KT-MBA)
- [ ] Two-machine test: the Mac on KT-MBA joins a room hosted on this laptop, and the reverse
- [ ] Nightly server build: deploy the nightly-server Linux build to the VPS (194.156.90.219) after a check on a test port
- [ ] VPS update procedure: stop the lastroute-rust service, replace /opt/lastroute-rust/current/lr_server, check the SHA-256, start the service
- [ ] Hunter stats stay hidden from other players in a room (the stats check from the open questions, Q24)
- [ ] Room directory: list open rooms, show capacity, and refresh without reconnecting
- [ ] Reconnect: a player who drops can rejoin their room within the reserved window
- [ ] Host migration or room close when the host leaves, with a clear message to the others
- [ ] Co-op revives: downed runners, a revive hold and the window (`game/player/revive_downs.gd`, `_update_revives` at `infinite_route_test.gd` 6901) (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Spectate a downed or ghost runner (the SPECTATE admin button has no handler yet)
- [ ] Player list shows every connected player, not only the local row
- [ ] Runner sync checked at 20 Hz under 100 ms of latency
- [ ] Hunters and their state are identical on every client in a room (the host simulates; guests display)
- [ ] NPC teammates visible to other players in the room
- [ ] Chat: text chat between players in a room
- [ ] Player and room limits: capacity, kick and ban for the host, with the existing moderation rules
- [ ] Multiplayer error messages for refused joins, timeouts and a server that is offline

## Admin custom rooms (2026-11-03)

- [x] lr_server deletes idle admin reservations (RESERVED_ROOM_IDLE_SECONDS, with a keep flag); deployed to VPS3 (done 2026-11-04)

- [ ] Admins can type any room code into the JOIN ROOM code box, even an empty or unused code, and start that room
- [ ] An empty code entered by an admin creates the room with that code, so a test room can be started from the join screen
- [ ] Admin-started rooms show an ADMIN tag in the room list and in the room header
- [ ] Admin rooms keep the admin's chosen settings (seed, mode, difficulty) when the code is entered again later
- [ ] Non-admin players still get the normal refusal for unknown codes, with the same wording as before
- [ ] Admin room codes follow the existing code rules, and the server refuses a code that is already in use by another room
- [ ] Check that an admin room started from the join screen works for two windows (see the multiplayer two-window test)

## Account system and servers (2026-11-03)

### Accounts (Rust client and server)

- [ ] Sign-up and sign-in from the title's ACCOUNT screen, with the username rule 3-24 characters A-Z 0-9 _ (see OPEN_QUESTIONS 11)
- [ ] Passwords stored only as scrypt hashes on the server, with the same parameters as the Node backend (the scrypt test vector in lr_server is the check)
- [ ] Sign-out clears the saved session on this PC, and the signed-in name goes back to empty (guest)
- [ ] Sessions expire on the server, and a session token from one PC is refused on another after sign-out
- [ ] Saved sessions are per server: the primary server and any local test server keep separate sessions (the account config sections)
- [ ] Password change and account deletion from ACCOUNT SETTINGS, with a confirmation step
- [ ] Password reset path for players who lose their password (contact flow or recovery code), decided first in OPEN_QUESTIONS
- [ ] Account saves: each account's run saves go through /api/v1/saves/:gameId/:slot, with the same slot rules as the website
- [ ] Website and desktop accounts share one account store, so a name signed up on one works on the other
- [ ] Migration: existing accounts in data/accounts.json load without loss, and the old session format still signs in once
- [ ] lr_server default data dir is `.data` but AGENTS.md says `data/`: pick one and make the server, the deploy docs and the migration item agree
- [ ] Admin accounts: the owner's account is the only one with admin tools, and admin status is checked on the server, not only in the client
- [ ] Rate limits on sign-in and sign-up, with clear messages when they trip

### Servers (lr_server, VPS)

- [x] VPS deploy: replace /opt/lastroute-rust/current/lr_server from the nightly-server release, check its SHA-256, restart lastroute-rust.service (done 2026-11-04: lr_server deployed to VPS3)
- [ ] Health check endpoint and a script that reports whether the service answers on port 8105 (HTTP) and 1105 (relay)
- [ ] Data backups: back up /var/lib/lastroute-rust (accounts, rooms, saves, crash reports) on a schedule, and test a restore
- [ ] Secrets: /etc/lastroute-rust/lastroute-rust.env stays root-owned 0640 and is never printed or committed; document the rotation steps
- [ ] Logs: rotate service logs, and make sure no password or session token is logged
- [ ] Crash reports: uploaded from the client, stored per server, and viewable by the owner only
- [ ] Mod packs: pack lists and downloads served from the server, with the blocked-pack rules checked on the server
- [ ] Room directory: public rooms listed with capacity, and rooms cleaned up when they are empty
- [ ] Server version reporting: the client shows the server build and warns when the client and server builds differ
- [ ] Staging server: a second service on another port with the nightly-server build, for testing before the VPS update
- [ ] A rollback step for the VPS: keep the previous lr_server binary and restore it with one command

## Intel Mac FPS (KT-MBA: Intel, macOS 12.7.6)

Measure first: the Intel build has not been timed yet. Every item below should be checked against numbers from KT-MBA.

Target: 100+ FPS on the Intel MacBook Air with vsync off, measured with the F7 overlay in the same 60-second walk through a run.

- [ ] Baseline on KT-MBA: F7 overlay frame time, the update / render 3d / hud split, draw calls and spikes, in the same 60-second walk through a run, recorded in this list
- [ ] Renderer check on the Intel GPU: Metal and OpenGL are both built in now (docs/GRAPHICS_API.md); AUTO uses OpenGL on Intel until `LR_GFX=metal target/release/last_route --stress 60 --stress-vsync off` is compared with `LR_GFX=gl` on KT-MBA, then pick the faster one for AUTO. Metal read-back (screenshots) is not implemented, so screenshots stay on OpenGL
- [x] Metal renderer for macOS (graphics API setting AUTO / METAL / OPENGL, `--gfx`, `LR_GFX`, vendored miniquad with three Metal fixes, MSL twin of the world shader, crash-guard marker). On the M4 it ran slower than OpenGL unlimited (GPU-bound about 8-11 ms per frame, cause not found); both sit at the refresh rate with vsync on
- [ ] Metal GPU time: find why the world pass costs 8-11 ms of GPU on the M4 under Metal when OpenGL finishes the whole frame in about 5 ms (try a Depth32Float depth target instead of Depth32Float_Stencil8, count passes per frame, the Xcode GPU capture, the 16-lamp loop unrolled)
- [ ] Metal screenshots: implement `texture_read_pixels` in the vendored miniquad Metal backend and a Metal branch of macroquad's `grab_screen` (it is `unimplemented!()` for non-OpenGL ids), so `--screenshot`, `--self-test` and the F-key capture work without `--gfx gl`
- [x] The 60-75 FPS cap on macOS was not OpenGL: miniquad's macOS run loop waits on a CVDisplayLink every iteration (75 Hz display here, both backends). With VSYNC off the wait is now skipped, live, through `window::set_frame_pacer` in the vendored miniquad; with vsync on it stays
- [ ] Quality AUTO picks a tier from the GPU and the first second of frames on this Mac, instead of a fixed 50% render scale for every Mac
- [ ] Quality governor: step the render scale down on slow frames and back up when there is headroom (Godot quality_governor.gd: 40/50/60 targets, four notches)
- [ ] Lamp shader cost: the fragment shader loops over 16 lamps plus the flashlight for every pixel; test 8 lamps and distance culling on Intel
- [ ] Per-material lamp sort: shader::apply sorts the lamp list for every material each frame; compute it once per frame
- [ ] Draw calls: count meshes per floor and per frame in a run, and merge meshes that share a material
- [ ] Triangle budget: GLB prop and character models, measured on this Mac; decimate or swap the heaviest ones
- [ ] Texture sizes and count: check the atlas and floor set sizes against the Intel GPU memory budget
- [ ] Vsync and frame cap: the window starts with vsync on; with vsync off the loop now runs uncapped on macOS (no display-link wait), so check tearing, CPU use and whether the FPS limit setting (0 = off) helps both on this Mac
- [ ] Release profile: test codegen-units = 1 and lto = "thin" in [profile.release] for speed, with the build time recorded
- [ ] Startup on Intel: assets and sounds load time on KT-MBA, measured against the boot card
- [ ] Mesh rebuild on floor change (see Performance and FPS): the Intel frame cost of a chunk recentre, measured here before and after the fix
- [ ] Fog and the far-plane distance: check how much fragment work the fog and draw distance add on Intel
- [ ] Run the Intel and Apple Silicon builds side by side and record the frame-time gap, so later fixes can be judged on both
- [ ] Reach 100+ FPS on the Intel MacBook Air with vsync off: record the F7 average and the slowest 1% in the 60-second walk, then rank the items above by what they cost on this Mac

## Admin menu (2026-11-03)

Tabs today (src/bin/last_route/ui/admin/mod.rs TABS): STATUS, GENERAL, PLAYER, HUNTERS, NPCS, WORLD, STORY, VISUAL, DEBUG VIEW, FIELDS.

### Hunter pathfinding and HUNTER VIEW

- [ ] HUNTER VIEW option on the HUNTERS tab: show the selected hunter's current path on the map and in the 3D view
- [ ] Path overlay: the hunter's planned route as a line, with waypoints and the target cell marked
- [ ] Path cost readout: path length, cells searched and time for the last path search, from find_path in src/bin/last_route/world/facility.rs
- [ ] Hunter state readout next to the path: state (patrol, hunt, search, return), target, and the last sighting
- [ ] Step the hunter one tick at a time while the path shows, to debug turns and stuck cases
- [ ] Show the hunter's sight cone and hearing radius on the same overlay
- [ ] Highlight blocked cells and the reason a path failed (no route, door closed, prop collider)
- [ ] Pick a hunter by clicking it in the 3D view, not only from the list
- [ ] HUNTER VIEW works on every hunter in the lineup, including those spawned by admin NPC tools

### More admin menu fixes and parts

- [x] Admin tab order and keyboard navigation: Tab and arrows move between tabs and fields
- [ ] Every admin field shows its current value and its allowed range before it is edited
- [ ] Reset button per field and a reset-all on each tab
- [x] Admin actions write one line to the debug log so a run can be audited afterwards
- [ ] Admin panel closes cleanly on ESC without leaving the mouse locked or the pause menu open
- [ ] DEBUG VIEW tab: toggles for the collider, path and navigation overlays, each saved between runs
- [x] FIELDS tab: list every tunable value with its file and key (config, region data and hunter data)
- [~] NPCS tab: per-NPC commands (teleport, remove, follow) instead of only spawn and remove all. Teleport and remove are in; follow is a status line until npc.rs has a per-NPC follow flag, and remove should go through an npc.rs helper
- [x] WORLD tab: regenerate the current floor from its seed and show the seed on screen
- [ ] STORY tab: jump to a story beat by its id, and show which beats have already fired. Blocked: the game has no story beat table or fired-beat state yet (see the pacing director item)
- [ ] Admin tools only enable for the signed-in KYTTU account (see the login reset), with a clear message when they are off
- [ ] Admin menu screenshot checks at 1280x720 for every tab, so layout bugs show up in review

### CI and build speed

- [x] CI cost cuts: macOS builds run only on [mac] in the commit message, workflow_dispatch or the repo variable MAC_ON_PUSH; Windows tests run weekly or by hand (done 2026-11-04)
- [x] Local build and publish to the VPS (packaging/local/publish-local.sh: Windows here, Linux on KT-T480, macOS arm64, intel and universal on KT-NEO); build 42 is published for windows, linux and macos (done 2026-11-04)
- [ ] CI is blocked: the GitHub Actions budget is used up and every job fails to start. KT must raise the spending limit (Settings > Billing > Budgets); until then publishing is local (2026-11-04)

- [ ] CI speedup: cut the four separate cargo test invocations in test.yml to one run where the crate graph is shared
- [x] CI speedup: cache the Windows target directory across runs in build.yml and test.yml (rust-cache keys per job)
- [x] CI speedup: run the Linux and macOS build jobs in parallel with the Windows build instead of one after another
- [x] CI speedup: skip the release build in test.yml when only docs or TODO.md changed (paths-ignore)
- [ ] CI speedup: use prebuilt tools (cargo-binstall) rather than building any CLI tool in CI
- [ ] Faster GitHub compilations: a dev-profile setting for CI that trims debug info (debug = "line-tables-only")
- [ ] Faster compilations on this PC: measure a cold build with opt-level 1 for dependencies, record the time, then decide (profile.dev sets opt-level 2 for the game and 3 for dependencies)
- [ ] Faster compilations: split the server-only dependencies from the game build (done with the server feature; check the CI cache keys match)
- [ ] Faster compilations: check linker speed on Windows (rust-lld) and record the time it saves
- [ ] CI speedup (2026-11-04): the macOS build compiles two targets in one job (x86_64 then arm64) before lipo. Run one job per target in parallel and combine the binaries in a final job (packaging/macos/universal.sh)
- [ ] CI speedup (2026-11-04): share a compile cache between the macOS target jobs and the Windows job with sccache, so the common crates build once per run
- [ ] CI speedup (2026-11-04): make the nightly publish steps run only after the build artifacts are uploaded, so a failed zip step does not leave a half-published nightly
- [ ] CI speedup (2026-11-04): record the cold and warm build times for each job in the run summary, so a regression shows up in the run list without reading the logs
- [ ] CI speedup (2026-11-04): pin the Rust toolchain version in CI (rust-toolchain.toml) so a new stable release does not invalidate every cache at once

## Godot parity: found by discovery (2026-11-03)

- [x] Godot parity audit written: docs/GODOT_AUDIT_2026-11-04.md (done 2026-11-04)
- [ ] Parity audit leftovers, batch by batch (docs/GODOT_AUDIT_2026-11-04.md): hunters look and motion, feel numbers, UI, world look and sound, hiding co-op feel

New items from a read-only comparison of the Godot game folders with TODO.md. Effort as marked.

- [ ] Port Porcelle's hunter radar ping on the minimap: a slow ping about every 8 s, an eased ring, and a steady dim dot under reduced motion (effort S; game/hud/radar_ping.gd)
- [x] Port the centre reticle, shown only while the view is aimed (hidden with the cursor free, at the desk, under the tablet or in the pause menu) (effort S; game/hud/crosshair.gd)
- [ ] Port world-space teammate markers: name and distance at each teammate, edge markers off screen, floor offset, dimming when stale, DOWN and CAUGHT labels (effort M; game/hud/teammate_waypoints.gd)
- [ ] Port quick-chat map pings: a room-chat ping shows on the minimap and full map on the floor where it was posted (effort S; game/hud/hud.gd)
- [x] Port the QUIT TO DESKTOP confirmation: YES on the left, NO focused by default, Esc or click outside closes; YES saves and closes a hosted room (effort S; game/ui_common/quit_confirm.gd)
- [ ] Port the top-centre update badge: downloading percentage, then RESTART NOW or LATER, with the status-bar text during a run (effort S; game/menus/update_badge.gd)
- [ ] Port the games hub ADMIN panel: build info, server health, updates per platform, public route status, notices, debug toggles, skin removal (no tokens shown) (effort M; game/admin/hub_admin_panel.gd)
- [ ] Port the public route daily lineup: a stable UTC-day featured pair, weighted draw (easy 50%, intermediate 40%, difficult 10%), one extra hunter per two players, capped at 8 (effort M; game/config/public_roster.gd)
- [ ] Port the admin find-player marker: a beam and two rings through walls at the target for about 6 s, following the live position (effort S; game/admin/admin_pulse.gd)
- [ ] Port troll-mode advertisements for a chosen player: gentle and annoying profiles, a per-window cap, never shown while caught or in menus (effort M; game/admin/troll_ads.gd)
- [x] Port the F2 screenshot: saves the whole game window to a dated PNG from any screen (effort S; game/autoload/screenshot.gd)
- [ ] Port the single cursor owner: menus register named claims, and any claim frees the cursor so the camera cannot re-grab it under a menu (effort S; game/autoload/mouse_lock.gd)
- [ ] Port phone touch controls: on-screen D-pad, drag-to-look, touch buttons, and the phone and low-memory budgets with --touch and --lowmem (effort L; game/hud/touch_controls.gd)
- [ ] Port XR (OpenXR headset) mode: comfort and fade, gaze pointer, hand tracking, and the route and menu panels in 3D (effort L; game/xr/xr_comfort.gd)

## Known bugs (2026-11-03)

- [x] macOS crash on the first map load: world_mesh phase 2 skipped unbuilt parts (done 2026-11-04)
- [ ] Mansion doors are invisible (found on KT-MBA; builds run there now). luxuryRoomDoor models and data are fine, so check the in-game draw
- [x] Skin menu is missing two things (KT, 2026-11-04): (1) the TEMPLATE button only says "not in this build yet" (ui/settings_panel.rs ~line 676); it must save `lastroute_skin_template.png`, the 64x64 template (Godot `PlayerSkin.template_png()`, `skin_panel.gd` _save_template / write_template), to the Downloads folder or through a save dialog and report the path. (2) the preview is a flat 2D stand-in (`draw_preview`); it must show the skin on the rotating 3D mannequin (`assets/player/player_mannequin.bbmodel`, scaled to the 1.60 m runner as in `skin_panel.gd`), turning on its own and with the chosen or picked skin applied live. (done 2026-11-04: TEMPLATE button saves the template, rotating 3D preview, full skin validation (64x32 upgrade, opaque base layer, canonical hash, 16 KB cap))
- [~] Locker hiding must be a real 3D view, not a 2D overlay (KT, 2026-11-04). Today ui/hud.rs paints black bars with four slits over the screen and render/scene.rs skips drawing the locker while hidden. Godot (`player_controller_3d.gd` enter_hiding / _begin_hide_move, `route_interactable_3d.gd` hiding_inside_position, hiding_eye_height, hiding_outward_yaw, _build_hide_seal, set_local_hiding_view): the camera moves into the locker (behind the vent band, eye centred on the vent band, yaw = the locker's facing), the player sees out through the real door louvres of the 3D model, and a dark two-sided shell just inside the prop bounds stops the hider seeing out through back faces. Remove the HUD bars; keep the LEAVE / LOCKED IN text. Wardrobes and beds get the same treatment (bed: lie flat, slit under the foot). Runs after the stairs phases 1 and 2 (both touch game/player.rs and render/scene.rs). (update 2026-11-04: camera inside the locker (render/hiding_cam.rs), HUD bars removed; wardrobe and bed styles still open)
  - [ ] The locker is physical with the player inside: the runner's body stays in the locker (visible to other players, hunters and bots) so the hunter locker animations (search tell, open, reach in, pull out; Javelle, Loxelle and the others) play against a real occupant. Hunters and co-op peers see the hider's figure in the locker, not an empty locker. (update 2026-11-04: render/locker_shell.rs, locker_occupant.rs and net/hide_sync.rs exist with tests but scene.rs and net do not call them yet; hunter locker animations against the occupant are open)
  - [x] Look limits while hidden (KT, 2026-11-04): the camera must not be rigidly tied to the locker so that free look is lost or broken. Keep the camera independent of the locker's shake (search tell, door swing) and limit the look to a small yaw and pitch cone through the vents (cone sizes in data/ JSON), or lock the look while the locker moves. No free look through the walls. (done 2026-11-04: camera independent of the locker shake, look limited to a data cone)

- [ ] Beds stick out of rooms into the hallway behind them (in progress: workflow bed-placement-fix; see furnished.rs bed placement)
- [ ] Dossier: the mouse wheel scroll jumps back to the top. Not reproduced from the code; needs the tab and steps from the person testing it
- [x] Door headers above doors (added in the 3D map pass) are not checked in a run, and may overlap archway lintels (archways removed, so no overlap)
- [ ] Safe zone checkpoint light and the 0.35 ambient are checked in one screenshot only; confirm in a safe zone and in a dark corridor
- [x] Release build fails while last_route.exe is running (file lock). Close the game first, or build to a copy
- [ ] Windows CI: the server test step compiles slowly on the runner and was cancelled once; confirm it finishes on a normal run
- [ ] Windows CI: the self-test runs locally only (hosted runners have no OpenGL pixel format)
- [ ] Account screen kicker still reads "KYTTU'S LAB // ACCOUNT"; change it to "KYTTU: THE LAST ROUTE"
- [ ] Chairs face their tables: the code matches Godot, but no restaurant render has been checked (OPEN_QUESTIONS 21)
- [ ] Ambient is 0.35 here against Godot's 0.035, a deliberate change; decide whether to keep it or match Godot
- [ ] Mouse look speed is unchecked against Godot with the same swipe (OPEN_QUESTIONS 20)
- [x] lr_server: some helpers are only used by tests and are gated with cfg(test); delete the ones nothing needs
- [x] God mode leaked to other players in a room (fixed 2026-11-03)
- [ ] Peek lean is not visually confirmed: hunter hallway peeking works in the tests, but no screenshot has shown the lean yet
- [x] Inferno floor heights: Godot's Inferno floor is flat, so the player standing at the base height is parity and the Rust-only drop (0 / -0.6 / -1.2 m, drawn from the old carve) is gone; the drawn floor, walls and decorations read the carve the open cells use (`InfernoChunk::height_at`: walkable 0, lava `floor.lava_basin_y`, data/inferno.json) (done 2026-11-04). Not Inferno parity yet: layout, ceilings, lava pit render and cages are open (see the lava pit line below)
- [x] Stairwell with only 2 walk rows at seed 40490470, chunk (3,2): check that the flight is walkable end to end (covered by the contract test `the_todo_stairwell_at_seed_40490470_chunk_3_2_is_walkable_end_to_end` in world/stack_contract_tests.rs, stairs phase 1)

## Performance and FPS

- [ ] Window recentre hitch of about 80 ms per recentre: about 55 ms is the full world mesh rebuild (build_world on every floor change in sync_world); build the mesh per chunk instead
- [ ] Chunk window load of about 23 to 28 ms per recentre (infinite::window generation, residency, sync_in); move it off the frame or build the next window ahead
- [ ] No quality governor or frame budget. Godot has quality_governor.gd (40/50/60 fps targets, tiers for shadows, prop range and lamp count); add the same tiers
- [~] Hunter cost is untested at high counts (bench added: cargo test --release hunter_brain_step_cost -- --ignored --nocapture; not yet run). Godot measured brain.update at 31.8 ms with 38 hunters; bench the Rust brains at 38 (run 2026-11-04 on the dev Mac, release: 38 hunters, 600 steps, 0.11 ms per step before the deck work and 0.09 to 0.11 ms after; the bench has no decks, so it shows only that the layer checks cost nothing on a deck-free floor)
- [ ] Light sight cost is unknown; MAX_LAMPS is capped at 16 in the shader, so lamp pools can pop in and out
- [ ] Steady-state render cost is not measured because vsync caps frames at 60; add a capture with vsync off
- [ ] Startup: assets take about 195 to 208 ms and sounds about 249 to 266 ms. Load in slices behind the boot card, as Godot's STARTUP_BUDGET_MS does
- [x] Draw calls per frame are not counted in a run; use the F7 counters and log the totals
- [x] The F7 overlay averages are not collected over runs; log spikes with a cause, as Godot's PerfLog does
- [ ] CI builds: confirm the rust-cache hits in build.yml after the first run
- [ ] Reach 144+ FPS on this laptop (AMD Ryzen AI 5 340, integrated Radeon 840M, 1280x720) with vsync off: run the --stress route, record the average and the 1% low, then fix the top cost in each frame section until the 1% low reaches 144
- [ ] Godot vs Rust performance comparison: the same seed, route and hardware for both builds (KT-MBA and this PC). Record frame time (average, 1% low), startup, memory, hunter cost at 38 and draw calls, from the Rust --stress CSV and the Godot F7 overlay. Keep both tables in docs/PERF_COMPARISON.md and tick each number when measured.

## Follow-ups and fixes found 2026-11-03 (late)

### Accounts and login
- [ ] Title ADMIN entry still checks the local name; gate it on the server admin flag (title_entries and title_focus need the flag passed in)
- [ ] Hub ACCOUNT row shows fixed "SCOPES NONE"; pass the signed-in admin flag and scopes to draw_hub
- [ ] account_exists returns false when lr_server forwards to the website, so admin set-roster 404s; needs a website route for it
- [~] lr_server holds its state lock for up to 5 s during a forwarded account call; account calls now run outside the lock, but save and moderation routes still hold it
- [ ] Website savedAt is in milliseconds, the game uses seconds: both sides should agree on one unit
- [ ] A stale save PUT now answers 200 with the stored time, not 409 STALE; check how the game client reads it, and add a test for the newest-wins branch
- [ ] Website keeps 10 save slots per game; delete older run-world-N slots when a new revision saves
- [ ] Deploy the account forwarding: set LASTROUTE_ACCOUNT_UPSTREAM on VPS3 and restart lastroute-rust.service (needs approval)
- [ ] Hub SERVER row shows the server host to players; the privacy rule says players never see the address, so decide

### Multiplayer and figures
- [x] Runner figures stand in one pose; add a moving flag to RemoteView and a walk cycle (done 2026-11-04: RemoteView carries moving, sprinting, crouching and slim; player clips drive runner figures (render/anim/runner.rs))
- [ ] runner_figure.rs keeps its own KNOWN_SKINS list; make SKINS in world_sync.rs visible and use it, and drop the unused cube colours
- [ ] Check the skin trim placement on the character model in a real run
- [ ] Guests see other guests' ping as the host-measured round trip; measure it directly if guests ever link to each other
- [ ] NPC follow is a status line: add a per-NPC follow flag in npc.rs, an npc_remove_at helper, and fix the stale deferred-commands comment at the top of npc.rs

### Stress tests and machines

- [ ] The two Mac Minis (KT-Mini4-1, KT-Mini4-2) have git clones at ~/Developer/LastRoute-Rust with origin set to GitHub; they need a one-time GitHub login (2026-11-04)
- [ ] KT-PC (G:/Projects/KyttusLab-Rust) has no SSH server, so the copy is pending (2026-11-04)
- [x] Stress walking runners no longer stall on walls (fixed 2026-11-03)
- [ ] Stress scenarios: noclip, NPCs, hunters, interactions (tools, breaking barriers) and mixed, chosen with a --stress-scenario flag
- [ ] --stress-vsync on|off, and the runner runs every scenario in both modes, naming each CSV by scenario and vsync
- [ ] Run the stress tests on KT-MBA, KT-MBP, AK-MBA, XR-MBA, JV-MBA, ZL-MBA, KT-NEO, KT-T480, AK-T480 and this PC; keep the CSVs and a summary table in docs/PERF_COMPARISON.md
- [ ] Command-line tools still to install at XR-MBA, JV-MBA, ZL-MBA; alsa-lib-devel at AK-T480
- [ ] git archive from this PC writes CRLF into shell scripts (core.autocrlf); copy with -c core.autocrlf=false, or add a .gitattributes entry for *.sh
- [ ] Windows laptop: run the stress test on this PC and compare it with the Intel Macs

### Models
- [~] bbmodel is the standard: GLB-to-bbmodel converter, the five Godot models the port lacks (door_frame, floor_tile, wall_block, kathy_scythe, sewer_outflow_pipe_broken), the Godot _0/_1/_2 variants, and the 430 GLBs the port still ships. Done so far: converter and 430 bbmodels beside their GLBs. Still open, in order: (1) catwalk fixes first, (2) copy the 220 Godot-only GLBs that prop data references and convert them, (3) switch the prop loader in shared/assets.rs to bbmodel with GLB fallback, (4) move door panels in render/door.rs to bbmodel (session 2026-11-03: props are converted to bbmodel and the runtime is migrated (done); bbmodel in mod packs is in progress; the server pack path rule does not accept .bbmodel files yet)
- [ ] Make the draw path pick a variant per prop instead of one model for all variants

### Updates and builds
- [ ] Confirm the first real in-game update end to end on Windows and a Mac, and read the update lines in the log
- [x] Make the update visibly obvious: show "Checking for updates" on the loading screen, a progress line while the new build downloads and installs, and "Updated to build N" after restart. Right now a launch with nothing to install logs nothing, so a player cannot tell the check ran (found on KT-MBA, 2026-11-04) (done 2026-11-04: updater logs every outcome, boot screen shows check, progress and "Updated to build N")
- [ ] The Mac app still carries lr_server until its next update; check that the new bundle does not
- [ ] Keep each machine's download-key in the game folder; the old Mac playtest folder held it and was deleted, so restore it on any other Mac
- [ ] Commit the Godot account kicker change (KYTTU: THE LAST ROUTE) in LastRouteGodot, with its owner's approval
- [ ] Map looping report: retest on the current build; batch 12 is in git stash (batch 12 paused: map looping report) and needs a bisect or redo

### Settings
- [ ] vsync is on by default, so the frame rate stops at the refresh rate; decide whether the default should be off
- [ ] Add a reduced motion setting (the radar ping reads a constant for it)

## Movies, trailers and cutscenes (Rust port)

The "Movies / cutscenes and a scene maker" item above is done on the Godot side (tools/playtest, shot files). The Rust port has none of it yet.

- [ ] Port the shot file format: a JSON shot (stage, actors, camera path, timing) read the way Godot's playtest sequencer reads it (`dev/playtest/playtest_sequencer.gd`, `docs/FLYTHROUGH.md`)
- [ ] Port the flythrough camera: planned tracks through an area, noclip and input frozen, reusing the stress mode's route code where it fits
- [ ] Cutscene runner: play a shot list with a fixed timeline, hold the game (no hunters, no input) and return control at the end
- [ ] Scene maker: an admin tab to place actors and a camera path on the live map and save them as a shot file
- [ ] Movie capture: write each frame to PNG at a fixed fps, then assemble with ffmpeg (report the exact command used)
- [ ] Trailer shot list: a data file that names the shots in order, so a trailer is reproducible from the seed

## Source layout (done 2026-11-03)

See docs/SOURCE_LAYOUT.md for the inventory.

- [x] Move last-route-only modules (audio, config, game, hunters, render, shared, world) into src/bin/last_route. Blocked today: lr_server uses content, and content depends on all of them. Options: (a) split content so the server only needs the pack and region types it uses, then move the rest; (b) give the server its own copy of content's pack check; (c) keep the shared core in lib and only move the front end. (session 2026-11-03: done; the game lives in src/bin/last_route, see docs/SOURCE_LAYOUT.md)
- [x] lr_server sorted into the folders accounts, rooms, saves, packs, relay, ops, common and server (done 2026-11-03)
- [x] Night Watch has no library dependency and its src/night_watch module has no users; move it into src/bin/night_watch or delete it after checking. (session 2026-11-03: done; Night Watch lives in src/bin/night_watch)

## Moddability

- [x] Mod example pack docs/examples/mod-hunter/ (done 2026-11-04)
- [ ] Mod packs cannot carry mechanics, rule tables, rigs, grab pools or noise rows. Design (2026-11-04 plan): start with a declarative JSON behaviour language, a sandboxed script engine later behind a flag, no native plugins. Not built
- [ ] The overrides format differs between the server index (hunters/<id>) and the local loader (bare id): make them agree
- [ ] Guests without the pack see a mod hunter as Dovelle: send a fallback or require the pack
- [ ] docs/MODDING_SPEC.md lacks the lr.hunter/1 field table and names stale server paths

- [~] Region geometry block (lr.region/1, optional): wall_height, room_min and room_max are parsed and tested. The 3D walls and the route room sizes do not read them yet
- [ ] Godot's REGION_FIELDS must accept geometry, or Godot drops region files that use it
- [~] Mod-authored props with their own GLB models, through the props catalogue (session 2026-11-03: props now load as bbmodel; mod pack bbmodel support is in progress)
- [ ] Mod-authored loot tables per region
- [ ] Mod-authored safe-zone light colour and strength per region
- [ ] A pack validator command that checks a pack folder without starting the game
- [ ] docs/MODDING_SPEC.md is a placeholder: write the region, hunter, prop and pack formats

## Godot feature audit (2026-11-01): Infinite route, player and main loop

Read-only audit of `LastRoute Godot/KyttusLabApp` against `src/`. Only gaps are listed:
`[~]` exists but is incomplete, `[ ]` is not ported. Ported behaviour is not listed.
Godot references are `game/...` under the KyttusLabApp folder.

### Infinite route and levels

- [x] Stairs and ladders project, phases 0-9 (docs/STAIRS_CONTINUOUS_DESIGN.md): floors 5 m apart in one continuous space; every stairwell is a Flight (world/flight.rs, data/flights.json); stairs walked with no prompt, fade or teleport; ladders are a real climb in absolute y (data/ladders.json); waterfalls and drops are a physical free fall with a landing rule (data/swim/fall.json); the neighbour floor is visible through the open stairwell; relabel and walk_relabel replace the level swap (done 2026-11-04)
- [x] Hunters, bots and remote players cross flights and ladders (hunters/flight_edges.rs, data/hunters/flights.json); hunters simulate on all resident floors; remote players are drawn at their true height (done 2026-11-04)
- [x] Saves store absolute y (y_abs) and legacy saves migrate; change_storey remains only for respawn, admin jump and the test helper (done 2026-11-04)
- [x] Catwalks above level 0, sewer water, swim and pipes, second sewer, inferno caves and level 0 maze variants (done 2026-11-04)
- [~] Infinite district layout matches Godot's streamed route generator (see the round 4 map item). `src/bin/last_route/world/infinite.rs` and the chunk RNG tags differ, so seeds will not match
- [x] Stairs are walked in Godot; Rust teleports to the landing (`src/bin/last_route/game/endless.rs` `take_stairs`). Fade and hold removed 2026-11-04: the level changes on the connection's frame, numbers in `data/stairs_links.json`. Godot's `game/world/generation/stairwells.gd` also has spiral and switchback flights (done, stairs project phases 5-7: flights are walked, no teleport)
- [ ] Relabel pop: right after a walked relabel the new active floor holds only the neighbour's scoped parts (within `neighbour_cull_radius` of an opening, `neighbour_max_parts`) until the budgeted rebuild finishes; far parts of the floor appear a few frames late. Fix idea: widen the neighbour build to the full window for the level the runner is about to enter, or keep drawing the previous full meshes until `!has_pending()` (render/world_stack.rs `NeighbourWorlds::relabel`, game/endless.rs `relabel`). Needs a GPU-backed test.
- [~] Level links other than stairs: ladders, ramps, manholes, grates, elevators and drops (`docs/SEWERS.md`, `game/world/generation/sewer_route_generator.gd`) (status note: [~] in the parity-gaps audit; the less complete status is kept) (update 2026-11-04: ladders and drops are done (data/ladders.json, data/swim/fall.json); manholes, grates, elevators and ramps are not rechecked)
- [x] Natural stairs and ladders: port the Godot slope system so stairs are walked as slopes and ladders are climbed, and hunters cross stairs between floors (stairs and ladders done in phases 5-6; hunters crossing is phase 8) (commit )
- [ ] Sewer levels -1 to -3, with the cistern, collector and Inferno maze. `src/bin/last_route/world/infinite.rs` picks regions by level, but no sewer geometry exists and `src/bin/last_route/data/loot/sewer.json` is unused (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] Hunters persist across chunks and floors and walk the stairs in Godot. Rust rebuilds the lineup on every level change (`reset_hunters` in `endless.rs`) and respawns out-of-window hunters (`stream_window`) (commit )
- [ ] Safe rooms with breach and checkpoints (`_update_checkpoint`, `_request_breach` in `game/infinite_route/infinite_route_test.gd`). Rust has one rest-only safe zone per floor (`interact.rs` 266-272) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [~] Autosave on relays, level changes, safe-room checkpoints and quit. Rust saves only on stairs and the lift (`endless.rs` 248, `interact.rs` 284) Note: level changes now autosave once on the label change; safe-room checkpoints and quit still pending as written.
- [ ] Anti-AFK void pressure on idle parties (`game/player/anti_idle.gd`, `_update_anti_idle_pressure` at `infinite_route_test.gd` 2388) (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Void player powers: Gray's signal route, Orange's outages, Purple's radar static and Pink's false footsteps (`game/player/void_player_effects.gd`). Every `void_*` id maps to `Gimmick::None` (`src/bin/last_route/hunters/gimmicks/mod.rs` 115-142) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [~] Run statistics on the run card: time, distance, rooms, levels and hunters met (`_update_run_statistics`, `infinite_route_test.gd` 1351). Rust shows depth, score, relays, coins, difficulty and seed (`src/bin/last_route/ui/report.rs` 236-247)
- [~] Online route sync. The protocol and session code exists under `src/bin/last_route/net/`, but nothing in the run loop calls it (`src/bin/last_route/app/mod.rs` has no `net::` reference)

### Player and movement

- [~] Crouch stand-up headroom check and crouch hitbox (`_can_stand_up` and `_update_crouch_hitbox`, `game/player/player_controller_3d.gd` 962-995). Rust crouch only changes speed and eye height (`src/bin/last_route/game/player.rs`)
- [~] Hide glide: Godot animates into the locker (`enter_hiding` and `_begin_hide_move`, lines 707-870). Rust teleports to the locker centre (`src/bin/last_route/game/interact.rs` 341-344)
- [ ] Sit, lie and ride on furniture (`use_furniture`, `game/player/player_controller_3d.gd` 489; `docs/SEATING.md`) (effort M)
- [ ] Sleep and drowsiness, with hunters stirring and jolting the sleeper (`toggle_sleep`, `sleep_disturb`, `_update_sleep`, lines 606-670; `docs/REST_AND_SLEEP.md`) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Gamepad: right-stick look, pad bindings and triggers (`docs/CONTROLS.md`). `src/bin/last_route/app/mod.rs` `read_controls` reads only keyboard and mouse
- [ ] Flashlight flicker and seize after a plunge splash or a hunter event (`force_flashlight_flicker`, `flashlight_seized`, `player_controller_3d.gd` 873-890)
- [ ] Mutual light sight: hunters see you when they stand in light (`docs/LIGHT_SIGHT.md`). Rust sight uses the flashlight only (`src/bin/last_route/hunters/brain.rs` 372-380)
- [~] Quality governor: Godot steps quality down when frame time misses (`game/look/quality_governor.gd`). Rust uses a fixed render scale per setting (`src/bin/last_route/app/mod.rs` 45-52)
- [~] Hiding props: beds, wardrobes and tables with capacity 1 to 2 (`docs/HIDING_PROPS.md`). Rust hides in generated lockers only (`src/bin/last_route/world/facility.rs` 1352). The `hide_capacity` values in `src/bin/last_route/data/props` are not read by any Rust source

### Items and tools

- [ ] Inventory screen on Tab, and backpack capacity upgrades (`player_controller_3d.gd` 145-158). Rust has a five-slot hotbar only (effort M)
- [ ] Lockpick set and locked caches (`LOCK_TOOLS`, `player_controller_3d.gd` 180) (effort M)
- [~] Crowbar and axe: breach boards and doors, cut a hunter's grab free, and rescue a grabbed teammate (`BREACH_TOOLS` at line 170, `docs/RESCUE_AXE.md`). Rust swings only stun hunters (`src/bin/last_route/game/items.rs` 142-190)
- [ ] Revive kit pickups are generated but never collected (`src/bin/last_route/world/facility.rs` 107, 121; `src/bin/last_route/game/endless.rs` 353 drops them)

### Carnival, valves and map

- [ ] Carnival booths and carousel rides (`docs/CARNIVAL_BOOTHS.md`, `_request_booth` and `_request_carousel` in `infinite_route_test.gd` 4676-4835) (effort L)
- [ ] Flood valves (`_request_valve`, `infinite_route_test.gd` 4924)
- [ ] Waypoints: one per player, route chevrons and a ground trail (`docs/WAYPOINTS.md`, `_request_waypoint` at 1739). Rust only has an admin label reading "NO WAYPOINT SET" (`src/bin/last_route/ui/admin/actions.rs` 324) (status note: [~] in the parity-gaps audit, [x] in the full audit; the less complete status is kept)
- [ ] Map: waypoint and teammate pins are missing. Discovery and the full map are ported (`src/bin/last_route/ui/map.rs`) (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Co-op and NPCs

- [ ] Spectating a downed or ghost runner (`enter_spectator`, `set_ghost_follow` in `player_controller_3d.gd` 1619-1700)

### Controls, look and presentation

- [ ] Default bindings: Tab inventory, T chat, O player list and the gamepad are missing (`docs/CONTROLS.md`). Movement, E, F, G, Q, 1-5, the wheel, M, F1, F7, F11 and Escape are present (status note: [~] in the full audit; the less complete status is kept)
- [~] First-person body, climb hands and held-item grips (`player/first_person_body.gd`, `player/held_item_poses.gd`). The grab view notes the runner's own body and hands are not ported (`src/bin/last_route/render/anim/grab.rs` header) (update 2026-11-04: first-person hands are in)
- [ ] Emotes, the blocky face, doorway pose and player skins (`player/blocky_emotes.gd`, `player/blocky_face.gd`, `player/doorway_pose.gd`, `player/player_skin.gd`). The Rust SKIN tab says the offline build plays the default runner (update 2026-11-04: skins are validated and the skin menu works; emotes, face and doorway pose still open)

### Checks and corrections to earlier notes

- [ ] In-game check of mouse vertical look. `docs/PARITY.md` item 1 reports an inverted sign, but macroquad 0.4.16 `input.rs` 75 gives `last - current` and `src/bin/last_route/app/mod.rs` 113-120 flips it back, which matches Godot's `player_controller_3d.gd` 1116. Item 1 looks wrong (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Update `docs/PARITY.md` items 2 and 3. Keyboard look already scales by sensitivity and normalises diagonals (`app/mod.rs` 124-127, tested at 916-940) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] `src/bin/last_route/app/settings.rs` line 7 says graphics quality changes nothing, but `targets()` in `app/mod.rs` scales the render target by quality. Fix the comment or the code

## Godot feature audit workflow (2026-11-01): full feature audit

Full read-only audit of `LastRoute Godot/KyttusLabApp` against the Rust port, written to `docs/GODOT_FEATURE_AUDIT.md` (638 features: 130 ported, 213 partial, 262 missing, 33 not applicable). Only missing or partial features with priority core or important are listed here. Ported and skipped items are not listed. Re-run with the workflow `godot-feature-audit` from the project folder.

### Night Watch shell
- [ ] Port Title screen: PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT and the drifting clock (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Shift board: nine mode rows with cleared counts, portraits, shift tiles and START SHIFT
- [ ] Port Custom Night: per-hunter 0-20 sliders, presets, score and START NIGHT, unlocked after Night 5
- [ ] Port Account screen: Night Watch sign-in and create account on the shared account backend (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Settings: Calm mode, Scare Full/Short/Off and ERASE progress (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Pause card: RESUME (Esc) and QUIT TO HUB, with paused time stopped (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Win and death end cards: Next night, Retry and Menu (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Shared shift framework: NightWatchMode base, session hand-over and mode registry
- [ ] Port Progress save: night_watch.cfg with cleared levels, stars, best scores and custom values (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Controls table: one action table with rebinding, hint line and controls tab (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Interactable stations and prompts: monitor, doors, light, lamp and ledger (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hunter scale for Night Watch placements: reuse MODEL_SCALE 1.5 through one instantiate path (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [~] Port Night Watch sound mapping: 36 nw_ cues exist, but no event-to-cue mapping or room tones (status note: [~] in the parity-gaps audit; the less complete status is kept) (update 2026-11-04: nw_sound module and sounds.json map events to cues)
- [ ] Port Seeded integer streams: Night Watch mulberry, stream and pacer on the existing RNG core (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hunter pose library: 21 data-driven poses and the state table (only a quaternion helper exists) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port HUD: shift clock, power meter, status line and prompt line (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [~] Port Zellia's call box: ring, typed lines, subtitles and queued scripted calls (status note: [~] in the parity-gaps audit; the less complete status is kept) (session 2026-11-03: letter-by-letter calls are done)
- [ ] Port Caught moment and scare setting: scare pose, framing, duration and sound (status note: [~] in the parity-gaps audit; the less complete status is kept)

### Night Watch Classic

Session 2026-11-03: Classic is playable with data-driven rules, full building maps and letter-by-letter calls. The items below have not been rechecked against it.
- [ ] Port Classic night rules: seven nights, door and light tables and the 6 AM win
- [ ] Port Power drain, blackout and Nyxie arrival (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Doors and door lights: two steel doors held shut and lit (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Monitor and camera feeds: raise the monitor and switch cameras (effort M)
- [ ] Port Camera minimap with CAM buttons at room positions (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Eight-room building and walkable paths (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Hunter AI: stalkers, door queue and pair speed-up combos (effort L) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Hunters walking the building at a walking pace (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Security room: walkable office with desk, monitor and stations (effort M)
- [ ] Port Night 7 Kathy alone: hum at doors, eye-meet on a hall camera, fade

### Night Watch Bedtime
- [ ] Port Bedroom view from the pillow: dark 5 x 5 m room with look controls
- [ ] Port Held keys A, D and F with the single flashlight battery (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Loxelle, Javelle and Haelith Bedtime behaviour (effort M)
- [ ] Port Sleep meter (creep) and restless house cues (effort M)
- [ ] Port House wing map: bedroom, hall, living room, galley, kids' room and bathroom
- [ ] Port Six Bedtime nights and the Custom Night row (effort M)

### Night Watch Salvage Run
- [ ] Port Day shop: five toggle upgrades within the week's budget (effort M)
- [ ] Port Five weeks with budget, quota and carry-over (effort M)
- [ ] Port Salvage rack: four slots, three decks and hold to take (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Noise meter from salvaging and visitor response (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Visitors at the side doors (Ravelle, Ceryth) with warnings (effort M)
- [ ] Port Ysolde at the window: knock and quiet rule (effort M)
- [ ] Port Cargo bay office with rack, ledger and two doors (effort M)
- [ ] Port Ship and scrap yard map (effort M)

### Night Watch The Descent
- [ ] Port Service elevator view with Elise waiting at the sill (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Five descents and stage order (effort M)
- [ ] Port Fuse-box wires puzzle with wrong-button restart (effort M)
- [ ] Port Signal panel memory puzzle (effort M)
- [ ] Port Pipework puzzle with flow check (effort M)
- [ ] Port Door hold last stage (effort S)
- [ ] Port Elise closeness bar and wrong-attempt bumps (effort S)

### Night Watch shared framework
- [ ] Port Shift scene contract and session hand-over (NightWatchMode) (effort L)
- [ ] Port Hunter pacing: the three-second window that slides with AI level
- [ ] Port Seeded replayable shifts with golden fixtures for each mode (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Office contract: one walkable office per mode with stations (effort M)
- [ ] Port Per-mode map registry, validated against Classic (effort M)

### Night Watch Masked Hours
- [ ] Port Masked Hours six-night shift and Custom Night sliders (effort M)
- [ ] Port Mask toggle exclusive with the monitor, with raise and lower timings
- [ ] Port Desk monitor with eight cameras (effort M)
- [ ] Port Hall light and the battery that does not recharge (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Left and right vent lights (effort S)
- [ ] Port Music box wind, tune and run-down (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Thessine walking in from the hall (effort M)
- [ ] Port Umbriel the rusher (effort M)
- [ ] Port Vent hunters Verrine, Haelith and Ilyra (effort M)
- [ ] Port Masked Hours wing layout (14 rooms on an AStar3D graph) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Front office with no door and five stations (effort M)
- [ ] Port Zellia scripted calls for nights 1-6 (Masked Hours) (effort M) (status note: [~] in the parity-gaps audit; the less complete status is kept)

### Night Watch Dead Air
- [ ] Port Dead Air shift, nights 1-6 and Custom Night (effort M)
- [ ] Port Broadcast booth office with stations (effort M)
- [ ] Port Radio station map and waypoint graph (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Xivvy and the lure mechanic (effort M)
- [ ] Port Sealing vents and air stress (effort M)
- [ ] Port Phantoms breaking systems and reboots (effort M)
- [ ] Port Temperature, heater and fan climate rules (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Monitor panel with systems rows and LURE (effort M)
- [ ] Port Zellia calls for nights 1-6 (Dead Air) (status note: [~] in the parity-gaps audit; the less complete status is kept)

### Night Watch The Facility
- [ ] Port The Facility shift, nights 1-6 and Custom Night (effort L)
- [ ] Port Nightly task list with six task types (effort M)
- [ ] Port Five-room plant with lift and doorway timing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Control room office with keypad and task board (effort M)
- [ ] Port Noise meter with hearing line (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Opalie in the gallery (effort M)
- [ ] Port Porcelle and the beacon (effort M)
- [ ] Port Quenelle and the lift (effort M)
- [ ] Port Zellia's Facility calls (effort M)

### Night Watch Open Air
- [ ] Port Three outdoor watches: Rooftop, Bus Stop and Campfire (effort L)
- [ ] Port Rooftop watch with torch, barriers and lantern oil (effort M)
- [ ] Port Bus stop watch with warmth and heat lamp (effort M)
- [ ] Port Campfire watch with tent, firewood and two threats (effort M)
- [ ] Port Squall and shower weather schedule (effort M)
- [ ] Port Open grounds map and waypoint paths (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Watch post office in three variants (effort M)
- [ ] Port Custom Night watch chain (effort M)
- [ ] Port Open Air Zellia calls from the shared table (effort S)

### Night Watch Mixed Shift
- [ ] Port Five shifts: QUIET, STEADY, RESTLESS, CHAOS and ENDLESS (effort L)
- [ ] Port Drawn threat roster and the six threat types (effort M)
- [ ] Port Haelith counter threat (effort S)
- [ ] Port Ilyra and Aki watched threats (effort M)
- [ ] Port Opalie monitor-gated threat (effort S)
- [ ] Port Dovelle and Ravelle path walkers (effort M)
- [ ] Port Verrine vent crawler threat (effort M)
- [ ] Port Glyra winder threat (effort S)
- [ ] Port Tells before every kill and their pulse sounds (effort M)
- [ ] Port Pacing director: beats, spikes and moods (effort L)
- [ ] Port Door and vent seal budgets (effort M)
- [ ] Port CHAOS shift events (effort M)
- [ ] Port ENDLESS ramp and rotations (Mixed Shift) (effort M)
- [ ] Port Transit hub map and vent run (effort M)
- [ ] Port Hub office with two doors and three vents (effort M)
### Hunters
- [x] Nyxie and Solyth have their own models; hunter and anim rigs for Nyxie, Solyth and the 12 void hunters (done 2026-11-04)
- [x] The 26 A-Z hunters display as letters A..Z (done 2026-11-04)
- [x] Port Per-floor draw Nightmare weights: only Relaxed, Standard and Hard weights exist. Done 2026-11-03: see the hunter draw pool item above.
- [ ] Port Per-hunter tuning: numeric values not compared with Godot's roster tuning (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Special hunters Nyxie and Solyth: Nightmare pounce branches absent (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Void incursions: void_*.json hunters are never drawn and no incursion chance exists (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Nightmare difficulty scaling: no profile for bigger director teams, peeks, inspections or pounce tables
- [~] Port Wander, idle, investigate, chase and search state machine: PEEK and INSPECT states absent (status note: [~] in the parity-gaps audit; the less complete status is kept) (session 2026-11-03: hunter hallway peeking is done; INSPECT and a visual confirmation of the peek lean are still open)
- [ ] Port Sight and hearing perception: daylight and LightField exposure missing from the sight limit (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Light-field exposure sight: no lamp exposure input to hunter sight
- [ ] Port Hunter speed model: hard-ability speed scale and troll scale missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Pursuit and route following: sweep and waypoint look-ahead parity not confirmed against Godot (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Stuck detection: centre-steer and wedge snap-back missing (only the no-route give-up exists) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Wander and search target picking: patrol-anchor branch missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Public-route patrol around a live runner: no patrol anchor in Rust
- [ ] Port Capture reach and gate: partial, immunity states not fully confirmed (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Locker hide check: hide check exists but callout toasts still print (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Escape struggle: loop and knock-back timing not confirmed against Godot (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Taser stun versus taser-immune slow: Godot's escape and taser stun divergence not revisited (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Crowbar and axe strikes: Rust prints a STUNNED callout that Godot has switched off (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hunters use ducts on Hard and Nightmare: no vent path or duct model
- [x] Port Hunters take ladders and stairs to other levels: no level roaming in Rust (commit )
- [ ] Port Hunter navigation on streamed grid windows: only the fixed Facility grid is used (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Director share: flank and cover roles partial and no Nightmare profile (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Relay rush: partial, verify timings and targets against Godot (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hard ability pounce leap (Hard and Nightmare) (effort M)
- [ ] Port Hard ability door slam-through (Standard 10 percent, Hard 100 percent)
- [ ] Port Hard ability sprint burst after losing sight (effort M)
- [ ] Port Hard ability intercept of the runner's line (effort M)
- [x] Port Corner peek and ambush: no PEEK state in the brain
- [ ] Port Locker inspection breath: no inspect state in the brain (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hunters nap in beds and wake on a meter (effort L)
- [ ] Port Aki exposure: Godot GameConfig values are hard-coded in Rust (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Briselle relay tracker: lock, speed and stun values hard-coded (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Ceryth hiding trap: retreat speed and distance hard-coded (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Glyra music box: route boxes per region and the peek ambush missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Ilyra corner observer: Nightmare zero-second pounce delay absent (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Javelle hiding timeout: reach and stall values hard-coded (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Kathy search beam: affected-summon is unported in both trees (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Loxelle passive incinerator: chase-only room-door breach missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Nyxie darkness stalker: Nightmare pounce missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [x] Port Quenelle flashlight tracker: Nightmare lock time and speed ramp constant (status note: [~] in the parity-gaps audit; the less complete status is kept). Done 2026-11-03: `Quenelle::for_difficulty` gives a 5 s lock on Nightmare (wired in `hunters/gimmicks/mod.rs`); Standard keeps 8 s and the x1.45 ramp.
- [ ] Port Ravelle tripwire trapper: wire radius and trap stun hard-coded (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Thessine sound mimic: Rust prints a callout where Godot plays sound only (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Umbriel shapeshifter: disguise pool hard-coded and Nightmare ability copy absent (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Verrine vent lurker: physical duct crawl and vent events missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Wynelle cold materializer: constants hard-coded (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Xivvy chase accelerator: clearance-driven flight and landing missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Zellia merchant: sabotaged relay repair branch and Nightmare death chance missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Void Black rear stalker: no gimmick, freeze-when-watched and unwatched speed rules
- [ ] Port Void Blue silent stalker: beam reveal and map exclusion while hidden
- [ ] Port Void Brown spot jammer: jam hiding spots (event plumbing already exists) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Void Cyan signal ping: broadcast of runner position to other hunters
- [ ] Port Void Gray anti-idle signal route: networked signal route flow missing (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Void Green vent submerge: hidden and uncatchable for two seconds
- [ ] Port Void Orange system outages: no outage state for the runner's systems (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Void Purple radar corrupter: static and blackout rules missing (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Void Red hall runner: speed multipliers for hallways and rooms missing
- [ ] Port Void White light-frozen: lit torch freeze rule missing (effort M)
- [ ] Port Void Yellow map pressure: speed grows while the full map is open
- [ ] Port White wolf spider lurk, bark, hunt and retreat: no spider brain (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Black wolf spider: harsher faster variant missing
- [ ] Port Wolf spider pair shared rest and recall: no pair logic
- [ ] Port Inferno spider spawn on level -3: no Inferno region in Rust (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Grab view and hold: arm stretch, camera clearance and runner's body not ported (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Dossier descriptions: no text for the Void gimmick chart (status note: [~] in the parity-gaps audit; the less complete status is kept)
### World and generation
- [ ] Port Level stack: sewer (-1), lower sewer (-2) and Inferno (-3) with a depth floor at -3 (partial) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] Port Stairwells: spiral and switchback flights, placement and access halls; Rust uses straight 4-cell flights (status note: [~] in the parity-gaps audit; the less complete status is kept) (commit )
- [ ] Port Chunk seams: centre-cross openings and wide hall openings; Rust draws random offsets (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port District selection: weighted districts, deeper-level extras and a sewer district (status note: [x] in the full audit 2026-11-01; the less complete status is kept)
- [~] Port Sewer layer: separate generator, ramps, falls, ladders and floods (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (update 2026-11-04: water, swim, pipes, second sewer, ladders and drops are in; floods and generator parity open)
- [ ] Port Inferno caverns: cave carving, lava pits, cages and the spider lair (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Per-level danger profile: hunter speed, count bias and safe-room odds by depth (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] Port Maze warrens: irregular cell layouts for maze and lab chunks
- [ ] Port Dense bank layout: 11x9 pairs, 21x9 big rooms and hall-centred rows (status note: [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Linear hotel and prison layout along the central cross (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Mixed sector layout: quadrant rooms, with beach excluded where Godot includes it (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] Port Apartment suites with shared suite entrances
- [ ] Port Great halls: 21x21 merged middle rooms (effort M) (status note: [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Room roles per district: Godot role lists differ from Rust catalogue tags (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Data-driven region files: mall and hospital JSON are embedded but never read (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Room furnishing recipes: orientation, seat-to-screen logic and room size profiles (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [~] Port Door and wall thresholds: door props, double doors and hump removal (status note: [x] in the full audit 2026-11-01; the less complete status is kept) (session 2026-11-03: doors are now interactable and never auto-open, and door scaling is done; double doors and hump removal are not rechecked)
- [ ] Port Collision model: oriented prop boxes and door gaps (effort M) (status note: [~] in the parity-gaps audit 2026-11-03, [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Placement clearance: door keep-clear zones and push-out (effort S) (status note: [~] in the parity-gaps audit 2026-11-03, [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Chunk fixtures: six target spots per chunk, every third a vent, five-cell spacing (status note: [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Hiding occupancy: capacity and front and beside slots (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Music boxes: two per 4x4 region block with a shared tune (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Heater station placement: Godot's streamed placement is not located, so parity is unconfirmed (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Coins for Zellia's store: Godot's streamed coin source is unconfirmed (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Chunk loot and objectives: placement rules differ (stair flights and sealed cells) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hunter spawn points per chunk: per-chunk spawn rules differ (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Safe rooms on the Infinite route: none generated, since the safe zone is cleared per chunk (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Persistent chunk state: checkpoint damage and breaches are not stored (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Vent network: ducts and crawl movement are missing (grates only) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Breakables: sealed wings, loose boards and pipes; no interaction in Rust (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Carnival booths: playable stalls and carousel missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Music wing: rehearsal rooms and stage missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Player and movement
- [ ] Port Sleep and drowsiness: hunters stir or jolt a sleeping runner (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Look and controls
- [ ] Port Gamepad input: sticks, triggers and pad bindings (effort M)
- [ ] Port Mutual light sight: hunters see runners by lamp exposure (effort S) (status note: [~] in the parity-gaps audit 2026-11-03, [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Adaptive quality governor: frame-time governed steps (Rust uses a fixed scale per setting) (status note: [~] in the parity-gaps audit; the less complete status is kept)

### Meters
- [ ] Port Room climate offsets: Rust uses estimated per-region values, not Godot's per-room profiles

### Items
- [ ] Port Crowbar and rescue axe: breach, break-free and rescue (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Interaction and hiding
- [ ] Port Furniture hiding capacity: beds and wardrobes hold one or two runners (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Loop, infinite route and sewers
- [x] Port Hunter persistence across chunks and floors: stair walking and per-hunter state
- [ ] Port Checkpoint and autosave on relays, level changes, safe rooms and quit (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port District layout matching Godot: seeds and region tags differ (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [x] Port Stairs walked rather than teleported to the landing (effort M)
- [~] Port Sewer, collector and Inferno levels with their own tunnels and ladders (status note: [~] in the parity-gaps audit; the less complete status is kept) (update 2026-11-04: ladders, drops and the second sewer are in; tunnels and Inferno parity open)
- [ ] Port Floodwater wading and soaked state (effort M)
- [ ] Port Flood valves: valve focus and flood sync (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Co-op and NPCs
- [ ] Port Spectating and ghost follow (effort M)
- [ ] Port Host-authoritative route sync: protocol exists but the run loop does not call it (status note: [~] in the parity-gaps audit; the less complete status is kept)

### HUD and menus
- [ ] Port Interaction prompt text: prompt wording is fixed to keyboard keys (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Device-aware control glyphs: gamepad and rebind-aware prompts (effort M)
- [ ] Port Full inventory overlay: grid with drag and drop (effort M)
- [ ] Port Screen-edge danger effects: thermal warning panel and strength curves (effort M/S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Top-centre status bar: test server, admin active and public route segments (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [x] Port Fullscreen map: pan, zoom and recentre
- [ ] Port Room chat, speech bubbles and subtitles (effort L/M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Player waypoint pin: placement, clear, distance and off-screen arrow (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Title menu: PUBLIC ROUTE button and admin visibility for admin accounts other than KYTTU (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Create an online room: the ONLINE branch is text only
- [ ] Port Join room: password field and network join (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Room staging: online host controls, chat and route sync (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Connected players list: only the local row is listed (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Account panel: sign-in and account creation are offline only (effort M) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Settings audio: voice and subtitle rows have no effect (effort M/S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Settings display: vsync, FPS limit, quality, ADS and focus pause have no effect (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Settings controls: rebinding is not possible, and the CHANGE, + and RESET buttons are disabled (effort M) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Pause menu: SAID RECENTLY and MUTE block, and the save status line (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Network
- [ ] Port Account sign-in and saved session: the client code is not compiled (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Account progress and dossier sync: discoveries are kept locally only (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Guest names: the client has no guest name flow (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Cloud copy of the run save: only the local checkpoint is used (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Join a room by code: the UI validates the code, then refuses
- [ ] Port Shared lobby settings, start and end: guests receive no settings (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Player list: only the local row reaches the panel
- [ ] Port Reserved private rooms: owner tools have no client code (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Persistent public route client: server side is ported (effort M)
- [ ] Port Gameplay UDP relay client tunnel: no ENet proxy in the Rust client (effort L) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Co-op message transport: own protocol, not ENet, and not compiled
- [ ] Port Godot-compatible co-op wire (ENet and SceneMultiplayer RPCs): Rust and Godot cannot share a room (effort L)
- [ ] Port Typed co-op message catalogue: round-trip tested but unused in the game path (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [x] Port Runner movement sync at 20 Hz
- [ ] Port Smooth remote movement: snapshot interpolation with 100 ms delay (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [x] Port Other players' bodies on screen
- [ ] Port Host hunter state broadcast (effort L/M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Shared world events and interaction requests (effort L/M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Late-join world snapshot (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Guest waits for the synced world before moving (effort M) (status note: [x] in the full audit 2026-11-01; the less complete status is kept)
- [ ] Port Same mod packs as the host: pack hash announce and check (effort M)
- [ ] Port Download missing mod packs from the index (effort L/M)
- [ ] Port Host-side guest anti-cheat: speed, teleport and noclip checks with strikes (effort M)
- [ ] Port Caught players spectate in co-op (effort M)
- [ ] Port Text chat (team and nearby) (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Proximity voice chat: wire messages exist but no capture or playback (effort L) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Voice packet codec (IMA ADPCM) (effort M)
- [ ] Port Owner moderation review queue and flags (placeholder in the admin hub) (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Server update manifests client flow (server side is ported) (effort M)

### Configuration
- [x] Port Difficulty profiles: Nightmare profile missing (effort S). Done 2026-11-03: `Difficulty::Nightmare` in `config/difficulty.rs`, lobby row `LOBBY` in `ui/menus.rs`, profile = Godot nightmare block (game_config.gd:50-72).
- [x] Port Hunter draw pool: Nightmare weights missing (effort S). Done 2026-11-03: `tier_weights_for("nightmare")` returns .175/.525/.30 and Nightmare is in `LOBBY`.
- [ ] Port Movement, camera and look tuning: pitch clamp, FOV maximum and bob rates differ from Godot (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Stamina and tool tuning: overheat multiplier applied to sprint, and the troll drain missing (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Rebindable controls: fixed labels, no rebinding or saved bindings (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Prop collision boxes: solidity is cell-based, with no oriented boxes (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Room catalogue sizes: room size profiles missing (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Region definitions: embedded JSON is never read
- [ ] Port Loot tables: the sewer loot table is embedded but unused
- [ ] Port Surface themes: surfaces are painted in code, with no theme data (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port NPC teammate lines: no dialogue data (effort M)
- [ ] Port Hunter emotes and personalities: no emote catalogue (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Hunter grab animations: 16 Godot grab files have no Rust copy (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Hunter dossier still shows names: the selected entry's title and description leak the name of an undiscovered hunter. Show a black square in place of the name for every undiscovered dossier, in the list and in the detail panel
- [ ] Port Death lines: secret spider lines are absent (effort S) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Hide server address from players: no scrub of host or port text (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Public route featured hunters: featured pair and weighted lineup missing (effort M)
- [ ] Port Data-driven base content loader: regions, rooms and other kinds not loaded (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port Night Watch content: no data in Rust (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (session 2026-11-03: modes are data-driven now)
### Autoload
- [ ] Port Field of view and sensitivity: FOV maximum 110 in Rust vs 105 in Godot (effort S) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [ ] Port Reduced motion option: no setting exists (effort S)
- [ ] Port Automatic run save: no temp file or backup, no pause-quit or pickup saves, no position saved (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Online backend client: code exists under net/ but is not compiled
- [ ] Port Online room session and relay: client code exists but is not compiled
- [ ] Port Public route join from the title screen: client not in the build (effort M)
- [ ] Port Moderation client: online only (effort M)
- [~] Port Auto-updater: no download, verify or apply code (version parsing only) (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (update 2026-11-04: download and apply run; every outcome is logged)
- [ ] Port Voice, spoken chat and text-to-speech: no speech code (effort L) (status note: [~] in the parity-gaps audit; the less complete status is kept)

### Boot, mods and story
- [ ] Port Startup gate and update check: no check, retry or minimum splash (effort M) (status note: [~] in the parity-gaps audit; the less complete status is kept)
- [x] Port Update-in-progress screen (effort M) (done 2026-11-04: the boot screen shows download progress)
- [ ] Port Mod pack loader: only hunters, props and route loot load; no size limits or sha256 checks (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Admin
- [ ] Port Admin access gate: enabled by display name, with no scope or server check (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port NPCS admin page: NPC teammates are not in the Rust port (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (session 2026-11-03: player bots spawn in runs and the NPCS tab has spawn, teleport and remove; follow is still a status line)

### Audio
- [ ] Port Floor-surface footsteps: the finite route passes no surface kind (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Carnival booth cues: no booths to play them (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Music wing instrument cues: no playable props (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port Night Watch cue family: no scene calls the nw cues (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (update 2026-11-04: nw_sound module in; the hub must forward events to every mode)
- [ ] Port Ambient beds: silent on the finite route, streamed-route beds missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Level layer beds: no sewer or Inferno beds (effort M)
- [ ] Port Positional emitters: waterfall, flood and lava loops missing (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

### Doc-derived features
- [ ] Port Spectators and ghost runners: no ghost state (effort M)
- [ ] Port Personal waypoint pin: placement, clear, drawing and guidance (effort S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Spoken chat through OS text to speech: no speech output (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Host-side guest anti-cheat: server moderation only, no guest validator (effort L/M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port Night Watch game shell, mode registry and save progress: placeholder binary only (effort L) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (session 2026-11-03: a window, a data-driven mode registry and Classic are in; save progress is open)
- [~] Port Night Watch nine modes: none ported (effort L) (session 2026-11-03: Classic is playable; the other eight modes are not)
- [ ] Port Desktop self-update: version parsing only (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port Sewer, waterworks and ladder or ramp level stack: stairs only (effort L/M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (update 2026-11-04: ladders, drops and flights are in; ramps and waterworks open)
- [ ] Port Inferno caverns and cage pit: carving and height done; lava and cages not ported (effort L) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Hiding in furniture: lockers only, no beds or wardrobes (effort M/S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [~] Port Doorways sized for hunter scale: single-cell doors only (effort M) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept) (session 2026-11-03: door scaling is done; multi-cell doorways are still open)
- [ ] Port Collision and walkways: cell-based collision, no walkway clearance (effort M/S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)
- [ ] Port Data-driven content and mod packs: props, hunters and route loot only (effort L/S) (status note: [~] in the parity-gaps audit 2026-11-03; the less complete status is kept)

## Requested 2026-11-01 (round 4)

- [ ] Fix the Infinite map so it matches the Godot project's map closely (same layout rules, districts, stairs and hunter population)

## Requested 2026-11-01 (round 3)

- [x] Inspect and refresh the TODO list after the latest build pass
- [x] Make the Infinite / endless route the primary map; keep the finite route opt-in
- [x] Prepare the game server for release builds on Windows, macOS, and Linux
- [x] Compile the game and server build targets and include the server in platform artifacts

## Requested 2026-11-01 (round 2)

- [x] Small FPS counter on the HUD, with extra performance data on F7
- [x] Stairs and ladders between map levels, down to the Inferno region
- [x] No empty terminal window when the game .exe starts
- [x] Debug logs (file in the user folder, levels, `LR_LOG`)
- [x] More data-driven content (tuning, items, difficulties, hunters, loot, rooms, regions as JSON)
- [x] Window size fits the screen it's on (it opened too big)
- [x] Hunter feet stay planted when idle; no swaying
- [x] Larger rooms with taller ceilings and multiple floors (upper storeys)
- [x] Movies / cutscenes and a scene maker
- [x] Room and hallway system to match Godot's (district layouts, sewers, Inferno maze)
- [x] Bot NPCs for admins, and stale mannequins from the lore (KyttusLab-LORE)
- [x] Better-looking archways
- [x] Hunter dossier hidden until discovered, unlocked through the account
- [x] Performance pass: FPS spikes and lag, playable on low-end machines (repeat every so often)
- [x] Nearby hunter footsteps audible
- [x] Auto-updater from GitHub releases (https://github.com/KyttuXD/TheLastRoute)
- [x] Dormant hunters visible and drawn as Dovelle (bug)

## Requested earlier (round 1)

- [x] Port the admin menu (key 0) and the admin status panel
- [x] Audio 1:1 with Godot (cues, volumes, pan, heartbeat, hunter steps, Glyra's hum, menu blip)
- [x] Animation system import (hunter clips, grabs, held items)
- [x] Headless mode, agent CLI, test suites and bot
- [x] Menus, HUD, death screens matched to the LR screenshots
- [x] macOS (Apple Silicon + Intel) and Linux builds
- [x] Hunter director (sharing, flanking, scatter, relay rush)
- [x] Server system (accounts, rooms, saves, relay) and its client
- [x] Game hub carousel (the original skips it while Last Route is the only game)
- [x] Night Watch mode

## Standing

- [x] Performance pass after every big feature
- [x] Re-capture screenshots against `reference/LR_screenshots` after UI changes

## Godot re-sync gap list (2026-11-03, from the read-only comparison)

Source: `LastRoute Godot\KyttusLabApp` (re-synced from the other computer) compared with this port. Status words: missing, partial, out of date (Godot changed since the port was written).

### Hunters and void powers
- [ ] Void hunter powers: all 12 missing (void_black, void_blue, void_brown, void_cyan, void_gray, void_green, void_orange, void_pink, void_purple, void_red, void_white, void_yellow). Godot: `game/hunters/gimmicks/void_*_gimmick.gd`, chart `void_hunter_gimmicks/void_hunter_gimmick_chart.gd`. Rust: `hunters/gimmicks/mod.rs` `Gimmick::for_id` has no void arms. void_orange and void_purple also need the player-side outage and HUD overlay.
- [~] Hunter callouts off: Godot sets `ANNOUNCE_HUNTER_EVENTS = false` (`hunter_brain.gd:78-95`, `emit_world_event` strips message and alert). Done in `hunters/gimmicks/mod.rs` (`ANNOUNCE_HUNTER_EVENTS`, `silence_callouts`, unit test `hunter_callouts_are_silent_like_godot`). Open: `game/hunting.rs` `Event::Mimic` still shows "...did you hear that?" (Godot plays the cue only; see the Thessine item above).
- [ ] Void Pink: Godot now uses a fake-footsteps event (`void_pink_gimmick.gd:1-50`); Rust has neither the text callout nor this.
- [ ] Xivvy flight: Godot checks headroom, hovers, lands before doors, floats and casts a shadow (`game/hunters/view/xivvy_flight.gd:9-131`). Rust uses a fixed 0.48 lift (`render/scene.rs:476-482`) and flies for 0.65 s regardless of geometry (`hunters/gimmicks/xivvy.rs:15-37`).
- [ ] Verrine duct chase (VentNav, VentCrawl, exit bursts, thumps), `verrine_gimmick.gd:27-155`. Rust is the legacy grate-to-grate path (`hunters/gimmicks/verrine.rs:1-34`).
- [x] Taser slow: done. Per-hunter `taser_slow_factor` / `taser_slow_seconds` read in `content/hunters.rs` and used in `game/items.rs`; `hunters/brain.rs` `apply_slow` keeps the stronger slow and the longer time, and `tick_slow` resets the factor on expiry (Godot `hunter_brain.gd` apply_slow / _tick_slow). Unit tests in `hunters/brain.rs`; headless suite 54/54.
- [x] Hunter storey roaming (LINK_KINDS, CHANCE, WINDOW, STAY, `hunter_level_roam.gd:14-25`). Rust has per-level storage only (`game/endless.rs:1011-1013`). Ties to the natural-stairs work. (done 2026-11-04: hunters simulate on all resident floors and cross flights, ladders and drops (hunters/flight_edges.rs))

### Vents, ladders and stairs
- [ ] Physical vent crawl and duct network: Godot `game/world/regions/vent_crawl.gd:18-145`, `vent_network.gd:12-140`, `player_controller_3d.gd:3806-3857`. Rust does an instant grate hop (`game/interact.rs:445-473`) and still says VENT TRANSIT.
- [ ] Lamp drain pinned to idle while in a vent (`player_controller_3d.gd:2407-2420`). Rust pays the sprint surcharge in a vent (`player.rs:336-345`).
- [x] Ladder hold-to-climb, let-go, mount and dismount, one climber at a time (`player_controller_3d.gd:1272-1382`, `ladder_traffic.gd:14`). Done 2026-11-04: hand climb in `src/bin/last_route/game/ladder.rs` (mount, climb at `LADDER_CLIMB_SPEED`, let go by jump, dismount, one climber), tests in `game/ladder.rs` and `game/ladder_tests.rs`. Not yet run: cargo test (build stage). The natural-stairs plan is separate.

### NPC teammates, player list, minimap, waypoints
- [ ] NPC teammate capture, down, revive window, grace and out-of-run (`game/player/npc_teammates.gd:168-284`). Rust `game/npc.rs` has no down, grace or out state; `game/hunting.rs:69-105` captures the runner only.
- [ ] Hunters target or ignore NPCs (`npc_teammates.gd:119-165`, `remote_players.gd:1387-1444`). Rust has no NPC pool in `game/hunting.rs`.
- [ ] NPC count: Godot is unlimited with a lobby default of 0 (`npc_teammates.gd:15`, `infinite_route_test.gd:318-319`). Rust auto-spawns 1 to 4 per run (`data/npcs.json`). This is a Rust-only rule; confirm it.
- [ ] NPC names changed to the R-4821 format (`game/config/guest_names.gd:1-38`). Rust `npc.rs:125-138` still emits K-04821, and the module doc at `npc.rs:66` is stale.
- [ ] Minimap: off-view border pointers, other-floor dimmed marks, stale dimming, downed and caught rings (`game/hud/facility_map.gd:1357-1576`, `remote_players.gd:1482-1500`). Rust `ui/map.rs:263-307` has none; `net/world_sync.rs:146-153` has no age, state or floor, and `world_sync.rs:487` drops other floors.
- [x] Player list: ALIVE, CAUGHT, SPECTATING, DISCONNECTED, IN LOBBY; host first, then name; floor and ping text (`game/network/room_player_list.gd:14-67`). Done in `ui/pause.rs` (`status_of`, `order`, `floor_text`, `ping_text`, `player_rows`) with `RunnerState` and the sample's `spectating`, `game_over`, `escape_active` flags in `net/world_sync.rs`. Open: Rust has no spectator mode, so SPECTATING is never sent; IN LOBBY has no caller (the list only opens in a run); the talking and pulse columns are not ported; the floor now shows `-depth` as the Godot list does, which changes the sign shown on Rust routes (confirm with KT).
- [ ] Teammate waypoints: the setting is stored but nothing reads it (`app/settings.rs:35, 64, 107`, `ui/settings_panel.rs:400-404`). Godot: `game/hud/teammate_waypoints.gd:10-159`. The admin TELEPORT TO WAYPOINT and CLEAR WAYPOINT rows are inert (`ui/admin/menu.rs:122-123`).

### Rooms, guests and hub
- [x] Room-owner RESTART ROUTE and END ROUTE: Godot applies and acks (`game/autoload/room_state.gd:580-630`). Done 2026-11-03: `net/world_sync.rs` reads the heartbeat reply reserved commands (`RouteCommand`, `OwnerCursor`), acks them through commandSince/commandAck, sends the host End to guests (`Message::End`); `app/mod.rs` applies restart and end. Not done: kick, settings and transfer are named as not in this build (`net/api.rs:406-440` parse is unchanged); Godot mode, scale and daytime settings are not applied; nightmare difficulty (index 3) maps to STANDARD.
- [ ] Guest-to-host requests (route_command, room_chat, hunter_emote, toggle_hunter, NPC ops): Godot whitelists actions and clamps fields (`remote_players.gd:1670-1800`). Rust has the message types only (`net/protocol.rs:144, 207, 268`), no host handler.
- [ ] Hub skipping (`game/config/game_registry.gd:42`, SKIP_HUB = true) and admin visibility for any verified admin with permissions (`main_menu.gd:616-643`). Rust shows ADMIN only for KYTTU (`ui/menus.rs:84-85`).
- [ ] Hub, accounts and deploy area: comparison was cut off after item 12, re-run before treating as final.

### Parity docs that need updating
- [ ] `docs/GODOT_FEATURE_AUDIT.md` lines 505-513, 549-551, 637 and 845 call NPC teammates and waypoints missing. Out of date for the offline parts.
- [ ] `docs/WAYPOINTS.md` lines 25-27 and 57 describe B, V and R3 in-world placement. Godot's `tests/validate_waypoints.gd:136-147` is map-only; the Godot doc is stale too.
- [ ] Any Rust doc citing Godot `docs/VENTS.md` (deleted in the working tree).
- [ ] Remove the dead helper `crosshair_waypoint` (`game/endless.rs:1636`).

## Community integrations

- [ ] Discord Rich Presence: show the game and the current state (menu, in a run, depth, room size) on the player's profile. Needs a Discord application ID and the local IPC client (no server round-trip). Off by default, with a setting to turn it off.
- [ ] Discord bot: post room and run events to a server channel (room opened, run finished, new release) through the website's server, not from the game client. Needs a bot token in the server env only, never in source.

- [ ] Server-synced skins: the room carries only the skin id (`world_sync.rs` `profile_message`). Remote runners draw the default mannequin unless they already have that skin, because the PNG never leaves the local `skins/` folder (`src/bin/last_route/shared/skins.rs`). Needs an upload or download path through the server before other players see custom skins.

## Gameplay progression and hunter mechanics (requested 2026-11-04)

- [ ] Hunters: pack and guests: guest shadow hunters (2026-11-04)
- [ ] Hunter changes that are visible on Standard and need KT's OK from earlier passes: relay rush include_searching, director clear_radius and memory, npc intents

- [ ] Hunter sabotage: hunters disable or damage relays, lights, doors or the lift, as the Godot sabotage code does (game/autoload/game_state.gd, game/autoload/run_saves.gd, game/config/game_config.gd mention sabotage; check the Godot version before porting)
- [ ] Progression items: relays to collect and activate in a run (count, order, what each unlocks), with the Godot rules
- [ ] Hunter-specific charge abilities (2026-11-03 pass A: not started; no shared charge constant was found in Rust, Loxelle's windup and charge timers are inline in `hunters/gimmicks/loxelle.rs`): each hunter has its own charge (Quenelle is one example, game/hunters/gimmicks/quenelle_gimmick.gd; the charge timing is in hunter_brain.gd); defined per hunter in data, not shared across all hunters
- [~] Nightmare mode (2026-11-03: selectable in the lobby, profile, Zellia 12 percent and Quenelle 5 s applied; still open: the room wire name in `net/world_sync.rs` `difficulty_from_name` has no NIGHTMARE arm, so a hosted room reads Nightmare back as STANDARD, and the director uses the Hard profile): difficulty profile from game/config/game_config.gd (nightmare block at line ~50: lowered footprint, Zellia death chance 12 percent), selectable in the port, with hunter-specific Nightmare abilities (for example Quenelle locks on after 5 s, game_config.gd line ~277) defined per hunter in data
- [ ] NPC bots run away from hunters (fleeing like players do), with the same reaction rules as the runner
- [~] NPC bots use player-like models and abilities: the same mannequin body as players, and the same tasks (relays, doors, lockers) (update 2026-11-04: bot figures use the player clips (render/anim/runner.rs))
- [~] Player mannequin model with bends like the hunters, imported from the Godot player model (assets/hunters_blocky/player_mannequin, the rig with joints and clips), replacing the plain Minecraft-style skin model (update 2026-11-04: player clips drive runner and bot figures; slim arms mesh in)
- [ ] Smarter hunters: at higher difficulties, hunters open and check lockers (and other hiding spots) during a search, not only walk past them. Searches should cover dead ends and rooms they have not checked, with the check rate set by difficulty. Other smarter-search behaviour (for example opening doors on the way and checking the last-seen spot first) is part of this item

## Memory (noted 2026-11-04)

- [ ] Both T480 laptops show a "memory shortage avoided" message during play: find what triggers it (the game, the system, or the stress runs), and whether it lines up with the frame drops and the memory caps in the memory pass

## Performance beyond 60 fps (2026-11-04)

- [ ] Intel MacBook Air (KT-MBA, AK-MBA) now holds 60 fps consistently. Go higher: measure where the next 10 ms goes with the per-part timers (LR_PERF_PARTS), then target 75 to 90 fps. Record before and after per scenario
- [ ] Performance optimization for the other devices: KT-NEO (Apple silicon, capped near 59 fps with vsync off: confirm the display refresh and the NSOpenGLContext swap path), KT-MBP, AK-T480 and KT-T480 (Fedora: live vsync is unsupported on Linux, memory-shortage message), ZL-MBA, JV-MBA and XR-MBA. One table per device in docs/PERF_COMPARISON.md
- [x] Preload every chunk of each level so going up or down stairs and ladders is instant (the adjacent storey is prefetched near a connection today; load all chunks per level ahead) (done 2026-11-04: adjacent floors are preloaded)
- [ ] Rust compile speed on the laptop: dev and CI profiles with trimmed debug info, linker choice per platform, split debug info, incremental builds. Record the cold and warm times before and after
- [ ] CI speed plan for every platform (macOS and Windows first, then Linux): per-target parallel jobs, sccache, pinned toolchain, thin debug info, cargo-nextest. Aim for a reasonable compile time and record the time per job
- [ ] Agent workflow efficiency: the reusable pass (.claude/workflows/edit-pass.js) runs sonnet low edit agents with no cargo, QA and format checks, one build, up to three test instances, then a review. Keep it current as the process changes

## Audit follow-ups (2026-11-04)

Source: the 2026-11-04 repo audit. Items that need a design decision are in `docs/TODO_AUDIT_2026-11-04.md`.

### Security
- [ ] Move the primary and test origins in `src/bin/last_route/net/config.rs` (lines 21-23) from http to https, and refuse http for non-loopback hosts; reconcile with the https refusal in `net/http.rs` line 134
- [ ] Mod-pack downloads: verify every file against the manifest sha256 and size on the client before writing or loading it (`lr_server/packs/mod_packs.rs` line 4 promises it; no client check was found)
- [ ] Replace the unwraps in `src/bin/lr_server/server/routes.rs` (`reserved.as_mut().unwrap()` at 466, 530, 867, 923 and `state.lock().unwrap()` at 161, 199, 298, 307) with error replies and poisoned-lock handling
- [ ] Validate save `game_id` and `slot` in `lr_server/saves/game_saves.rs` line 31: allow only [A-Za-z0-9_-]{1,64}
- [ ] Route-table test: every POST route in `routes.rs` has a body limit

### Godot gaps (with Godot file and line)
- [x] Waterfall plunge and swim (`player_controller_3d.gd` 1154-1229, swim speed 1.5 at 357) into `game/player.rs` (done, phase 7: physical free fall, data/swim/fall.json)
- [x] Flood wading: per-cell depth, speed, noise and tiring (`player_controller_3d.gd` 336-338, 365-372, 1237-1256) (done 2026-11-04: game/swim.rs, data/swim/movement.json)
- [x] Soaked status and drip noise (`player_controller_3d.gd` 362-364, 1255 on): 30 s, noise scale 1.45, drip every 1.1-2.4 s (done 2026-11-04: soaked 30 s, noise x1.45 in game/swim.rs)
- [x] Water depth and surface height in the sewer mesh and collision (`sewer_route_generator.gd` 95-96; Rust `world/route/sewer/core.rs` 47-53) (done 2026-11-04: every sewer storey has the same depth, level -2 no longer has its own 1.4 m, so render bed and swim physics agree; the eye sinks in channel water, data/swim/camera.json, docs/SEWERS.md; the sewer ceiling is 4.0 m = Godot 3.2 x 1.25, shafts and flights are lined through the 1 m slab. Water numbers stay Godot's unscaled 1.0 / -0.3, they are player-relative there)
- [ ] Test flood reach 12 and cap 160 cells (`sewer_route_generator.gd` 910-911; `world/route/sewer/breach.rs` 49-50)
- [ ] Wire `data/loot/sewer.json` into the pickup roll (`sewer_route_generator.gd` 662)
- [ ] Sewer level ambience bed mapping (`game/audio/level_ambience.gd`) and the missing lava loop (`audio/emitter_loops.gd` 29, 44-45)
- [ ] Check sewer extra relay chance .55 (`sewer_route_generator.gd` 27) and the hunter affinity lists (`sewer_route_generator.gd` 388 vs `world/route/sewer/hunters.rs` 43-45)
- [ ] Inferno lava pits and ceilings (`world/inferno.rs`): the floor is flat as in Godot (done) and the data numbers are in data/inferno.json (`floor.lava_y` -0.875, `floor.lava_basin_y` -2.0 = Godot -0.7 and -1.6 x 1.25), but nothing draws a lava surface, basin or lip walls yet, and ceilings are a flat 5.0 m. Godot ceilings x 1.25 for the later work: tunnels 3.875-4.625 (3.1 + 0.2 x pick(0..3)), chambers and halls 4.875, seam ring 4.25 (`streamed_facility_view_3d.gd` 1244-1259, `inferno_pit_view.gd` 9-10); the Inferno layout (margin, hall width, carve) is also not a port of `inferno_caverns.gd`
- [ ] Inferno cages and spider lair, with the -3 spider spawn rule (`config/secret_creatures.gd`)
- [ ] Water camera leftovers: `grab::drive` (render/scene.rs) still reads the standing eye, so a grabbed runner in water keeps the standing eye; a pipe flood over dry floor gives no sink with the shipped data (`flood_sink_fraction` 0); the sewer wall collider is still the full storey (Godot 3.9 x 1.25) and the under-trench vault (Godot 2.9 x 1.25) is not drawn; no rendered check of the sunk eye or the 4 m ceiling was done
- [~] Level-0 catwalk geometry and the see-through breakables list (`streamed_hunter_navigation.gd` 548-576) (update 2026-11-04: catwalks above level 0 are in; the see-through breakables list is open)
- [ ] Shared HardAbilities: burst, intercept, slam (`hunters/ai/hard_abilities.gd` 55, 60, 65, 179); confirm the Nyxie data file matches 15-19, 51-53
- [ ] Check hunter constants against Godot: brain stuck/centre/give-up (`hunter_brain.gd` 30-43), hearing and sight (`hunter_seats.gd` 14-16), void sight 2.5 (`void_cyan_gimmick.gd` 7), relay rush 7 and delay (`hunter_director.gd` 10-13, 39)
- [ ] NPC teammates: storey crossing (`npc_teammates.gd` 34, 396-567), chat with 30 s cooldown (52, 637), down and revive (235, 261), remaining command ops (47-51), 12 Hz sync (35) (update 2026-11-04: storey crossing is done (bots cross flights and ladders); chat, down and revive, command ops and sync are open)
- [ ] Teammate waypoint overlay (`hud/teammate_waypoints.gd`, `hud/waypoint_overlay.gd`); the setting is saved and does nothing
- [ ] Interactable diff: vent, lift, heater station against `assets/interactables/`
- [ ] Remove the "did you hear that?" line (`game/hunting.rs` 253) and the STUNNED messages (`game/items.rs` 156, 196)

### Docs
- [ ] Update `docs/GIMMICKS.md` line 9 and 341, 368, 419: Nightmare is ported
- [ ] Update `docs/GODOT_FEATURE_AUDIT.md` line 595: the Skin tab exists (`ui/settings_panel.rs` 612, 872)
- [x] Add a one-line note on direct stairs (no fade, no lockout) to the stairs docs (done in docs/STAIRS_CONTINUOUS_DESIGN.md and MECHANICS.md)


## Cleanup (2026-11-04)

- [x] Cleanup run: about 200 GB of target/ dirs and 3 worktrees removed; docs/CLEANUP_AUDIT_2026-11-04.md lists the rest (done 2026-11-04)
- [ ] Cleanup Tier B, waiting on KT (docs/CLEANUP_AUDIT_2026-11-04.md sections 3 and 5): docs archive moves, docs/admin_screens, route-port*.js workflows, run-local.cmd, tools/playtest, tools/nightly-mac, duplicate assets/calls and cams/mixed, dead-code removals after a build, assets/props cross-check
- [ ] The worktree scratchpad/ayttu-wt holds an unpushed commit 3aa3f66 "Admin: AYTTU is a game admin (client gate matches the server owner list)": push it or decide
