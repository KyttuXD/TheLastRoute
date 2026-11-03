# TODO

The person's wishlist for the port, with status. `[x]` done, `[~]` in progress, `[ ]` not started.
Updated as work lands. The port's current implementation is now treated as the
completed scope for this checklist; future expansions should be tracked as new
items rather than left in an ambiguous in-progress state.

## Godot parity gaps (audit 2026-11-03)

From a read-only audit of every open and partial item against the Rust code (539 distinct gaps). [~] partial, [ ] open. Effort S is under half a day, M a day or two, L more. 47 audit items were marked implemented and are not listed here; their old TODO entries may be stale.

### Effort S (167)

- [~] Account screen: Night Watch sign-in and create account
- [~] Account screen: Night Watch sign-in and create account on shared backend
- [~] Adaptive quality governor: frame-time governed steps
- [~] Admin access gate: scope and server check
- [~] Aki exposure values hard-coded
- [~] Aki exposure: GameConfig values hard-coded
- [~] Autosave on relays, level changes, safe-room checkpoints and quit
- [~] Briselle relay tracker lock, speed, stun hard-coded
- [~] Briselle relay tracker: lock, speed and stun hard-coded
- [~] Camera bob with step-locked cadence and lateral sway
- [~] Camera minimap with CAM buttons at room positions
- [~] Capture reach and immunity gate
- [~] Capture reach, gate and immunity states not fully confirmed
- [~] Ceryth hiding trap retreat speed and distance
- [~] Ceryth hiding trap: retreat speed and distance hard-coded
- [~] Chunk fixtures: six target spots, every third a vent, five-cell spacing
- [~] Chunk seams: centre-cross openings and wide hall openings
- [~] Coins for Zellia's store: streamed coin source is unconfirmed
- [~] Collision and walkways: cell-based collision, no walkway clearance
- [~] Connected players list: only the local row is listed
- [~] Controls table: one action table with rebinding, hint line, controls tab
- [~] Controls table: one action table with rebinding, hint line, tab
- [~] Crowbar and axe STUNNED callout Godot has switched off
- [~] Crowbar and axe STUNNED callout still printed
- [~] Crowbar and rescue axe: breach, break-free and rescue
- [~] Data-driven region files: mall and hospital JSON embedded but never read
- [~] Death lines: secret spider lines
- [ ] Default bindings: Tab inventory, T chat, O player list, gamepad
- [ ] Difficulty profiles: Nightmare profile
- [~] Director share: flank and cover partial, no Nightmare profile
- [~] Door and wall thresholds: door props, double doors, hump removal
- [ ] Door hold last stage
- [~] Dossier descriptions: no text for Void gimmick chart
- [ ] Dossier text for the Void gimmick chart
- [~] Dossier: black square instead of name for undiscovered hunters
- [ ] Elise closeness bar and wrong-attempt bumps
- [~] Escape struggle loop and knock-back timing
- [~] Escape struggle loop and knock-back timing unconfirmed
- [~] Field of view and sensitivity: 105 vs 110 max
- [ ] Fix settings.rs comment saying graphics quality changes nothing
- [ ] Flashlight flicker and seize after plunge splash or hunter event
- [~] Floor-surface footsteps: surface kind passed
- [~] Furniture hiding capacity: beds and wardrobes hold one or two runners
- [ ] Glyra winder threat
- [~] Guest names flow
- [ ] Haelith counter threat
- [~] Hall light and battery that does not recharge
- [~] Hall light and the battery that does not recharge
- [~] Heater station placement: streamed parity is unconfirmed
- [~] Held keys A, D and F with single flashlight battery
- [~] Held keys A, D and F with the single flashlight battery
- [~] Hide glide animating into the locker
- [~] Hold drop key to drop a whole stack
- [ ] Hunter draw pool: Nightmare weights
- [~] Hunter scale for Night Watch placements via one instantiate path
- [~] Hunter scale: MODEL_SCALE 1.5 through one Night Watch instantiate path
- [~] Hunter speed model: hard-ability and troll scales missing
- [~] Hunters walking the building at a walking pace
- [~] Ilyra corner observer: Nightmare zero-second pounce delay absent
- [~] Ilyra Nightmare zero-second pounce delay
- [~] In-game check of mouse vertical look sign
- [~] Interaction prompt text: wording fixed to keyboard keys
- [~] Javelle hiding timeout reach and stall values
- [~] Javelle hiding timeout: reach and stall values hard-coded
- [~] Join room: password field and network join
- [ ] Left and right vent lights
- [~] Locker hide check still prints callouts
- [~] Locker hide check: callout toasts still print
- [~] Locker inspection breath: no inspect state
- [ ] Loot tables: sewer loot table unused
- [~] Map: waypoint and teammate pins
- [ ] Mask toggle exclusive with monitor, raise and lower timings
- [~] Minimap and full map: teammate and waypoint pins, pan and zoom
- [~] Movement, camera and look tuning values
- [~] Music box wind, tune and run-down
- [~] Music boxes: two per 4x4 region block with a shared tune
- [~] Mutual light sight: hunters see runners by lamp exposure
- [~] Navigation on streamed grid windows
- [~] Noise meter from salvaging and visitor response
- [~] Noise meter with hearing line
- [ ] Opalie monitor-gated threat
- [ ] Open Air Zellia calls from the shared table
- [~] Pause card: RESUME (Esc) and QUIT TO HUB, paused time stopped
- [~] Pause card: RESUME (Esc), QUIT TO HUB, paused time stopped
- [~] Pause menu: SAID RECENTLY and MUTE block, and the save status line
- [~] Per-hunter tuning not compared with Godot roster
- [~] Personal waypoint pin: placement, clear, drawing and guidance
- [~] Placement clearance: door keep-clear zones and push-out
- [~] Player list shows the room's runners
- [~] Player waypoint pin: placement, clear, distance, off-screen arrow
- [~] Port Admin access gate: enabled by display name, with no scope or server check
- [~] Port Automatic run save: no temp file or backup, no pause-quit or pickup saves, no position saved
- [~] Port Carnival booth cues: no booths to play them
- [~] Port Coins for Zellia's store (Godot streamed coin source unconfirmed)
- [~] Port Data-driven content and mod packs: props, hunters and route loot only
- [~] Port Data-driven region files (mall and hospital JSON embedded but never read)
- [ ] Port Death lines: secret spider lines are absent
- [ ] Port Difficulty profiles: Nightmare profile missing
- [ ] Port Field of view and sensitivity: FOV maximum 110 in Rust vs 105 in Godot
- [~] Port Floor-surface footsteps: the finite route passes no surface kind
- [~] Port Guest names: the client has no guest name flow
- [~] Port Heater station placement (Godot streamed placement not located)
- [~] Port Hide server address from players: no scrub of host or port text
- [~] Port Hiding in furniture: lockers only, no beds or wardrobes
- [~] Port Hiding occupancy (capacity, front and beside slots)
- [ ] Port Hunter draw pool: Nightmare weights missing
- [~] Port Hunter grab animations: 16 Godot grab files have no Rust copy
- [~] Port Hunter spawn points per chunk (per-chunk spawn rules differ)
- [~] Port Interaction prompt text: prompt wording fixed to keyboard keys
- [~] Port Join room: password field and network join
- [~] Port Movement, camera and look tuning: pitch clamp, FOV maximum and bob rates differ from Godot
- [~] Port Music boxes (two per 4x4 region block, shared tune)
- [~] Port Night Watch cue family: no scene calls the nw cues
- [ ] Port Pause menu: SAID RECENTLY and MUTE block, and the save status line
- [~] Port Per-level danger profile (hunter speed, count bias, safe-room odds)
- [~] Port Persistent chunk state (checkpoint damage and breaches not stored)
- [~] Port Personal waypoint pin: placement, clear, drawing and guidance
- [~] Port Player waypoint pin: placement, clear, distance and off-screen arrow
- [~] Port Rebindable controls: fixed labels, no rebinding or saved bindings
- [ ] Port Reduced motion option: no setting exists
- [~] Port Room catalogue sizes: room size profiles missing
- [~] Port Room roles per district (Godot role lists vs Rust catalogue tags)
- [~] Port Safe rooms on Infinite route (none generated, safe zone cleared per chunk)
- [~] Port Screen-edge danger effects: thermal warning panel and strength curves
- [ ] Port Settings audio: voice and subtitle rows have no effect
- [~] Port Settings display: vsync, FPS limit, quality, ADS and focus pause have no effect
- [~] Port Stamina and tool tuning: overheat multiplier applied to sprint, and the troll drain missing
- [~] Port Top-centre status bar: test server, admin active and public route segments
- [~] Port Typed co-op message catalogue: round-trip tested but unused in the game path
- [~] Progress save: night_watch.cfg with cleared levels, stars, best scores
- [~] Pursuit and route-following sweep and look-ahead parity unconfirmed
- [~] Quality governor stepping quality down on slow frames
- [~] Quenelle flashlight tracker: Nightmare lock time and ramp
- [~] Quenelle lock time and speed ramp constants
- [~] Ravelle tripwire: wire radius and trap stun hard-coded
- [~] Ravelle wire radius and trap stun hard-coded
- [ ] Reduced motion option
- [~] Relay rush timings and targets against Godot
- [~] Relay rush timings and targets need verifying
- [~] Room catalogue sizes and size profiles
- [~] Run statistics on run card: time, distance, rooms, levels, hunters met
- [~] Safe rooms on the Infinite route: none generated, zone cleared per chunk
- [~] Salvage rack: four slots, three decks, hold to take
- [~] Seeded integer streams: Night Watch mulberry, stream and pacer
- [~] Settings: Calm mode, Scare Full/Short/Off, ERASE progress
- [~] Shared lobby settings, start and end
- [ ] Smooth remote movement with 100 ms delay
- [~] Stamina and tool tuning: overheat and troll drain
- [~] Stuck detection: centre-steer and wedge snap-back
- [~] Sweep and waypoint look-ahead parity unconfirmed
- [~] Taser stun versus taser-immune slow divergence
- [~] Taser stun versus taser-immune slow divergence not revisited
- [~] Thessine mimic callout where Godot plays sound
- [~] Thessine mimic: callout printed where Godot plays sound only
- [~] Title menu: PUBLIC ROUTE button and admin visibility for non-KYTTU admins
- [~] Title screen: PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT, clock
- [~] Title screen: PLAY, CUSTOM NIGHT, ACCOUNT, SETTINGS, CREDITS, QUIT, drifting clock
- [ ] Top-centre status bar: test server, admin active, public route segments
- [~] Typed co-op message catalogue used in game path
- [~] Umbriel disguise pool hard-coded, Nightmare copy absent
- [~] Umbriel shapeshifter: disguise pool hard-coded, no Nightmare copy
- [~] Update docs/PARITY.md items 2 and 3 for keyboard look
- [~] Void Purple radar corrupter: static and blackout rules missing
- [~] Wander and search picking: patrol-anchor branch missing
- [~] Win and death end cards: Next night, Retry, Menu
- [~] Wynelle cold materializer: constants hard-coded
- [~] Zellia merchant: sabotaged relay repair and Nightmare death chance

### Effort M (302)

- [~] Account panel: sign-in and account creation offline only
- [~] Account progress and dossier sync
- [~] Account sign-in and saved session
- [~] Anti-AFK void pressure on idle parties
- [~] Automatic run save: backups and more save points
- [ ] Bedroom view from the pillow: dark 5x5 room with look controls
- [ ] Black wolf spider: harsher faster variant
- [~] Breakables: sealed wings, loose boards and pipes; no interaction
- [ ] Broadcast booth office with stations
- [ ] Bus stop watch with warmth and heat lamp
- [ ] Campfire watch with tent, firewood and two threats
- [ ] Cargo bay office with rack, ledger and two doors
- [ ] Carnival booth cues: booths to play them
- [~] Carnival booths: playable stalls and carousel missing
- [~] Caught moment and scare setting: pose, framing, duration, sound
- [~] Caught moment and scare setting: scare pose, framing, duration, sound
- [ ] Caught players spectate in co-op
- [ ] CHAOS shift events
- [~] Checkpoint and autosave on relays, level changes, safe rooms and quit
- [~] Chunk loot and objectives: stair flights and sealed cells
- [ ] Classic night rules: seven nights, door and light tables, 6 AM win
- [~] Cloud copy of the run save
- [~] Co-op revives: downed state, revive kit hold and window
- [~] Collision model: oriented prop boxes and door gaps
- [ ] Control room office with keypad and task board
- [ ] Corner peek and ambush PEEK state
- [~] Corner peek and ambush: no PEEK state
- [~] Crouch stand-up headroom check and crouch hitbox
- [~] Crowbar and axe: breach boards and doors, cut grabs, rescue teammates
- [ ] Custom Night watch chain
- [ ] Custom Night: per-hunter 0-20 sliders, presets, score, START NIGHT
- [ ] Day shop: five toggle upgrades within the week budget
- [ ] Day shop: five toggle upgrades within the week's budget
- [~] Daylight and LightField exposure in sight limit
- [~] Daylight and LightField exposure missing from sight limit
- [ ] Dead Air shift, nights 1-6 and Custom Night
- [~] Dense bank layout: 11x9 pairs, 21x9 big rooms, hall-centred rows
- [ ] Desk monitor with eight cameras
- [~] Desktop self-update: version parsing only
- [ ] Device-aware control glyphs: gamepad and rebind-aware prompts
- [~] Director flank and cover roles, no Nightmare profile
- [~] District layout matching Godot: seeds and region tags differ
- [~] District selection: weighted districts, deeper-level extras, sewer district
- [ ] Door and vent seal budgets
- [~] Doors and door lights: two steel doors held shut and lit
- [~] Doorways sized for hunter scale: single-cell doors only
- [ ] Dovelle and Ravelle path walkers
- [ ] Drawn threat roster and the six threat types
- [~] Eight-room building and walkable paths
- [ ] ENDLESS ramp and rotations
- [ ] ENDLESS ramp and rotations (Mixed Shift)
- [ ] Facility shift, nights 1-6 and Custom Night
- [~] First-person body, climb hands and held-item grips
- [ ] Five descents and stage order
- [ ] Five weeks with budget, quota and carry-over
- [~] Five-room plant with lift and doorway timing
- [~] Flood valves
- [~] Flood valves: valve focus and flood sync
- [ ] Floodwater wading and soaked state
- [ ] Front office with no door and five stations
- [ ] Full inventory overlay: grid with drag and drop
- [ ] Fuse-box wires puzzle with wrong-button restart
- [ ] Gamepad input: sticks, triggers and pad bindings
- [ ] Gamepad: right-stick look, pad bindings and triggers
- [~] Glyra music box route boxes and peek ambush
- [~] Glyra music box: per-region route boxes and peek ambush missing
- [~] Grab view: arm stretch, camera clearance, runner body
- [ ] Great halls: 21x21 merged middle rooms
- [ ] Hard ability door slam-through
- [ ] Hard ability intercept of the runner's line
- [ ] Hard ability pounce leap
- [ ] Hard ability pounce leap (Hard and Nightmare)
- [ ] Hard ability sprint burst after losing sight
- [~] Hard-ability speed scale and troll scale
- [~] Hiding in furniture: lockers only, no beds or wardrobes
- [~] Hiding occupancy: capacity and front and beside slots
- [~] Hiding props: beds, wardrobes, tables with capacity 1 to 2
- [~] Host-authoritative route sync: protocol exists but run loop does not call it
- [ ] Host-side guest anti-cheat with strikes
- [ ] House wing map: bedroom, hall, living room, galley, kids' room, bathroom
- [ ] Hub office with two doors and three vents
- [~] HUD: shift clock, power meter, status line, prompt line
- [~] Hunter emotes and personalities catalogue
- [~] Hunter navigation on streamed grid windows
- [ ] Hunter pacing: three-second window that slides with AI level
- [~] Hunter pose library: 21 data-driven poses and state table
- [~] Hunter spawn points per chunk: per-chunk spawn rules differ
- [~] Hunters persist across chunks and floors and walk the stairs
- [ ] Ilyra and Aki watched threats
- [~] Inferno spider spawn on level -3
- [~] Interactable stations and prompts: monitor, doors, light, lamp, ledger
- [ ] Inventory screen and backpack upgrades (Tab)
- [ ] Inventory screen on Tab and backpack capacity upgrades
- [~] Kathy search beam affected-summon unported
- [~] Kathy search beam: affected-summon unported in both trees
- [ ] Lamp exposure input to hunter sight
- [~] Late-join world snapshot
- [ ] Level layer beds: sewer and Inferno
- [~] Level stack: sewer (-1), lower sewer (-2), Inferno (-3) with depth floor
- [~] Linear hotel and prison layout along the central cross
- [~] Locker inspection breath needs inspect state
- [ ] Lockpick set and locked caches
- [~] Loxelle incinerator: chase-only room-door breach missing
- [~] Loxelle passive incinerator room-door breach
- [ ] Loxelle, Javelle and Haelith Bedtime behaviour
- [ ] Masked Hours six-night shift and Custom Night sliders
- [~] Masked Hours wing layout: 14 rooms on an AStar3D graph
- [~] Masked Hours wing layout: 14 rooms on AStar3D graph
- [~] Mixed sector layout: quadrant rooms, beach excluded where Godot includes it
- [~] Mod pack loader: size limits and sha256 checks
- [ ] Moderation client (online only)
- [ ] Monitor and camera feeds: raise monitor, switch cameras
- [ ] Monitor and camera feeds: raise the monitor and switch cameras
- [ ] Monitor panel with systems rows and LURE
- [~] Music wing instrument cues: playable props
- [~] Music wing: rehearsal rooms and stage missing
- [ ] Mutual light sight: hunters see runners standing in light
- [ ] Night 7 Kathy alone: hum at doors, eye-meet on hall camera, fade
- [~] Night Watch cue family: scenes call them
- [~] Night Watch sound mapping: event-to-cue and room tones
- [~] Night Watch sound mapping: event-to-cue table and room tones
- [ ] Nightly task list with six task types
- [ ] Nightmare difficulty scaling profile for teams and pounces
- [ ] NPC teammate lines: dialogue data
- [~] NPC teammates
- [~] NPCS admin page: NPC commands
- [~] Nyxie and Solyth Nightmare pounce branches
- [~] Nyxie darkness stalker Nightmare pounce
- [~] Nyxie darkness stalker: Nightmare pounce missing
- [ ] Office contract: one walkable office per mode with stations
- [~] Online backend client compiled and wired
- [ ] Opalie in the gallery
- [~] Open grounds map and waypoint paths
- [~] Patrol-anchor branch in wander and search picks
- [~] PEEK and INSPECT states in the brain
- [ ] Per-floor draw Nightmare weights
- [~] Per-hunter tuning numbers not compared with Godot roster
- [~] Per-level danger profile: hunter speed, count bias, safe-room odds
- [ ] Per-mode map registry, validated against Classic
- [~] Persistent chunk state: checkpoint damage and breaches are not stored
- [ ] Phantoms breaking systems and reboots
- [ ] Pipework puzzle with flow check
- [ ] Porcelle and the beacon
- [ ] Port Account panel: sign-in and account creation are offline only
- [~] Port Account progress and dossier sync: discoveries are kept locally only
- [~] Port Account sign-in and saved session: the client code is not compiled
- [~] Port Ambient beds: silent on the finite route, streamed-route beds missing
- [~] Port Auto-updater: no download, verify or apply code (version parsing only)
- [~] Port Breakables (sealed wings, loose boards, pipes; no interaction)
- [~] Port Carnival booths (playable stalls and carousel missing)
- [ ] Port Caught players spectate in co-op
- [~] Port Chunk loot and objectives (stair flights and sealed cells differ)
- [~] Port Chunk seams (centre-cross openings, wide hall openings)
- [~] Port Cloud copy of the run save: only the local checkpoint is used
- [~] Port Collision and walkways: cell-based collision, no walkway clearance
- [~] Port Data-driven base content loader: regions, rooms and other kinds not loaded
- [~] Port Desktop self-update: version parsing only
- [ ] Port Device-aware control glyphs: gamepad and rebind-aware prompts
- [~] Port Doorways sized for hunter scale: single-cell doors only
- [ ] Port Download missing mod packs from the index
- [ ] Port Full inventory overlay: grid with drag and drop
- [ ] Port Guest waits for the synced world before moving
- [~] Port Host hunter state broadcast
- [~] Port Host-side guest anti-cheat: server moderation only, no guest validator
- [ ] Port Host-side guest anti-cheat: speed, teleport and noclip checks with strikes
- [~] Port Hunter emotes and personalities: no emote catalogue
- [~] Port Late-join world snapshot
- [ ] Port Level layer beds: no sewer or Inferno beds
- [ ] Port Linear hotel and prison layout along the central cross
- [~] Port Mixed sector layout (quadrant rooms, beach excluded where Godot includes it)
- [~] Port Mod pack loader: only hunters, props and route loot load; no size limits or sha256 checks
- [ ] Port Moderation client: online only
- [~] Port Music wing instrument cues: no playable props
- [~] Port Night Watch content: no data in Rust
- [ ] Port NPC teammate lines: no dialogue data
- [~] Port NPCS admin page: NPC teammates are not in the Rust port
- [~] Port Owner moderation review queue and flags (placeholder in the admin hub)
- [ ] Port Persistent public route client: server side is ported
- [~] Port Positional emitters: waterfall, flood and lava loops missing
- [~] Port Prop collision boxes: solidity is cell-based, with no oriented boxes
- [ ] Port Public route featured hunters: featured pair and weighted lineup missing
- [ ] Port Public route join from the title screen: client not in the build
- [~] Port Reserved private rooms: owner tools have no client code
- [~] Port Room chat, speech bubbles and subtitles
- [~] Port Room furnishing recipes (orientation, seat-to-screen logic, room size profiles)
- [~] Port Room staging: online host controls, chat and route sync
- [ ] Port Same mod packs as the host: pack hash announce and check
- [ ] Port Server update manifests client flow (server side is ported)
- [ ] Port Settings controls: rebinding is not possible, and the CHANGE, + and RESET buttons are disabled
- [~] Port Sewer, waterworks and ladder or ramp level stack: stairs only
- [~] Port Shared lobby settings, start and end: guests receive no settings
- [~] Port Shared world events and interaction requests
- [~] Port Smooth remote movement: snapshot interpolation with 100 ms delay
- [ ] Port Spectating banner and ghost follow
- [ ] Port Spectators and ghost runners: no ghost state
- [ ] Port Spoken chat through OS text to speech: no speech output
- [~] Port Stairwells (spiral and switchback flights, placement, access halls)
- [ ] Port Startup gate and update check: no check, retry or minimum splash
- [~] Port Surface themes: surfaces are painted in code, with no theme data
- [~] Port Text chat (team and nearby)
- [~] Port Title menu: PUBLIC ROUTE button and admin visibility for admin accounts other than KYTTU
- [ ] Port Update-in-progress screen
- [~] Port Vent network (ducts and crawl movement missing, grates only)
- [ ] Port Voice packet codec (IMA ADPCM)
- [~] Positional emitters: waterfall, flood, lava loops
- [~] Power drain, blackout and Nyxie arrival
- [ ] Public route featured hunters and lineup
- [ ] Public-route patrol around a live runner: no patrol anchor
- [ ] Quenelle and the lift
- [~] Radio station map and waypoint graph
- [ ] Rebindable controls with saved bindings
- [~] Reserved private rooms and owner tools
- [ ] Revive kit pickups collected
- [ ] Rooftop watch with torch, barriers and lantern oil
- [~] Room climate offsets: per-room profiles instead of per-region estimates
- [~] Room furnishing recipes: orientation, seat-to-screen logic, room size profiles
- [~] Room roles per district: Godot role lists differ from Rust tags
- [~] Room staging: online host controls, chat and route sync
- [~] Safe rooms with breach and checkpoints
- [ ] Same mod packs: pack hash announce and check
- [ ] Screen-edge danger effects: thermal warning panel and strength curves
- [ ] Sealing vents and air stress
- [ ] Security room: walkable office with desk, monitor and stations
- [~] Seeded replayable shifts with golden fixtures for each mode
- [ ] Server update manifests client flow
- [~] Service elevator view with Elise waiting at the sill
- [~] Settings audio: voice and subtitle rows have no effect
- [~] Settings controls: rebinding not possible; CHANGE, +, RESET disabled
- [~] Settings display: vsync, FPS limit, quality, ADS, focus pause have no effect
- [~] Sewer layer: separate generator, ramps, falls, ladders and floods
- [ ] Shift board: nine mode rows, cleared counts, portraits, START SHIFT
- [ ] Ship and scrap yard map
- [ ] Signal panel memory puzzle
- [ ] Sit, lie and ride on furniture
- [ ] Sit, lie and ride on furniture (use_furniture, rest spots)
- [ ] Six Bedtime nights and the Custom Night row
- [ ] Six-night Masked Hours shift and Custom Night sliders
- [~] Sleep and drowsiness, hunters stirring and jolting the sleeper
- [~] Sleep and drowsiness: hunters stir or jolt a sleeping runner
- [ ] Sleep meter (creep) and restless house cues
- [ ] Spectating a downed or ghost runner
- [ ] Spectating and ghost follow
- [ ] Spectating banner and ghost follow
- [ ] Spectators and ghost runners: no ghost state
- [~] Spoken chat through OS text to speech: no speech output
- [ ] Squall and shower weather schedule
- [~] Stairs walked in Godot, not teleported; spiral and switchback flights
- [ ] Stairs walked rather than teleported to the landing
- [~] Stairwells: spiral and switchback flights, placement and access halls
- [~] Startup gate and update check
- [~] Stuck detection: centre-steer and wedge snap-back missing
- [ ] Surface themes from theme data
- [ ] Tells before every kill and their pulse sounds
- [~] Temperature, heater and fan climate rules
- [~] Text chat (team and nearby)
- [~] Text-to-speech chat for nearby players
- [ ] Thessine walking in from the hall
- [ ] Transit hub map and vent run
- [ ] Umbriel the rusher
- [ ] Update-in-progress screen
- [ ] Vent hunters Verrine, Haelith and Ilyra
- [ ] Verrine vent crawler threat
- [~] Verrine vent lurker: physical duct crawl and vent events missing
- [ ] Visitors at the side doors (Ravelle, Ceryth) with warnings
- [ ] Voice packet codec (IMA ADPCM)
- [ ] Void Black rear stalker: no gimmick, freeze and unwatched rules
- [ ] Void Blue silent stalker: beam reveal and map exclusion
- [~] Void Brown jam hiding spots
- [~] Void Brown spot jammer: jam hiding spots
- [ ] Void Cyan runner position broadcast to hunters
- [ ] Void Cyan signal ping: runner position broadcast to hunters
- [~] Void Gray anti-idle signal route: networked flow missing
- [ ] Void Green vent submerge, uncatchable two seconds
- [ ] Void Green vent submerge: hidden and uncatchable two seconds
- [~] Void incursions: no incursion chance, void hunters rarely drawn
- [~] Void Orange outage state for runner systems
- [ ] Void Orange system outages: no outage state for runner systems
- [~] Void Purple radar static and blackout rules
- [ ] Void Red hall and room speed multipliers
- [ ] Void Red hall runner: hallway and room speed multipliers
- [ ] Void White light-frozen: lit torch freeze rule missing
- [ ] Void White lit-torch freeze rule
- [ ] Void Yellow map pressure while map is open
- [ ] Void Yellow map pressure: speed grows while map is open
- [~] Wander, idle, investigate, chase, search: PEEK and INSPECT states
- [ ] Watch post office in three variants
- [~] Waypoints: one per player, route chevrons and ground trail
- [~] Win and death end cards: Next night, Retry and Menu
- [ ] Wolf spider pair shared rest and recall
- [ ] Wolf spider pair shared rest and recall logic
- [ ] Xivvy and the lure mechanic
- [~] Xivvy chase accelerator: clearance-driven flight and landing missing
- [~] Xivvy clearance-driven flight and landing
- [ ] Ysolde at the window: knock and quiet rule
- [~] Zellia calls for nights 1-6 (Dead Air)
- [~] Zellia sabotaged relay repair and Nightmare death chance
- [~] Zellia scripted calls for nights 1-6 (Masked Hours)
- [ ] Zellia's call box: ring, typed lines, subtitles, queued calls
- [~] Zellia's call box: ring, typed lines, subtitles, queued scripted calls
- [ ] Zellia's calls for nights 1-6 (Dead Air)
- [ ] Zellia's Facility calls
- [ ] Zellia's scripted calls for nights 1-6

### Effort L (70)

- [~] Ambient beds on streamed route
- [~] Auto-updater: download, verify, apply
- [ ] Carnival booths and carousel rides
- [ ] Carnival booths and carousel rides as interactions
- [ ] Co-op revives: downed runners, revive hold and window
- [~] Data-driven base content loader: rooms and kinds
- [~] Data-driven content and mod packs: props, hunters and route loot only
- [~] District layout matching Godot seeds and region tags
- [ ] Download missing mod packs from the index
- [ ] Emotes, blocky face, doorway pose and player skins
- [ ] Five shifts: QUIET, STEADY, RESTLESS, CHAOS and ENDLESS
- [~] Fix Infinite map to match Godot layout, districts, stairs, population
- [~] Gameplay UDP relay client tunnel (ENet proxy)
- [ ] Godot-compatible co-op wire (ENet and RPCs)
- [ ] Host hunter state broadcast
- [~] Host-side guest anti-cheat: server moderation only, no guest validator
- [~] Hunter AI: stalkers, door queue and pair speed-up combos
- [ ] Hunters nap in beds and wake on a meter
- [ ] Hunters take ladders and stairs to other levels
- [ ] Hunters use ducts on Hard and Nightmare
- [~] Inferno caverns and cage pit: no carving or lava
- [~] Inferno caverns: cave carving, lava pits, cages and spider lair
- [~] Infinite district layout matches Godot's streamed route generator
- [~] Level links other than stairs: ladders, ramps, manholes, grates, elevators, drops
- [ ] Night Watch content data
- [~] Night Watch game shell, mode registry and save progress: placeholder binary only
- [ ] Night Watch nine modes: none ported
- [ ] Nightmare difficulty scaling profile
- [~] Online route sync in the run loop
- [ ] Owner moderation review queue and flags
- [ ] Pacing director: beats, spikes and moods
- [ ] Persistent public route client
- [~] Port Gameplay UDP relay client tunnel: no ENet proxy in the Rust client
- [ ] Port Godot-compatible co-op wire (ENet and SceneMultiplayer RPCs): Rust and Godot cannot share a room
- [~] Port Inferno caverns (cave carving, lava pits, cages, spider lair)
- [ ] Port Inferno caverns and cage pit: no carving or lava
- [~] Port Level stack (sewer -1, lower sewer -2, Inferno -3, floor at -3)
- [~] Port Night Watch game shell, mode registry and save progress: placeholder binary only
- [ ] Port Night Watch nine modes: none ported
- [~] Port NPC teammates with dialogue, text chat with TTS, full online mode
- [~] Port Proximity voice chat
- [~] Port Proximity voice chat: wire messages exist but no capture or playback
- [~] Port Sewer layer (separate generator, ramps, falls, ladders, floods)
- [ ] Port Voice, spoken chat and text-to-speech: no speech code
- [~] Prop collision boxes: oriented, not cell-based
- [~] Proximity voice chat
- [~] Proximity voice chat: wire messages exist but no capture or playback
- [ ] Public route join from title screen
- [ ] Public-route patrol around a live runner
- [ ] Room chat, speech bubbles and subtitles
- [~] Sewer levels -1 to -3 with cistern, collector and Inferno maze
- [~] Sewer, collector and Inferno levels with own tunnels and ladders
- [~] Sewer, waterworks and ladder or ramp level stack: stairs only
- [ ] Shared shift framework: NightWatchMode base, hand-over, mode registry
- [ ] Shared world events and interaction requests
- [ ] Shift scene contract and session hand-over (NightWatchMode)
- [ ] The Facility shift, nights 1-6 and Custom Night
- [ ] Three outdoor watches: Rooftop, Bus Stop and Campfire
- [~] Vent network: ducts and crawl movement missing (grates only)
- [~] Verrine physical duct crawl and vent events
- [~] Voice, spoken chat and text-to-speech
- [ ] Void Black rear stalker gimmick and speed rules
- [ ] Void Blue beam reveal and map exclusion
- [~] Void Gray networked signal route flow
- [~] Void incursions and void_*.json hunters never drawn
- [ ] Void player powers: Gray signal route, Orange outages, Purple static, Pink footsteps
- [~] Void player powers: Gray, Orange, Purple and Pink effects
- [ ] Void player powers: signal route, outages, radar static, false footsteps
- [ ] White wolf spider lurk, bark, hunt and retreat brain
- [~] White wolf spider lurk, bark, hunt, retreat

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

Sources: the website game at KyttusLab/games/night-watch (src/modes, docs/ROADMAP.md, docs/LAST_ROUTE_MATCH.md) and the Godot version at LastRoute Godot/KyttusLabApp/game/night_watch. The Rust binary (src/bin/night_watch) is a stub: 28 lines and no game yet. Each item says which version it ports from.

### Shell and shared framework

- [ ] Rust Night Watch app shell: window, title, hub menu with the eight night types, and quit (website: index.html hub; Godot: night_watch_menu.gd)
- [ ] Shared roster: the 26 Last Route characters with their roles (website: docs/LAST_ROUTE_MATCH.md; Godot: night_watch_data.gd)
- [ ] Shared save and progress across night types, using the same account store as Last Route (website: account; Godot: night_watch_game.gd)
- [ ] Mode registry: each night type registered once, with its name, unlock rule and entry scene (website: src/modes/registry.js; Godot: night_watch_mode_registry.gd)
- [ ] Shared HUD, timers, power and camera-map layout (website: ui/; Godot: night_watch_hud.gd, night_watch_cam_map.gd)
- [ ] Night Watch art and hunter scale through one instantiate path (Godot: night_watch_art.gd, night_watch_hunter_scale.gd)
- [ ] Night Watch sound cues wired to the scene code (Godot: the nw cues; the Rust nw.rs cues are not called yet)
- [ ] Admin and debug panel for Night Watch, with the same owner-only gate as Last Route (Godot: night_watch/admin; website: debug-private)
- [ ] Office building layout with doors, lights and cameras (website: docs/OFFICES.md; Godot: night_watch_building.gd)

### The eight night types

- [ ] 1. Classic: static office, 12 to 6 AM, two doors, two lights, cameras, power budget (website: src/modes/classic; Godot: night_watch_classic.gd)
- [ ] 2. Masked Hours: mask to fool attackers, flashlight, one resource to keep alive, vents (website: src/modes/mask; Godot: mask/)
- [ ] 3. Dead Air: audio lures, reboot subsystems, hallucinations, vent seal, temperature (website: src/modes/lure; Godot: lure/)
- [ ] 4. Bedtime: bedroom with no cameras, listen at doors, breathing check (website: src/modes/bedroom; Godot: bedroom/)
- [ ] 5. The Facility: scripted nights with guided hallway, vent and crawl sections, and a shock button (website: src/modes/story; Godot: night_watch_story or the admin story scenes)
- [ ] 6. The Descent: elevator shaft repair puzzles under a hunter's eye (website: src/modes/descent; Godot: descent/)
- [ ] 7. Salvage Run: day management and night salvage with an upgrade layer (website: src/modes/dayshift; Godot: dayshift/)
- [ ] 8. Open Air: three outdoor watches with the roster outside the building (website: src/modes/openair; Godot: night_watch_mixed.gd or the open-air data)
- [ ] Mixed Shift: a drawn roster of threats paced by a director, with QUIET, STEADY, RESTLESS, CHAOS and ENDLESS shifts (website: src/modes/mixed and docs/MIXED_SHIFT.md; Godot: mixed/)
- [ ] Variety pass: make the existing modes less repetitive (website: docs/VARIETY.md)
- [ ] New modes drafted on the website (website: docs/NEW_MODES_DRAFT.md): decide which to port

### Checks

- [ ] Each night type reviewed in a run against the website for timing, power and AI levels (website is the reference for feel; Godot for numbers)
- [ ] Night Watch save migration: website saves read into the Rust app without loss

## Multiplayer (Last Route)

- [ ] Two-window room test: one host and one guest on this laptop, room code join, both see each other move
- [ ] Two-machine test: the Mac on KT-MBA joins a room hosted on this laptop, and the reverse
- [ ] Nightly server build: deploy the nightly-server Linux build to the VPS (194.156.90.219) after a check on a test port
- [ ] VPS update procedure: stop the lastroute-rust service, replace /opt/lastroute-rust/current/lr_server, check the SHA-256, start the service
- [ ] Hunter stats stay hidden from other players in a room (the stats check from the open questions, Q24)
- [ ] Room directory: list open rooms, show capacity, and refresh without reconnecting
- [ ] Reconnect: a player who drops can rejoin their room within the reserved window
- [ ] Host migration or room close when the host leaves, with a clear message to the others
- [ ] Co-op revives: downed runners, a revive hold, and a time window (open in the audit)
- [ ] Spectate a downed or ghost runner (the SPECTATE admin button has no handler yet)
- [ ] Player list shows every connected player, not only the local row
- [ ] Runner sync checked at 20 Hz under 100 ms of latency
- [ ] Hunters and their state are identical on every client in a room (the host simulates; guests display)
- [ ] NPC teammates visible to other players in the room
- [ ] Chat: text chat between players in a room
- [ ] Player and room limits: capacity, kick and ban for the host, with the existing moderation rules
- [ ] Multiplayer error messages for refused joins, timeouts and a server that is offline

## Admin custom rooms (2026-11-03)

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
- [ ] Admin accounts: the owner's account is the only one with admin tools, and admin status is checked on the server, not only in the client
- [ ] Rate limits on sign-in and sign-up, with clear messages when they trip

### Servers (lr_server, VPS)

- [ ] VPS deploy: replace /opt/lastroute-rust/current/lr_server from the nightly-server release, check its SHA-256, restart lastroute-rust.service
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
- [ ] Renderer check on the Intel GPU: confirm the backend miniquad uses on macOS (OpenGL profile, not Metal) and whether it is the bottleneck
- [ ] Quality AUTO picks a tier from the GPU and the first second of frames on this Mac, instead of a fixed 50% render scale for every Mac
- [ ] Quality governor: step the render scale down on slow frames and back up when there is headroom (Godot quality_governor.gd: 40/50/60 targets, four notches)
- [ ] Lamp shader cost: the fragment shader loops over 16 lamps plus the flashlight for every pixel; test 8 lamps and distance culling on Intel
- [ ] Per-material lamp sort: shader::apply sorts the lamp list for every material each frame; compute it once per frame
- [ ] Draw calls: count meshes per floor and per frame in a run, and merge meshes that share a material
- [ ] Triangle budget: GLB prop and character models, measured on this Mac; decimate or swap the heaviest ones
- [ ] Texture sizes and count: check the atlas and floor set sizes against the Intel GPU memory budget
- [ ] Vsync and frame cap: the window starts with vsync on; check whether the FPS limit setting (0 = off) helps tearing and power on this Mac
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
- [ ] Path cost readout: path length, cells searched and time for the last path search, from find_path in src/world/facility.rs
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
- [~] WORLD tab: regenerate the current floor from its seed and show the seed on screen
- [ ] STORY tab: jump to a story beat by its id, and show which beats have already fired. Blocked: the game has no story beat table or fired-beat state yet (see the pacing director item)
- [ ] Admin tools only enable for the signed-in KYTTU account (see the login reset), with a clear message when they are off
- [ ] Admin menu screenshot checks at 1280x720 for every tab, so layout bugs show up in review

### CI and build speed

- [ ] CI speedup: cut the four separate cargo test invocations in test.yml to one run where the crate graph is shared
- [x] CI speedup: cache the Windows target directory across runs in build.yml and test.yml (rust-cache keys per job)
- [x] CI speedup: run the Linux and macOS build jobs in parallel with the Windows build instead of one after another
- [x] CI speedup: skip the release build in test.yml when only docs or TODO.md changed (paths-ignore)
- [ ] CI speedup: use prebuilt tools (cargo-binstall) rather than building any CLI tool in CI
- [ ] Faster GitHub compilations: a dev-profile setting for CI that trims debug info (debug = "line-tables-only")
- [ ] Faster compilations on this PC: measure a cold build with opt-level 1 for dependencies, record the time, then decide
- [ ] Faster compilations: split the server-only dependencies from the game build (done with the server feature; check the CI cache keys match)
- [ ] Faster compilations: check linker speed on Windows (rust-lld) and record the time it saves

## Godot parity: found by discovery (2026-11-03)

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

- [ ] Beds stick out of rooms into the hallway behind them (in progress: workflow bed-placement-fix; see furnished.rs bed placement)
- [ ] Dossier: the mouse wheel scroll jumps back to the top. Not reproduced from the code; needs the tab and steps from the person testing it
- [ ] Door headers above doors (added in the 3D map pass) are not checked in a run, and may overlap archway lintels
- [ ] Safe zone checkpoint light and the 0.35 ambient are checked in one screenshot only; confirm in a safe zone and in a dark corridor
- [x] Release build fails while last_route.exe is running (file lock). Close the game first, or build to a copy
- [ ] Windows CI: the server test step compiles slowly on the runner and was cancelled once; confirm it finishes on a normal run
- [ ] Windows CI: the self-test runs locally only (hosted runners have no OpenGL pixel format)
- [ ] Account screen kicker still reads "KYTTU'S LAB // ACCOUNT"; change it to "KYTTU: THE LAST ROUTE"
- [ ] Chairs face their tables: the code matches Godot, but no restaurant render has been checked (OPEN_QUESTIONS 21)
- [ ] Ambient is 0.35 here against Godot's 0.035, a deliberate change; decide whether to keep it or match Godot
- [ ] Mouse look speed is unchecked against Godot with the same swipe (OPEN_QUESTIONS 20)
- [ ] Two-window multiplayer room test still needs a person at the screen (KT-MBA)
- [x] lr_server: some helpers are only used by tests and are gated with cfg(test); delete the ones nothing needs

## Performance and FPS

- [ ] Window recentre hitch of about 80 ms per recentre: about 55 ms is the full world mesh rebuild (build_world on every floor change in sync_world); build the mesh per chunk instead
- [ ] Chunk window load of about 23 to 28 ms per recentre (infinite::window generation, residency, sync_in); move it off the frame or build the next window ahead
- [ ] No quality governor or frame budget. Godot has quality_governor.gd (40/50/60 fps targets, tiers for shadows, prop range and lamp count); add the same tiers
- [~] Hunter cost is untested at high counts (bench added: cargo test --release hunter_brain_step_cost -- --ignored --nocapture; not yet run). Godot measured brain.update at 31.8 ms with 38 hunters; bench the Rust brains at 38
- [ ] Light sight cost is unknown; MAX_LAMPS is capped at 16 in the shader, so lamp pools can pop in and out
- [ ] Steady-state render cost is not measured because vsync caps frames at 60; add a capture with vsync off
- [ ] Startup: assets take about 195 to 208 ms and sounds about 249 to 266 ms. Load in slices behind the boot card, as Godot's STARTUP_BUDGET_MS does
- [x] Draw calls per frame are not counted in a run; use the F7 counters and log the totals
- [x] The F7 overlay averages are not collected over runs; log spikes with a cause, as Godot's PerfLog does
- [ ] Clean build time: profile.dev sets opt-level 2 for the game and 3 for dependencies. Measure opt-level 1 for dependencies on a cold build
- [ ] CI builds: confirm the rust-cache hits in build.yml after the first run
- [ ] Reach 144+ FPS on this laptop (AMD Ryzen AI 5 340, integrated Radeon 840M, 1280x720) with vsync off: run the --stress route, record the average and the 1% low, then fix the top cost in each frame section until the 1% low reaches 144
- [ ] Godot vs Rust performance comparison: the same seed, route and hardware for both builds (KT-MBA and this PC). Record frame time (average, 1% low), startup, memory, hunter cost at 38 and draw calls, from the Rust --stress CSV and the Godot F7 overlay. Keep both tables in docs/PERF_COMPARISON.md and tick each number when measured.

## Moddability

- [~] Region geometry block (lr.region/1, optional): wall_height, room_min and room_max are parsed and tested. The 3D walls and the route room sizes do not read them yet
- [ ] Godot's REGION_FIELDS must accept geometry, or Godot drops region files that use it
- [ ] Mod-authored props with their own GLB models, through the props catalogue
- [ ] Mod-authored loot tables per region
- [ ] Mod-authored safe-zone light colour and strength per region
- [ ] A pack validator command that checks a pack folder without starting the game
- [ ] docs/MODDING_SPEC.md is a placeholder: write the region, hunter, prop and pack formats

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
- [x] Port District selection: weighted districts, deeper-level extras and a sewer district
- [ ] Port Sewer layer: separate generator, ramps, falls, ladders and floods
- [ ] Port Inferno caverns: cave carving, lava pits, cages and the spider lair
- [ ] Port Per-level danger profile: hunter speed, count bias and safe-room odds by depth
- [x] Port Maze warrens: irregular cell layouts for maze and lab chunks
- [x] Port Dense bank layout: 11x9 pairs, 21x9 big rooms and hall-centred rows
- [ ] Port Linear hotel and prison layout along the central cross
- [ ] Port Mixed sector layout: quadrant rooms, with beach excluded where Godot includes it
- [x] Port Apartment suites with shared suite entrances
- [x] Port Great halls: 21x21 merged middle rooms
- [ ] Port Room roles per district: Godot role lists differ from Rust catalogue tags
- [ ] Port Data-driven region files: mall and hospital JSON are embedded but never read
- [ ] Port Room furnishing recipes: orientation, seat-to-screen logic and room size profiles
- [x] Port Door and wall thresholds: door props, double doors and hump removal
- [x] Port Collision model: oriented prop boxes and door gaps
- [x] Port Placement clearance: door keep-clear zones and push-out
- [x] Port Chunk fixtures: six target spots per chunk, every third a vent, five-cell spacing
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
- [x] Port Mutual light sight: hunters see runners by lamp exposure
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
- [x] Port Hunter persistence across chunks and floors: stair walking and per-hunter state
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
- [x] Port Waypoints and route trail

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
- [x] Port Fullscreen map: pan, zoom and recentre
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
- [x] Port Runner movement sync at 20 Hz
- [ ] Port Smooth remote movement: snapshot interpolation with 100 ms delay
- [x] Port Other players' bodies on screen
- [ ] Port Host hunter state broadcast
- [ ] Port Shared world events and interaction requests
- [ ] Port Late-join world snapshot
- [x] Port Guest waits for the synced world before moving
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
- [x] Port Maze warrens: irregular cell layouts for maze and lab chunks
- [ ] Port Dense bank layout: 11x9 pairs, 21x9 big rooms and hall-centred rows
- [ ] Port Linear hotel and prison layout along the central cross
- [ ] Port Mixed sector layout: quadrant rooms, with beach excluded where Godot includes it
- [x] Port Apartment suites with shared suite entrances
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

- [x] Port Hunter persistence across chunks and floors: stair walking and per-hunter state
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
- [x] Port Waypoints and route trail

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
- [x] Port Fullscreen map: pan, zoom and recentre
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
- [x] Port Runner movement sync at 20 Hz
- [ ] Port Smooth remote movement: snapshot interpolation with 100 ms delay
- [x] Port Other players' bodies on screen
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

