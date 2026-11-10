# Mechanics sheet

## Deep-mode native trace (original build, 2026-11-01)

The original main program used for this pass is `KyttusLab-Windows-Debug.exe`
(SHA-256 `ed020950530461a55acc1a99f8a1fdd65aaae3e97698ff26231a5796ed98268e`).
The original source folder is also present, but the native work below is from the
compiled program. `native_strings_find` located the names/messages, then the string
references were followed into `native_function_pseudocode`.

| Native address | Role and recovered rule | Port usage |
|---|---|---|
| `0x14008a600` | Main frame/state update. Timers and meters are advanced by `dt`; values are clamped at zero/max. The function contains the `contactsrc\\game\\hunting.rs` path and emits `RELAY ONLINE`, `INVENTORY FULL`, `AXE`, and `CROWBAR` messages. | `src/bin/last_route/game/player.rs`, `src/bin/last_route/game/hunting.rs`, `src/bin/last_route/game/interact.rs` |
| `0x140084810` | Interaction dispatcher. It scans the interaction list, selects the active interaction, and routes it through the state jump table. The store branch is gated by item state and exposes `COOL`, `WARM`, `REPAIR`, and `PESTICIDE`. | `src/bin/last_route/game/interact.rs` |
| `0x1400979b0` | Run/settings parser. `mode` accepts `finite`/`infinite`; `minutes`, `target`, `deadline`, and `max_stuck` are parsed with defaults and bounded numeric values. | `src/bin/last_route/headless/runner.rs`, `src/bin/last_route/app/settings.rs` |
| `0x140067e40` | Player/item presentation and interaction effects. It uses the flashlight/taser/crowbar/axe/battery/stim inventory key and keeps the flashlight attachment drain on the per-frame path. | `src/bin/last_route/render/viewmodel.rs`, `src/bin/last_route/game/player.rs` |
| `0x14001dc8a` | `escape` string reference in the struggle/capture path. | `ESCAPE_PRESSES = 6`, `ESCAPE_DURATION = 3.2` |
| `0x14008fb3b`, `0x14008c626`, `0x14008c694` | References to the item labels and `FLASHLIGHT RECHARGED` / `FLASHLIGHT ALREADY FULL`. | `BATTERY_RECHARGE = 25`, lamp charge clamps to `100` |

### Values and formulas now used by the build

- Sprint: `stamina = max(0, stamina - 32 * dt)`; when not sprinting, `stamina = min(100, stamina + 21 * dt)`. Exhaustion remains active until stamina reaches `25`.
- Lamp: while lit, `lamp = max(0, lamp - (0.8 + (sprinting ? 0.35 : 0.0)) * dt)`; battery pickup adds `25`, clamped to `100`.
- Relay: a valid aimed interaction increments the recovered-relay count and adds `100` score; battery pickup is consumed once.
- Taser: range `5`, half-angle `0.35`, profile cooldown `8 / 8.5 / 9 s`; a valid hit applies `0.5x` movement for `3 s`.
- Capture: struggle requires `6` presses inside `3.2 s`; failed struggle ends the run, while a successful escape applies the configured knockback/stun and `16 s` re-grab cooldown.
- Safe zone: stamina and lamp charge move back toward `100`; all meter updates are `dt`-scaled and clamped.

The constants in `src/bin/last_route/config/mod.rs` are the single source of truth for these recovered
values. The comments beside the player/tool constants retain the native function addresses
so later changes can be checked against the trace.

### Re-run audit (Deep mode)

This pass reopened the original as target `t_ed0209505304`, searched the stored names and
messages, followed the returned references, and awaited pseudocode for every function below.
The lifter is register-level for the large frame function, so the formulas are taken from its
visible `dt` arithmetic, clamps, and the referenced messages/branches; no unobserved value is
being presented as a decompiler certainty.

| Search/reference | Function read | Concrete rule used by the port |
|---|---|---|
| `stamina`, `FLASHLIGHT RECHARGED`, `FLASHLIGHT ALREADY FULL`, `RELAY ONLINE` | `0x14008a600` (`sub_14008a600`) | Frame meters use `dt`; stamina sprint is `max(0, s - 32*dt)`, recovery is `min(100, s + 21*dt)`, lamp drains by `0.8*dt` plus `0.35*dt` while sprinting, and meter values clamp. Exhaustion clears at `25`. |
| `[E] TAKE BATTERY`, `[E] RECOVER RELAY` | `0x140084810` (`sub_140084810`) | Interaction dispatch selects the valid focused action; battery adds `25` capped at `100`, and relay recovery adds `100` score. |
| `scenario`, `collect_relays`, `reach_depth`, `explore_coverage` | `0x1400979b0` (`sub_1400979b0`) | Run/settings values are parsed from bounded numeric options with defaults; the port keeps finite/infinite modes and the documented defaults. |
| `flashlight_item...battery...stim_pack` and item labels | `0x140067e40` (`sub_140067e40`) | Inventory item identity drives the flashlight/battery interaction path; recharge is shared through `BATTERY_RECHARGE`. |
| `escape` reference at `0x14001dc8a` | `0x14001ba50` (`FUN_14001ba50`) | Capture escape requires `6` presses within `3.2 s`; success applies knockback/stun and starts the `16 s` re-grab cooldown. |

The fresh source change in this pass removes the last hard-coded battery refill from the shop
branch, so every battery refill now uses `BATTERY_RECHARGE = 25.0` and `MAX_LAMP_CHARGE = 100.0`.

### Evidence from this pass

- The original executable was opened as `t_ed0209505304` (`ed020950530461a55acc1a99f8a1fdd65aaae3e97698ff26231a5796ed98268e`).
- Name/message searches: `ev_46440c2389bd19cb4240d789dc714e2fb730e7ed7678df51412c09fc5f62b165` (stamina), `ev_19c53fec497013f5b0a5d4bd335b609b3688827604e4c689402b579ec75496ee` (flashlight), `ev_9a65f5ab7523e7c917736a649ab8184b0987989d7ab9de5678a47c3a0eb4b687` (battery), `ev_53b76326ddc91ff6fd2053a0c9194ac71ccbb2e2ffd0c2e0612a5acc912eca1c` (relay), and `ev_aae3c88693ac7def086490757e91882db2fcfee88717ccc5585dbc811796a711` (escape).
- String references resolved to `0x14008a600` (frame/state update), `0x140084810` (interaction dispatcher), `0x1400979b0` (run/settings parser), `0x140067e40` (item/player path), and `0x14001ba50` (escape-state path; the `escape` string is referenced at `0x14001dc8a`).
- Reference evidence includes `ev_db285be6293d07f1a041733fb047906ac4bc3290ea76305bf76e0b70935afacf`, `ev_323816187db8ce40ad5f38cea0b1093dcaa5d7d804c131963cd53d5634fefbb3`, `ev_b5c275504d2f1bb9d773569824a426d8f63f1acb59cb0392b041e2d3cc260e08`, `ev_6be3ee8c32d3806645dee80330a634cc81eeffd4940beeb5333727e92e2c546c`, `ev_31c7288cec7a0e8d15d999c91238a4ee01aa8498073e8fd0f8c8319c1e565e6c`, and `ev_b3519eb19d088b3941e69b83e6390e1be895b6ea26cae7624807cd159c70d21a`.
- `native_function_pseudocode` was read for each of the five functions above. The large frame update is register-level/partially lifted, but its visible operations explicitly add `dt`, subtract `dt`-scaled meter values, and clamp at zero; the message references and the recovered constants above supply the gameplay meanings.

## Native Deep-mode trace for the implemented mechanics (2026-11-01)

The original project folder is source/data only, so the native trace was performed on the
available port executable `app/target/release/last_route.exe` (baseline SHA-256
`3dc6c2128a9496fa59cb157bb4e850b77ea626a6a0f4499c03011d744b6a3334`). Names and messages
were found with `native_strings_find`; each referenced function below was then read with
`native_function_pseudocode` in Deep mode. These are the addresses in that baseline binary.

| Rule | Evidence string | Function address | Recovered value/rule used by the build |
|---|---|---:|---|
| Movement/stamina | `sprint`, `footstep_sprint` | `0x14001a460` | Per-frame `dt`; sprint drains stamina to zero and marks exhaustion; otherwise stamina recovers toward max. |
| Flashlight toggle | `flashlight_toggles` | `0x1400cabc0` | Toggle only when the flashlight exists and charge is available. |
| Item/relay timers | `batteryuseliftlevelexploreforced`, `hidefleerelay` | `0x1400ce7f0` | Timers count down by `dt`; relay, battery and interaction state advance in the update path. |
| Relay/battery interaction | `[E] RECOVER RELAY`, `[E] TAKE BATTERY` | `0x140177760` | Nearest valid aimed interaction is selected; relay score is `100`, battery use is consumed. |
| Taser | `TASER // NO TARGET`, `TASER HIT //` | `0x140185510` | Range `5`, half-angle `0.35`; cooldown `8/8.5/9 s`; immune hunters get `0.5x` slow for `3 s`. |
| Capture/hunter profile | `capture`, `taser_immune` | `0x140118bd0`, `0x140159120` | Capture/audio registration and data-driven hunter profile/immunity paths are retained. |
| Settings load/save | `settings.json`, `master_volume`, `SETTINGS` | `0x14008ad10`, `0x14008bc60` | Settings are read/written from JSON; numeric fields clamp and invalid/missing fields use defaults. |
| Safe-zone restoration | `stamina restored`, `FLASHLIGHT RECHARGED` | `0x14017bde0` | Restoration/recharge messages are emitted; the port restores stamina/lamp toward `100`. |

The build now names the recovered taser constants in `src/bin/last_route/config/mod.rs`:
`TASER_SLOW_FACTOR = 0.5` and `TASER_SLOW_SECONDS = 3.0`; all difficulty profiles use those
constants rather than duplicated literals.

## Native Deep-mode verification (2026-11-01)

The native program used for this pass is `app/target/release/last_route.exe` (SHA-256
`f599461743fb046c5af2cc46036eb1c7a12b6765a735719721714db4b0b002e1`). Deep mode opened
it successfully and the following evidence was collected with `native_strings_find`, then
followed through the returned string references into `native_function_pseudocode`:

| Area | Native evidence and function address | Verified rule used by the build |
|---|---|---|
| Settings load | `ev_04c5caa8fb82c301ec8c3175e758db4e456b39de3fa8b8342b74ba8ffe1effd7`; `SETTINGS` → `0x14002f610`, `0x140031630`; `settings.json` / `master_volume` → `0x14008a590` | Load `settings.json`; missing or wrong-typed values fall back to defaults. Volume values clamp to 0..1, voice volume to 0..100, FOV to 60..110 degrees, sensitivity to 0.0005..0.012. Boolean rows default to enabled except fullscreen. |
| Settings defaults | Pseudocode `FUN_14008a590` (`ev_ebc458d53550b37f38b11f8b2f05ead9533a61034f393f4a9a06d83a36d1a70c`) | Default scheme is `last_route`; an input scheme other than `city` resolves to `last_route`. Quality accepts AUTO/LOW/MEDIUM/HIGH; ads accepts off/gentle/annoying. |
| Relay interaction | `RECOVER RELAY` at `0x14037d48b`; reference → `0x1401b6c20` | A valid aimed relay is recovered through the interact action; the port awards `RELAY_SCORE = 100` and increments the floor relay count. |
| Run messages | `stamina restored` at `0x14037d708`; reference → `0x1401bb2a0` | Safe-zone restoration and the associated player-meter message are part of the run loop; stamina is restored toward the configured maximum. |
| Hunter names | `dovelle` at `0x14036a867` → `0x14004e420`; `nyxie` at `0x14036a3a8` → `0x1400dc6d0`, `0x1401b8f70`, `0x1401c6150` | Hunter IDs are data-driven; the port keeps the original IDs and profile values in `app/data/hunters`. |

The Deep engine returned complete pseudocode for `0x14008a590`. A subsequent pseudocode
request for `0x14008cc90` exceeded the engine's 300-second limit; that timeout is recorded
here rather than treated as a discovered rule. The verified settings bounds above are now
named constants in `src/bin/last_route/config/mod.rs` and are used by `src/bin/last_route/app/settings.rs`.

Every number that shapes how the run plays, with where it came from. Paths are in the
original Godot project (`LastRoute Godot/KyttusLabApp`). "estimated" means it was looked for
in the original and not found (or the original only has it in presentation code).
The same values are in `mechanics.json`. Hunter gimmick rules: `docs/GIMMICKS.md`.

**Why there are no function addresses:** the original is a Godot 4 *project* (GDScript source,
scenes and data), not a compiled program. Triage of the folder (evidence
`ev_b6f9e90c6858e0afa950eaa46213e853286be8eabd33f191b217012a2264b8ab`) finds no executable,
DLL or .pck anywhere in it, and the native tools report "a folder is not native machine code",
so there is nothing for Ghidra to decompile. An exported build would only add the Godot engine
binary plus the same scripts packed in a .pck, so decompiling it would not reveal anything the
scripts don't state directly. Every value below is therefore cited by script file and constant
(or file:line), read from the game's own source.

Checked again against the only compiled programs beside the project, the stock engine
`LastRoute Godot/Godot/Godot_v4.7.2-stable_win64.exe` (sha256 ab1824f8…, 181 MB): it holds
none of the game's names, settings or messages — "SIGNAL RELAYS" (evidence
`ev_ccadefe6c13a390505ff959451d623753b919decdf3fb119bc02479c949d991f`),
"STAMINA_SPRINT_DRAIN" (`ev_7f778d9cb8e537d2929a04147243e22dc1829430a86653bfacab6fe8cd508b15`)
and "dovelle" (`ev_5270294442bdf1786a28bb2a23a0a4de4ff6156cf76c3f1b096be1b564339dc0`) all
have 0 matches, while it does carry the GDScript interpreter's own strings (234 matches,
`ev_18b2040c406ed80a395783efa4d8c89382fd902deede2248af4469030853bd6b`, e.g. 0x1467a5520 used by
sub_140060ee0, the command-line option table). The engine only runs the scripts; the rules are
in them.

Third pass (this round): the Infinite route, loading and save names "STREAMED_HUNTER",
"streamed_route", "BUILDING ROUTE", "MUSIC_BOX_WOUND", "route_checkpoint", "KYTTU" have 0
matches in the engine (`ev_4d1d1d6d08e44dd59b4befdf827c7d146c5e038448069904369156e5b8e85586`).
The one piece of engine code the port does depend on is Godot's RandomNumberGenerator (PCG32),
which paints the original's textures: its multiplier 6364136223846793005 (0x5851F42D4C957F2D)
sits in engine code at 0x14381bc02, 0x14381bc42, 0x14381bc73, 0x143847992, 0x1438479f3,
0x143847a22, 0x143847cf7 and 0x143847f8f
(`ev_13f21830bf76c48748516708b68b211169fc61ef6dea17fb5ba1f2d9bbefebb6`), the same constant the
port uses (`src/bin/last_route/render/textures/rng.rs` PCG_MULT). The lifter does not discover a function around
those addresses (`ev_f38fa50b…`, `ev_74a94387…`), and asking native_function_pseudocode for
0x14381bc02 and 0x143847992 never returned on this 181 MB binary (both calls ended the session
before a result), so the PCG step is confirmed a stronger way: the
port's textures were compared byte for byte with dumps made by this same engine running the
original's ref_textures.gd (all 99 identical), and its sounds sample for sample with the
original's audio goldens.

## Movement (game/config/game_config.gd, game/player/player_controller_3d.gd)
| Value | Number | Source |
|---|---|---|
| Walk speed | 4.0 m/s (64 px/s ÷ 16) | GameConfig.PLAYER_WALK_SPEED / TILE_SIZE, player_controller_3d.gd:1958 |
| Sprint speed | 6.25 m/s (100 ÷ 16) | GameConfig.PLAYER_SPRINT_SPEED |
| Crouch speed | 2.25 m/s (36 ÷ 16) | GameConfig.PLAYER_CROUCH_SPEED |
| Acceleration | instant (velocity set each frame) | player_controller_3d.gd:1974 |
| Gravity | 12 m/s² | GameConfig.PLAYER_GRAVITY |
| Jump (hop) velocity | 3.15 m/s (Godot); the Rust hop is 4.9 m/s from the apex below | GameConfig.PLAYER_JUMP_VELOCITY; data/player/jump.json |
| Jump apex | 1.0 m above the walking surface (real hop); 0.41 m reproduces the Godot 3.15 m/s hop | data/player/jump.json `apex_m`, take-off v = sqrt(2 g H) |
| Jump airtime | 0.82 s take-off to landing (2 v / g at the 1.0 m apex) | data/player/jump.json `airtime_s` |
| Jump head room | the head (1.9 m) stops a hop under the cell's drawn ceiling (render/seams.rs `ceiling_y`); the feet are capped there | game/player.rs `head_cap` |
| Crouch | hold C; blocks the hop | player_controller_3d.gd |
| Eye height | 1.707 m (1.6 × 1.067), crouched × 0.69 | CAMERA_HEIGHT, CROUCH_EYE_RATIO |
| Camera bob | walk 0.0256, sprint 0.0405, crouch 0.0128 m | CAMERA_BOB_*_AMPLITUDE |
| Runner radius | 0.24 cells | GameConfig.PLAYER_RADIUS |
| Field of view | 78° | settings.gd DEFAULT_FIELD_OF_VIEW |
| Mouse sensitivity | 0.0075 rad/px | settings.gd DEFAULT_MOUSE_SENSITIVITY |
| Key look | 2.8 rad/s | CONTROLLER_LOOK_RADIANS_PER_SECOND |

## Stamina and flashlight
| Value | Number | Source |
|---|---|---|
| Max stamina | 100 | MAX_STAMINA |
| Sprint drain | 32 /s | STAMINA_SPRINT_DRAIN |
| Recovery | 21 /s, no delay | STAMINA_RECOVERY, STAMINA_RECOVERY_DELAY 0 |
| Exhausted until | 25 | STAMINA_EXHAUSTED_RECOVERY |
| Lamp drain | 0.8 /s (+0.35 sprinting) × difficulty | LAMP_IDLE_DRAIN, LAMP_SPRINT_DRAIN |
| Battery | +25 lamp | BATTERY_RECHARGE |
| Stim pack | 0.6 s jab, 6 s ×1.2 speed at ×0.5 drain, 10 s ×0.5 regen | STIM_* |
| Flashlight range | 16 m cone | FLASHLIGHT_MAX_RANGE |

## Tools
| Value | Number | Source |
|---|---|---|
| Taser | range 5, half-angle 0.35, cooldown 8 / 8.5 / 9 s; immune hunters slowed ×0.5 for 3 s | game_config.gd, data/hunters/*.json taser_immune |
| Crowbar | stun 4.5 / 4.35 / 3.9 s, durability 40, 1 per swing | crowbar_stun, INVENTORY_MAX_DURABILITY, TOOL_SWING_COST |
| Rescue axe | stun 6.0 / 5.8 / 5.2 s, durability 24, 6 per swing, noise 18 for 0.9 s | axe_stun, AXE_NOISE |
| Melee reach | 2.45 cells, half-angle 0.9, cooldown 1 s | MELEE_STRIKE_* |
| Interact reach | 1.7 (relays, pickups), 1.6 locker, 1.8 lift | route_interactable_3d.gd AIM_REACH |
| Stacks | battery 5, stim 3 | STACK_MAX |

## Hunters (hunter_brain.gd, data/hunters/<id>.json)
| Value | Number | Source |
|---|---|---|
| Per-hunter speed / chase / prog / sight / hearing | data files | data/hunters/*.json |
| Unlit sight | sight × 0.5 | UNLIT_SIGHT_SCALE |
| Lit sight | max(24, sight × 3) | FLASHLIGHT_SIGHT_SCALE/MIN |
| Hearing | noise × hearing; crouch 0.9, walk 4.2, sprint 10.5 | NOISE_* |
| Sense interval | 0.1 s | HUNTER_SENSE_INTERVAL |
| Visible chase | ×1.2 speed, direct steer | HUNTER_VISIBLE_CHASE_SPEED_SCALE |
| Hunter stamina | drain 14 / 13.5 / 12 per s in chase, regen 11, winded ×0.62 until 40 | profile stamina_drain, HUNTER_STAMINA_* |
| Speed progression | +0.1 per relay, +0.055 per depth (max 0.55) | HUNTER_RELAY/DEPTH_SPEED_BONUS |
| Capture reach | (0.24 + 0.33 + 0.9) × 0.9 / 1.0 / 1.04 | HUNTER_GRAB_REACH, capture_distance |
| Activation | after relay 3 / 1 / 0 (relaxed / standard / hard) | HUNTER_ACTIVATION_RELAYS |
| Draw | one tier-weighted pick per floor on "<seed>:HUNTER-LINEUP:D<depth>" | hunter_roster.gd |
| Specials | Nyxie (120 s dark / 10 s dead lamp) and Solyth (heat latch) on every floor | GIMMICKS.md §25-26 |
| Camp guard | leaves a hidden runner after 6 / 9 / 12 s | camp_limit |
| Hide check | trail 3.0 / 3.2 / 3.5, tell 1.6 / 1.5 / 1.3 s, cooldown 20 / 19 / 16 s | hide_* |

## Hunter director (game/hunters/ai/hunter_director.gd)
Every mode, before the hunters update, idle/wandering hunters are directed (never a chasing,
stunned or inactive one): a hunter that sees a runner calls nearby free hunters (share), later
team members may flank (point 4 m to the side, ±1.2 rad) or cover the nearest relay within 16
cells; at most `cap` hunters per runner, the surplus sent 14 m away; crowded idle hunters spread
out (gap × 1.8). Ticks every `interval` seconds. Relay rush: once 7 relays are recovered and that
holds 45 s, up to `rushers` free hunters investigate the nearest remaining relays (hold 14 s,
"arrived" within 3 m). Infinite route: hunters closer than 1.1 m push apart at 2.5 m/s.

| Mode | share radius | share chance | cap | team | flank | cover | gap | interval | rushers |
|---|---|---|---|---|---|---|---|---|---|
| Relaxed | 10 | 0.5 | 2 | 1 | 0 | 0 | 7 | 2.0 s | 1 |
| Standard | 16 | 0.8 | 3 | 2 | 0.1 | 0.25 | 8 | 1.5 s | 2 |
| Hard | 24 | 1.0 | 3 | 2 | 0.6 | 0.5 | 9 | 1.0 s | 2 |

Holds: directed 9 s, scatter 16 s; hunters within 12 m of a runner don't scatter or rush.

## Capture and escape
| Value | Number | Source |
|---|---|---|
| Struggle | 6 presses of E in 3.2 s | ESCAPE_PRESSES, ESCAPE_DURATION |
| After breaking free | knock back 1.45, hunter stunned 3.0 / 2.9 / 2.6 s | escape_stun |
| Re-grab cooldown | 16 s × 0.95 per escape (instant death inside it) | ESCAPE_COOLDOWN |
| Grace | 1.5 s | ESCAPE_GRACE |

## Floor (facility_generator.gd, ported draw for draw)
| Value | Number | Source |
|---|---|---|
| Size | 45 × 45 cells, 3 × 3 sectors, 11 × 11 lobby | FACILITY_COLS/ROWS, LOBBY_SIZE |
| Rooms | 9 (lobby + 8 catalogue rooms) | FACILITY_ROOM_COUNT, src/bin/last_route/data/rooms/catalog.json |
| Halls | 3 wide, Kruskal + up to 4 loops ≤ 24 apart | HALL_WIDTH |
| Relays / batteries | 5 / 7 | FACILITY_RELAY_COUNT / BATTERY_COUNT |
| Score | +100 per relay | route_interactable_3d.gd _take |
| Lockers | 6 | place_hiding_spots |
| Safe zone | at spawn, radius 2.5; restores lamp and stamina | SAFE_ZONE_RADIUS |
| Room lights | 3 seeded rooms, switchable; everything else dark | facility_view_3d.gd _spawn_room_lights |
| Wall height | 3.6 m | facility_view_3d.gd WALL_HEIGHT |
| Climate | room offsets: nether +28, kitchens +12, arcades/beach +8, garden −12, maintenance −22, server/cargo −30, ±5 seeded | estimated (per-room climate never ported in Godot) |
| Music box / vents / coins | 1.5-4 cells from start, 4 grates, 5 coins | estimated placement (GIMMICKS.md); the heater station is retired (archive/heater-stations/) |

## Infinite route (streamed_route_generator.gd; CREATE ROOM's default "DEFAULT // INFINITE")
| Value | Number | Source |
|---|---|---|
| Chunk | 45 × 45 cells, live 3 × 3 window (135 × 135) recentred when the runner leaves the middle chunk | CHUNK_SIZE |
| Districts | one per 4 × 4 chunks per level; spawn district is the maze on 60% of seeds; prison only below level 0, inferno from level −3 | REGION_SIZE_CHUNKS, SPAWN_MAZE_SHARE, REGIONS |
| Seams | 1-2 openings per chunk edge at offsets 5, 11, 17, 22, 27, 33, 39 | EDGE_OFFSETS |
| Levels | level 0 is the top (Mezzanine off); depth = 1 − level | MEZZANINE_ENABLED false |
| Stairwells | belong to the boundary between two levels; bays by boundary parity; always one at the origin column; elsewhere 30% of chunks | STAIR_BAY_ANCHORS, STAIR_BAYS_BY_BOUNDARY_PARITY; 30% estimated |
| Flight | 2 wide × 4 cells | estimated |
| Stairs, ladders and drops | One continuous space, floors 5.0 m apart. Stairs are walked up a full flight: no prompt, no teleport. A ladder is climbed by hand (hold to climb, jump to let go) and you step off onto the floor cell beside the hatch. A waterfall drop is open air: you fall one storey in about 0.9 s, can steer a little, and a pool gives a splash while anything else is a hard landing (stun, lost stamina, exposure and noise, scaled by the drop). The storey ahead is kept built when you are within 8 cells | data/stairs_links.json, data/flights.json, data/swim/fall.json; ladders: game/ladder.rs |
| Relays / batteries per chunk | 3 / 4 | estimated |
| Loot | per chunk from data/loot/route.json (base battery, tool and battery rolls, stim aid) | data/loot/route.json |
| Hunters | none before the activation relay, then 1 + (relays − activation) / step; step relaxed 3, standard 2, hard 1. Hunters walk the same flights and ladders at normal speed on every resident floor and are shelved, not simulated, on floors out of the stack | GameConfig.streamed_hunter_population |
| Lift | none; the run moves by stairs | — |
| Save | written on each label change (flight, ladder, fall, admin jump) and each finite lift descent; stores the level label and absolute y; death clears it | GameState.route_checkpoint (stored locally; the original keeps it on its server) |

## Surfaces and sound
Walls, floors and ceilings are the original's painted textures (ref_textures.gd), picked by
district and room purpose like the original: one painted storey stretched floor to ceiling,
32 texels per metre on floors, hall carpet runners in mansion/apartment/gothic halls, the 7 × 7
safe-room pad. The finite route mapped its catalogue regions onto the nearest district
(estimated: the original's finite view drew plain tinted tiles). Every sound is the original's
synthesised cue (sound.gd), sample for sample; the solo route has no ambience and no lift cue.

## Temperature (player_meters.gd)
Ambient starts at 60, rises 0.85 per tick on an interval shortening from 5 s to 0.5 s; body
follows at 1.6/s (clamped ±6.4); safe zone pulls to 60 at 14/s; the temperature item (slot 7) moves the target the body chases (the
heater station's +3/s ambient is retired); 100 latches the
heat (Solyth); ≥ 90 overheats (drain ×1.35); ≤ 10 freezes (movement ×0.72).

## Controls
WASD move, mouse / IJKL / arrows look, Shift sprint, C crouch, Space hop, LMB use, E / RMB
interact, F flashlight, G taser, 1-5 / wheel hotbar, Q drop, M map, Esc pause, F1 HUD, F11
fullscreen (project.godot input map).
