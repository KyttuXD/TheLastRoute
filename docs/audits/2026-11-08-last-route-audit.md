# last_route audit, 2026-11-08

Read-only audit of `src/bin/last_route` in 16 areas. Each finding was checked by a second agent that tried to refute it. 68 confirmed, 17 refuted (listed at the end). Two findings could not be checked (the checker failed) and are not listed. Nothing here was built or run; line numbers are from the audit read.

## crash

### Guest depth is negated unchecked and arrives truncated from an i64

- Where: `game/guest_apply.rs:172` (game-movement), confidence high
- Scenario: A guest's sample carries depth = -2147483648. A debug build panics inside host_turn_valve on the valve request. In a release build the request is refused only because the wrapped value misses the valve's storey, and the sample is never rejected as invalid.
- Fix: Reject out-of-range depth in Sample::from_json using i32::try_from, and use checked_neg instead of unary minus at guest_apply.rs line 172 and hunter_pipes.rs line 232.

### GLB image buffer-view slice panics on a truncated or malformed file

- Where: `render/prop_model.rs:287` (render-models), confidence high
- Scenario: A mod pack's .glb prop whose embedded texture view runs past the end of its binary buffer (truncated download, broken export) makes the slice panic inside parse_glb while the prop loads. That takes the whole game down, although new_texture already has a white fallback for an undecodable picture and the prop could have drawn without its texture.
- Fix: Replace the slice with a checked get: compute end = offset.checked_add(length) and use buffers.get(idx).and_then(|b| b.get(offset..end)), returning None (so the image is skipped and the white fallback is used) when it fails. Apply the same check to the sibling-URI and BIN paths if they can be short.

### Peer depth of i32::MIN overflows on negation in PLAYERS rows

- Where: `ui/pause.rs:236` (ui-screens), confidence high
- Scenario: A remote runner sends a sample with depth 2147483648. The truncated depth becomes i32::MIN. While the pause menu's PLAYERS tab or the hold-O player list is drawn, `-s.depth` panics with 'attempt to negate with overflow' in a debug build on every frame. In a release build it wraps and shows the floor as -2147483648.
- Fix: Reject or clamp out-of-range depths at parse time, for example `depth: i32::try_from(o.get("depth")?.as_i64()?).ok()?`, and/or use `s.depth.checked_neg()` (or `saturating_neg()`) at ui/pause.rs:236 so the display path cannot panic.

### Unchecked i32 subtraction on a peer-supplied hide cell

- Where: `net/world_sync.rs:399` (net-sync), confidence high
- Scenario: A runner sends hide: [-2147483648, 0, 0, 0] while the viewer's origin.0 is positive. In a debug build the subtraction panics on the frame that draws that runner. In a release build it wraps to a large positive cell, so the figure and locker occupancy resolve to the wrong place.
- Fix: Replace the subtraction in remote_view with checked or saturating arithmetic, or clamp the cell to a bounded range in hide_spot_from_json (for example, within the map's cell range) so a hostile value is dropped or pinned. Adding a test that sends i32::MIN with a positive origin would lock this in.

### public_settings panics on an answer without a settings key

- Where: `net/api.rs:428` (net-io-shared), confidence high
- Scenario: A captive portal, a misrouted proxy or an older server answers GET /api/public/settings with HTTP 200 and an HTML body. parse_body returns an empty map, check() returns Ok, and from_answer indexes the missing key. In the browser build the page aborts. Fix: use p.get("settings") and treat None as the existing None return.
- Fix: In from_answer, replace `let v = &p["settings"];` with `let v = p.get("settings")?;` (or `let v = p.get("settings").unwrap_or(&Value::Null);`) so a missing key yields None, which public_settings already maps to the NetError::Refused path. Add a unit test that from_answer on an empty map returns None.

### Byte-slice of a long JSON value panics on a multi-byte character

- Where: `headless/expect.rs:66` (headless-audio), confidence high
- Scenario: A suite step expects run.message == 'X' (or any check) and the actual value is a string longer than 160 bytes containing a non-ASCII character such as an accented letter or a curly quote that straddles byte 160. The check fails, short() panics, and the suite runner dies for every remaining test and suite.
- Fix: Truncate on a char boundary instead of a byte index, for example `let cut = s.char_indices().map(|(i, _)| i).take_while(|&i| i <= 160).last().unwrap_or(0); format!("{}...", &s[..cut])`, or use `s.chars().take(160).collect::<String>()` with a char-count check. Optionally also wrap run_test's check_all in catch_unwind so a suite cannot abort.

### Vertex attributes indexed by the POSITION count, so shorter NORMAL, TEXCOORD or COLOR accessors panic

- Where: `render/prop_model.rs:339` (render-models), confidence medium
- Scenario: A mod .glb whose TEXCOORD_0 accessor has fewer entries than POSITION (non-conforming, and not rejected here) makes `uvs[i]` go out of bounds at line 343 and panic while the prop is loaded. The same happens to a short NORMAL accessor at line 339. Whether the gltf crate already rejects such counts was not verified; the reads here do not check them.
- Fix: Before the loop, check that normals, uvs and cols each have length pos.len() (or use .get(i) with defaults in the loop) and skip or reject the primitive when they differ.

### solve_arm clamp bounds can cross, and f32::clamp then panics

- Where: `render/anim/grab.rs:640` (render-models), confidence medium
- Scenario: During a hunter grab, a forearm or upper-arm segment between 1e-4 and 0.002 units long (for example a grip point that collapses onto the elbow), or any NaN bone position coming from a NaN yaw or position upstream, makes solve_arm call clamp with min > max, and the game panics on that frame. Ordinary rigs are far longer than 0.002, so this needs a degenerate pose or NaN input to trigger.
- Fix: Replace the f32::clamp call with a guarded computation, for example compute lo = (upper-lower).abs()+0.002 and hi = upper+lower-0.002, return early if !(lo <= hi) or any input is non-finite, then use span.length().max(lo).min(hi). Also change the guard at line 633 to `if !(upper >= 1e-4 && lower >= 1e-4) || !goal/target finite` so NaN returns early.

## netcode

### Guest axe breach bypasses host durability and line of sight

- Where: `game/hunter_pipes.rs:236` (game-movement), confidence high
- Scenario: A hostile guest sends a BurstRequest for a pipe id on its own storey, placed within MELEE_STRIKE_RANGE + 1 cell of its reported position but on the other side of a wall. The host bursts that pipe on the first request, with no strikes and no line-of-sight check. Up to max_valve_requests such requests per frame are accepted (world_sync.rs:1682).
- Fix: In host_burst_pipe, require the same checks the host's own swing uses: call sight_to_cell from the guest's position to the pipe cell, and gate the burst on strike_pipe (or pipes.strike) so the breach consumes durability. Alternatively, skip the host's own strike path for guest requests and make the guest send only strike events that the host counts.

### Guest applies an unvalidated host level to existing hunters and window origin shifts are unbounded

- Where: `game/hunting.rs:143` (game-core), confidence medium
- Scenario: A host (or a forged frame) sends an existing hunter's id with level 999999 and origin [1e18, 0]. Each frame that hunter is moved into a new resident list at level 999999 and gets presentation_y = floor_y_of(999999) and a position about 1e18 cells away, so it vanishes or flickers on the guest. The next keep_stack drops the list, and the next frame recreates it. The impact is display and audio only, not a crash, but the guest accepts any level and any offset from the peer.
- Fix: Clamp level to the guest's held range (or reject existing-hunter levels that are not held) before line 143 and bound origin shift to the window size, e.g. skip or clamp the entry when |shift| exceeds a sane cell limit, matching the check in create_guest_hunter.

## bug

### Spiral slab rail runs to about 1e16 cells when the end radial is horizontal

- Where: `world/flight.rs:1322` (world-core), confidence high
- Scenario: Tuning data/flights.json to spiral.start_offset_deg 0 with turns 0.75 (a clockwise spiral, as every south well and the hashed bay spirals use) passes validate, and the slab rail render at render/stairs.rs:378 draws a segment about 3e16 cells long, which breaks the mesh and the GPU buffers.
- Fix: In spiral_slab_rail_with, return None (or clamp t to the landing-row extent) when cu.abs() is below a small epsilon such as 1e-4, and add the same check to validate so offset 0 with turns 0.25 or 0.75 is rejected or handled. Add a test that offset 0 with turns 0.75 yields a finite, bounded rail.

### Emergency lights can stand on an outflow main or crawl pipe footprint

- Where: `world/sewer_emergency.rs:88` (world-sewer-inferno), confidence high
- Scenario: An outflow main at middle M is placed in a chunk. A light is then chosen at M+w (DRY, brick behind it at M+2w), and the sewerEmergency prop is placed on the outflow pipe's footprint, contrary to the documented rule.
- Fix: In sewer_emergency.rs props(), build `taken` from every region prop's footprint: extend it with `p.occupied_cells.iter().flatten()` (and doorway cells if they apply) in addition to `p.cell`. Add a test asserting no light's cell is in any region prop's occupied_cells.

### visited errands are not keyed by storey

- Where: `game/npc.rs:1541` (game-npc-endless), confidence high
- Scenario: A bot dwells at locker 2 on level 0 (visited = [Locker(2)]). The runner descends to level -1. The bot follows and later wants a locker errand there. Locker 2 of level -1 is skipped as visited, so the bot picks another locker or none, though it never went to that one.
- Fix: Clear n.visited and n.errand on storey change, or key visited by level.

### Bot targets carry temperature 0, so Wynelle chasing a bot ends the runner's run

- Where: `game/resident_hunters.rs:217` (game-core), confidence high
- Scenario: Standard difficulty, Wynelle is a route hunter on the runner's floor, a bot is on her level and the hunter chooses it as target (choose_target allows it). The runner stands in a warm room (temperature around 60, no heater). After about 100 s of chasing the bot, progress reaches 100 and Wynelle materialises at the bot. Ten seconds later InescapableCapture fires and end_run(from, "cold") downs or kills the runner, although the runner was never her target and was not cold. A pure bot-target problem: the runner's own temperature is never read on this path.
- Fix: Stop the gimmick from reading a bot's target as the runner. Either gate the meter in Wynelle::before on b.target_is_bot (it is set before update), or drop Event::InescapableCapture when chose_bot in hunting.rs update_hunters (lines 361-366) as the direct capture already is at line 349. Also set the bot Target's temperature to the runner's reading, or drop Wynelle's meter when the target is a bot, so she does not materialise beside the bot.

### Chunk origin multiplication overflows on a guest-named burst id

- Where: `game/hunter_pipes.rs:255` (game-movement), confidence high
- Scenario: A guest sends flow id BRK:2000000000:-1:0:0 while its reported storey is -1. The check at line 232 passes, the multiply at line 255 overflows (panic in debug builds, wrap in release), and a sewer chunk is generated for a coordinate far outside the world. Requests are capped per frame, so this costs CPU rather than memory.
- Fix: Bound the guest chunk before generating: reject parse_burst_id results whose x or z lies outside a sane range (or use checked_mul/checked_add on chunk.0 * CHUNK and chunk.2 * CHUNK and return false on None) at the top of pipe_cells_abs or host_burst_pipe, before sewer::chunk runs.

### Upper-floor ladder hatch pushes a lamp the stairs module says is never pushed

- Where: `render/world_mesh.rs:973` (render-world), confidence high
- Scenario: A level whose upper floor has a ladder hatch (dir < 0): the floor's lamp list gains a warm light 1.2 m above the hole. It lights the floor round the hatch that the design says should stay dark, and it takes one of the nearest-lamp slots (cap 8 or 16) that a real room lamp or stairwell lamp should get.
- Fix: Change the filter at render/world_mesh.rs:973 to `s.dir > 0`. A ladder foot already has dir > 0, so the is_ladder() clause adds nothing except admitting the dir < 0 hatch.

### Failed load stops blocking title clicks, against its own doc

- Where: `ui/boot.rs:752` (ui-screens), confidence high
- Scenario: Content::load returns an empty list, so fail(FAIL_LOAD) is set and the card says it cannot build a route. The player clicks CREATE ROOM or PUBLIC ROUTE. blocks_input() returns false, app/mod.rs:1958 lets the Action through, and the game starts a run or room on the content that just failed to load.
- Fix: Change BootLoader::blocks_input (ui/boot.rs:753) to `!self.state.is_ready()`. fail() is already a no-op once step is Ready, so a failure can never coexist with Ready and the gate stays closed for failed loads, matching the doc comment. Consider also a test that a failed BootLoader blocks title actions.

### Cached routes and goal fields ignore door state, and fields never expire

- Where: `hunters/path_scheduler.rs:168` (hunters-ai), confidence high
- Scenario: Several hunters chase toward a goal G. A shared field for G is built. Then the runner closes a room door on the route. Each hunter using the field descends into the doorway, its next step fails sweep_clear (brain.rs:1617), it adds a 2 s avoid entry and replans by A*. So each hunter loses a walk to the door, and the stale field keeps sending new requests there until it is evicted. The reverse also holds: a door opened later does not shorten routes that use the old field.
- Fix: Add a door-state version (a counter bumped in toggle_door and in update_doors whenever want_open or open changes) to path_key and field_key, or key on a hash of the door gaps. Also make has_field/field treat a Field as absent when its frame is older than cache_ttl_frames, or when its door version differs.

### Window recentre leaves gimmick world positions in the old frame (spider lair stuck)

- Where: `game/endless.rs:1252` (hunters-gimmicks), confidence high
- Scenario: Spider hunts the runner, then begins Retreat. The runner crosses a chunk boundary, so recentre_window runs and moves b.pos by (-sx,-sy) but not lair. From then on Retreat targets a point roughly one CHUNK away in the old frame, which is outside the new window, so the spider wanders toward it indefinitely and stays in Retreat. The barking/hunting cycle is over for the rest of the run.
- Fix: In recentre_window's loop over self.hunters (endless.rs:1248-1254), replace the hand-written field shifts with shift_brain(h, (-sx, -sy)), which also shifts the spider lair and clears the path. Alternatively, call spider.shift for each active WolfSpider.

### Ferryn's return point is not shifted when the window recentres

- Where: `hunters/gimmicks/ferryn.rs:21` (hunters-gimmicks), confidence high
- Scenario: Ferryn is placed in a room 40 cells away in window coordinates. The runner walks across a chunk boundary, so the window recentres by CHUNK cells. The runner then hides. Ferryn sets b.pos to the stale origin, which is now in the wrong place, possibly inside a wall or outside the loaded window, and that point is not checked with move_with_collision or nearest_open.
- Fix: Make Ferryn's origin shift with the window: add a Gimmick::Ferryn arm to shift_brain (endless.rs:2032) that offsets its origin by d, and make recentre_window's active-hunter loop call shift_brain (or the same shift) so self.hunters also gets it. Also run the stored origin through fac.nearest_open_or before assigning b.pos.

### Timed-out guest keeps receiving reliable sends forever on the UDP relay

- Where: `net/world_sync.rs:2249` (net-sync), confidence high
- Scenario: A native guest on the UDP relay closes without PeerLeave. After 45 s prune removes it from guests, but peers still lists it. Each later send_room_and_roster, Start, Event or PlayerEvent goes out as a reliable EVENTS packet to that dead session (session.rs:122-124). Each one stays in unacked, and resend() (session.rs:290-306) re-sends every unacked packet every 200 ms. The resend set and the bandwidth grow for the life of the room, and the PeerPlace broadcast in send_due also keeps going to the dead session.
- Fix: In prune(), for each gone peer also call self.coop.kick(peer) (or a new CoopSession method that drops peers, pings and pending_pings for that id without the link close), so the dead session leaves peers and its unacked queue is dropped. Alternatively give unacked entries an age limit in resend().

### connect gives the first resolved address the whole budget, so a blackholed address blocks the fallback

- Where: `net/ws_native.rs:524` (net-io-shared), confidence high
- Scenario: A host with an AAAA record that does not route and a working A record: NativeWsLink::host or guest fails after 8 s with TIMED OUT, and every HTTP call and skin download to that host fails after 15 s, even though IPv4 would connect at once. Fix: give each address a slice of the remaining time (remaining divided by the addresses left), or race the addresses.
- Fix: In both loops, cap each attempt at a fair share of the remaining time, for example timeout = left / (addresses still to try), with a small floor so the last address still gets a real attempt. Alternatively, race the addresses on threads and take the first success. Apply the same change to net/http.rs connect().

### Stored packs that fail the loader checks are loaded, not skipped

- Where: `content/pack_import.rs:784` (app-content), confidence high
- Scenario: After a new build changes base content, a stored mod whose hunters/<name>.json no longer parses or whose id now collides with a base id still loads partially, while the MODS tab still lists it as an active import; the player gets a warning but no skip.
- Fix: In load_stored_from, run dry_run(&pack, &others) (or an equivalent load into a scratch Content) before reg.insert, and skip the pack with a c.warnings entry when it returns Err, as accept() does. Also add a test that stores a pack whose hunter JSON is broken or whose id collides with a base id, and asserts it is not in loaded_ids or the registry.

### Non-finite or non-positive --minutes gives a PASS after zero frames

- Where: `headless/runner.rs:97` (headless-audio), confidence high
- Scenario: `last_route --bot --minutes NaN` (or `--minutes -2`) prints 'PLAYTEST PASS' with frames 0. `--minutes 1e300` leaves the run unbounded and the deadline thread dead, so the process never produces report.json if the game stalls.
- Fix: Validate in runner.rs: require minutes to be finite and > 0 (else return 2 with a PLAYTEST SKIP message), and clamp deadline to a finite positive range (e.g. filter is_finite && >= 0 and cap at some maximum) before Duration::from_secs_f64. Reuse the dedicated.rs:138 pattern.

### Negative or huge hide_for panics in the 'remote' admin command

- Where: `headless/protocol.rs:964` (headless-audio), confidence high
- Scenario: {"cmd":"admin","name":"remote","hide":[1,2,0,1],"hide_for":-3} returns internal error instead of setting or clamping the hide time.
- Fix: Clamp the value before conversion, for example let secs = f(c, "hide_for").unwrap_or(10.0); let secs = if secs.is_finite() { secs.clamp(0.0, 1.0e6) } else { 10.0 }; then Duration::from_secs_f32(secs). Alternatively use Duration::try_from_secs_f32 and return err() on failure.

### Room hunter spawns skip the pickup, fixture and breakable exclusions

- Where: `world/route/sewer/hunters.rs:127` (world-sewer-inferno), confidence medium
- Scenario: overflow_chamber in sewer_rooms.json has hunter cell [3,3] and loot cell [3,3] (lines 83-84), and storage_alcove has hunter [3,2] and loot [3,2] (lines 102-103). A hunter is placed directly on a pickup, which the base pass would have excluded. The same happens with any room fixture (hiding spot) cell.
- Fix: In add_room_spawns, build the excluded set once with excluded_cells(chunk, fixtures, pickup_cells) (thread fixtures and pickup_cells through chunk_hunter_spawns) and skip any room hunter cell in it. If Godot is confirmed to skip this for room hunters, document the deviation in the function's doc comment instead.

### Downed.hunter index is not re-indexed when the active list changes, so the run report can name the wrong hunter

- Where: `game/revive.rs:62` (game-core), confidence medium
- Scenario: Runner is caught by hunter index 2 and goes down. While it lies there, hunter 0 walks a flight; move_crossers (resident_hunters.rs 662-699) calls remove_active(0), so the former hunter 2 is now index 1. Hunter 1 then leaves the floor too, index 2 no longer exists, and the timeout's finish_run gets None from hunters.get(2) and writes an empty hunter id and name into the run card. Otherwise the report names an innocent hunter.
- Fix: Store a stable identity instead of a bare index: capture the hunter's def.id and name in go_down, and have finish_run use that. Alternatively, in remove_active, adjust self.downed.hunter the same way as escape.hunter (decrement when greater than i), and handle the case where the caught hunter itself is removed.

### The frame in which a revive timeout ends the run keeps simulating, so a capture that frame can down a finished run

- Where: `game/mod.rs:885` (game-core), confidence medium
- Scenario: The runner's downed timer reaches zero on the same frame an inescapable hunter catches it (for example Wynelle's materialised cold timer or a hunter's contact). The report is written, then go_down revives the dead body into a down state with a new 45 s timer that never ticks again. The run card and the wire pose then disagree (report set, downed set, "YOU ARE DOWN" message).
- Fix: In step(), after tick_revives (game/mod.rs:956), return early if self.report.is_some() && !self.spectating(), mirroring the top-of-frame guard. Optionally also make go_down return early when report.is_some() so a finished run can never be taken down again.

### Held-item motion state only advances while the layer draws, so stale changes fire a spurious tween

- Where: `render/viewmodel.rs:74` (render-models), confidence medium
- Scenario: The flashlight is toggled or a swing lands while the runner is hidden or on a ladder. On the next HeldItem frame, line 74 sees last_torch != flashlight_on and starts a TorchClick tween the player did not trigger. Line 60 does the same for a swing by comparing g.swing_t with the stale last_swing_t. The result is a wrong animation that looks like a glitch, not a crash.
- Fix: Sync last_torch, last_swing_t and last_taser_fx every frame, not only on drawn frames, or reset them to current values when the content is not HeldItem.

### A dropped file with neither a path nor bytes is reported as too big

- Where: `ui/skin_tab.rs:508` (ui-admin-map), confidence medium
- Scenario: On web, a PNG drop that delivers no bytes shows 'Not applied: x.png: the file is too big' even though it is small. The user is sent to resize a file that was never read.
- Fix: Add a distinct arm for (None, None), e.g. Err("the file could not be read".to_string()), and keep the size-cap message only for the (None, Some(b)) with b.len() > cap case.

### Resident-floor route requests are never answered from the queue

- Where: `hunters/path_scheduler.rs:491` (hunters-ai), confidence medium
- Scenario: Infinite run, a hunter on a resident floor above the runner has used up its path. In a busy frame the active floor's hunters consume all searches_per_frame units before the resident hunter updates. Its grid_route returns Deferred, path is empty, so pursue returns and the hunter does not move. Its queued entry is never answered by begin_frame, so it stays frozen on every frame the active floor keeps the budget spent, and it moves again only after a calm frame gives it spare budget.
- Fix: Give resident floors a share of the frame budget, for example by round-robin servicing queued requests across all facilities in begin_frame, or by reserving a fixed number of units per resident floor before the active floor's hunters run.

## broken-feature

### Door swing state is dropped every time the Infinite window is rebuilt

- Where: `world/facility.rs:958` (world-core), confidence high
- Scenario: Runner toggles a door open at a doorway in the middle chunk, walks ~40 cells across a chunk seam, and the window recentres. The new Facility has no door_state entry, so the next update_doors call sets want_open=false and open=0. The door is drawn shut (door_open_amount 0, render/scene.rs 1071) and blocks passage unless a body happens to stand in its sweep. Doors the hunters or a bot had opened also snap shut.
- Fix: Keep door swing state keyed by absolute cell (the window origin plus the local cell) outside the Facility, for example in the Infinite state, and seed Facility.door_state from it in load_window or Facility::merge before the first update_doors. Alternatively, copy door_state from the outgoing self.fac in adopt_window, shifting the keys by the origin change, and add a regression test that toggles a door, recentres, and checks door_is_open.

### Extra room silently dropped when the picked room does not fit the spot

- Where: `world/route/sewer/core.rs:1639` (world-sewer-inferno), confidence high
- Scenario: A chunk with a free 9x7 spot whose corridor enters from the top (or bottom) edge: spot_is_free returns true because junction_chamber connects, pick returns overflow_chamber (weight 1.5), connects(overflow_chamber) is false, place_room returns, and the chunk gets no extra room although a valid room exists. The 40 tries in pick are never used for this case.
- Fix: Make the spot test def-specific: have pick call connects(def, grid, x, z) inside its try loop (or pass the def into the free closure and check connects there) so a rejected def triggers another try instead of returning a spot that place_room will drop. Keep place_room's connects check as a defensive assertion.

### Room prop yaw is parsed but quantised away to a quarter turn

- Where: `world/sewer_props.rs:33` (world-sewer-inferno), confidence high
- Scenario: The storage_alcove crate at yaw 15 (sewer_rooms.json line 98) gets rotation 0, and the 15 degree turn that the data asks for is never rendered or used.
- Fix: If sub-quarter yaw must render, add a `yaw: f32` field to Prop, set it from `p.yaw` in sewer_props.rs, and apply it in the world_mesh.rs prop draw sites in place of `rotation as f32 * FRAC_PI_2`. If quarter turns are intended, change the sewer_rooms.json value (likely 90 or 0) and document that the data must be a multiple of 90.

### flee_step and min_away are parsed but never read

- Where: `game/npc.rs:100` (game-npc-endless), confidence high
- Scenario: A designer sets threat.flee_step to 4 or min_away to 0.5 in data/npcs.json. Nothing changes in game, so the setting silently does nothing.
- Fix: Either wire flee_step and min_away into the flight and away-vector code, or remove them from data/npcs.json and the Tuning struct so designers are not misled. Until then, document that they are inert.

### movement.json wade and plunge noise, stamina and soak values are never read

- Where: `game/swim.rs:452` (game-movement), confidence high
- Scenario: A designer sets wade.stamina_drain to 0 in data/swim/movement.json expecting wading to stop costing stamina. Stamina still drains at effects.json swim.stamina_drain and wade.stamina_drain rates; the edit has no effect in game and no warning is given.
- Fix: Either delete the unused wade noise, stamina, soak and plunge fields from movement.json, or route the four helper functions and the stamina and noise paths in swim_effects.rs through rules() so one file is the source of truth. Add a load-time warning or test that flags movement.json keys nothing reads.

### locker_shell.json roughness and vent band are parsed but have no effect in the game

- Where: `render/locker_shell.rs:27` (render-models), confidence high
- Scenario: A designer changes roughness, or vent_band_min and vent_band_max, in data/render/locker_shell.json and sees no difference in game. The data loads without error, so nothing warns about the dead keys.
- Fix: Either remove roughness, vent_band_min and vent_band_max (and two_sided, unused by the draw path) from data/render/locker_shell.json and the struct, or wire roughness into the draw path and point the vent band at the hiding_view.json values. Document in the JSON or module doc which file actually controls the vent slit so designers do not edit dead keys.

### Left/Right/Tab switch tabs even while a FIELDS text box has focus

- Where: `ui/admin/mod.rs:453` (ui-admin-map), confidence high
- Scenario: Admin opens FIELDS, clicks the SAFE TELEPORT box and types 120,45. They press Left to fix a digit. The menu jumps to DEBUG VIEW and the box loses focus. Typing more does nothing, and pressing Enter no longer runs the teleport (the Enter handler at fields_page is only reached while field_focus is Some).
- Fix: Add the same guard used for scrolling to the tab step in keyboard_nav(): skip the step when self.tab == 9 && self.field_focus.is_some() (and the popup is not open), so Left/Right/Tab reach the text edit while a FIELDS box has focus. Tab can still exit the field via a click or Escape.

### GitHub rate limit or 403 is shown as NONE PUBLISHED, and each refresh makes 3 identical calls

- Where: `ui/admin/hub.rs:228` (ui-admin-map), confidence high
- Scenario: After a few opens or REFRESH presses the GitHub limit is hit. Every UPDATES line then reads MACOS/WINDOWS/LINUX: NONE PUBLISHED, which says no build exists, when the real cause is a rate limit. The duplicate requests make the limit run out faster.
- Fix: In update_line, treat any non-2xx answer without a "nightly" or "update" key as HTTP {status} (keep NONE PUBLISHED only for 404). Also fetch the nightly release once per refresh, not once per platform, by sharing one request across the macos, windows and linux lines.

### Sabotaged relays stay in play and the tracker is seeded by depth only (latent: rules off everywhere)

- Where: `hunters/sabotage.rs:183` (hunters-ai), confidence high
- Scenario: Once a difficulty is enabled in the data: a relay is sabotaged and reported offline, but the director's relay rush still sends a hunter to it. A dormant hunter standing near a relay can roll a sabotage with nobody chasing. Two different world seeds at the same depth produce identical sabotage rolls.
- Fix: In hunting.rs, compute an online-relay list (remaining filtered by !sabotage.is_blocked) and pass that to director.tick, and only pass hunters that are active to sabotage tick. Seed Tracker::for_floor from the world seed as well as depth (for example format!("{world_seed}:{depth}:HUNTER-SABOTAGE")).

### Void Gray signal route is never shown or answered, so she always hunts after idle

- Where: `hunters/gimmicks/void/void_gray.rs:244` (hunters-gimmicks), confidence high
- Scenario: Runner stands still in the open for 45 s (Standard) after the 180 s relay grace. tick() sets route_active=true and queues SignalRoute, which mod.rs drops. No prompt appears. After 15.5 s route_left hits 0, fail() sets hunting=true, and Gray chases the runner. This happens on every run, not only on failed route presses.
- Fix: Wire the SignalRoute event into the player side (show the E prompts and call route_result(b, ok) with the result). Until that exists, skip opening the route, or have mod.rs call force_hunt or keep Gray dormant, so the route cannot time out into a chase on every run.

### Co-op chat is not wired on either side

- Where: `net/world_sync.rs:2162` (net-sync), confidence high
- Scenario: A guest submits a team or nearby chat line with SubmitChatPayload. The host's on_message ignores it, so it is never moderated and never broadcast. Other runners never see it, and TTS never speaks it, even though the rules, moderation and tests in chat.rs all pass.
- Fix: In world_sync.rs on_message, add a host arm for Message::SubmitChatPayload that calls chat::relay_submission with the sender's peer id and display name, and broadcasts the resulting ChatPayload. Add a guest arm for Message::ChatPayload that builds a ChatLine and emits a new NetEvent::Chat, which the app forwards to the TTS hook. Also add a caller for chat::submit in the guest input path.

### Public-route join limit and patrol logic are never called

- Where: `app/public_route.rs:100` (app-content), confidence high
- Scenario: A player who rapidly re-joins the public route is never throttled client-side, and idle hunters never patrol around runners, even though the data file says they should.
- Fix: Wire join_allowed into the public-route join path in net/world_sync.rs (the enter_public/join path around line 843-885) and call PatrolBook::anchor and filter_candidates from the hunter brain's idle wander, or explicitly mark these items as deferred in TODO.md and drop the dead-code allow once they are used.

### Level -2 overlay never changes the dry or water cells of the main cross

- Where: `world/route/sewer/core.rs:1621` (world-sewer-inferno), confidence medium
- Scenario: Play any LOWER chunk: level2::chunk(...).grid[CENTER-2][x] is WATER, but chunk(coord).sewer_grid[CENTER-2][x] is still DRY, so the walkway the level-2 data meant to be water stays walkable and the channel is 1 cell narrower than the data says.
- Fix: Either make overlay_level2 also copy WATER over DRY cells that belong to the played cross (keeping the sealed-cell check), or correct the core::chunk doc and the level2 channel_half/tunnel_half data so they match the intended behaviour. Add a test that asserts level2 WATER at CENTER-2 appears in the played lower grid.

### Bot on a non-resident level is frozen for good

- Where: `game/npc.rs:2178` (game-npc-endless), confidence medium
- Scenario: An owner summon arrives with level = 0 while the host's runner has already moved to level -1 (or the client's level is stale). The bot is made on level 0, which is not resident, so it never follows the runner and never becomes active. Its walk field stays Some if it was mid-edge, and the bot does not advance until level 0 becomes resident again. The doc comment at lines 2137-2138 says this only happens after a rebuild, which is not what the code does.
- Fix: Clamp or reject the summon level in spawn_owner_npcs against the host's own runner_level() (or against resident levels) before npc_spawn_at. Add a recovery branch in update_npcs_with: when a bot's level is non-resident and it is not on an edge, snap it to the runner's level via settle_on_nearer_end or a reset, and drop n.walk if the far end is gone. Correct the comment at npc.rs:2179.

### an owed clear that gets 409 STALE is deferred forever and blocks adopting the cloud run

- Where: `net/saves.rs:99` (net-io-shared), confidence medium
- Scenario: The client clock runs behind the server's stored updatedAt, for example by an hour after another machine saved. Each title visit PUTs the clear with the local now, gets 409 STALE, and defers again. The local save stays stale and the newer run from the other machine is never pulled. The loop ends only when the clock catches up. Fix: on Stale for a clear, drop the claim and pull, as the Push branch does.
- Fix: In sync_blocking's Owed::Clear arm, on CloudPut::Stale drop the claim (self.mark(Owed::None)) and set pull = true, mirroring the Push branch, so the newer cloud record is fetched and adopted instead of deferred.

### Audio thread has no recovery from device loss or startup failure

- Where: `audio/quad-snd/src/wasapi_snd.rs:175` (headless-audio), confidence medium
- Scenario: A player unplugs their Bluetooth headset during a run. The default endpoint is invalidated, the event stops firing, and the game is silent from then on with no error and no recovery, with a console line for each sound played.
- Fix: Replace the single-shot setup with a loop that reinitializes the enumerator, endpoint and audio client when GetCurrentPadding or GetBuffer returns AUDCLNT_E_DEVICE_INVALIDATED (or any error), and use WaitForSingleObject with a timeout (for example 100 ms) so a silent endpoint cannot block forever. Return errors from startup instead of asserting, retrying with a delay.

### Speech worker has no timeout, so one hung speech process silences chat for the session

- Where: `audio/tts.rs:251` (headless-audio), confidence medium
- Scenario: espeak or powershell stalls on one line. Every subsequent chat line from nearby players is dropped silently because pending never falls below queue_max, and the worker never speaks again.
- Fix: In run(), poll child.try_wait() against a deadline (a few seconds per line), call child.kill() and child.wait() when it expires, then return so the worker decrements pending. Alternatively, decrement pending on timeout in the worker.

## perf

### Locker reach walks every model vertex for every locker, every frame

- Where: `render/scene.rs:484` (render-world), confidence high
- Scenario: A floor with dozens of lockers: each frame walks the vertices of the hiding_spot model once per locker (and once more for the door) before the cheap distance test, even for lockers far beyond the fog that the test then skips. The reach could be computed once when the model loads.
- Fix: Compute the reach once when the hiding_spot model loads (store it alongside the PropModel, or memoize it in a OnceCell keyed by the model), and pass the cached f32 into locker_fully_fogged instead of calling locker_reach(m) each frame.

### Each waterfall is rebuilt every frame for the sound position and for drawing

- Where: `render/waterfall.rs:380` (render-world), confidence high
- Scenario: A level with several falls in the 3x3 window: each frame builds every fall's sheet and mist puffs at least twice (once for the sound, once for the drawn falls), although only the sound needs the position. Computing the sound position directly from site.lip and the mist's base offset would avoid the full build.
- Fix: Extract a sound_pos(cfg, bottom_y, out) helper in waterfall.rs returning [0.0, bottom_y + cfg.mist.above_surface_m, out * (cfg.throw_m + cfg.mist.offset_out_m)], have build() use it for sound_pos, and make nearest_sound call it instead of build(). Optionally reuse the falls already built in level_fx for the sound pass.

### visible_window is quadratic and runs every frame for the SKIN link box

- Where: `ui/style.rs:924` (ui-screens), confidence high
- Scenario: A 500-character link is focused with the caret at its end. The loop runs about 440 iterations per frame, and each iteration allocates and measures a slice of up to 512 chars, roughly 130k char operations and several hundred allocations every frame, even while the user is not typing.
- Fix: Replace the first loop with a single backward scan that tracks the width of the suffix incrementally, or binary-search start over char indices, so each frame does O(n) work with at most one allocation. Optionally cache the result keyed on (text, caret, avail) so it is not recomputed while the field is idle.

### STATUS tab reads checkpoint.json from disk every frame

- Where: `ui/admin/actions.rs:1036` (ui-admin-map), confidence high
- Scenario: With the admin menu open on STATUS and no active checkpoint, the game does a file read and JSON parse per frame, around 60 times a second, for a value that rarely changes. This shows as frame-time spikes on the admin overlay.
- Fix: Compute the checkpoint label and build label once when the STATUS tab or menu opens, or refresh them on a timer (for example every 0.5 s) instead of every frame. Alternatively, cache the result of checkpoint::load() in the admin state and invalidate it on save/clear.

### MODS tab rebuilds the installed list and re-wraps its text every frame

- Where: `ui/mods_tab.rs:317` (ui-admin-map), confidence high
- Scenario: With the browser MODS tab open and a few dozen imported packs, each frame clones the registry metadata and re-measures every wrapped line. That is avoidable per-frame allocation in a hot UI path.
- Fix: Cache the built rows and items in ModsUi, keyed on a registry revision counter (or a hash of the held metadata and content.packs length) plus s.lines and the store's available/save_failed flags, and rebuild only when that key changes. Keep the cheap scroll and hit-test math per frame.

### admin give loops over an unbounded count without stopping at a full inventory

- Where: `headless/protocol.rs:881` (headless-audio), confidence high
- Scenario: A client sends {"cmd":"admin","name":"give","item":"battery","count":2000000000}. The loop runs about 2e9 give calls and blocks every later command until it finishes.
- Fix: Break out of the loop once g.give returns false (inventory full), and clamp count to a small maximum such as the stack limit, for example `let n = n.clamp(1, item.stack_max().max(1) as i64)` or an equivalent cap, before the loop.

### Per-frame String allocations for every hunter in the audio update

- Where: `audio/world.rs:216` (headless-audio), confidence high
- Scenario: With about 30 active hunters the game performs roughly 1800 String allocations per second in the audio path alone, even when no hunter state changes.
- Fix: Replace the unconditional insert with get_mut (for example, if let Some(v) = w.was_active.get_mut(h.id) { *v = h.active } else { w.was_active.insert(h.id.to_string(), h.active) }) so the String is only allocated on first sight of a hunter; optionally reuse the per-frame Vec<HunterAudio> in game/mod.rs:918.

### Waypoint routing scans the whole discovered floor per cell step

- Where: `game/endless.rs:2149` (game-npc-endless), confidence medium
- Scenario: Level 0, waypoint placed outside the 3x3 window, a long run with many discovered chunks. Each cell crossing runs one or two A* searches and a full HashMap scan over every discovered open cell, which grows without bound with exploration. On the wasm build this is a per-step hitch that gets worse the further the runner has gone.
- Fix: Cache the discovered-floor route and frontier per (waypoint, floor_chunks) instead of recomputing per cell step, or only replan when `from` leaves the current route. Alternatively, compute a BFS distance field from the waypoint once per floor change and walk it per step.

### Opening lists are rebuilt every frame in draw_scene and ActorCull

- Where: `render/scene.rs:353` (render-world), confidence medium
- Scenario: A floor with a stairwell or drop cells and one resident neighbour: about three openings() calls with their allocations, sorts and sets run each frame, plus the full lamp sort, although the inputs only change when the window or level changes. This adds frame time in the hot path and should be cached per window.
- Fix: Compute openings(&g.fac) once per frame before the neighbour loop (or cache it per window/level in the Game state and invalidate on level change), reuse the same Vec in ActorCull::new, and keep the own-lamp list pre-sorted or cached so merge_lamps does not clone and sort it every frame.

### Cross-floor goal with no flight is re-searched every frame, outside the budget

- Where: `hunters/brain.rs:1512` (hunters-ai), confidence medium
- Scenario: A hunter on floor 0 chases a runner on floor -2 where no stair, ladder or drop links the stack. Every frame for about 1.6 s it runs a full union-graph search on each resident level grid. Several such hunters in one frame cost several full searches per frame, which shows up as a frame-time spike. Caching the no-hop result or sending hop_to through the scheduler would remove this.
- Fix: In pursue(), when cross is true and hop_to returns None, cache the no-hop result against the nav graph version and goal cell, and skip the search until that version changes or give-up runs. Alternatively, route hop_to through path_scheduler so it shares the frame budget.

## risk

### Facility::maze_variant is written, dropped by merge, and read nowhere

- Where: `world/facility.rs:272` (world-core), confidence high
- Scenario: Any caller that reads window.maze_variant (for example to pick floors or room roles for a maze region) gets None on every Infinite window, although the chunk was dressed as a variant. Such a caller would silently fall back to the plain maze.
- Fix: Delete the unused maze_variant field and its wrong doc comment, or wire it through live.rs and merge so window facilities keep the id.

### npc_spawn_at loops over an unchecked count

- Where: `game/npc.rs:1974` (game-npc-endless), confidence high
- Scenario: An agent-protocol message with count = 2000000000 (or a value that wraps to a large positive i32) makes npc_spawn_at push that many bots. Each push allocates, so memory grows until the process runs out.
- Fix: Clamp count in npc_spawn_at (or in protocol.rs before the call) to 1..=MAX_SUMMON_NPCS, and replace the `as i32` cast with i32::try_from(...).unwrap_or(0) so out-of-range values are refused rather than wrapped.

### skin_preview.json "model" is parsed and required but never read

- Where: `ui/skin_preview.rs:54` (ui-admin-map), confidence high
- Scenario: A mod or edit changes skin_preview.json "model" to another body path and rebuilds. The preview ignores it and still draws a.npc_body, so the change has no effect even though the file validates.
- Fix: Either load the base body from cfg.model (for example via load_model(&cfg.model, a), mirroring the slim path at line 175 and falling back to a.npc_body), or remove the "model" key from skin_preview.json and Config so the file no longer advertises a setting that does nothing.

### Party pounce budget is process-global, real-time, and never reset between matches

- Where: `hunters/gimmicks/nyxie.rs:443` (hunters-gimmicks), confidence high
- Scenario: A Nightmare match ends with a pounce at process time T. The player leaves to the menu and starts a new match at T+10 s. Every hunter's first pounce in that match is refused until T+25 s, even though the new match just began. Pounce timing also differs between a paused game and a running one because the clock is real time, not game time.
- Fix: Clear the shared budget when a match starts, for example by resetting PARTY_LAST_POUNCE from Game::reset_director (or from a per-match reset in Nyxie). Optionally move the clock to match or game time so pauses do not count.

### Reliable reorder buffer has no cap, so a peer can grow it without bound

- Where: `net/session.rs:240` (net-sync), confidence high
- Scenario: A guest (or any relay session) sends reliable EVENTS packets with seq 1, 2, 3, ... and never seq 0. Each one is stored in held forever, at up to about 1.4 KB per packet, so host memory grows for as long as the peer keeps sending. Later in-order messages from that peer on that channel are never delivered, so the peer silently stops being heard.
- Fix: Cap the reliable reorder buffer, for example by dropping or disconnecting a peer when held.len() exceeds a fixed limit (such as 256 entries) or when the seq is more than N ahead of next_receive, and do not ACK packets that are rejected. Also consider a per-peer byte budget for held.

### the run checkpoint is written non-atomically

- Where: `shared/checkpoint.rs:144` (net-io-shared), confidence high
- Scenario: The game is closed or the machine loses power while a checkpoint is being written at a floor boundary. The next launch finds no checkpoint, so the player loses the continue option for that run without any error. Fix: write to checkpoint.json.tmp and rename it, as the other writers do.
- Fix: In save(), serialize to a sibling checkpoint.json.tmp, then std::fs::rename it over checkpoint.json (rename replaces the target on Windows and Unix). Optionally call sync_all on the temp file before the rename, as ConfigFile::save and the skins writer do.

### Neighbour-level builds ignore the quality governor's stream scale

- Where: `render/world_mesh.rs:677` (render-world), confidence medium
- Scenario: On a slow machine the governor cuts the stream budget to shed load and the window builds are spread over more frames. Resident neighbour parts (up to neighbour_chunks_per_frame, or the raised near-opening count, per frame) keep the full budget, so a neighbour build can still take the whole frame budget in a frame where the governor is trying to cut it.
- Fix: Pass the governor factor into neighbour_allows, for example by calling table.allows_scaled(built, elapsed, queued, scale) from rebuild_neighbour with scale = f64::from(crate::app::quality::scales().stream), or by scaling budget_ms inside neighbour_allows, and add a test that a scale below 1 cuts neighbour builds. If the unscaled neighbour budget is intended, document that in the doc comment.

### wasm reads a page-reported body length without a cap before the size limit is checked

- Where: `net/web_bridge.rs:66` (net-io-shared), confidence medium
- Scenario: A skin download (limit read_limit, about 64 KB) or any other capped request gets a multi-hundred-MB answer from the server. The browser build copies the whole body into wasm linear memory before refusing it, which can exhaust memory or fail the allocation. Fix: pass the caller's limit into the bridge and refuse before allocating, or make the page stop at cap+1.
- Fix: Pass req.limit into web_bridge::http_request and have read_blob refuse when the page-reported length exceeds the limit before allocating (return an error or a sentinel). Optionally, have the page stop copying at cap+1 in lr_http_body and in httpSend.

### Page-reported stored size drives an unchecked allocation

- Where: `content/pack_import.rs:855` (app-content), confidence medium
- Scenario: One stored record reports a length of about 2 GB, so the game aborts during start-up on the browser page instead of skipping that pack.
- Fix: Skip records whose size exceeds MAX_PACK_BYTES before the allocation at pack_import.rs lines 852-855.

### Second read can silently return None when a value grows between reads

- Where: `app/web_storage.rs:33` (app-content), confidence medium
- Scenario: A stored settings value that grows past its previous length during the two reads makes the browser build fall back to default settings for that load, silently.
- Fix: Loop the read until it succeeds or a bounded retry count is reached, instead of giving up after one resize (for example, a loop of up to 4 attempts that resizes to n on each n > cap and returns None only after the last attempt). Optionally log when the retries run out.

## Refuted

- `world/route/live.rs:145` Door clearance and wall-fixture moves use props that the maze variant later removes: The stated scenario does not hold. door_clear = Clearance::of(&f) is built only from sites_of(f) (door_clearance.rs:230-231), and sites_of uses only door props (f.props filtered by door_site_of_prop). Door props are never thinned: apply_maze_variant keeps every p with p.door.is_some() (facility.rs:1
- `world/route/music_boxes.rs:63` music_box_chunk panics when boxes_per_region exceeds the 16 region slots: The panic is unreachable even when boxes_per_region exceeds 16. In music_box_chunk (world/route/music_boxes.rs:59-72), `local` is the chunk's offset inside its region, so it is always one of the n*n slots (n = REGION_SIZE_CHUNKS, since div_euclid keeps c.x and c.z inside the region). Each non-matchi
- `game/npc.rs:1953` Crosshair spawn spot compares relative height against absolute y: The eye is not absolute. Player::eye() (game/player.rs:349-353) returns eye_y() - floor_y + bob, which is relative to the level floor, and its doc comment says so. The only caller of npc_crosshair_spot is ui/admin/actions.rs:316, which passes g.eye() (via game/mod.rs:740) directly. So the eye and fl
- `game/npc.rs:2354` Each bot relay claim reloads all content from disk: The per-claim load is real (game/npc.rs:2354 calls crate::content::Content::load() inside npc_claims), but it is not a hot path. claim_relay returns early when relays_taken[i] is already set, so each relay is recovered at most once per run, and only relay claims reach this line. That bounds the relo
- `game/npc.rs:1381` Zero ladder speed scale stops the stuck give-up on ladders: The scenario cannot happen. FlightEdgeRules::parse in hunters/flight_edges.rs (line 101) reads ladder_speed_scale with num(&v, "ladder_speed_scale", 0.1, 5.0), and the file's own doc says every value must be inside its limits or the file is refused. A value of 0 is rejected at load, so speed can nev
- `game/remote_targets.rs:308` Guest sample height h is unbounded and sets the parked body's absolute y: Placement is real: world_sync.rs:250-257 accepts any finite h and park_on (remote_targets.rs:294-308) sets it unclamped. The crash path is not reachable: body_fits_at_y is only called from player physics, which Game::step runs and step_dedicated replaces (game/mod.rs:871-872).
- `game/swim_effects.rs:365` Zero fall min_height is accepted and then used as a divisor: The scenario needs a Landing with drop_height 0, and the code never produces one. Landings are created only at player.rs:628 under `fall_rules().reports_landing(drop_height)`, which requires drop_height >= landing_report_gap. swim.rs:283 rejects landing_report_gap < min_gap, and min_gap is bounded t
- `render/anim/runner.rs:507` One NaN or infinite speed sample permanently poisons a remote runner's pose: The peer-supplied path cannot deliver a non-finite speed. In net/world_sync.rs from_json (lines 250-253), every numeric field is filtered by is_finite, and vx/vz go through interp::clamp_velocity, which maps NaN to (0,0) and scales infinities down to max_speed_mps (interp.rs tests at 593-596). vy is
- `ui/admin/actions.rs:955` partial_cmp().unwrap() on distances panics if a position is NaN: The code pattern is as described: actions.rs:662 (teleport_chunk, min_by partial_cmp unwrap), :957-958 (grant_relays sort_by partial_cmp unwrap) and :1071 (status() rows sort) would panic if any compared distance is NaN. But the NaN source is not confirmed. Relay distances use relay cell centers, wh
- `net/world_sync.rs:2007` Unregistered sessions can add host state and fan out their profiles and samples: The "permanent entries" claim is refuted. SessionEvent::PeerSeen (world_sync.rs:1963-1965) inserts every new relay peer into self.guests with last_seen = now, before any message is handled, so an unregistered session is in self.guests. prune() (world_sync.rs:2252-2269) then removes its remotes, skin
- `net/backend.rs:234` a second CREATE ROOM orphans the first hosted room: The overwrite is real at code level: Backend::create_room (net/backend.rs:234) has no check on self.hosted, and Op::Create success (backend.rs:316-331) overwrites self.hosted without unregistering the old room, so the old code is dropped and gets no heartbeats. But no production code calls Backend::
- `net/api.rs:770` a typed room code is spliced into the GET path without validation: The typed-code scenario is blocked before resolve_room. The JOIN dialog in ui/menus.rs:661 runs the text through world_sync::parse_join_code (world_sync.rs:2394), which trims, uppercases and calls api::valid_join_code, so "AB?x=1", "/" or any non-alphabet text is rejected with "ENTER A VALID FIVE-CH
- `app/web_update.rs:130` A NaN frame time freezes the update countdown forever: The arithmetic part of the report is correct: f64::clamp returns NaN for a NaN input, so step, now and Counting(left) become NaN, and `left <= 0.0` is false, so the phase stays Counting forever (seconds_shown shows 0). But the scenario needs a NaN dt, and nothing in the code produces one. The only c
- `app/log.rs:58` Panic hook re-enters the log file mutex: In app/log.rs, write() takes the FILE lock only at line 100 and the guard lives only across the if-let body (lines 100-105). Inside that region the only calls are f.write_all and f.flush, which return io::Result and do not panic. The line is formatted at line 99 before the lock is taken, and the std
- `app/updater.rs:63` update check reads the offer from a server reply without a size bound on the JSON: The claim that offer() has no body limit is wrong. Request::new defaults limit to MAX_RESPONSE_BYTES (4 MiB) (net/http.rs:113), and the transport enforces it on the response body in the content-length, chunked and close-delimited paths (net/http.rs:551, 715, 742, 756, 768). offer() only sets a 5-sec
- `headless/dedicated.rs:410` Owner DIFFICULTY changes the wire and self.difficulty but not the running game: The mid-route mismatch is intentional, not a defect. dedicated.rs:410-413 sets only self.difficulty and the room's wire difficulty, and begin_route (lines 446-453) is the only place the running game takes self.difficulty. The unit test the_difficulty_the_owner_picks_applies_from_the_next_route (line
- `audio/quad-snd/src/alsa_snd.rs:53` Negotiated ALSA rate is discarded; the mixer always assumes 44100 Hz: The code does discard the value: alsa_snd.rs:53-58 passes `&mut rate` to snd_pcm_hw_params_set_rate_near and never reads `rate` again. audio_thread (line 160) calls mixer_builder.build(), which is build_at(SOURCE_RATE) with increment 1.0 (mixer.rs:198-215). So the mismatch is real in principle. But 
