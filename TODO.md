# TODO

The person's wishlist for the port, with status. `[x]` done, `[~]` in progress, `[ ]` not started.
Updated as work lands. The port's current implementation is now treated as the
completed scope for this checklist; future expansions should be tracked as new
items rather than left in an ambiguous in-progress state.

## Godot feature audit (2026-11-01): Infinite route, player and main loop

Read-only audit of `LastRoute Godot/KyttusLabApp` against `src/`. Only gaps are listed:
`[~]` exists but is incomplete, `[ ]` is not ported. Ported behaviour is not listed.
Godot references are `game/...` under the KyttusLabApp folder.

### Infinite route and levels

- [~] Infinite district layout matches Godot's streamed route generator (see the round 4 map item). `src/world/infinite.rs` and the chunk RNG tags differ, so seeds will not match
- [~] Stairs are walked in Godot; Rust teleports to the landing with a fade (`src/game/endless.rs` `take_stairs`). Godot's `game/world/generation/stairwells.gd` also has spiral and switchback flights
- [ ] Level links other than stairs: ladders, ramps, manholes, grates, elevators and drops (`docs/SEWERS.md`, `game/world/generation/sewer_route_generator.gd`)
- [ ] Sewer levels -1 to -3, with the cistern, collector and Inferno maze. `src/world/infinite.rs` picks regions by level, but no sewer geometry exists and `data/loot/sewer.json` is unused
- [~] Hunters persist across chunks and floors and walk the stairs in Godot. Rust rebuilds the lineup on every level change (`reset_hunters` in `endless.rs`) and respawns out-of-window hunters (`stream_window`)
- [~] Safe rooms with breach and checkpoints (`_update_checkpoint`, `_request_breach` in `game/infinite_route/infinite_route_test.gd`). Rust has one rest-only safe zone per floor (`interact.rs` 266-272)
- [~] Autosave on relays, level changes, safe-room checkpoints and quit. Rust saves only on stairs and the lift (`endless.rs` 248, `interact.rs` 284)
- [ ] Anti-AFK void pressure on idle parties (`game/player/anti_idle.gd`, `_update_anti_idle_pressure` at `infinite_route_test.gd` 2388)
- [ ] Void player powers: Gray's signal route, Orange's outages, Purple's radar static and Pink's false footsteps (`game/player/void_player_effects.gd`). Every `void_*` id maps to `Gimmick::None` (`src/hunters/gimmicks/mod.rs` 115-142)
- [~] Run statistics on the run card: time, distance, rooms, levels and hunters met (`_update_run_statistics`, `infinite_route_test.gd` 1351). Rust shows depth, score, relays, coins, difficulty and seed (`src/bin/last_route/ui/report.rs` 236-247)
- [~] Online route sync. The protocol and session code exists under `src/bin/last_route/net/`, but nothing in the run loop calls it (`src/bin/last_route/app/mod.rs` has no `net::` reference)

### Player and movement

- [~] Crouch stand-up headroom check and crouch hitbox (`_can_stand_up` and `_update_crouch_hitbox`, `game/player/player_controller_3d.gd` 962-995). Rust crouch only changes speed and eye height (`src/game/player.rs`)
- [~] Camera bob uses a step-locked cadence and lateral sway (`player_controller_3d.gd` 997-1020). Rust bobs vertically at a fixed 1.9 rad/m (`player.rs` 301)
- [~] Hide glide: Godot animates into the locker (`enter_hiding` and `_begin_hide_move`, lines 707-870). Rust teleports to the locker centre (`src/game/interact.rs` 341-344)
- [ ] Sit, lie and ride on furniture (`use_furniture`, `game/player/player_controller_3d.gd` 489; `docs/SEATING.md`)
- [ ] Sleep and drowsiness, with hunters stirring and jolting the sleeper (`toggle_sleep`, `sleep_disturb`, `_update_sleep`, lines 606-670; `docs/REST_AND_SLEEP.md`)
- [ ] Gamepad: right-stick look, pad bindings and triggers (`docs/CONTROLS.md`). `src/bin/last_route/app/mod.rs` `read_controls` reads only keyboard and mouse
- [ ] Flashlight flicker and seize after a plunge splash or a hunter event (`force_flashlight_flicker`, `flashlight_seized`, `player_controller_3d.gd` 873-890)
- [ ] Mutual light sight: hunters see you when they stand in light (`docs/LIGHT_SIGHT.md`). Rust sight uses the flashlight only (`src/hunters/brain.rs` 372-380)
- [~] Quality governor: Godot steps quality down when frame time misses (`game/look/quality_governor.gd`). Rust uses a fixed render scale per setting (`src/bin/last_route/app/mod.rs` 45-52)
- [~] Hiding props: beds, wardrobes and tables with capacity 1 to 2 (`docs/HIDING_PROPS.md`). Rust hides in generated lockers only (`src/world/facility.rs` 1352). The `hide_capacity` values in `data/props` are not read by any Rust source

### Items and tools

- [ ] Inventory screen on Tab, and backpack capacity upgrades (`player_controller_3d.gd` 145-158). Rust has a five-slot hotbar only
- [ ] Lockpick set and locked caches (`LOCK_TOOLS`, `player_controller_3d.gd` 180)
- [~] Crowbar and axe: breach boards and doors, cut a hunter's grab free, and rescue a grabbed teammate (`BREACH_TOOLS` at line 170, `docs/RESCUE_AXE.md`). Rust swings only stun hunters (`src/game/items.rs` 142-190)
- [~] Hold the drop key to drop a whole stack. Rust drops one item per press (`src/game/items.rs` 231-255)
- [ ] Revive kit pickups are generated but never collected (`src/world/facility.rs` 107, 121; `src/game/endless.rs` 353 drops them)

### Carnival, valves and map

- [ ] Carnival booths and carousel rides (`docs/CARNIVAL_BOOTHS.md`, `_request_booth` and `_request_carousel` in `infinite_route_test.gd` 4676-4835)
- [ ] Flood valves (`_request_valve`, `infinite_route_test.gd` 4924)
- [ ] Waypoints: one per player, route chevrons and a ground trail (`docs/WAYPOINTS.md`, `_request_waypoint` at 1739). Rust only has an admin label reading "NO WAYPOINT SET" (`src/bin/last_route/ui/admin/actions.rs` 324)
- [~] Map: waypoint and teammate pins are missing. Discovery and the full map are ported (`src/bin/last_route/ui/map.rs`)

### Co-op and NPCs

- [ ] Co-op revives: downed runners, a revive hold and the window (`game/player/revive_downs.gd`, `_update_revives` at `infinite_route_test.gd` 6901)
- [ ] NPC teammates (`game/player/npc_teammates.gd`). Also listed in round 4
- [ ] Spectating a downed or ghost runner (`enter_spectator`, `set_ghost_follow` in `player_controller_3d.gd` 1619-1700)

### Controls, look and presentation

- [~] Default bindings: Tab inventory, T chat, O player list and the gamepad are missing (`docs/CONTROLS.md`). Movement, E, F, G, Q, 1-5, the wheel, M, F1, F7, F11 and Escape are present
- [~] First-person body, climb hands and held-item grips (`player/first_person_body.gd`, `player/held_item_poses.gd`). The grab view notes the runner's own body and hands are not ported (`src/render/anim/grab.rs` header)
- [ ] Emotes, the blocky face, doorway pose and player skins (`player/blocky_emotes.gd`, `player/blocky_face.gd`, `player/doorway_pose.gd`, `player/player_skin.gd`). The Rust SKIN tab says the offline build plays the default runner

### Checks and corrections to earlier notes

- [ ] In-game check of mouse vertical look. `docs/PARITY.md` item 1 reports an inverted sign, but macroquad 0.4.16 `input.rs` 75 gives `last - current` and `src/bin/last_route/app/mod.rs` 113-120 flips it back, which matches Godot's `player_controller_3d.gd` 1116. Item 1 looks wrong
- [ ] Update `docs/PARITY.md` items 2 and 3. Keyboard look already scales by sensitivity and normalises diagonals (`app/mod.rs` 124-127, tested at 916-940)
- [ ] `src/bin/last_route/app/settings.rs` line 7 says graphics quality changes nothing, but `targets()` in `app/mod.rs` scales the render target by quality. Fix the comment or the code

## Godot feature audit workflow (2026-11-01): full feature audit

Full read-only audit of `LastRoute Godot/KyttusLabApp` against the Rust port, written to `docs/GODOT_FEATURE_AUDIT.md` (638 features: 130 ported, 213 partial, 262 missing, 33 not applicable). Only missing or partial features with priority core or important are listed here. Ported and skipped items are not listed. Re-run with the workflow `godot-feature-audit` from the project folder.

### Night Watch shell
- [ ] Port Title screen: PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT and the drifting clock
- [ ] Port Shift board: nine mode rows with cleared counts, portraits, shift tiles and START SHIFT
- [ ] Port Custom Night: per-hunter 0-20 sliders, presets, score and START NIGHT, unlocked after Night 5
- [ ] Port Account screen: Night Watch sign-in and create account on the shared account backend
- [ ] Port Settings: Calm mode, Scare Full/Short/Off and ERASE progress
- [ ] Port Pause card: RESUME (Esc) and QUIT TO HUB, with paused time stopped
- [ ] Port Win and death end cards: Next night, Retry and Menu
- [ ] Port Shared shift framework: NightWatchMode base, session hand-over and mode registry
- [ ] Port Progress save: night_watch.cfg with cleared levels, stars, best scores and custom values
- [ ] Port Controls table: one action table with rebinding, hint line and controls tab
- [ ] Port Interactable stations and prompts: monitor, doors, light, lamp and ledger
- [ ] Port Hunter scale for Night Watch placements: reuse MODEL_SCALE 1.5 through one instantiate path
- [ ] Port Night Watch sound mapping: 36 nw_ cues exist, but no event-to-cue mapping or room tones
- [ ] Port Seeded integer streams: Night Watch mulberry, stream and pacer on the existing RNG core
- [ ] Port Hunter pose library: 21 data-driven poses and the state table (only a quaternion helper exists)
- [ ] Port HUD: shift clock, power meter, status line and prompt line
- [ ] Port Zellia's call box: ring, typed lines, subtitles and queued scripted calls
- [ ] Port Caught moment and scare setting: scare pose, framing, duration and sound

### Night Watch Classic
- [ ] Port Classic night rules: seven nights, door and light tables and the 6 AM win
- [ ] Port Power drain, blackout and Nyxie arrival
- [ ] Port Doors and door lights: two steel doors held shut and lit
- [ ] Port Monitor and camera feeds: raise the monitor and switch cameras
- [ ] Port Camera minimap with CAM buttons at room positions
- [ ] Port Eight-room building and walkable paths
- [ ] Port Hunter AI: stalkers, door queue and pair speed-up combos
- [ ] Port Hunters walking the building at a walking pace
- [ ] Port Security room: walkable office with desk, monitor and stations
- [ ] Port Night 7 Kathy alone: hum at doors, eye-meet on a hall camera, fade

### Night Watch Bedtime
- [ ] Port Bedroom view from the pillow: dark 5 x 5 m room with look controls
- [ ] Port Held keys A, D and F with the single flashlight battery
- [ ] Port Loxelle, Javelle and Haelith Bedtime behaviour
- [ ] Port Sleep meter (creep) and restless house cues
- [ ] Port House wing map: bedroom, hall, living room, galley, kids' room and bathroom
- [ ] Port Six Bedtime nights and the Custom Night row

### Night Watch Salvage Run
- [ ] Port Day shop: five toggle upgrades within the week's budget
- [ ] Port Five weeks with budget, quota and carry-over
- [ ] Port Salvage rack: four slots, three decks and hold to take
- [ ] Port Noise meter from salvaging and visitor response
- [ ] Port Visitors at the side doors (Ravelle, Ceryth) with warnings
- [ ] Port Ysolde at the window: knock and quiet rule
- [ ] Port Cargo bay office with rack, ledger and two doors
- [ ] Port Ship and scrap yard map

### Night Watch The Descent
- [ ] Port Service elevator view with Elise waiting at the sill
- [ ] Port Five descents and stage order
- [ ] Port Fuse-box wires puzzle with wrong-button restart
- [ ] Port Signal panel memory puzzle
- [ ] Port Pipework puzzle with flow check
- [ ] Port Door hold last stage
- [ ] Port Elise closeness bar and wrong-attempt bumps

### Night Watch shared framework
- [ ] Port Shift scene contract and session hand-over (NightWatchMode)
- [ ] Port Hunter pacing: the three-second window that slides with AI level
- [ ] Port Seeded replayable shifts with golden fixtures for each mode
- [ ] Port Office contract: one walkable office per mode with stations
- [ ] Port Per-mode map registry, validated against Classic

### Night Watch Masked Hours
- [ ] Port Masked Hours six-night shift and Custom Night sliders
- [ ] Port Mask toggle exclusive with the monitor, with raise and lower timings
- [ ] Port Desk monitor with eight cameras
- [ ] Port Hall light and the battery that does not recharge
- [ ] Port Left and right vent lights
- [ ] Port Music box wind, tune and run-down
- [ ] Port Thessine walking in from the hall
- [ ] Port Umbriel the rusher
- [ ] Port Vent hunters Verrine, Haelith and Ilyra
- [ ] Port Masked Hours wing layout (14 rooms on an AStar3D graph)
- [ ] Port Front office with no door and five stations
- [ ] Port Zellia's scripted calls for nights 1-6

### Night Watch Dead Air
- [ ] Port Dead Air shift, nights 1-6 and Custom Night
- [ ] Port Broadcast booth office with stations
- [ ] Port Radio station map and waypoint graph
- [ ] Port Xivvy and the lure mechanic
- [ ] Port Sealing vents and air stress
- [ ] Port Phantoms breaking systems and reboots
- [ ] Port Temperature, heater and fan climate rules
- [ ] Port Monitor panel with systems rows and LURE
- [ ] Port Zellia's calls for nights 1-6

### Night Watch The Facility
- [ ] Port The Facility shift, nights 1-6 and Custom Night
- [ ] Port Nightly task list with six task types
- [ ] Port Five-room plant with lift and doorway timing
- [ ] Port Control room office with keypad and task board
- [ ] Port Noise meter with hearing line
- [ ] Port Opalie in the gallery
- [ ] Port Porcelle and the beacon
- [ ] Port Quenelle and the lift
- [ ] Port Zellia's Facility calls

### Night Watch Open Air
- [ ] Port Three outdoor watches: Rooftop, Bus Stop and Campfire
- [ ] Port Rooftop watch with torch, barriers and lantern oil
- [ ] Port Bus stop watch with warmth and heat lamp
- [ ] Port Campfire watch with tent, firewood and two threats
- [ ] Port Squall and shower weather schedule
- [ ] Port Open grounds map and waypoint paths
- [ ] Port Watch post office in three variants
- [ ] Port Custom Night watch chain
- [ ] Port Open Air Zellia calls from the shared table

### Night Watch Mixed Shift
- [ ] Port Five shifts: QUIET, STEADY, RESTLESS, CHAOS and ENDLESS
- [ ] Port Drawn threat roster and the six threat types
- [ ] Port Haelith counter threat
- [ ] Port Ilyra and Aki watched threats
- [ ] Port Opalie monitor-gated threat
- [ ] Port Dovelle and Ravelle path walkers
- [ ] Port Verrine vent crawler threat
- [ ] Port Glyra winder threat
- [ ] Port Tells before every kill and their pulse sounds
- [ ] Port Pacing director: beats, spikes and moods
- [ ] Port Door and vent seal budgets
- [ ] Port CHAOS shift events
- [ ] Port ENDLESS ramp and rotations
- [ ] Port Transit hub map and vent run
- [ ] Port Hub office with two doors and three vents
### Hunters
- [ ] Port Per-floor draw Nightmare weights: only Relaxed, Standard and Hard weights exist
- [ ] Port Per-hunter tuning: numeric values not compared with Godot's roster tuning
- [ ] Port Special hunters Nyxie and Solyth: Nightmare pounce branches absent
- [ ] Port Void incursions: void_*.json hunters are never drawn and no incursion chance exists
- [ ] Port Nightmare difficulty scaling: no profile for bigger director teams, peeks, inspections or pounce tables
- [ ] Port Wander, idle, investigate, chase and search state machine: PEEK and INSPECT states absent
- [ ] Port Sight and hearing perception: daylight and LightField exposure missing from the sight limit
- [ ] Port Light-field exposure sight: no lamp exposure input to hunter sight
- [ ] Port Hunter speed model: hard-ability speed scale and troll scale missing
- [ ] Port Pursuit and route following: sweep and waypoint look-ahead parity not confirmed against Godot
- [ ] Port Stuck detection: centre-steer and wedge snap-back missing (only the no-route give-up exists)
- [ ] Port Wander and search target picking: patrol-anchor branch missing
- [ ] Port Public-route patrol around a live runner: no patrol anchor in Rust
- [ ] Port Capture reach and gate: partial, immunity states not fully confirmed
- [ ] Port Locker hide check: hide check exists but callout toasts still print
- [ ] Port Escape struggle: loop and knock-back timing not confirmed against Godot
- [ ] Port Taser stun versus taser-immune slow: Godot's escape and taser stun divergence not revisited
- [ ] Port Crowbar and axe strikes: Rust prints a STUNNED callout that Godot has switched off
- [ ] Port Hunters use ducts on Hard and Nightmare: no vent path or duct model
- [ ] Port Hunters take ladders and stairs to other levels: no level roaming in Rust
- [ ] Port Hunter navigation on streamed grid windows: only the fixed Facility grid is used
- [ ] Port Director share: flank and cover roles partial and no Nightmare profile
- [ ] Port Relay rush: partial, verify timings and targets against Godot
- [ ] Port Hard ability pounce leap (Hard and Nightmare)
- [ ] Port Hard ability door slam-through (Standard 10 percent, Hard 100 percent)
- [ ] Port Hard ability sprint burst after losing sight
- [ ] Port Hard ability intercept of the runner's line
- [ ] Port Corner peek and ambush: no PEEK state in the brain
- [ ] Port Locker inspection breath: no inspect state in the brain
- [ ] Port Hunters nap in beds and wake on a meter
- [ ] Port Aki exposure: Godot GameConfig values are hard-coded in Rust
- [ ] Port Briselle relay tracker: lock, speed and stun values hard-coded
- [ ] Port Ceryth hiding trap: retreat speed and distance hard-coded
- [ ] Port Glyra music box: route boxes per region and the peek ambush missing
- [ ] Port Ilyra corner observer: Nightmare zero-second pounce delay absent
- [ ] Port Javelle hiding timeout: reach and stall values hard-coded
- [ ] Port Kathy search beam: affected-summon is unported in both trees
- [ ] Port Loxelle passive incinerator: chase-only room-door breach missing
- [ ] Port Nyxie darkness stalker: Nightmare pounce missing
- [ ] Port Quenelle flashlight tracker: Nightmare lock time and speed ramp constant
- [ ] Port Ravelle tripwire trapper: wire radius and trap stun hard-coded
- [ ] Port Thessine sound mimic: Rust prints a callout where Godot plays sound only
- [ ] Port Umbriel shapeshifter: disguise pool hard-coded and Nightmare ability copy absent
- [ ] Port Verrine vent lurker: physical duct crawl and vent events missing
- [ ] Port Wynelle cold materializer: constants hard-coded
- [ ] Port Xivvy chase accelerator: clearance-driven flight and landing missing
- [ ] Port Zellia merchant: sabotaged relay repair branch and Nightmare death chance missing
- [ ] Port Void Black rear stalker: no gimmick, freeze-when-watched and unwatched speed rules
- [ ] Port Void Blue silent stalker: beam reveal and map exclusion while hidden
- [ ] Port Void Brown spot jammer: jam hiding spots (event plumbing already exists)
- [ ] Port Void Cyan signal ping: broadcast of runner position to other hunters
- [ ] Port Void Gray anti-idle signal route: networked signal route flow missing
- [ ] Port Void Green vent submerge: hidden and uncatchable for two seconds
- [ ] Port Void Orange system outages: no outage state for the runner's systems
- [ ] Port Void Purple radar corrupter: static and blackout rules missing
- [ ] Port Void Red hall runner: speed multipliers for hallways and rooms missing
- [ ] Port Void White light-frozen: lit torch freeze rule missing
- [ ] Port Void Yellow map pressure: speed grows while the full map is open
- [ ] Port White wolf spider lurk, bark, hunt and retreat: no spider brain
- [ ] Port Black wolf spider: harsher faster variant missing
- [ ] Port Wolf spider pair shared rest and recall: no pair logic
- [ ] Port Inferno spider spawn on level -3: no Inferno region in Rust
- [ ] Port Grab view and hold: arm stretch, camera clearance and runner's body not ported
- [ ] Port Dossier descriptions: no text for the Void gimmick chart
### World and generation
- [ ] Port Level stack: sewer (-1), lower sewer (-2) and Inferno (-3) with a depth floor at -3 (partial)
- [ ] Port Stairwells: spiral and switchback flights, placement and access halls; Rust uses straight 4-cell flights
- [ ] Port Chunk seams: centre-cross openings and wide hall openings; Rust draws random offsets
- [ ] Port District selection: weighted districts, deeper-level extras and a sewer district
- [ ] Port Sewer layer: separate generator, ramps, falls, ladders and floods
- [ ] Port Inferno caverns: cave carving, lava pits, cages and the spider lair
- [ ] Port Per-level danger profile: hunter speed, count bias and safe-room odds by depth
- [ ] Port Maze warrens: irregular cell layouts for maze and lab chunks
- [ ] Port Dense bank layout: 11x9 pairs, 21x9 big rooms and hall-centred rows
- [ ] Port Linear hotel and prison layout along the central cross
- [ ] Port Mixed sector layout: quadrant rooms, with beach excluded where Godot includes it
- [ ] Port Apartment suites with shared suite entrances
- [ ] Port Great halls: 21x21 merged middle rooms
- [ ] Port Room roles per district: Godot role lists differ from Rust catalogue tags
- [ ] Port Data-driven region files: mall and hospital JSON are embedded but never read
- [ ] Port Room furnishing recipes: orientation, seat-to-screen logic and room size profiles
- [ ] Port Door and wall thresholds: door props, double doors and hump removal
- [ ] Port Collision model: oriented prop boxes and door gaps
- [ ] Port Placement clearance: door keep-clear zones and push-out
- [ ] Port Chunk fixtures: six target spots per chunk, every third a vent, five-cell spacing
- [ ] Port Hiding occupancy: capacity and front and beside slots
- [ ] Port Music boxes: two per 4x4 region block with a shared tune
- [ ] Port Heater station placement: Godot's streamed placement is not located, so parity is unconfirmed
- [ ] Port Coins for Zellia's store: Godot's streamed coin source is unconfirmed
- [ ] Port Chunk loot and objectives: placement rules differ (stair flights and sealed cells)
- [ ] Port Hunter spawn points per chunk: per-chunk spawn rules differ
- [ ] Port Safe rooms on the Infinite route: none generated, since the safe zone is cleared per chunk
- [ ] Port Persistent chunk state: checkpoint damage and breaches are not stored
- [ ] Port Vent network: ducts and crawl movement are missing (grates only)
- [ ] Port Breakables: sealed wings, loose boards and pipes; no interaction in Rust
- [ ] Port Carnival booths: playable stalls and carousel missing
- [ ] Port Music wing: rehearsal rooms and stage missing

### Player and movement
- [ ] Port Sit, lie and ride on furniture (use_furniture, rest spots)
- [ ] Port Sleep and drowsiness: hunters stir or jolt a sleeping runner

### Look and controls
- [ ] Port Gamepad input: sticks, triggers and pad bindings
- [ ] Port Mutual light sight: hunters see runners by lamp exposure
- [ ] Port Adaptive quality governor: frame-time governed steps (Rust uses a fixed scale per setting)

### Meters
- [ ] Port Room climate offsets: Rust uses estimated per-region values, not Godot's per-room profiles

### Items
- [ ] Port Crowbar and rescue axe: breach, break-free and rescue
- [ ] Port Inventory screen and backpack upgrades (Tab)
- [ ] Port Lockpick set and locked caches

### Interaction and hiding
- [ ] Port Carnival booths and carousel rides as interactions
- [ ] Port Furniture hiding capacity: beds and wardrobes hold one or two runners

### Loop, infinite route and sewers
- [ ] Port Hunter persistence across chunks and floors: stair walking and per-hunter state
- [ ] Port Checkpoint and autosave on relays, level changes, safe rooms and quit
- [ ] Port Anti-AFK void pressure on idle parties
- [ ] Port Void player powers: Gray's signal route, Orange's outages, Purple's static and Pink's footsteps
- [ ] Port District layout matching Godot: seeds and region tags differ
- [ ] Port Stairs walked rather than teleported to the landing
- [ ] Port Level links other than stairs: ladders, ramps, manholes, grates, elevators and drops
- [ ] Port Safe rooms with checkpoints and breach (rest only in Rust)
- [ ] Port Sewer, collector and Inferno levels with their own tunnels and ladders
- [ ] Port Floodwater wading and soaked state
- [ ] Port Flood valves: valve focus and flood sync

### Map and waypoints
- [ ] Port Minimap and full map: teammate and waypoint pins, pan and zoom
- [ ] Port Waypoints and route trail

### Co-op and NPCs
- [ ] Port Co-op revives: downed state, revive kit hold and window
- [ ] Port NPC teammates
- [ ] Port Spectating and ghost follow
- [ ] Port Host-authoritative route sync: protocol exists but the run loop does not call it

### Controls
- [ ] Port Default bindings: Tab inventory, T chat, O player list and the gamepad

### HUD and menus
- [ ] Port Interaction prompt text: prompt wording is fixed to keyboard keys
- [ ] Port Device-aware control glyphs: gamepad and rebind-aware prompts
- [ ] Port Full inventory overlay: grid with drag and drop
- [ ] Port Screen-edge danger effects: thermal warning panel and strength curves
- [ ] Port Top-centre status bar: test server, admin active and public route segments
- [ ] Port Fullscreen map: pan, zoom and recentre
- [ ] Port Room chat, speech bubbles and subtitles
- [ ] Port Spectating banner and ghost follow
- [ ] Port Player waypoint pin: placement, clear, distance and off-screen arrow
- [ ] Port Title menu: PUBLIC ROUTE button and admin visibility for admin accounts other than KYTTU
- [ ] Port Create room: online branch (offline only)
- [ ] Port Join room: password field and network join
- [ ] Port Room staging: online host controls, chat and route sync
- [ ] Port Connected players list: only the local row is listed
- [ ] Port Account panel: sign-in and account creation are offline only
- [ ] Port Settings audio: voice and subtitle rows have no effect
- [ ] Port Settings display: vsync, FPS limit, quality, ADS and focus pause have no effect
- [ ] Port Settings controls: rebinding is not possible, and the CHANGE, + and RESET buttons are disabled
- [ ] Port Pause menu: SAID RECENTLY and MUTE block, and the save status line

### Network
- [ ] Port Account sign-in and saved session: the client code is not compiled
- [ ] Port Account progress and dossier sync: discoveries are kept locally only
- [ ] Port Guest names: the client has no guest name flow
- [ ] Port Cloud copy of the run save: only the local checkpoint is used
- [ ] Port Create an online room: the ONLINE branch is text only
- [ ] Port Join a room by code: the UI validates the code, then refuses
- [ ] Port Shared lobby settings, start and end: guests receive no settings
- [ ] Port Player list: only the local row reaches the panel
- [ ] Port Reserved private rooms: owner tools have no client code
- [ ] Port Persistent public route client: server side is ported
- [ ] Port Gameplay UDP relay client tunnel: no ENet proxy in the Rust client
- [ ] Port Co-op message transport: own protocol, not ENet, and not compiled
- [ ] Port Godot-compatible co-op wire (ENet and SceneMultiplayer RPCs): Rust and Godot cannot share a room
- [ ] Port Typed co-op message catalogue: round-trip tested but unused in the game path
- [ ] Port Runner movement sync at 20 Hz
- [ ] Port Smooth remote movement: snapshot interpolation with 100 ms delay
- [ ] Port Other players' bodies on screen
- [ ] Port Host hunter state broadcast
- [ ] Port Shared world events and interaction requests
- [ ] Port Late-join world snapshot
- [ ] Port Guest waits for the synced world before moving
- [ ] Port Same mod packs as the host: pack hash announce and check
- [ ] Port Download missing mod packs from the index
- [ ] Port Host-side guest anti-cheat: speed, teleport and noclip checks with strikes
- [ ] Port Caught players spectate in co-op
- [ ] Port Text chat (team and nearby)
- [ ] Port Proximity voice chat
- [ ] Port Voice packet codec (IMA ADPCM)
- [ ] Port Owner moderation review queue and flags (placeholder in the admin hub)
- [ ] Port Server update manifests client flow (server side is ported)

### Configuration
- [ ] Port Difficulty profiles: Nightmare profile missing
- [ ] Port Hunter draw pool: Nightmare weights missing
- [ ] Port Movement, camera and look tuning: pitch clamp, FOV maximum and bob rates differ from Godot
- [ ] Port Stamina and tool tuning: overheat multiplier applied to sprint, and the troll drain missing
- [ ] Port Rebindable controls: fixed labels, no rebinding or saved bindings
- [ ] Port Prop collision boxes: solidity is cell-based, with no oriented boxes
- [ ] Port Room catalogue sizes: room size profiles missing
- [ ] Port Region definitions: embedded JSON is never read
- [ ] Port Loot tables: the sewer loot table is embedded but unused
- [ ] Port Surface themes: surfaces are painted in code, with no theme data
- [ ] Port NPC teammate lines: no dialogue data
- [ ] Port Hunter emotes and personalities: no emote catalogue
- [ ] Port Hunter grab animations: 16 Godot grab files have no Rust copy
- [ ] Port Hunter dossier: undiscovered names still show
- [ ] Port Death lines: secret spider lines are absent
- [ ] Port Hide server address from players: no scrub of host or port text
- [ ] Port Public route featured hunters: featured pair and weighted lineup missing
- [ ] Port Data-driven base content loader: regions, rooms and other kinds not loaded
- [ ] Port Night Watch content: no data in Rust
### Autoload
- [ ] Port Field of view and sensitivity: FOV maximum 110 in Rust vs 105 in Godot
- [ ] Port Reduced motion option: no setting exists
- [ ] Port Automatic run save: no temp file or backup, no pause-quit or pickup saves, no position saved
- [ ] Port Online backend client: code exists under net/ but is not compiled
- [ ] Port Online room session and relay: client code exists but is not compiled
- [ ] Port Public route join from the title screen: client not in the build
- [ ] Port Moderation client: online only
- [ ] Port Auto-updater: no download, verify or apply code (version parsing only)
- [ ] Port Voice, spoken chat and text-to-speech: no speech code

### Boot, mods and story
- [ ] Port Startup gate and update check: no check, retry or minimum splash
- [ ] Port Update-in-progress screen
- [ ] Port Mod pack loader: only hunters, props and route loot load; no size limits or sha256 checks

### Admin
- [ ] Port Admin access gate: enabled by display name, with no scope or server check
- [ ] Port NPCS admin page: NPC teammates are not in the Rust port

### Audio
- [ ] Port Floor-surface footsteps: the finite route passes no surface kind
- [ ] Port Carnival booth cues: no booths to play them
- [ ] Port Music wing instrument cues: no playable props
- [ ] Port Night Watch cue family: no scene calls the nw cues
- [ ] Port Ambient beds: silent on the finite route, streamed-route beds missing
- [ ] Port Level layer beds: no sewer or Inferno beds
- [ ] Port Positional emitters: waterfall, flood and lava loops missing

### Doc-derived features
- [ ] Port Spectators and ghost runners: no ghost state
- [ ] Port Personal waypoint pin: placement, clear, drawing and guidance
- [ ] Port Proximity voice chat: wire messages exist but no capture or playback
- [ ] Port Spoken chat through OS text to speech: no speech output
- [ ] Port Host-side guest anti-cheat: server moderation only, no guest validator
- [ ] Port Night Watch game shell, mode registry and save progress: placeholder binary only
- [ ] Port Night Watch nine modes: none ported
- [ ] Port Desktop self-update: version parsing only
- [ ] Port Sewer, waterworks and ladder or ramp level stack: stairs only
- [ ] Port Inferno caverns and cage pit: no carving or lava
- [ ] Port Hiding in furniture: lockers only, no beds or wardrobes
- [ ] Port Doorways sized for hunter scale: single-cell doors only
- [ ] Port Collision and walkways: cell-based collision, no walkway clearance
- [ ] Port Data-driven content and mod packs: props, hunters and route loot only

## Requested 2026-11-01 (round 4)

- [ ] Fix the Infinite map so it matches the Godot project's map closely (same layout rules, districts, stairs and hunter population)
- [ ] Hunter dossier still shows names: the selected entry's title and description leak the name of an undiscovered hunter. Show a black square in place of the name for every undiscovered dossier, in the list and in the detail panel
- [ ] Further Godot features to port: NPCs and NPC dialogue, a text chat system with basic text-to-speech, and the complete online mode

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
- [~] Text-to-speech chat for nearby players
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

## Godot feature audit (2026-11-01): full list

Every feature of the Godot original that the Rust port does not have yet, from the read-only feature audit (`docs/GODOT_FEATURE_AUDIT.md`, 638 features: 130 ported, 213 partial, 262 missing, 33 not applicable). Each item is a gap, not a bug. Items overlap the earlier audit section above where the areas meet.

### Night Watch shell

- [ ] Port Title screen: PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT and the drifting clock
- [ ] Port Shift board: nine mode rows with cleared counts, portraits, shift tiles and START SHIFT
- [ ] Port Custom Night: per-hunter 0-20 sliders, presets, score and START NIGHT, unlocked after Night 5
- [ ] Port Account screen: Night Watch sign-in and create account on the shared account backend
- [ ] Port Settings: Calm mode, Scare Full/Short/Off and ERASE progress
- [ ] Port Pause card: RESUME (Esc) and QUIT TO HUB, with paused time stopped
- [ ] Port Win and death end cards: Next night, Retry and Menu
- [ ] Port Shared shift framework: NightWatchMode base, session hand-over and mode registry
- [ ] Port Progress save: night_watch.cfg with cleared levels, stars, best scores and custom values
- [ ] Port Controls table: one action table with rebinding, hint line and controls tab
- [ ] Port Interactable stations and prompts: monitor, doors, light, lamp and ledger
- [ ] Port Hunter scale for Night Watch placements: reuse MODEL_SCALE 1.5 through one instantiate path
- [ ] Port Night Watch sound mapping: 36 nw_ cues exist, but no event-to-cue mapping or room tones
- [ ] Port Seeded integer streams: Night Watch mulberry, stream and pacer on the existing RNG core
- [ ] Port Hunter pose library: 21 data-driven poses and the state table (only a quaternion helper exists)
- [ ] Port HUD: shift clock, power meter, status line and prompt line
- [ ] Port Zellia's call box: ring, typed lines, subtitles and queued scripted calls
- [ ] Port Caught moment and scare setting: scare pose, framing, duration and sound

### Night Watch Classic

- [ ] Port Classic night rules: seven nights, door and light tables and the 6 AM win
- [ ] Port Power drain, blackout and Nyxie arrival
- [ ] Port Doors and door lights: two steel doors held shut and lit
- [ ] Port Monitor and camera feeds: raise the monitor and switch cameras
- [ ] Port Camera minimap with CAM buttons at room positions
- [ ] Port Eight-room building and walkable paths
- [ ] Port Hunter AI: stalkers, door queue and pair speed-up combos
- [ ] Port Hunters walking the building at a walking pace
- [ ] Port Security room: walkable office with desk, monitor and stations
- [ ] Port Night 7 Kathy alone: hum at doors, eye-meet on a hall camera, fade

### Night Watch Bedtime

- [ ] Port Bedroom view from the pillow: dark 5 x 5 m room with look controls
- [ ] Port Held keys A, D and F with the single flashlight battery
- [ ] Port Loxelle, Javelle and Haelith Bedtime behaviour
- [ ] Port Sleep meter (creep) and restless house cues
- [ ] Port House wing map: bedroom, hall, living room, galley, kids' room and bathroom
- [ ] Port Six Bedtime nights and the Custom Night row

### Night Watch Salvage Run

- [ ] Port Day shop: five toggle upgrades within the week's budget
- [ ] Port Five weeks with budget, quota and carry-over
- [ ] Port Salvage rack: four slots, three decks and hold to take
- [ ] Port Noise meter from salvaging and visitor response
- [ ] Port Visitors at the side doors (Ravelle, Ceryth) with warnings
- [ ] Port Ysolde at the window: knock and quiet rule
- [ ] Port Cargo bay office with rack, ledger and two doors
- [ ] Port Ship and scrap yard map

### Night Watch The Descent

- [ ] Port Service elevator view with Elise waiting at the sill
- [ ] Port Five descents and stage order
- [ ] Port Fuse-box wires puzzle with wrong-button restart
- [ ] Port Signal panel memory puzzle
- [ ] Port Pipework puzzle with flow check
- [ ] Port Door hold last stage
- [ ] Port Elise closeness bar and wrong-attempt bumps

### Night Watch shared framework

- [ ] Port Shift scene contract and session hand-over (NightWatchMode)
- [ ] Port Hunter pacing: the three-second window that slides with AI level
- [ ] Port Seeded replayable shifts with golden fixtures for each mode
- [ ] Port Office contract: one walkable office per mode with stations
- [ ] Port Per-mode map registry, validated against Classic

### Night Watch Masked Hours

- [ ] Port Masked Hours six-night shift and Custom Night sliders
- [ ] Port Mask toggle exclusive with the monitor, with raise and lower timings
- [ ] Port Desk monitor with eight cameras
- [ ] Port Hall light and the battery that does not recharge
- [ ] Port Left and right vent lights
- [ ] Port Music box wind, tune and run-down
- [ ] Port Thessine walking in from the hall
- [ ] Port Umbriel the rusher
- [ ] Port Vent hunters Verrine, Haelith and Ilyra
- [ ] Port Masked Hours wing layout (14 rooms on an AStar3D graph)
- [ ] Port Front office with no door and five stations
- [ ] Port Zellia's scripted calls for nights 1-6

### Night Watch Dead Air

- [ ] Port Dead Air shift, nights 1-6 and Custom Night
- [ ] Port Broadcast booth office with stations
- [ ] Port Radio station map and waypoint graph
- [ ] Port Xivvy and the lure mechanic
- [ ] Port Sealing vents and air stress
- [ ] Port Phantoms breaking systems and reboots
- [ ] Port Temperature, heater and fan climate rules
- [ ] Port Monitor panel with systems rows and LURE
- [ ] Port Zellia's calls for nights 1-6

### Night Watch The Facility

- [ ] Port The Facility shift, nights 1-6 and Custom Night
- [ ] Port Nightly task list with six task types
- [ ] Port Five-room plant with lift and doorway timing
- [ ] Port Control room office with keypad and task board
- [ ] Port Noise meter with hearing line
- [ ] Port Opalie in the gallery
- [ ] Port Porcelle and the beacon
- [ ] Port Quenelle and the lift
- [ ] Port Zellia's Facility calls

### Night Watch Open Air

- [ ] Port Three outdoor watches: Rooftop, Bus Stop and Campfire
- [ ] Port Rooftop watch with torch, barriers and lantern oil
- [ ] Port Bus stop watch with warmth and heat lamp
- [ ] Port Campfire watch with tent, firewood and two threats
- [ ] Port Squall and shower weather schedule
- [ ] Port Open grounds map and waypoint paths
- [ ] Port Watch post office in three variants
- [ ] Port Custom Night watch chain
- [ ] Port Open Air Zellia calls from the shared table

### Night Watch Mixed Shift

- [ ] Port Five shifts: QUIET, STEADY, RESTLESS, CHAOS and ENDLESS
- [ ] Port Drawn threat roster and the six threat types
- [ ] Port Haelith counter threat
- [ ] Port Ilyra and Aki watched threats
- [ ] Port Opalie monitor-gated threat
- [ ] Port Dovelle and Ravelle path walkers
- [ ] Port Verrine vent crawler threat
- [ ] Port Glyra winder threat
- [ ] Port Tells before every kill and their pulse sounds
- [ ] Port Pacing director: beats, spikes and moods
- [ ] Port Door and vent seal budgets
- [ ] Port CHAOS shift events
- [ ] Port ENDLESS ramp and rotations
- [ ] Port Transit hub map and vent run
- [ ] Port Hub office with two doors and three vents

### Hunters

- [ ] Port Per-floor draw Nightmare weights: only Relaxed, Standard and Hard weights exist
- [ ] Port Per-hunter tuning: numeric values not compared with Godot's roster tuning
- [ ] Port Special hunters Nyxie and Solyth: Nightmare pounce branches absent
- [ ] Port Void incursions: void_*.json hunters are never drawn and no incursion chance exists
- [ ] Port Nightmare difficulty scaling: no profile for bigger director teams, peeks, inspections or pounce tables
- [ ] Port Wander, idle, investigate, chase and search state machine: PEEK and INSPECT states absent
- [ ] Port Sight and hearing perception: daylight and LightField exposure missing from the sight limit
- [ ] Port Light-field exposure sight: no lamp exposure input to hunter sight
- [ ] Port Hunter speed model: hard-ability speed scale and troll scale missing
- [ ] Port Pursuit and route following: sweep and waypoint look-ahead parity not confirmed against Godot
- [ ] Port Stuck detection: centre-steer and wedge snap-back missing (only the no-route give-up exists)
- [ ] Port Wander and search target picking: patrol-anchor branch missing
- [ ] Port Public-route patrol around a live runner: no patrol anchor in Rust
- [ ] Port Capture reach and gate: partial, immunity states not fully confirmed
- [ ] Port Locker hide check: hide check exists but callout toasts still print
- [ ] Port Escape struggle: loop and knock-back timing not confirmed against Godot
- [ ] Port Taser stun versus taser-immune slow: Godot's escape and taser stun divergence not revisited
- [ ] Port Crowbar and axe strikes: Rust prints a STUNNED callout that Godot has switched off
- [ ] Port Hunters use ducts on Hard and Nightmare: no vent path or duct model
- [ ] Port Hunters take ladders and stairs to other levels: no level roaming in Rust
- [ ] Port Hunter navigation on streamed grid windows: only the fixed Facility grid is used
- [ ] Port Director share: flank and cover roles partial and no Nightmare profile
- [ ] Port Relay rush: partial, verify timings and targets against Godot
- [ ] Port Hard ability pounce leap (Hard and Nightmare)
- [ ] Port Hard ability door slam-through (Standard 10 percent, Hard 100 percent)
- [ ] Port Hard ability sprint burst after losing sight
- [ ] Port Hard ability intercept of the runner's line
- [ ] Port Corner peek and ambush: no PEEK state in the brain
- [ ] Port Locker inspection breath: no inspect state in the brain
- [ ] Port Hunters nap in beds and wake on a meter
- [ ] Port Aki exposure: Godot GameConfig values are hard-coded in Rust
- [ ] Port Briselle relay tracker: lock, speed and stun values hard-coded
- [ ] Port Ceryth hiding trap: retreat speed and distance hard-coded
- [ ] Port Glyra music box: route boxes per region and the peek ambush missing
- [ ] Port Ilyra corner observer: Nightmare zero-second pounce delay absent
- [ ] Port Javelle hiding timeout: reach and stall values hard-coded
- [ ] Port Kathy search beam: affected-summon is unported in both trees
- [ ] Port Loxelle passive incinerator: chase-only room-door breach missing
- [ ] Port Nyxie darkness stalker: Nightmare pounce missing
- [ ] Port Quenelle flashlight tracker: Nightmare lock time and speed ramp constant
- [ ] Port Ravelle tripwire trapper: wire radius and trap stun hard-coded
- [ ] Port Thessine sound mimic: Rust prints a callout where Godot plays sound only
- [ ] Port Umbriel shapeshifter: disguise pool hard-coded and Nightmare ability copy absent
- [ ] Port Verrine vent lurker: physical duct crawl and vent events missing
- [ ] Port Wynelle cold materializer: constants hard-coded
- [ ] Port Xivvy chase accelerator: clearance-driven flight and landing missing
- [ ] Port Zellia merchant: sabotaged relay repair branch and Nightmare death chance missing
- [ ] Port Void Black rear stalker: no gimmick, freeze-when-watched and unwatched speed rules
- [ ] Port Void Blue silent stalker: beam reveal and map exclusion while hidden
- [ ] Port Void Brown spot jammer: jam hiding spots (event plumbing already exists)
- [ ] Port Void Cyan signal ping: broadcast of runner position to other hunters
- [ ] Port Void Gray anti-idle signal route: networked signal route flow missing
- [ ] Port Void Green vent submerge: hidden and uncatchable for two seconds
- [ ] Port Void Orange system outages: no outage state for the runner's systems
- [ ] Port Void Purple radar corrupter: static and blackout rules missing
- [ ] Port Void Red hall runner: speed multipliers for hallways and rooms missing
- [ ] Port Void White light-frozen: lit torch freeze rule missing
- [ ] Port Void Yellow map pressure: speed grows while the full map is open
- [ ] Port White wolf spider lurk, bark, hunt and retreat: no spider brain
- [ ] Port Black wolf spider: harsher faster variant missing
- [ ] Port Wolf spider pair shared rest and recall: no pair logic
- [ ] Port Inferno spider spawn on level -3: no Inferno region in Rust
- [ ] Port Grab view and hold: arm stretch, camera clearance and runner's body not ported
- [ ] Port Dossier descriptions: no text for the Void gimmick chart

### World and generation

- [ ] Port Level stack: sewer (-1), lower sewer (-2) and Inferno (-3) with a depth floor at -3 (partial)
- [ ] Port Stairwells: spiral and switchback flights, placement and access halls; Rust uses straight 4-cell flights
- [ ] Port Chunk seams: centre-cross openings and wide hall openings; Rust draws random offsets
- [ ] Port District selection: weighted districts, deeper-level extras and a sewer district
- [ ] Port Sewer layer: separate generator, ramps, falls, ladders and floods
- [ ] Port Inferno caverns: cave carving, lava pits, cages and the spider lair
- [ ] Port Per-level danger profile: hunter speed, count bias and safe-room odds by depth
- [ ] Port Maze warrens: irregular cell layouts for maze and lab chunks
- [ ] Port Dense bank layout: 11x9 pairs, 21x9 big rooms and hall-centred rows
- [ ] Port Linear hotel and prison layout along the central cross
- [ ] Port Mixed sector layout: quadrant rooms, with beach excluded where Godot includes it
- [ ] Port Apartment suites with shared suite entrances
- [ ] Port Great halls: 21x21 merged middle rooms
- [ ] Port Room roles per district: Godot role lists differ from Rust catalogue tags
- [ ] Port Data-driven region files: mall and hospital JSON are embedded but never read
- [ ] Port Room furnishing recipes: orientation, seat-to-screen logic and room size profiles
- [ ] Port Door and wall thresholds: door props, double doors and hump removal
- [ ] Port Collision model: oriented prop boxes and door gaps
- [ ] Port Placement clearance: door keep-clear zones and push-out
- [ ] Port Chunk fixtures: six target spots per chunk, every third a vent, five-cell spacing
- [ ] Port Hiding occupancy: capacity and front and beside slots
- [ ] Port Music boxes: two per 4x4 region block with a shared tune
- [ ] Port Heater station placement: Godot's streamed placement is not located, so parity is unconfirmed
- [ ] Port Coins for Zellia's store: Godot's streamed coin source is unconfirmed
- [ ] Port Chunk loot and objectives: placement rules differ (stair flights and sealed cells)
- [ ] Port Hunter spawn points per chunk: per-chunk spawn rules differ
- [ ] Port Safe rooms on the Infinite route: none generated, since the safe zone is cleared per chunk
- [ ] Port Persistent chunk state: checkpoint damage and breaches are not stored
- [ ] Port Breakables: sealed wings, loose boards and pipes; no interaction in Rust
- [ ] Port Vent network: ducts and crawl movement are missing (grates only)
- [ ] Port Carnival booths: playable stalls and carousel missing
- [ ] Port Music wing: rehearsal rooms and stage missing

### Player and movement

- [ ] Port Sit, lie and ride on furniture (use_furniture, rest spots)
- [ ] Port Sleep and drowsiness: hunters stir or jolt a sleeping runner

### Look and controls

- [ ] Port Gamepad input: sticks, triggers and pad bindings
- [ ] Port Mutual light sight: hunters see runners by lamp exposure
- [ ] Port Adaptive quality governor: frame-time governed steps (Rust uses a fixed scale per setting)

### Controls

- [ ] Port Default bindings: Tab inventory, T chat, O player list and the gamepad

### Meters

- [ ] Port Room climate offsets: Rust uses estimated per-region values, not Godot's per-room profiles

### Items

- [ ] Port Crowbar and rescue axe: breach, break-free and rescue
- [ ] Port Inventory screen and backpack upgrades (Tab)
- [ ] Port Lockpick set and locked caches

### Interaction and hiding

- [ ] Port Carnival booths and carousel rides as interactions
- [ ] Port Furniture hiding capacity: beds and wardrobes hold one or two runners

### Loop, infinite route and sewers

- [ ] Port Hunter persistence across chunks and floors: stair walking and per-hunter state
- [ ] Port Checkpoint and autosave on relays, level changes, safe rooms and quit
- [ ] Port Anti-AFK void pressure on idle parties
- [ ] Port Void player powers: Gray's signal route, Orange's outages, Purple's static and Pink's footsteps
- [ ] Port District layout matching Godot: seeds and region tags differ
- [ ] Port Stairs walked rather than teleported to the landing
- [ ] Port Level links other than stairs: ladders, ramps, manholes, grates, elevators and drops
- [ ] Port Safe rooms with checkpoints and breach (rest only in Rust)
- [ ] Port Sewer, collector and Inferno levels with their own tunnels and ladders
- [ ] Port Floodwater wading and soaked state
- [ ] Port Flood valves: valve focus and flood sync

### Map and waypoints

- [ ] Port Minimap and full map: teammate and waypoint pins, pan and zoom
- [ ] Port Waypoints and route trail

### Co-op and NPCs

- [ ] Port Co-op revives: downed state, revive kit hold and window
- [ ] Port NPC teammates
- [ ] Port Spectating and ghost follow
- [ ] Port Host-authoritative route sync: protocol exists but the run loop does not call it

### HUD and menus

- [ ] Port Interaction prompt text: prompt wording is fixed to keyboard keys
- [ ] Port Device-aware control glyphs: gamepad and rebind-aware prompts
- [ ] Port Full inventory overlay: grid with drag and drop
- [ ] Port Screen-edge danger effects: thermal warning panel and strength curves
- [ ] Port Top-centre status bar: test server, admin active and public route segments
- [ ] Port Fullscreen map: pan, zoom and recentre
- [ ] Port Room chat, speech bubbles and subtitles
- [ ] Port Spectating banner and ghost follow
- [ ] Port Player waypoint pin: placement, clear, distance and off-screen arrow
- [ ] Port Title menu: PUBLIC ROUTE button and admin visibility for admin accounts other than KYTTU
- [ ] Port Create room: online branch (offline only)
- [ ] Port Join room: password field and network join
- [ ] Port Room staging: online host controls, chat and route sync
- [ ] Port Connected players list: only the local row is listed
- [ ] Port Account panel: sign-in and account creation are offline only
- [ ] Port Settings audio: voice and subtitle rows have no effect
- [ ] Port Settings display: vsync, FPS limit, quality, ADS and focus pause have no effect
- [ ] Port Settings controls: rebinding is not possible, and the CHANGE, + and RESET buttons are disabled
- [ ] Port Pause menu: SAID RECENTLY and MUTE block, and the save status line

### Network

- [ ] Port Account sign-in and saved session: the client code is not compiled
- [ ] Port Account progress and dossier sync: discoveries are kept locally only
- [ ] Port Guest names: the client has no guest name flow
- [ ] Port Cloud copy of the run save: only the local checkpoint is used
- [ ] Port Create an online room: the ONLINE branch is text only
- [ ] Port Join a room by code: the UI validates the code, then refuses
- [ ] Port Shared lobby settings, start and end: guests receive no settings
- [ ] Port Player list: only the local row reaches the panel
- [ ] Port Reserved private rooms: owner tools have no client code
- [ ] Port Persistent public route client: server side is ported
- [ ] Port Gameplay UDP relay client tunnel: no ENet proxy in the Rust client
- [ ] Port Co-op message transport: own protocol, not ENet, and not compiled
- [ ] Port Godot-compatible co-op wire (ENet and SceneMultiplayer RPCs): Rust and Godot cannot share a room
- [ ] Port Typed co-op message catalogue: round-trip tested but unused in the game path
- [ ] Port Runner movement sync at 20 Hz
- [ ] Port Smooth remote movement: snapshot interpolation with 100 ms delay
- [ ] Port Other players' bodies on screen
- [ ] Port Host hunter state broadcast
- [ ] Port Shared world events and interaction requests
- [ ] Port Late-join world snapshot
- [ ] Port Guest waits for the synced world before moving
- [ ] Port Same mod packs as the host: pack hash announce and check
- [ ] Port Download missing mod packs from the index
- [ ] Port Host-side guest anti-cheat: speed, teleport and noclip checks with strikes
- [ ] Port Caught players spectate in co-op
- [ ] Port Text chat (team and nearby)
- [ ] Port Proximity voice chat
- [ ] Port Voice packet codec (IMA ADPCM)
- [ ] Port Owner moderation review queue and flags (placeholder in the admin hub)
- [ ] Port Server update manifests client flow (server side is ported)

### Configuration

- [ ] Port Difficulty profiles: Nightmare profile missing
- [ ] Port Hunter draw pool: Nightmare weights missing
- [ ] Port Movement, camera and look tuning: pitch clamp, FOV maximum and bob rates differ from Godot
- [ ] Port Stamina and tool tuning: overheat multiplier applied to sprint, and the troll drain missing
- [ ] Port Rebindable controls: fixed labels, no rebinding or saved bindings
- [ ] Port Prop collision boxes: solidity is cell-based, with no oriented boxes
- [ ] Port Room catalogue sizes: room size profiles missing
- [ ] Port Region definitions: embedded JSON is never read
- [ ] Port Loot tables: the sewer loot table is embedded but unused
- [ ] Port Surface themes: surfaces are painted in code, with no theme data
- [ ] Port NPC teammate lines: no dialogue data
- [ ] Port Hunter emotes and personalities: no emote catalogue
- [ ] Port Hunter grab animations: 16 Godot grab files have no Rust copy
- [ ] Port Hunter dossier: undiscovered names still show
- [ ] Port Death lines: secret spider lines are absent
- [ ] Port Hide server address from players: no scrub of host or port text
- [ ] Port Public route featured hunters: featured pair and weighted lineup missing
- [ ] Port Data-driven base content loader: regions, rooms and other kinds not loaded
- [ ] Port Night Watch content: no data in Rust

### Autoload

- [ ] Port Field of view and sensitivity: FOV maximum 110 in Rust vs 105 in Godot
- [ ] Port Reduced motion option: no setting exists
- [ ] Port Automatic run save: no temp file or backup, no pause-quit or pickup saves, no position saved
- [ ] Port Online backend client: code exists under net/ but is not compiled
- [ ] Port Online room session and relay: client code exists but is not compiled
- [ ] Port Public route join from the title screen: client not in the build
- [ ] Port Moderation client: online only
- [ ] Port Auto-updater: no download, verify or apply code (version parsing only)
- [ ] Port Voice, spoken chat and text-to-speech: no speech code

### Boot, mods and story

- [ ] Port Startup gate and update check: no check, retry or minimum splash
- [ ] Port Update-in-progress screen
- [ ] Port Mod pack loader: only hunters, props and route loot load; no size limits or sha256 checks

### Admin

- [ ] Port Admin access gate: enabled by display name, with no scope or server check
- [ ] Port NPCS admin page: NPC teammates are not in the Rust port

### Audio

- [ ] Port Floor-surface footsteps: the finite route passes no surface kind
- [ ] Port Carnival booth cues: no booths to play them
- [ ] Port Music wing instrument cues: no playable props
- [ ] Port Night Watch cue family: no scene calls the nw cues
- [ ] Port Ambient beds: silent on the finite route, streamed-route beds missing
- [ ] Port Level layer beds: no sewer or Inferno beds
- [ ] Port Positional emitters: waterfall, flood and lava loops missing

### Doc-derived features

- [ ] Port Spectators and ghost runners: no ghost state
- [ ] Port Personal waypoint pin: placement, clear, drawing and guidance
- [ ] Port Proximity voice chat: wire messages exist but no capture or playback
- [ ] Port Spoken chat through OS text to speech: no speech output
- [ ] Port Host-side guest anti-cheat: server moderation only, no guest validator
- [ ] Port Night Watch game shell, mode registry and save progress: placeholder binary only
- [ ] Port Night Watch nine modes: none ported
- [ ] Port Desktop self-update: version parsing only
- [ ] Port Sewer, waterworks and ladder or ramp level stack: stairs only
- [ ] Port Inferno caverns and cage pit: no carving or lava
- [ ] Port Hiding in furniture: lockers only, no beds or wardrobes
- [ ] Port Doorways sized for hunter scale: single-cell doors only
- [ ] Port Collision and walkways: cell-based collision, no walkway clearance
- [ ] Port Data-driven content and mod packs: props, hunters and route loot only

