# Hunter gimmicks: implementation spec for the Rust port

Source of truth (read-only): `LastRoute Godot/KyttusLabApp`. Paths below are relative to that project.
Main files: `game/hunters/gimmicks/*.gd`, `game/hunters/ai/hunter_brain.gd`, `game/hunters/ai/hard_abilities.gd`,
`game/hunters/ai/melee_strike.gd`, `game/config/game_config.gd`, `game/config/hunter_roster.gd`, `data/hunters/<id>.json`,
`game/player/player_meters.gd`, `game/main_3d.gd` (the finite floor scene, the one the port copies).

Units: grid cells (1 cell = 1 m), seconds. "GC.X" means `GameConfig.X`. Difficulty ids are `relaxed`, `standard`, `hard`
(plus `nightmare`, which is ported: `Difficulty::Nightmare` in `src/bin/last_route/config/difficulty.rs`, in the lobby row; its values are listed where the code branches on them, with the Rust data or module that holds each).

Tags used below:
- **[observed]**: read directly in the code.
- **[quirk]**: what the Godot code actually does, which looks unintended. The port should copy it only if bit-exact parity matters.
- **[unknown]**: I could not determine it from the code.

---

## 0. Shared plumbing

### 0.1 Base class `HunterGimmick` (`hunter_gimmick.gd`) defaults

| Hook | Default | Called from (brain) |
|---|---|---|
| `before_perception(brain, delta, target, nav)` | no-op | `update()`, see order below |
| `after_perception(brain, delta, target, nav)` | no-op | `update()` |
| `speed_scale(brain) -> f32` | 1.0 | `_current_speed()` (multiplies) |
| `sight_scale(brain) -> f32` | 1.0 | `sight_limit_for()` (multiplies the final limit) |
| `holds_position() -> bool` | false | `_update_movement()`, `_consider_peek()`, `_note_displacement()` |
| `capture_is_inescapable() -> bool` | false | main_3d `_capture_target` / `_trigger_capture` (skips the escape struggle) |
| `runs_while_inactive() -> bool` | false | `update()` (lets `before_perception` run while `active == false`) |
| `immune_to_capture() -> bool` | false | `update()` capture check |
| `breaches_hiding() -> bool` | false | `_update_perception()` and the capture check |
| `peek_chance(brain, base) -> f32` | base | `_consider_peek()` |
| `hidden_from_view() -> bool` | false | presentation only (HunterView3D fades the model to alpha **0.28**, `MIN_VISIBLE_ALPHA`; never fully invisible) |
| `on_runner_respawn(brain)` | no-op | `respawn_reset()` (not used by any of the 26 gimmicks below) |
| `network_state()/apply_network_state()` | `{}` | co-op only; the solo port can skip it |

`HunterGimmick.for_id(id)` returns null for **dovelle** (and for any id not in its list). A null gimmick means every hook is skipped.

### 0.2 Brain call order per frame (`HunterBrain.update(delta, nav, target, depth, relays)`)

1. `troll_target`, `relays_seen` set; `_tick_emote(delta)`; `_tick_slow(delta)` (the taser slow ticks even while inactive).
2. `self_summoning = gimmick && gimmick.runs_while_inactive()`. If true, **`before_perception` runs here, once**, before any other check (Nyxie, Solyth, Wynelle).
3. `if !active -> return false`.
4. `if stun_timer > 0 -> stun_timer -= delta; return false` (no perception, no hooks, no capture). Note that a self-summoning gimmick's `before_perception` already ran in step 2, even while she is stunned.
5. `_camp_guard(delta, ...)`: returns `ignoring_hider`.
6. `_ability_direct_sight = false`.
7. If not self-summoning: **`before_perception`**.
8. `_update_beam_attention(delta, nav, target)` computes `beam_attention`, `beam_alerted`, `flash_triggered` (see 0.3). **This means a gimmick reading `flash_triggered` in `before_perception` sees LAST frame's value (Aki), and one reading it in `after_perception` sees THIS frame's value (Briselle, Ferryn, Ilyra).** [observed]
9. `_update_perception(...)` unless `ignoring_hider`. It uses `sight_limit_for()`, which multiplies by `gimmick.sight_scale()` last, and `breaches_hiding()` (`can_sense = !in_safe_zone && (!hidden || breaches)`).
10. **`after_perception`** (it overrides what perception just decided).
11. `_update_stamina`, peek/inspect cooldowns, `_consider_peek(nav)`. That function returns early if `holds_position()`; otherwise `odds = HunterPeek.chance(id, diff)`, then `odds = gimmick.peek_chance(self, odds)`.
12. `hard.update(...)` (HardAbilities, see 0.6). If it returns true (a pounce leap is driving her), ordinary movement is skipped.
13. `_update_movement`: the PEEK/INSPECT/IDLE/SEARCH timers run first; **then `if gimmick.holds_position() return`**; then `_current_speed()` (multiplies by `gimmick.speed_scale()`, then `hard.speed_scale()`, troll 1.15, the slow factor, and the difficulty `hunter_speed`).
14. Capture: `may_grab_hidden = hidden && breaches_hiding() && has_last_known && last_known.distance(target) < 0.9`.
    The grab lands if `(!hidden || may_grab_hidden) && !in_safe_zone && !immune_to_capture() && !hard.blocks_capture() && distance(target) < capture_reach()`.
    `capture_reach = (0.24 + 0.33 + 0.9) * 1.0 * capture_distance`, i.e. relaxed **1.323**, standard **1.47**, hard **1.5288**.

`update_without_target` (no runner selectable) calls **no gimmick hooks at all**.

Speed formula (used by every `speed_scale` below):
`progression = relays * 0.1 + min(0.55, (max(1, depth) - 1) * 0.055)`.
In CHASE: `speed = chase + progression * prog`; otherwise `speed = speed_json + progression`.
Then winded x0.62, gimmick scale, hard scale, troll, slow, difficulty `hunter_speed` (relaxed 0.82, standard 1.0, hard 1.085).
A `speed_scale` therefore multiplies the walk speed too, not only the chase speed.

### 0.3 Player beam attention (the "flash" several gimmicks read)

`lit = target.flashlight_on && beam_reaches(player_pos, player_facing, hunter_pos, range 12.0, half_angle 0.28 rad) && LOS`.
- lit: `attention = min(0.35, attention + dt)`; not lit: `attention = max(0, attention - dt * 1.6)`.
- `beam_alerted = lit && attention >= 0.35`.
- `_flash_armed` resets to true only when `attention <= 0`.
- `flash_triggered = beam_alerted && prev_attention < 0.35 && _flash_armed`. On trigger, `_flash_armed = false`.

So a "flash" means holding the beam on her for 0.35 s. A second flash needs the beam fully off her first (attention back to 0). Note that `flashlight_on` here is the raw toggle; lamp charge is not checked in this function.

`beam_reaches(from, facing, to, range, half_angle)`: `0.001 < dist <= range`, `|facing| > 0.001`, `dot(norm(to - from), norm(facing)) >= cos(half_angle)`, and LOS.

### 0.4 RNG streams

- Every gimmick draw goes through the **brain's own `SeededRandom`** (mulberry32 seeded by FNV-1a of the brain's seed text): `brain.next_random_float()` / `next_random_index(n) = floor(next_float() * n)`. The stream is shared with the brain's own idle/wander/search/peek draws, so the order of draws matters for parity.
- Brain seed texts in main_3d: route hunter `"<level_seed>:HUNTER"`; specials `"<level_seed>:NYXIE"` and `"<level_seed>:SOLYTH"`.
- Exceptions: VerrineGimmick's physical-vent mode uses a Godot `RandomNumberGenerator` seeded with `hash("<pos>:VERRINE")`. That mode is not used on the finite floor (see Verrine). The Zellia death roll uses Zellia's brain stream, from main_3d.
- Hunter draw: `HunterRoster.pick_for_floor(level_seed, depth, diff)` uses its own stream `"<seed>:HUNTER-LINEUP:D<depth>"`. Tier weights: relaxed easy .70 / intermediate .25 / difficult .05; standard .35/.50/.15; hard .2625/.5125/.225; nightmare .175/.525/.30 (by id: `pick_for_floor_id`, `tier_weights_for`). Tiers are walked in sorted order (`difficult`, `easy`, `intermediate`). One `next_float` is drawn for the tier, then `next_index` within it.
  Draw pool (IMPLEMENTED_IDS): dovelle, quenelle, elise, xivvy, kathy, ravelle, mavrine, umbriel, wynelle, thessine, haelith, ysolde, opalie, aki, porcelle, javelle, loxelle, ceryth, briselle, ferryn, glyra, ilyra, verrine, zellia. Nyxie and Solyth are never drawn.

### 0.5 How main_3d applies gimmick output (host loop `_update_gameplay`)

The route hunter activates when `relays_collected >= min(HUNTER_ACTIVATION_RELAYS[diff], relay_count)`: relaxed **3**, standard **1**, hard **0**. Activation calls `activate_at(player_pos)` (INVESTIGATE at the player's position). Specials are spawned inert next to the route hunter's spawn and wake themselves.

After each `brain.update()`, main_3d drains `brain.world_events`:
- `emit_world_event` runs with `ANNOUNCE_HUNTER_EVENTS = false` (shipping value). Any `what == "message"` event is **dropped**; the `message` key is stripped from every event; `sound == "alert"` is stripped. So in the shipping game **none of the gimmick toasts below are shown**. They are listed as "callout (off by default)".
- `facility_alert {at, except?}`: collected and applied **after all hunters updated**. Every active hunter except `except` goes INVESTIGATE at `at` (`has_last_known = true`, `last_known = at`).
- `stun {seconds}`: player `stun_timer = max(stun_timer, seconds)`. A stunned player cannot move or use items.
- `pickup {cell, kind}`: `facility_view.apply_event` marks the interactable taken (removed), the same as a player pickup.
- `room_door`, `vent_burst`, `vent_thump`, `thessine_mimic`: facility_view event + `_play_world_audio`. Only `thessine_mimic` has finite-floor audio logic.
- Javelle: if `gimmick.ejecting`, then player `eject` (`leave_hiding(ejected_to, forced = true)`).
- Loxelle: drains `gimmick.toggles` (eject + `meter_effect`, or hiding spot disable/enable).
- Wynelle: if `has_frozen_player()`, then `_capture_target(view, target, inescapable = true)` with death cause `"cold"`, followed by `consume_frozen_player`.
- `captured == true`: `_capture_target`. That starts the escape struggle unless the capture is inescapable (gimmick flag, void, forced, or escape cooldown).
- `meter_effect {exposure_add, noise_add, noise_reclaim, temperature_add, heat_lock}` on the player: `add_exposure`, `reclaim_noise`, `add_noise` (banked for the next meter update), `temperature = clamp(t + add, 0, 100)`, and the heat_lock latch.

Struggle constants (GC): `ESCAPE_DURATION 3.2`, `ESCAPE_PRESSES 6`, `ESCAPE_COOLDOWN 16`, `ESCAPE_GRACE 1.5`. On a break-free the player is knocked back 1.45 cells, and the hunter gets `enter_search_from(new_pos, 1.8)` plus a stun of `escape_stun` (relaxed 3.0 / standard 2.9 / hard 2.6).

### 0.6 HardAbilities that ride on some of these hunters (`hard_abilities.gd`)

- Relaxed: none (`for_brain` returns null).
- Standard is gated on (FREQUENCY .12 > 0), but the party budget allows only one activation per ability every 360 s. Hard budget: 45 s.

| Ability | standard | hard |
|---|---|---|
| pounce | haelith | haelith, solyth, xivvy |
| slam (door) | elise | elise, javelle, solyth |
| burst | quenelle | quenelle, ferryn, umbriel |
| intercept | mavrine | mavrine, kathy, quenelle |

Pounce: standard `{min 2.2, max 3.5, rate .35/s, cooldown 120, per_chase 1, stumble 1.6, seconds .45}`; hard `{2.2, 4.5, .35, 28, 2, 1.1, .5}`. Leap speed 7.2; target lead .15 s; hitting a wall gives a .8 s stun; stumble is x0.3 speed and blocks capture.
Per-hunter pounce gates: solyth needs temperature >= 60; nyxie needs the flashlight off (nightmare only).
Burst: standard `{chance .12, x1.12, 1.0 s, cd 90}`; hard `{.6, x1.25, 1.6 s, cd 25}`. It fires when visible chase sight is lost.
Intercept: standard `{.30, lead .4 s}`; hard `{1.0, lead 1.0}`. It moves `last_known` ahead along the runner's velocity.
Slam: standard `{.10, 8 s}`; hard `{1.0, 20 s}`. **Its only call site is in infinite_route_test.gd (room doors). It is not wired in main_3d.**

These sit outside the gimmicks. They are mentioned per hunter only so they don't get lost.

### 0.7 Player meters (`player_meters.gd`), as they matter to gimmicks

Updated by the player controller every frame with `{flashlight_on, noise, crouching, hidden, in_safe_zone, heater_on}`. (Rust: the meters no longer take `heater_on`; the temperature item moves the target instead, see section 27 and the note under Wynelle.)

- **Exposure** 0-100 (the port does not track it yet; Aki and Xivvy read it):
  `target = hidden ? 0 : (18 + (flashlight ? 55 : 0) + clamp(noise / 10.5, 0, 1) * 40) * (crouching ? 0.55 : 1)`, clamped to 0-100.
  `exposure += (target - exposure) * clamp(1.8 * dt, 0, 1)`.
  `add_exposure(a)` adds and clamps to 0-100.
- **Noise meter** 0-100: `rise = banked_input + (hidden ? 0 : clamp(noise / 10.5, 0, 1) * 8 * dt)`; `meter = clamp(meter + rise - 5 * dt, 0, 100)`.
  `add_noise(a)` banks the amount for the next update.
  Player noise values: crouch 0.9, walk 4.2, sprint 10.5; 0 when not moving.
- **Temperature** 0-100, start/neutral 60 (not tracked in the port yet). See section 27 for the full model (it is what wakes Solyth and Wynelle).

### 0.8 Per-hunter tuning (`data/hunters/<id>.json`)

Every hunter listed has `taser_immune: true`, so the taser only slows her: `taser_slow_factor 0.5` for `taser_slow_seconds 3.0` (all difficulties). Every hunter has `melee_immune: false`, so a crowbar or axe stuns her (heavy): crowbar relaxed 4.5 / standard 4.35 / hard 3.9; axe 6.0 / 5.8 / 5.2. Melee range 2.45, half-angle 0.9.

| id | tier | speed | chase | prog | sight | hearing |
|---|---|---|---|---|---|---|
| aki | easy | 0.82 | 3.55 | 0.20 | 8.2 | 1.0 |
| briselle | easy | 0.84 | 3.62 | 0.18 | 7.7 | 1.0 |
| ceryth | intermediate | 0.78 | 3.45 | 0.17 | 8.6 | 0.88 |
| dovelle | easy | 0.80 | 3.55 | 0.18 | 8.0 | 1.0 |
| elise | difficult | 0.82 | 3.58 | 0.18 | 8.1 | 1.05 |
| ferryn | intermediate | 0.80 | 2.75 | 0.17 | 8.3 | 0.92 |
| glyra | easy | 0.81 | 3.52 | 0.18 | 7.9 | 1.12 |
| haelith | easy | 0.83 | 3.60 | 0.18 | 7.8 | 1.18 |
| ilyra | intermediate | 0.77 | 3.68 | 0.17 | 8.7 | 0.80 |
| javelle | easy | 0.80 | 3.56 | 0.18 | 8.0 | 1.0 |
| kathy | difficult | 0.82 | 4.15 | 0.20 | 8.2 | 1.0 |
| loxelle | easy | 0.80 | 3.555 | 0.18 | 7.9 | 1.12 |
| mavrine | intermediate | 0.80 | 3.50 | 0.17 | 8.2 | 0.95 |
| nyxie | special | 0.90 | 3.90 | 0.15 | 8.4 | 0.0 |
| opalie | easy | 0.83 | 3.47 | 0.18 | 7.7 | 1.1 |
| porcelle | intermediate | 0.80 | 3.55 | 0.17 | 8.0 | 0.9 |
| quenelle | easy | 0.81 | 3.50 | 0.17 | 8.1 | 0.85 |
| ravelle | difficult | 0.81 | 3.15 | 0.18 | 7.7 | 1.0 |
| solyth | special | 0.80 | 3.00 | 0.17 | 7.6 | 0.95 |
| thessine | intermediate | 0.78 | 3.44 | 0.17 | 8.4 | 0.9 |
| umbriel | difficult | 0.82 | 3.46 | 0.18 | 8.5 | 0.86 |
| verrine | intermediate | 0.83 | 3.66 | 0.18 | 7.7 | 1.1 |
| wynelle | difficult | 0.81 | 3.00 | 0.18 | 7.8 | 1.15 |
| xivvy | intermediate | 0.97 | 4.35 | 0.12 | 6.72 | 0.84 |
| ysolde | easy | 0.80 | 3.54 | 0.18 | 8.1 | 1.0 |
| zellia | easy | 0.83 | 3.60 | 0.17 | 8.0 | 0.95 |

Corner-peek chance per hunter (`hunter_peek.gd`, before the difficulty scale: standard x1.0, hard x1.15; relaxed falls back to standard). Hunters in NEVER have chance 0: wynelle, haelith, ysolde, opalie, verrine, zellia, mavrine, ceryth, briselle, ferryn, javelle, aki. Tuned ones: nyxie .9 ambush, thessine .7 ambush, glyra .8 ambush, ilyra .65, quenelle .55 ambush, umbriel .55 ambush, dovelle .45, porcelle .45 ambush, xivvy .3, ravelle .3, kathy .15, elise .12, loxelle .1, solyth .1. Everyone else uses the default .35 burst.

---

## Note that matters for several hunters: what exposure and noise actually do in Godot

[observed] `meters.exposure` is read only by the HUD and by Xivvy's lock radius. `meters.noise_meter` is read only by the HUD. Hunter perception does **not** read either meter: brain sight uses `LightField` (`target_exposure`) and the per-frame `target.noise`. So Aki's exposure bursts and Ferryn's `add_noise(100)` are HUD-only effects in the Godot build. The mechanical part of Ferryn's flash is the `facility_alert`.

For the port: a faithful minimum is to track the two meters for the HUD. If you want Aki to bite, the most defensible extension is "while `exposure >= 99`, every active hunter treats the runner as flashlight-on for sight". That is **not** in the Godot code; it is a suggestion.

---

## 1. Aki: phantom exposure (`aki_gimmick.gd`)

**Hooks:** `before_perception` only. Everything else is default: capturable, ordinary chaser via the shared brain.

**State:** `phase in {IDLE, DELAY, INTERVAL}` (starts IDLE), `bursts_remaining: i32 = 0`, `timer: f32 = 0`.

**before_perception (per frame):**
1. IDLE: if `brain.flash_triggered` (the previous frame's value, see 0.2 step 8): `phase = DELAY`, `timer = AKI_INITIAL_DELAY (10.0)`, `bursts_remaining = AKI_BURST_COUNT (3)`, return. Otherwise return.
2. DELAY or INTERVAL: `timer = max(0, timer - dt)`. If `timer > 0`, return.
3. Burst: `meters.add_exposure(AKI_BURST_EXPOSURE = 100.0)`; `bursts_remaining -= 1` (floored at 0). Emit `{"what":"message","sound":"alert","message":"AKI PHANTOM BURST // EXPOSURE 100%"}`, which is dropped at shipping settings.
4. If `bursts_remaining <= 0`: `phase = IDLE`, `timer = 0`. Otherwise `phase = INTERVAL`, `timer = AKI_BURST_INTERVAL (5.0)`.

Timeline: flash at t, then bursts at about t+10, t+15, t+20. Flashes during DELAY/INTERVAL are ignored. Re-arming needs the beam fully released (beam attention back to 0) and then another 0.35 s of beam on her.

**Same on every difficulty** (there are no per-difficulty constants). No RNG.

**World needs:** the player exposure meter (missing in the port), player facing, and beam attention (0.3).
**Port with existing systems:** implement the exposure meter from 0.7 (it is pure math on flashlight/noise/crouch/hidden), then apply `add_exposure(100)`. If you skip the meter, Aki is a plain chaser plus an invisible timer, which is what the player experiences in Godot anyway (see the note above).

**Tells:** none in shipping (the toast is off). The HUD exposure bar spikes to 100 three times.
**Peek:** never (NEVER list). **Hard abilities:** none.

---

## 2. Briselle: relay tracker (`briselle_gimmick.gd`)

**Hooks:** `after_perception`, `speed_scale`.

**State:** `locked = false`, `lock_remaining = 0`, `relay_cell = (-1,-1)`, `collected_seen = -1`, `known: Vec<Cell>` (the collected relay cells from last frame).

**after_perception:**
1. `collected = relays.filter(collected)`.
   - First frame (`collected_seen < 0`): set `collected_seen = collected.len()` and do nothing else.
   - Else if `collected.len() > collected_seen`: `newest = last element of collected`, replaced by the last entry whose cell is not in `known`. Then call `lock_on(newest)`.
   - Then `collected_seen = collected.len()` and `known = cells(collected)`.
2. If `!locked`, return.
3. If `brain.flash_triggered` (this frame): `locked = false`, `lock_remaining = 0`, `brain.apply_stun(BRISELLE_FLASH_STUN_SECONDS = 3.0)`, `state = SEARCH`, `has_last_known = false`. Emit message "SIGNAL LOCK BROKEN" (dropped). Return.
   [quirk] SEARCH with no `search_target` set and `search_timer` left over: after the stun she runs the leftover search timer, then goes IDLE.
4. `lock_remaining -= dt`. If `<= 0`, then `locked = false`, return.
5. If `state != CHASE`: `state = INVESTIGATE`, `last_known = relay_cell + 0.5`, `has_last_known = true`.

`lock_on(relay)`: `relay_cell = relay`, `locked = true`, `lock_remaining = BRISELLE_HUNT_SECONDS (12.0)`, `state = INVESTIGATE`, `last_known = relay centre`. Emit message "BRISELLE DETECTED RELAY // BREAK HER LOCK WITH LIGHT" (dropped).

**speed_scale:** `1.25` (BRISELLE_SPEED_SCALE) while locked, else 1.0.

**Important:** the hook runs only while she is active. On standard she activates at relay 1. The first frame after activation only records the count, so the relay that woke her does not trigger a lock. Every later pickup does.

**Unused constants:** `BRISELLE_ROOM_CHANCE 0.5`, `BRISELLE_SABOTAGE_DWELL 5.0`, `RELAY_RESECURE_SECONDS 2.6`. These belong to the retired relay saboteur; nothing calls them. **No relay sabotage is performed by any gimmick in this codebase.** [observed]

**World needs:** relays with a `collected` flag (the port has them). Fully portable.
**Tells:** none in shipping. She suddenly beelines for the relay you just took, 25% faster.
**Peek:** never.

---

## 3. Ceryth: hiding trap (`ceryth_gimmick.gd`)

**Hooks:** `after_perception`, `holds_position`, `speed_scale`, `hidden_from_view`, `immune_to_capture` (always **true**: she never grabs). Plus a custom `resolve_entry(cell, flashlight_active)` that the player's hide action calls.

**State:** `trap_spot = (-1,-1)`, `eyes_red = true`, `dispersed = false`, `retreating = false`, `answered = false`, `next_spot = (-1,-1)`.

**after_perception:**
1. If `dispersed`, return.
2. If `answered && !retreating`: `answered = false`, then `start_retreat()`. If that set `dispersed`, return.
3. If `retreating`:
   - `perch = hiding_exit_position(next_spot)` (the nearest open cell for the player radius 0.24 next to that spot).
   - Force `state = INVESTIGATE`, `last_known = perch`.
   - If `distance(pos, perch) < 0.9`: `retreating = false`, `trap_spot = next_spot`, `eyes_red = (next_index(2) == 0)`, snap `pos = perch`.
   - Return.
4. If `trap_spot.x < 0` (the first time she is active): if there are no hiding spots, return. Else `spot = hiding_spots[next_index(len)]`, `trap_spot = spot`, `eyes_red = next_index(2) == 0`, **teleport** `pos = hiding_exit_position(spot)`.

`start_retreat`: `options` = every hiding spot except `trap_spot` with `distance(cell, pos) >= CERYTH_MINIMUM_RETREAT (7.0)`, using integer cell coordinates (not centres) against her position. If empty, `dispersed = true`. Else `next_spot = options[next_index(len)]`, `retreating = true`, `trap_spot = (-2,-2)`.

**holds_position:** `!dispersed && !retreating && trap_spot.x >= 0`. She never moves except while retreating.
**speed_scale:** while retreating, `6.4 / max(0.1, base_speed)` (CERYTH_RETREAT_SPEED = 6.4). Base speed is 0.78, so that is x8.205 applied to her INVESTIGATE (walk) speed. Effective speed is about `(0.78 + progression) * 8.205 * hunter_speed`, i.e. about 6.4 c/s at depth 1 with no relays. Otherwise 1.0.
**hidden_from_view:** `dispersed`. HunterView3D also retires her model when dispersed.

**resolve_entry(cell, flashlight_active)** (the player presses hide on a spot; `flashlight_active = flashlight_on && lamp_charge > 0`):
- If `dispersed || cell != trap_spot`: return `{}` (normal hide).
- `answered = eyes_red ? !flashlight_active : flashlight_active`. Red eyes want the light off; blue eyes want it on.
- Wrong answer: return `{trapped: true}`. The player controller then hides the player, sets `hiding_lock_timer = CERYTH_LOCK_SECONDS (30.0)` and forces the flashlight off. `leave_hiding` is refused while the lock runs ("Locked in hiding for Ns."), except a forced pull (Javelle/Loxelle). The HUD prompt reads "LOCKED IN // Ns".
- Right answer: set `answered = true` and return `{cleared: true}`. Normal hide. Next frame she retreats to a new spot at least 7 cells away with re-rolled eyes. She disperses only when no such spot exists.

**RNG:** brain stream (spot pick, eye colour, retreat pick).
**Same on all difficulties.**
**World needs:** lockers (the port has them) with an `open_direction` / exit cell, and the hide interaction routed through a resolver. Fully portable.
**Tells:** her eye colour (the model's red or blue eyes) and her standing beside a locker. The callouts are off. [unknown] The exact eye visual code: `eyes_red` is replicated, but I did not find its renderer in HunterView3D. Render it as eye glow colour #ff0000 vs blue.
**Peek:** never. **Capture:** never.

---

## 4. Dovelle: no gimmick

`HunterGimmick.for_id("dovelle")` returns null. She is the pure shared brain with her JSON tuning (0.8 / 3.55 / 0.18 / 8.0 / 1.0, peek .45 burst). Nothing to implement beyond the brain. Fully portable (already done).

---

## 5. Elise: running-data ratchet (`elise_gimmick.gd`)

**Hooks:** `before_perception`, `breaches_hiding` (always **true**).

**State:** `progress: f32 = 0` (0..1, **never decays**), `locked = false`.

**before_perception:**
1. If `!locked`:
   - If `target.sprinting && !target.in_safe_zone`: `progress = min(1, progress + dt / ELISE_SECONDS_TO_FULL (50.0))`.
     `sprinting` is `movement_mode == "sprint" && noise > 0`, so the player must actually be moving.
   - If `progress >= 1`: `locked = true`.
2. If `locked`, every frame: `state = CHASE`, `last_known = target.position`, `has_last_known = true`, `lost_sight_timer = 0`. This includes while the player is hidden or in the safe zone. In the safe zone she cannot grab (the capture check excludes it), but she still paths to the player.

Progress only accumulates while she is active (the hook does not run while inactive), so sprinting before activation is free.

**breaches_hiding = true:**
- (a) Perception can sense a hidden player (`can_sense` ignores hidden).
- (b) She can grab a hidden player if `has_last_known && last_known.distance(player) < 0.9` and they are in reach. Once locked, `last_known` is always the player, so a locker is no defence at all.

**Hard abilities:** slam on standard and hard (door slam: not wired on the finite floor).
**Difficulty:** none in the gimmick.
**World needs:** the sprint flag. Fully portable.
**Tells:** none. In Godot the progress is invisible.
**Peek:** .12 burst.

---

## 6. Ferryn: persistent stalker (`ferryn_gimmick.gd`)

**Hooks:** `after_perception` only.

**State:** `stalker_origin: Vec2`, `has_origin = false`.

**after_perception:**
1. On the first call: `stalker_origin = pick_origin()`; **teleport** `pos = origin`.
   `pick_origin`: `start_id = room_at(start cell centre).id`; candidates = every room whose id differs; `room = candidates[next_index(len)]`; return `nearest_open_position(room.center + 0.5, radius 0.33)`. If there are no candidates or no open cell, keep the current position.
2. `flashed = brain.flash_triggered` (this frame), `hidden`, `in_safe_zone`.
3. If any of the three is true: **teleport** `pos = origin`, `state = WANDER`, `has_last_known = false`.
   If `flashed`: `meters.add_noise(FERRYN_FLASH_NOISE = 100.0)` (HUD only), and emit `{"what":"facility_alert","at":player_pos,"except":"ferryn"}`. main_3d sends every other active hunter to INVESTIGATE at that point (normally Nyxie/Solyth if awake).
   Return.
4. Otherwise: `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`. She is an omniscient permanent chase.

**Notes:**
- `has_wander_target` is not reset on a teleport. The next movement frame uses whatever wander target she had. [quirk]
- She is re-teleported every frame the player stays hidden or in the safe zone, so she sits at her origin until the player comes out.
- Her chase speed is very low on purpose (2.75).

**Hard abilities:** burst (hard).
**RNG:** one `next_index` on the first active frame.
**World needs:** rooms with ids/centres, the start room, and beam attention. Fully portable.
**Not ported in Godot:** "emberVeil" visibility. Skip it.
**Tells:** none, apart from her vanishing (teleport). The alert toast is off.
**Peek:** never.

---

## 7. Glyra: music boxes (`glyra_gimmick.gd`, `world/interactables/music_boxes.gd`)

**Hooks:** `after_perception`, `speed_scale`, `peek_chance`, `immune_to_capture`. Also `humming()` (presentation) and `wind()` (an interaction).

**MusicBoxes clock:**
- `left[id]` defaults to `wound_seconds()` = `MUSIC_BOX_WOUND_SECONDS[diff]`: standard **45**, hard **35**, nightmare 25 (ported: `data/music_box.json`). Relaxed has no key and falls back to **45**.
- `mood(left)`: `left <= 0` is STOPPED; `left <= MUSIC_BOX_WINDDOWN_SECONDS (15)` is WINDING_DOWN; otherwise PLAYING.
- `wind(id)` sets `left = wound_seconds`. `tick(dt)` subtracts `dt` from every box (floored at 0).
- Winding is a hold of `MUSIC_BOX_WIND_HOLD 1.5` s on the interactable ("HOLD INTERACT // WIND MUSIC BOX").

**State:** `boxes`, `mood = PLAYING`, `box_id`, `box_position`, `settle_left = 0`, `base_hearing = -1`, `was_mood = PLAYING`, `was_hunting = false`.

**after_perception:**
1. On the first call: `base_hearing = brain.hearing_scale`.
2. `pick_box`:
   - If `generator.music_boxes` is empty or missing (always the case on the finite floor): `box_id = "glyra-spawn-box"`, `box_position = her position at the first call`.
   - Else the nearest listed box.
3. `mood = boxes.mood(box_id)`.
4. If mood is PLAYING and `was_mood != PLAYING`: `settle_left = GLYRA_CALM_AFTER_WIND (3.0)`, `was_hunting = true`.
5. If mood is STOPPED and `was_mood != STOPPED`: if the target position is valid, `state != CHASE` and the player is not hidden, call `brain.direct_to(player_pos)` (INVESTIGATE there).
6. `was_mood = mood`. `hearing_scale = base_hearing * (STOPPED ? GLYRA_HUNT_HEARING_SCALE (1.5) : 1)`.
7. If WINDING_DOWN and `state == IDLE`: `idle_timer = min(idle_timer, 0.15)`.
8. If mood is not PLAYING, return.
9. PLAYING: `settle_left = max(0, settle_left - dt)`. If `> 0`, return.
10. `close = !hidden && distance(player) <= GLYRA_CALM_SIGHT (4.0)`.
11. If `state == CHASE && direct_chase_visible && was_hunting`, return (she finishes a visible chase).
12. `was_hunting = false`.
13. If `state in {CHASE, INVESTIGATE, SEARCH, PEEK} && !close`: `drift_home()`.
    Else if `state in {WANDER, IDLE} && distance(box_position) > GLYRA_CALM_LEASH (8.0)`: `drift_home()`.

`drift_home`: `state = WANDER`, `direct_chase_visible = false`, `has_last_known = false`. If there is no wander target, or it is more than 2.5 from the box: `wander_target = box_position + (1.2, 0).rotated((abs(hash(box_id)) % 628) / 100)`, then clear the path.

**speed_scale:** PLAYING `GLYRA_CALM_SPEED_SCALE 0.6`; STOPPED `GLYRA_HUNT_SPEED_SCALE[diff]` (standard 1.3, hard 1.3, nightmare 1.5; relaxed falls back to 1.3; nightmare ported in `hunters/gimmicks/glyra.rs`, `hunt_speed`); WINDING_DOWN 1.0.
**peek_chance:** 0 while PLAYING, else base (.8 ambush).
**immune_to_capture:** `mood == PLAYING`.
**humming():** `mood == PLAYING` (HunterView3D plays a hum loop).
**wind():** `boxes.wind(box_id)` (creates "glyra-spawn-box" if needed). main_3d `_wind_music_box` calls it when the player uses the MUSIC_BOX interactable, which is placed `_near_start(1, 4)` cells from start. Toast: "Music box wound. The room settles."

**[quirk] Finite floor:** nothing in main_3d ever calls `boxes.tick(dt)`. Only `infinite_route_test.gd` ticks its shared clock. Her private box therefore stays at 45 s forever: **on the finite floor Glyra is permanently PLAYING** (calm, slow, uncatchable). Also, her "box" is her own spawn position, not the physical music box, which sits near the player start. **For the port, do what was intended:** tick her box every frame. Either use the physical box (`box_position` = the box cell) or keep the spawn-anchored box but make winding the physical box rewind her clock.

**World needs:** a music box interactable (missing). Simplification: one music-box prop in the start room, 1-4 cells from start, with a 1.5 s hold-to-wind, plus one timer. Everything else uses existing systems.
**Tells:** humming while PLAYING; silence when STOPPED; the box tune slows and goes off-key in its last 15 s (MusicBoxTune: positional, about 12 m). Toast on wind.
**Peek:** .8 ambush (only when not PLAYING). **Hard abilities:** none.

---

## 8. Haelith: relay pursuer (`haelith_gimmick.gd`)

**Hooks:** `after_perception` only. Capturable.

**after_perception (every frame, no state):**
1. `pursuit = relays`; `open = uncollected(relays)`.
2. If `open.len() > 1`: remove `open[0]` from `pursuit`. The first uncollected relay is left to Opalie, **even when Opalie is not on the floor**. [observed; with one route hunter per floor that is always the case]
3. `relay = nearest uncollected in pursuit`, as a cell centre (+0.5). If none is left, return (she falls back to the ordinary brain).
4. `state = INVESTIGATE`, `has_last_known = true`, `last_known = relay`.

Consequences:
- She **never stays in CHASE**: perception may set CHASE, and this hook overrides it the same frame.
- On reaching the relay, `_handle_arrival` sets SEARCH, and the next frame she is pushed back to INVESTIGATE on the same relay. So she camps it until it is taken, then moves to the next nearest.
- A runner who walks into reach is still grabbed (the capture check is positional).

**[quirk]** She is on the pounce roster (standard and hard), but a pounce needs `state == CHASE && direct_chase_visible` at `hard.update` time, which runs after this hook. So a pounce never fires for her. Burst and intercept are not hers.

**World needs:** relays with `collected`. Fully portable.
**Tells:** footsteps only. Ordinary positional hunter footsteps (main_3d `_update_local_audio`, within 9 cells, pan by side) apply to every non-IDLE hunter. Nothing gimmick-specific.
**Peek:** never.

---

## 9. Ilyra: corner observer and shadow pounce (`ilyra_gimmick.gd`)

**Hooks:** `after_perception`, `holds_position` (always **true**: she never walks), `sight_scale` (always **1.8**), `hidden_from_view` (true while banished), `immune_to_capture` (always **true**).

**State:** `has_corner = false`, `hidden_remaining = 0`, `saw_player = false`, `tagged = false`, `pounce_remaining = 0`.

**after_perception:**
1. On the first call: `respawn_corner()`.
   That picks `room = rooms[next_index(len)]`, then one of 4 corners `[(x+1.5, y+1.5), (x+w-1.5, y+1.5), (x+1.5, y+h-1.5), (x+w-1.5, y+h-1.5)]` with `next_index(4)`. She **teleports** to `nearest_open_position(corner, 0.33)`, or to the corner itself if none is found.
2. If `hidden_remaining > 0`: `hidden_remaining -= dt`, return. While banished, flashes and sightings are ignored.
3. If `brain.flash_triggered` (this frame, so a 0.35 s beam hold on her): `hidden_remaining = ILYRA_BANISH_SECONDS (30.0)`, then clear `saw_player`, `tagged` and `pounce_remaining`. Return.
4. `sees = brain.direct_chase_visible || shares_room(target)`.
   `shares_room`: the target is not hidden, not in the safe zone, and `room_at(her).id == room_at(player).id`, with her room non-empty. Corridors never count.
5. If `sees`: `saw_player = true`.
   Else if `saw_player && !tagged`: `tagged = true`, `pounce_remaining = (nightmare ? 0 : ILYRA_POUNCE_DELAY 15.0)` (nightmare ported: `hunters/gimmicks/ilyra.rs`, `delay_for`, tested by `shipped_data_pounces_at_once_on_nightmare_only`), `saw_player = false`.
6. If `tagged`: `pounce_remaining -= dt`. If `<= 0`: emit `{"what":"stun","seconds":ILYRA_STUN_SECONDS (5.0)}`. main_3d sets the player `stun_timer = max(..., 5.0)` **wherever the player is**. Then `tagged = false` and `respawn_corner()` (a new random room and corner).

Her state machine still runs (she may "CHASE" in place), but she never moves. Sight is x1.8, so 8.7 x 1.8 = 15.66 base before the light and difficulty factors.

A 5 s stun is dangerous only if another awake hunter is near: Nyxie/Solyth, or the route hunter. Ilyra is the route hunter, so normally only the specials.

**Difficulty:** relaxed, standard and hard all use 15 s (nightmare 0).
**RNG:** brain stream (room plus corner on every respawn).
**World needs:** rooms with a rect (x, y, w, h) and `room_at`. Fully portable.
**Tells:** none in Godot (the toast is off). She stands motionless in a room corner; while banished she fades to alpha 0.28. The player suddenly being stunned 15 s after breaking line of sight is the tell.
**Peek:** .65 burst on paper, but `holds_position` blocks peeking, so in effect never.

---

## 10. Javelle: hiding timeout (`javelle_gimmick.gd`)

**Hooks:** `after_perception` only. Capturable normally.

**State:** `held = 0`, `ejecting = false` (read by main_3d the same frame), `ejected_to: Vec2`.

**after_perception:**
1. `ejecting = false`.
2. If the target is not hidden: `held = 0`, return.
3. `cell = target.hiding_spot`. If invalid: `held = 0`, return.
4. `spot = cell + 0.5`. If `distance(her, spot) >= JAVELLE_REACH (2.0)`: `held = 0`, return.
5. Pin her there: `state = INVESTIGATE`, `last_known = spot`, `search_visits = 0`.
6. `held += dt`. If `held < JAVELLE_HIDING_SECONDS (8.0)`, return.
7. `held = 0`, `ejecting = true`, `ejected_to = hiding_exit_position(cell)`, `brain.apply_stun(JAVELLE_EJECT_STALL = 2.2)`.

main_3d then dispatches `{"what":"eject","at":ejected_to}`, which calls `player.leave_hiding(at, forced = true)` (this also overrides Ceryth's lock). The player stands in the open next to her while she is stunned for 2.2 s.

**Interaction with the camp guard** [observed]:
- `camp_limit` is relaxed 6 / standard 9 / hard 12 (within `CAMP_RADIUS 2.0` of the hidden player).
- On relaxed, the guard fires at 6 s and sets WANDER. But Javelle's `after_perception` runs after it and re-pins her (the guard only skips perception), so the 8 s eject still happens.
- Port the hook order exactly and you get the same result.

**Hard abilities:** slam (hard; not wired on the finite floor).
**World needs:** lockers and the player's `hiding_spot` cell. Fully portable.
**Tells:** none (the toast "JAVELLE PULLED YOU OUT OF HIDING" is off). Physically: she stands over your locker, and you are pulled out after 8 s.
**Peek:** never.

---

## 11. Kathy: search beam (`kathy_gimmick.gd`)

**Hooks:** `before_perception` only.

**before_perception (no state):**
1. If the target is hidden or in the safe zone, return.
2. If `beam_reaches(her pos, her facing, player pos, KATHY_BEAM_RANGE 11.5, KATHY_BEAM_HALF_ANGLE 0.24 rad)` holds, including LOS:
   - `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`.
   - `direct_chase_visible = true`, `_ability_direct_sight = true`. Perception's lost-sight branch keeps her visible-direct-steer with this.

`facing` is her last movement direction (unchanged while standing still). The cone is independent of the player's light, the difficulty sight multiplier and darkness. It is pure geometry.

**Not ported (by design):** `affectedSummon`. Its precondition, other active route hunters, never occurs.
**Hard abilities:** intercept (hard).
**World needs:** none new. Fully portable.
**Tells:** none in code beyond the model. [unknown] Whether a visible beam cone is rendered; I found no reference.
**Peek:** .15 burst.

---

## 12. Loxelle: passive incinerator (`loxelle_gimmick.gd`)

**Hooks:** `after_perception`, `holds_position` (true during the windup or charge), `immune_to_capture` (always **true**: she never grabs). Output goes through `toggles: Vec<Toggle>`, drained by main_3d.

**State:** `passive_timer = 0`, `regenerating: Map<Cell, f32>`, `charge_phase in {"", approach, windup, charge}`, `charge_cell`, `charge_timer`, `charge_candidate` (the spot dict). There is also a door-breach phase set, which is **only triggered from infinite_route_test.gd**; on the finite floor it is never entered. Skip it, or note it as future work.

**after_perception:**
1. If a door charge is active, update it and return (unused on the finite floor).
2. If `target.pesticide_active` (Zellia's 60 s pesticide), return. Note that regeneration does not tick either.
3. Tick regeneration: for each `regenerating[cell] -= dt`. At `<= 0`, remove it and push the toggle `{cell, disabled: false}`.
4. If `charge_phase != ""`: `update_charge()`, return.
5. If `state in {CHASE, INVESTIGATE}`: `passive_timer = 0`, return.
6. `passive_timer += dt`. If `< LOXELLE_INTERVAL (5.0)`, return. Reset it to 0.
7. Roll: if `next_index(1000) >= int(LOXELLE_CHANCE 0.25 * 1000) = 250`, return.
8. `room = room_at(her pos)`. If empty (she is in a hall), return.
9. `candidate` = the nearest hiding spot with `spot.room_id == room.id` that is not regenerating. If none, return.
10. `charge_cell = candidate`, `charge_phase = "approach"`, `state = INVESTIGATE`, `last_known = cell + 0.5`.

**update_charge** (it forces `state = INVESTIGATE` and `last_known = impact = cell + 0.5` every frame, so **it overrides a chase**):
- approach: ordinary pathing. When `distance <= LOXELLE_CHARGE_START_DISTANCE (0.95)`, go to windup with `timer = LOXELLE_CHARGE_WINDUP (0.42)`.
  [quirk] There is no timeout. If the spot is unreachable she keeps trying.
- windup: `timer -= dt`. At `<= 0`, go to charge with `timer = LOXELLE_CHARGE_SECONDS (0.70)`.
- charge: `timer -= dt`. If `distance > LOXELLE_CHARGE_IMPACT_DISTANCE (0.20)`: `travel = min(LOXELLE_CHARGE_SPEED 6.8 * dt, dist)`, `facing = dir`, and move only if `can_occupy(next, 0.33)` (one check, no sub-stepping). When `distance <= 0.20` or `timer <= 0`, call `resolve_charge()`.

**resolve_charge:**
- Occupied (`target.hidden && target.hiding_spot == charge_cell`): push `{cell, eject_to: hiding_exit_position(spot), charge_impact: true, temperature_add: LOXELLE_BREAK_TEMPERATURE 30.0, noise_add: LOXELLE_BREAK_NOISE 11.0}`.
  main_3d ejects the player (forced) and sends `meter_effect`: temperature +30 (clamped to 100), noise +11 banked. The spot is **not** disabled in this case.
- Empty: push `{cell, disabled: true, charge_impact: true}`, and `regenerating[cell] = LOXELLE_REGEN_SECONDS (30.0)`. main_3d calls `hiding_spot.set_disabled(true)`, so the spot cannot be used until it is re-enabled after 30 s.
- Both cases: clear the charge, then `brain.apply_stun(LOXELLE_BREAK_STALL = 10.0)`. She stalls 10 s.

**Difficulty:** none (the comment mentions a nightmare regen split; it is not implemented).
**RNG:** brain stream, one `next_index(1000)` per 5 s while idle or wandering or searching.
**World needs:** lockers with a `room_id`, a disable/enable flag on a locker, and **temperature** (for the +30; the port does not track it yet). Simplification if temperature is skipped: drop the `temperature_add`. If temperature exists, +30 can push the player toward the Solyth latch (section 27).
**Tells:** a visible brace-and-charge animation ("charge" clip during windup and charge, HunterView3D/Emoter). Disabled lockers. Toasts are off.
**Peek:** .1 burst (she never captures anyway).

---

## 13. Mavrine: lattice mark (`mavrine_gimmick.gd`)

**Hooks:** `before_perception`, `holds_position` (`phase == SURVEY`).

**State:** `phase` (starts COOLDOWN), `timer` (starts `MAVRINE_COOLDOWN_DURATION 4.2`), `has_mark = false`, `mark: Vec2`, `last_mark_cell = (-1,-1)`.

Phase durations: COOLDOWN 4.2, SURVEY 0.9, PROJECT 6.0, PING 0.45. `MAVRINE_SURVEY_DELAY 6.2` is used only in the chase hold.

**before_perception:**
1. If `state == CHASE && phase in {COOLDOWN, SURVEY}`: `timer = max(timer, 6.2 * 0.35 = 2.17)`, return. A live chase freezes the cycle.
2. `timer -= dt`. If `timer <= 0`:
   - COOLDOWN goes to SURVEY.
   - SURVEY: `has_mark = false`, `place_mark()`, then PROJECT. This happens even if no mark was found; then PROJECT just runs out.
   - PROJECT or PING: `has_mark = false`, then COOLDOWN.
3. If `phase != PROJECT || !has_mark`, return. If the player is hidden or in the safe zone, return. If `distance(player, mark) > MAVRINE_MARK_TRIGGER_RADIUS (0.62)`, return.
4. Enter PING. `state = INVESTIGATE`, `last_known = mark`: the trip point, not the player.

**place_mark** (in this order):
1. Candidates are floor cells that pass every filter:
   - `cell != last_mark_cell`
   - not a safe-zone blocked cell
   - `>= 4.0` from the start centre and `>= 2.5` from the lift centre
   - `>= 3.0` from her
   - `>= 3` of 4 neighbours are floor
   - `can_occupy(centre, 0.16, blocked)`
   - `<= 30.0` from her
   - not within `1.25` of any relay centre or hiding-spot centre
2. Fisher-Yates shuffle from the end with `brain.next_index(i + 1)`. This consumes (n-1) RNG draws, which matters for parity.
3. Try the first `min(48, n)` candidates. The first one with a non-empty path from her cell becomes the mark, and `last_mark_cell` is updated.

A cycle is about 4.2 + 0.9 + 6.0 = 11.1 s, plus 0.45 on a ping.

**Hard abilities:** intercept (standard and hard).
**World needs:** none new. Fully portable.
**Tells:** the mark is **invisible** (no renderer found). The visible tell is her standing still for 0.9 s (SURVEY), then heading straight to a spot you just crossed.
**Peek:** never.

---

## 14. Opalie: relay guardian (`opalie_gimmick.gd`)

**Hooks:** `after_perception`, `holds_position` (`on_station`), `immune_to_capture` (`on_station`). Plus a custom `blocks(point)` that main_3d's interaction veto calls.

**State:** `station: Vec2 = ZERO`, `on_station = false`.

**after_perception:**
1. If there are no relays, return.
2. `relay` = the first relay with `!collected` (relays[0] if all are collected). `post = nearest_open_position(relay + 0.5, 0.33)`. If empty, return.
3. If `state == CHASE && !target.hidden`: `on_station = false`, `station = ZERO`, return. She chases like anyone once she sees or hears you.
4. If `distance(her, post) > OPALIE_GUARD_ARRIVAL (0.75)`: `on_station = false`, `station = ZERO`, `state = INVESTIGATE`, `last_known = post`, return. She walks back.
5. Else: `on_station = true`, `station = relay centre`.

**blocks(point):** `on_station && distance(station, point) <= OPALIE_SUPPRESSION_RADIUS (1.5)`.

**main_3d `_guardian_blocking(interactable)`** (`player.interaction_veto`): evaluated for every interaction at `source_cell + 0.5`. If any active hunter's gimmick blocks it:
- **every active hunter** gets `enter_search_from(player_pos, 1.8)`: SEARCH at the player position with a 1.8 s search timer;
- the interaction is refused with the message `"<NAME> IS GUARDING THIS // DRAW HER OFF"`. This message IS shown: it is the interaction result, not a world event.

That covers anything within 1.5 cells: the relay, a battery, a locker, the lift, and so on.

Note that a hidden player makes step 3 false, so she goes back to her post.

**World needs:** relays and an interaction veto hook. Fully portable.
**Tells:** she stands still on a relay; the refusal message.
**Peek:** never.

---

## 15. Porcelle: map ghost (`porcelle_gimmick.gd`)

**Hooks:** none that change behaviour. `hidden_from_view` is overridden but returns **false**. Her AI is the plain shared brain.

**The gimmick lives in main_3d presentation:**
- `_radar_marks()`: Porcelle is the **only** hunter the minimap shows without the admin reveal, and only while she is active. `_map_marks` flags her `ghost: true`, and the fullscreen map shows her too.
- Opening the map while she is active unlocks her dossier (`Backend.account_discover_hunter`).

**World needs:** the minimap and fullscreen map with a hunter marker. Portable (just the marker).
**Tells:** her radar dot. **Peek:** .45 ambush.

---

## 16. Quenelle: flashlight tracker (`quenelle_gimmick.gd`)

**Hooks:** `before_perception`, `speed_scale`.

**State:** `tracking = false` (a **permanent** latch), `track_elapsed = 0`, `activations = 0`, `was_on = false`, `speed_scale = 1`.

**before_perception:**
1. `lit = target.flashlight_on && lamp_charge > 0 && !hidden`.
2. If `lit && !was_on`: `activations += 1` (counts off-to-on flicks, before and after the lock). Then `was_on = lit`.
3. If `!tracking`: `track_elapsed = lit ? track_elapsed + dt : 0`. When `track_elapsed >= lock_seconds`, set `tracking = true` and emit message "QUENELLE LOCKED ON" (dropped).
   `lock_seconds`: `QUENELLE_FLASHLIGHT_LOCK_SECONDS 8.0` on relaxed, standard and hard; nightmare 5.0. The light must stay on continuously for 8 s.
4. `progress = min(1, activations / QUENELLE_SPEED_RAMP_ACTIVATIONS (10))`.
   `speed_scale = (tracking && lit) ? 1 + progress * (QUENELLE_MAX_SPEED_SCALE 1.45 - 1) : 1`, so up to x1.45 after 10 flicks.
5. If `tracking && !in_safe_zone`: `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`. This applies even while the player is hidden: she paths to the locker. She cannot grab a hidden player, since she has no `breaches_hiding`.

Counter-intuitive: once locked she tracks you forever, but she is only *faster* while your light is on.

**Hard abilities:** burst (standard and hard), intercept (hard).
**World needs:** none new. Fully portable.
**Tells:** none (the toast is off). **Peek:** .55 ambush.

---

## 17. Ravelle: tripwire trapper (`ravelle_gimmick.gd`)

**Hooks:** `before_perception`, `speed_scale`. A custom `cut_wire(i)` exists (see below).

**State:** `wires: Vec<{a, b, position, cell, triggered}>`, `has_wires = false`, `refresh_timer = 0`, `window_triggered = false`, `triggered_count = 0` (permanent for the floor).

**before_perception:**
1. `refresh_timer -= dt`. If `!has_wires || refresh_timer <= 0`:
   - If `!has_wires || !window_triggered`, call `place_wires()`. A window in which a wire fired keeps the same (spent) wires.
   - Then `refresh_timer = RAVELLE_TRIPWIRE_REFRESH (30.0)` and `window_triggered = false`.
2. If the target is hidden or in the safe zone, return.
3. For each untriggered wire with `dist_to_segment(player, a, b) <= RAVELLE_TRIGGER_RADIUS (0.8)`:
   - `triggered = true`, `window_triggered = true`, `triggered_count += 1`.
   - Emit `{"what":"stun","seconds":RAVELLE_TRAP_STUN 1.6}`. The player is stunned 1.6 s.
   - Emit `{"what":"facility_alert","at":player_pos}` with **no `except`**, so every active hunter including Ravelle goes INVESTIGATE there after the loop. That overwrites her CHASE back to INVESTIGATE at the trip point. [quirk; perception re-acquires her next sense tick if she sees you.]
   - `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`.

**place_wires:**
- `candidates = doorway_cells()`: floor cells where (left and right are floor AND (up or down is not floor)), OR (up and down are floor AND (left or right is not floor)).
- While `wires < RAVELLE_TRIPWIRE_COUNT (2)`: pick `next_index(len)` and remove it.
- `centre = cell + 0.5`. `across = (0, 1.42)` if left and right are both floor, else `(1.42, 0)`. Then `a = centre - across`, `b = centre + across`.
  The wire is 2.84 long, perpendicular to the open axis, spanning a 3-wide hall.

**speed_scale:** `1 + triggered_count * RAVELLE_SPEED_PER_TRIGGER (0.08)`. It is permanent and cumulative, with no cap. Her base chase (3.15) is deliberately low.

**Wire visibility and cutting:** these live only in `infinite_route_test.gd`, not main_3d.
- There, live wires are drawn as a thin shimmering wire with bells, refreshed every 0.25 s.
- Holding INTERACT for `RAVELLE_CUT_SECONDS 1.2` within `RAVELLE_CUT_REACH 1.3` cuts the wire (`cut_wire(i)` sets `triggered = true` without a stun). Every active hunter within 14 cells then goes INVESTIGATE at the player, and an alert plays.

**On the finite floor (main_3d) the wires are invisible and cannot be cut.** The port can choose either. Recommendation: draw them faintly, since that is the newer design (docs/RAVELLE_TRAPS.md is referenced).

**World needs:** none new for the trigger. Fully portable. Optional: a wire mesh plus a hold-to-cut interaction.
**Tells:** the stun plus the bells. Finite has no bell sound: the alert sound is stripped. [observed]
**Peek:** .3 burst.

---

## 18. Thessine: sound mimic (`thessine_gimmick.gd`)

**Hooks:** `before_perception` only. Her AI is otherwise plain and capturable.

**State:** `timer = THESSINE_MIMIC_INTERVAL (10.0)`.

**before_perception:**
- `timer -= dt`. If `> 0`, return. Then `timer += 10.0` (overrun preserved).
- If `next_float() >= THESSINE_MIMIC_CHANCE (0.5)`, return.
- Emit `{"what":"thessine_mimic","rare": next_float() < THESSINE_MIMIC_RARE_CHANCE (0.08), "choice": next_float()}`. That is 3 RNG draws on success and 1 on failure.

**main_3d `_play_thessine_mimic`:**
- `cueable` = sorted ids of **active** hunters other than Thessine that are in `[nyxie, glyra, ferryn, briselle, loxelle, ravelle, haelith]`.
- If `rare` or `cueable` is empty: `Sound.mimic(rare)`, a procedural fake cue (rare variant if rare).
- Else `source = cueable[int(clamp(choice, 0, .999999) * len)]`:
  - nyxie: `Sound.nyxie_timer_wake()`
  - glyra: `Sound.music_box()`
  - haelith: `Sound.hunter_footstep(pan 0, vol .09)`
  - anyone else: `Sound.alert()`

On the finite floor, cueable can only contain Nyxie when she is awake (only one route hunter is drawn), so this is almost always `Sound.mimic`. The sounds are non-positional (pan 0).

**World needs:** audio cues only. Fully portable.
**Peek:** .7 ambush.

---

## 19. Umbriel: shapeshifter (`umbriel_gimmick.gd`)

**Hooks:** `before_perception` only. Cosmetic.

**State:** `disguise_id = ""`, `timer = UMBRIEL_DISGUISE_INTERVAL (30.0)`.

**before_perception:**
- `timer -= dt`. If `> 0`, return. Then `timer = 30.0`.
- `excluded = ["kathy"] + [self id] + [current disguise]`.
- `pool = HunterRoster.disguise_ids(excluded)`: every PROFILES id (A-Z plus the 12 void ids) except umbriel, nyxie, solyth and the excluded, sorted alphabetically.
- `disguise_id = pool[next_index(len)]`.

She starts as herself and changes 30 s after activation. Only the model and name change; HunterView3D swaps the model. Her dossier discovery records the disguise id. Stats and AI stay Umbriel's on all of relaxed, standard and hard.

**Hard abilities:** burst (hard).
**World needs:** model swap per hunter id. Portable (needs the other models).
**Peek:** .55 ambush.

---

## 20. Verrine: vent lurker (`verrine_gimmick.gd`)

There are two modes. **The finite floor (main_3d) uses the legacy mode**: main_3d never sets `gimmick.vents` (the VentNetwork) and never sets `brain.vent_nav`. The "physical ducts" mode is infinite-route only.

**Legacy hooks:** `after_perception`, `speed_scale`, `holds_position` (`!active_in_vent`), `hidden_from_view` (`!active_in_vent`, alpha 0.28), `immune_to_capture` (`!active_in_vent`). Plus a custom `use_vent(vent_id, brain, player, nav)` called by the vent interactable.

**World (finite):** `generator.place_vents(VERRINE_VENT_COUNT = 4)`.
- Rooms are picked by stream `"<seed>:VENT-NETWORK"`, skipping rooms with walkable structure.
- One vent per room, on a free cell. The mount is `wall` with probability .58 when wall candidates exist, else `floor`.
- Each entry is `{id: "vent-1".., x, y, room_id, index: 0..3, mount, wall_dir}`. The network is the **ordered ring** of these 4.

**Player state:** `in_vent`, `vent_entry_id`, and `vent_time_in`, which increases by dt while `in_vent`.

**use_vent(vent_id):**
1. If there are no vents, or the id is unknown: return "THE GRATE WON'T BUDGE"
2. `i = vent.index`, `n = 4`.
3. If `!player.in_vent` (entering): `in_vent = true`, `vent_entry_id = vent_id`, `vent_time_in = 0`. **Verrine teleports** to `vents[(i + 2) % n]` (cell + 0.5).
4. `next = vents[(i + 1) % n]`. If `next.id == vent_entry_id`: the player exits (`in_vent = false`, `entry = ""`) and the message is "VENT NETWORK EXITED". The player is not moved: they stay at the grate they pressed.
5. Else the player is **teleported** to `safe_vent_exit(next)`: `cell + 0.5 + open_direction`, where `open_direction = -wall_dir` if unset, then `nearest_open_position(.., 0.27, avoiding vent cells)`. The message is "VENT TRANSIT // VENT-k".

So each press hops one grate along the ring. From the entry, three presses bring you back to the start and exit you.

**after_perception (legacy):**
- `active_in_vent = target.in_vent`.
- If false: `state = IDLE`, `has_last_known = false`, `lost_sight_timer = 0`, return. She is invisible, frozen and harmless.
- If true: `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`.
  `speed_scale = clamp(VERRINE_INITIAL_SPEED_SCALE 0.4 + vent_time_in * VERRINE_SPEED_PER_SECOND 0.025, 0, VERRINE_MAX_SPEED_SCALE 2.4)`. It reaches 2.4 after 80 s in the network.

She walks the ordinary floor path toward the player's real position. The player is "in the vent" only logically: their body stands at a grate on the floor. Capture works normally while `in_vent`.

**Difficulty:** none.
**RNG:** none in legacy mode.
**World needs: vents (missing).** Recommended faithful simplification for the port: exactly the legacy model.
- 4 grate interactables in 4 different rooms (prefer a wall mount, 0.42 m up, against the wall, chance .58).
- An ordered ring, with press-to-hop teleports as above.
- A player `in_vent` flag and a `vent_time_in` timer.
- Verrine invisible and inert until `in_vent`, then a teleport to grate (entry + 2) and a chase with the speed ramp.
No duct geometry is needed. The physical VentCrawl mode (crawl ducts, `LURK_RADIUS 12`, `RETURN_SECONDS 8`, `PATROL_REACH 14`) can be skipped.

**Tells:** she appears (alpha 1.0) only while you are in the network. "VENT TRANSIT" messages (shown: they are interaction results).
**Peek:** never.

---

## 21. Wynelle: cold materializer (`wynelle_gimmick.gd`)

Wynelle is in the ordinary draw, but she wakes herself.

**Hooks:** `before_perception`, `runs_while_inactive` (always **true**). Two custom methods: `has_frozen_player()` and `consume_frozen_player(brain)`.

**State:** `materialized = false`, `progress = 0` (0..100), `death_timer = 0`, `froze_player = false`.

**before_perception** (runs every frame, before the brain's `active` check):
1. **Relaxed:** `active = false`, `state = IDLE`, `progress = 0`, return. She is permanently dormant.
2. If `materialized`:
   - If `target.heater_unfrozen` (heater state UNFROZEN or POWERED) or the player is in the safe zone: `materialized = false`, `progress = WYNELLE_DEMATERIALIZE_RESET_PROGRESS (50)`, `active = false`, `state = IDLE`, return.
   - Otherwise `death_timer -= dt`. When it reaches `<= 0`, set `froze_player = true`. Return.
3. Dormant: `active = false`, `state = IDLE`. **This overrides main_3d's relay activation** (`activate_at`) every frame.
4. If the heater is POWERED, the meters are missing, or the player is in the safe zone: `progress = 0`, return.
5. `T = meters.temperature`. If `T >= WYNELLE_WARM_THRESHOLD (60)`: `progress = 0`, return.
6. `rate = T < WYNELLE_DEEP_COLD_THRESHOLD (30) ? WYNELLE_DEEP_COLD_RATE (1.0) : (60 - T) * WYNELLE_PER_DEGREE_RATE (0.5)`.
   [quirk] Deep cold builds slower (1.0/s) than T = 31 does (14.5/s).
7. `progress = min(100, progress + rate * dt)`. If below 100, return.
8. Materialize: `materialized = true`, `active = true`, **teleport onto the player**, `state = CHASE`, `last_known = player`, `death_timer = WYNELLE_DEATH_GRACE_SECONDS (10)`.
   The same frame's capture check runs at distance 0. So unless the player is hidden or in the safe zone, she **grabs immediately**, with the normal escapable struggle.

**main_3d:** after `update`, if `has_frozen_player()`:
- set the death cause to `"cold"`;
- call `_capture_target(view, target, inescapable = true)`, an instant death;
- call `consume_frozen_player()`, which sets `progress = 50`, `materialized = false` and makes her inactive.

**Rust port: the heater station is retired (2026-11-06).** The temperature item (hotbar slot 7, `game/specialty.rs`) replaced it: heat set on the item is the only heater Wynelle knows (`Ctx.heater_on` and `heater_thawed` are fed from `heat_source_for_hunters` alone). The Godot description below stays as the original's reference; see [archive/heater-stations](../archive/heater-stations/README.md).

**Heater station** (one per floor, 3-7 cells from start): the interact cycle is FROZEN, UNFROZEN, POWERED, UNFROZEN, POWERED and so on.
- POWERED also sets `player.heater_on`, so the ambient temperature rises 3/s (see section 27).
- Messages: "Heater station thawed." / "Heater station powered. The facility is warming." / "Heater station powered down."

**[observed] Finite-floor reality:** the body temperature never goes below 60 on its own. The ambient starts at 60 and only rises; the safe zone pulls toward 60, never below it. The only cooling is Zellia's COOL offer (-15), and Zellia is also a route hunter, so she can never share a floor with Wynelle. **In main_3d, Wynelle therefore never wakes.** The port needs a cold source for her to matter. Suggested: per-room cold zones, or an ambient that drifts down instead of up while she is the drawn hunter. That is design work, not a faithful port.

**World needs:** temperature (missing), the heater station (missing), and the safe zone (present).
Simplification: a heater interactable 3-7 cells from start with the 3-state cycle, plus the temperature model in section 27. Without a cold source she is inert, so it is safe to ship her dormant.

**Tells:** the HUD cold-proximity tint while she is active: `strength = clamp(1 - max(0, d - 2) / (14 - 2), 0, 1)` (main_3d `_temperature_hunter_proximity`, radius 14). Death cause "cold".
**Peek:** never. **Hard abilities:** none.

---

## 22. Xivvy: chase accelerator (`xivvy_gimmick.gd`)

**Hooks:** `before_perception`, `speed_scale`. Presentation fields: `flying`, `fly_ok`, `altitude_hint`, `land()`, `airborne()`.

**State:** `chase_elapsed = 0`, `speed_scale = 1`, `flying = false`.

**before_perception:**
1. `lock_radius = XIVVY_CLOSE_LOCK_RADIUS (1.5) * (XIVVY_CLOSE_SCALE 0.5 + exposure / 200)`. That is **0.75** at exposure 0 and 1.5 at exposure 100. The exposure term is 0 if there are no meters.
2. If the player is not hidden, not in the safe zone, and `distance <= lock_radius`: `state = CHASE`, `last_known = player`. This needs no sight and no LOS.
3. `chase_elapsed = (state == CHASE) ? chase_elapsed + dt : 0`. This uses the state entering the frame, plus the lock above.
4. `flying = chase_elapsed >= XIVVY_TAKEOFF_SECONDS (0.65)`.
5. `speed_scale = min(XIVVY_MAX_SPEED_SCALE 1.8, 1 + chase_elapsed * XIVVY_SPEED_PER_SECOND 0.035)`. It reaches 1.8 after about 22.9 s of continuous chase and resets the moment she leaves CHASE.

**Flight** is presentation only. `fly_ok` is set by the view from route headroom, and she lands for doors and low spaces. `XIVVY_FLIGHT_HEIGHT 0.48`. A grab calls `land()`, which resets `chase_elapsed`. Movement stays on the 2D grid.

**Hard abilities:** pounce (hard).
**World needs:** exposure (optional; without it, use 0.75). Fully portable.
**Tells:** the wing flight clip after 0.65 s of chase, and a screech when swooping (view).
**Peek:** .3 burst.

---

## 23. Ysolde: timid battery thief (`ysolde_gimmick.gd`)

**Hooks:** `after_perception`, `holds_position` (`holding`), `speed_scale` (1.35 only for BEHAVIOUR "run"; the shipping value is **"avoid"**, so 1.0). `immune_to_capture` is **false**: a runner who walks into her is grabbed.

**Constants (file-local):** `BEHAVIOUR = "avoid"`, `AVOID_DISTANCE 9.0`, `RUN_DISTANCE 14.0`, `RUN_SPEED_SCALE 1.35`, `RUN_LOSE_INTEREST 6.0`, `FLEE_REPLAN 0.6`. `keep` is 9.0 (avoid) or 14.0 (run).

**State:** `fleeing`, `holding`, `flee_timer`, `replan`, `flee_point`, `stolen`.

**after_perception:** first `update_flee()`. If it returns true, return.
1. `holding = false`. Count `flee_timer` and `replan` down by dt.
2. `sees = !hidden && distance < keep && LOS(her, player)`. If `sees`: `fleeing = true`, `flee_timer = 1.5` (run: 6.0).
3. If `flee_timer <= 0 && fleeing`: `fleeing = false`, `has_last_known = false`, `state = WANDER`.
4. If not fleeing: if `distance < keep * 1.6 (14.4) && state in {CHASE, INVESTIGATE} && last_known.distance(player) < 2.5`, then `state = INVESTIGATE`, `last_known = her own position`, `holding = true`, and return true. She watches but never closes in. Otherwise return false.
5. Fleeing: when `replan <= 0` or she is within 0.8 of `flee_point`:
   - `replan = 0.6`.
   - `flee_point = flee_target(...)`, or, if that returns null, her own position with `holding = true` (cornered: she freezes).
   - Then `state = INVESTIGATE`, `last_known = flee_point`, and return true.

`flee_target(from, threat, keep)`:
- Candidate cells sit on the square (Chebyshev) rings at distance 3, 5, 7, 9 (step 2, up to `keep`) around her cell. Each must be occupiable and have at least 2 open 4-neighbours.
- Keep only cells whose `distance(threat) > current_gap + 0.75`.
- Try them best score first (the earlier ring index wins ties).
- Accept the first one that has a path, as long as no step of that path comes within `gap - 0.5` of the threat. Otherwise return null.

**Battery errand** (only when `update_flee` returned false):
1. If `pesticide_active`, return.
2. Take the nearest uncollected battery (cell + 0.5): `state = INVESTIGATE`, `last_known = battery`.
3. If `distance <= YSOLDE_STEAL_RANGE (0.8)`:
   - set `entry.collected = true` and `stolen += 1`;
   - emit `{"what":"pickup","cell":c,"kind":BATTERY}`. main_3d `facility_view.apply_event` removes the pickup for everyone; the message is stripped.

Once no batteries are left, the ordinary brain runs, but the flee and hold rules still stop her from closing in.

**Not ported:** the torch retaliation.
**World needs:** batteries with `collected` (7 per floor, `FACILITY_BATTERY_COUNT`). Fully portable.
**Tells:** batteries disappearing; she backs away from you. A scared emote/squeak is keyed on `noticed_at_msec` (view).
**Peek:** never. **Hard abilities:** none.

---

## 24. Zellia: merchant (`zellia_gimmick.gd`, main_3d `_use_shop` / `_perform_zellia_trade`)

**Hooks:** `after_perception`, `holds_position` (always **true**), `immune_to_capture` (always **true**). Plus custom shop methods.

She is a route hunter, so she wakes on the relay schedule (relaxed 3 / standard 1 / hard 0). Before that she is inert at the hunter spawn.

**after_perception:**
1. On the first call, settle:
   - Take the bounding box of all room centres; its midpoint is the facility centre.
   - Pick the room whose centre is nearest that midpoint.
   - **Teleport** to `nearest_open_position(center + 0.5, 0.33)`.
2. Every frame: `state = IDLE`, `idle_timer = 999`, `has_last_known = false`, `lost_sight_timer = 0`.
3. If the shop is open and the player is more than `ZELLIA_SHOP_RANGE 1.35 + 0.25` away, close the shop.

**Shop:**
- `in_range(player, her, facing)`: `distance <= 1.35` and (`distance <= 0.001` or `dot(norm(her - player), facing) >= cos(INTERACTION_FACING_HALF_ANGLE 0.7)`).
- Prompt: "OPEN ZELLIA'S STORE". [E] opens the menu: `"ZELLIA'S STORE // [1] COOL -2 // [2] WARM -2 // [3] REPAIR -3 // [4] PESTICIDE -4 // WALK AWAY TO LEAVE"`.
- Digit keys 1-4 choose an offer while the menu is open; choosing closes the menu.

| # | Offer | Cost (coins) | Effect |
|---|---|---|---|
| 1 | cool | 2 | `temperature = clamp(T - ZELLIA_TEMPERATURE_STEP 15, 0, 100)` |
| 2 | warm | 2 | `T + 15` |
| 3 | repair | 3 | Un-sabotages a relay if one is sabotaged (never on the finite floor: nothing sabotages); otherwise `player.restore_lamp(ZELLIA_LAMP_REPAIR 25)` |
| 4 | pesticide | 4 | `pesticide_timer = max(pesticide_timer, ZELLIA_PESTICIDE_SECONDS 60)`. It pauses Loxelle and Ysolde, which cannot be on the same floor as her, so it has no effect on the finite floor. |

- Not enough coins: "ZELLIA'S STORE // NOT ENOUGH COINS FOR <LABEL> (<cost>)".
- Success: coins are deducted, then "ZELLIA'S STORE // TRADE COMPLETE // <LABEL> // -<cost> COINS".
- **Then the death roll**, on Zellia's brain stream: `next_float() < ZELLIA_DEATH_CHANCE[diff]` (relaxed **.02**, standard **.05**, hard **.075**). On a hit: death cause `"deal"`, `_trigger_capture(zellia, inescapable = true)` (instant death), and the message "ZELLIA'S STORE // THE DEAL COST MORE THAN COINS".

**Coins:** `COIN_PICKUP_COUNT 5` per floor, each worth `COIN_PICKUP_VALUE 2` (`GameState.coins += 2` on pickup). They are placed by stream `"<seed>:COINS"`, one at the nearest open cell to a random room centre, avoiding relays, batteries, safe zones, lockers and vents. Coins persist across floors (run state).

**World needs:** coins and the merchant UI (missing). Simplification: 5 coin pickups at room centres, a 4-option prompt menu, and the death roll. COOL and WARM need temperature: drop them or keep them as no-ops until temperature exists. REPAIR becomes "+25 lamp".
**Tells:** a stationary figure in the central room; the menu text.
**Peek:** never.

---

## 25. Nyxie: darkness stalker (special) (`nyxie_gimmick.gd`)

She is always spawned, inert: `_extra_hunter_spawn` index 0, `angle = 0.8`, distance 2.0 from the route hunter's spawn. Seed `"<seed>:NYXIE"`.

**Hooks:** `before_perception`, `runs_while_inactive` (**true**), `capture_is_inescapable` (`wake_mode == EMPTY`).

**State:** `wake_mode in {NONE, TIMER, EMPTY}` (starts NONE), `darkness_elapsed = 0`, `empty_elapsed = 0`. A new floor builds a new brain, which resets all three.

**before_perception** (every frame, before the active check):
1. `lit = flashlight_on && lamp_charge > 0`.
   - If lit: reset both clocks.
   - Else: `darkness += dt`, and `empty = (lamp_charge <= 0) ? empty + dt : 0`.
   Hidden and safe zone are **not** checked, so darkness inside a locker counts. Ceryth's trap forces the light off.
2. Only while `wake_mode == NONE`:
   - if `empty >= NYXIE_EMPTY_DELAY (10)`, go to EMPTY;
   - else if `darkness >= NYXIE_TIMER_DELAY (120)`, go to TIMER.
3. NONE: `active = false`, `state = IDLE`, return.
4. EMPTY: `active = true` and **position = player position every frame**.
   - The capture check at distance 0 fires the same frame, and it is inescapable: an instant death (death cause "lamp").
   - It is blocked only while the player is hidden or in the safe zone, or while she is stunned (a melee stun skips the capture check). She waits on top of the player.
   - EMPTY never reverts, even if the lamp is recharged.
5. TIMER:
   - If not active yet: `active = true`, `position = spawn_position(player)`. That is the floor cell whose centre distance to the player is closest to `NYXIE_SPAWN_DISTANCE 10` (first match in `floor_cells()` order; no reachability check).
   - Else if dispelled: `wake_mode = NONE`, `active = false`, `IDLE`, `darkness = 0`, return.
     Dispelled means `flashlight_on && lamp > 0 && beam_reaches(player, facing, her, NYXIE_DESPAWN_RANGE 7.0, NYXIE_DISPEL_HALF_ANGLE 0.33)` with LOS. It is **instant**: no 0.35 s dwell is needed. It is not checked on the spawn frame.
   - Turning the light back on does not end TIMER; only the beam dispel does. TIMER cannot escalate to EMPTY.
6. If `state == INSPECT`, return. Otherwise: `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`. That is an omniscient chase; her hearing is 0.

A TIMER capture is a normal escapable grab (death cause "darkness").

**Port addition (owner request, 2026-11): dead flashlight.** The game feeds `Ctx::flashlight_dead` (the runner's lamp charge is empty, so it cannot light; a lamp merely switched off is not dead). A third wake, DEAD, fires after `dead_delay` (3 s, data/hunters/nyxie_wake.json) of that: she spawns `dead_spawn_distance` (10) cells away like TIMER and a live beam dispels her the same way (a battery, then the flashlight on, aimed). The 120 s darkness rule still covers the lamp-off case. A lamp that stays dead until `empty_delay` (10 s) still ends in EMPTY (she moves onto the runner, inescapable), so DEAD can escalate to EMPTY. The wake is the same on every difficulty. **Activation count (owner request, 2026-11-06): the dead lamp ignores it.** The hunter activation count (Relaxed 3 relays, Standard 1, Hard and Nightmare 0) holds back the route's own hunters. Floors and levels below 0 have Nyxie from the start of the run and were never held back. Level 0 of the Infinite route has no Nyxie in its streamed lineup (Godot; the count keeps even its own hunters out), so a dead flashlight there used to bring nobody. With `ignore_activation_gate` on (data/hunters/nyxie_wake.json) the first frame the flashlight is dead on level 0 makes one dormant Nyxie (`Game::keep_level0_nyxie`, marked `Nyxie::dead_only`, last in the list, not a layout spawn, kept for the run): she wakes after `dead_delay` (and escalates to the `empty_delay` lamp kill) with no relay recovered, on every difficulty. She has no 120 s darkness wake on level 0, which level 0 never had.

**Same on relaxed, standard and hard.** Pounce on nightmare only.
**World needs:** lamp charge, flashlight toggle and facing (all present). Fully portable.
**Tells:** `Sound.nyxie_timer_wake()` or `Sound.nyxie_empty_wake()` when she becomes active (main_3d `_update_local_audio`). A jumpscare on an EMPTY catch.
**Peek:** .9 ambush on paper; never in practice, since she is forced to CHASE.

---

## 26. Solyth: heat manifest (special) (`solyth_gimmick.gd`)

She is always spawned, inert: `_extra_hunter_spawn` index 1, `angle = 3.199963`, distance 2.75. Seed `"<seed>:SOLYTH"`.

**Hooks:** `before_perception`, `runs_while_inactive` (**true**). Capturable normally, with the escapable struggle.

**State:** `summoned = false`, `burn_timer = 0`. A new brain per floor.

**before_perception:**
1. **Relaxed:** `active = false`, `IDLE`, return. She never wakes.
2. If there are no meters, return.
3. If not summoned:
   - If `!meters.heat_locked`: `active = false`, `IDLE`, return.
   - Else: `summoned = true`, `active = true`, and `position = spawn_position(player)` (the floor cell closest to `SOLYTH_SPAWN_DISTANCE 12` from the player; same rule as Nyxie).
4. Burn batteries: `burn_timer += dt`. When it reaches `SOLYTH_BATTERY_BURN_INTERVAL (30)`: subtract 30 and set `uncollected(batteries)[0].collected = true`, the first one in generator order.
   [quirk] This only flips the generator flag. No pickup event is emitted, so on the finite floor **the physical battery stays visible and collectible**. The port should remove it, through the same path as a Ysolde pickup.
5. `state = CHASE`, `last_known = player`, `lost_sight_timer = 0`: a permanent omniscient chase.

The latch is permanent. `heat_locked` survives floors (and main_3d never calls `reset_for_floor`), so on every later floor she wakes on frame one.

**Hard abilities:** pounce (hard; needs temperature >= 60, always true once latched), slam (hard; not wired).
**World needs:** temperature (missing).
**Tells:** the HUD heat-proximity tint (same formula as Wynelle, radius 14).
**Peek:** .1 (never in practice).

---

## 27. Temperature model (`player_meters.gd`): what wakes Solyth (and would wake Wynelle)

Per frame, with `heater_on = heater POWERED && !system_offline("climate")`:

**1. Ambient drift**
- If `heater_on`: `ambient = min(100, ambient + 3.0 * dt)` (TEMPERATURE_HEATER_WARM). No timer drift that frame.
- Otherwise:
  - `heat_step = max(0, floor((ambient - 60) / 5))`
  - `interval = max(0.5, 5 - 0.5 * heat_step)`
  - `timer += dt`. When `timer >= interval`: `timer -= interval` and `ambient = clamp(ambient + 0.85, 0, 100)` (TEMPERATURE_AMBIENT_DRIFT).
- **The ambient never falls.** From 60 it reaches 100 in about 153 s: 5.88 ticks per 5 degrees, with intervals 5 / 4.5 / 4 / 3.5 / 3 / 2.5 / 2 / 1.5 s.

**2. Body**
- `diff = ambient - T`
- `proposed = clamp(diff * 1.6, -6.4, 6.4)` (BODY_TEMPERATURE_RESPONSE 1.6; the clamp is 4x that)
- `rate = (|proposed * dt| > |diff|) ? diff / dt : proposed`
- `T = clamp(T + rate * dt, 0, 100)`

**3. Safe zone**
- `T = move_toward(T, 60, 14.0 * dt)` (TEMPERATURE_SAFE_ZONE_PULL).
- The equilibrium inside is `T = ambient - 8.75`, so with the ambient at 100 the zone holds you near 91.

**4. Latch and flags**
- If `T >= 100`, set `heat_locked = true`. If latched, `T = max(T, 90)`.
- `overheating = T >= 90`: lamp and stamina drain x1.35.
- `freezing = T <= 10`: movement x0.72.

**Other inputs**
- Loxelle `temperature_add +30`, Zellia +/-15 and `meter_effect` add directly, then clamp to 0..100.
- `GameConfig.NORMAL_HUNTER_EXTREME_SPEED_SCALE 1.3` is defined but referenced nowhere. [observed]

**[quirk, by analysis, not run]** With a fixed 60 Hz step (`k = 1.6 * dt` about 0.027), the exponential approach to the ambient stalls a few ulps below 100 in float64. The no-overshoot snap needs `1.6 * dt > 1`. So drift alone likely never reaches `T >= 100` exactly; only an additive push clamped to 100 does (Loxelle +30, Zellia WARM). **For the port, implement the intent:** latch when `T >= 100 - 1e-3`, or snap `T = ambient` when `|diff| < 1e-3`. Solyth then wakes about 2.6 min into a floor if you never touch the heater or the safe zone; the heater makes it faster.

[observed] `reset_for_floor()` exists but **nothing calls it** in main_3d. The meters carry across floors. [unknown] Whether a solo restart builds a fresh PlayerMeters; I did not trace `start_new_run` into the controller.

---

## 28. Summary: portability

**Self-contained** (they run on the existing brain, relays, batteries, lockers, safe zone, flashlight and beam):
- dovelle (no gimmick), elise, ferryn, haelith, kathy, mavrine, opalie, porcelle (map marker only), quenelle, ravelle, thessine (audio cues), umbriel (model swap), xivvy, ysolde, nyxie, briselle, ilyra, javelle, ceryth (locker resolver plus lock timer).
- aki also fits here once you add the exposure meter, which is simple math and HUD-only in Godot. Loxelle fits too if you drop her +30 temperature, or add temperature.

**Needs new world features:**
- glyra: a music box with a hold-to-wind and a ticking clock. Godot never ticks it on the finite floor, so there she is stuck calm.
- verrine: 4 vent grates in a ring, an `in_vent` flag and a timer (legacy hop model).
- zellia: coins and a shop menu. Her COOL/WARM offers need temperature.
- solyth: temperature, the heat latch and the battery burn.
- wynelle: temperature, the heater station, and a cold source. As shipped she can never wake on the finite floor.

None of the 26 gimmicks sabotages relays. Briselle's sabotage constants are dead.
