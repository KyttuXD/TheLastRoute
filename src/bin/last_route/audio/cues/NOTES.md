# Sound cues: source map and play rules

A port of the original's procedural sound. Paths below are in the original project (`LastRoute Godot/KyttusLabApp`).
`sound.gd` = `game/autoload/sound.gd`, `pc` = `game/player/player_controller_3d.gd`,
`ri` = `game/world/interactables/route_interactable_3d.gd`, `main` = `game/main_3d.gd` (the finite route: the solo
floor-by-floor run this port is), `hv` = `game/hunters/view/hunter_view_3d.gd`.

## Exactness

`render(cue, pan, intensity)` produces the same 16-bit samples as `Sound.render_cue`. The tests check this against
the original's own goldens (`tests/validate_audio.gd` 193-207): 8 cue hashes, plus the SHA-256 aggregates of all 96
runner surface steps and all 20 heartbeat intensities. They all match on the dev PC (x86-64, Windows, Rust std libm).
What makes it exact:

- the component tables are copied line for line, with f64 maths in the source's order;
- the f32 mix buffers (`PackedFloat32Array`) round on every `+=`;
- 16-bit output `round(clamp(x, -.98, .98) * 32767)`;
- the noise is a 31-bit LCG (`state = (state * 1103515245 + 12345) & 0x7fffffff`, `white = state / 1073741823.5 - 1`)
  seeded with `max(1, djb2(cache_key))`, where `cache_key = "%s|%.2f|%.2f" % [cue, pan, intensity]` after pan and
  intensity are clamped and snapped to 0.05. One noise state is shared by every noise layer of a cue, in order.

So **pan and intensity are baked into the render and also choose the noise**. Cache loaded sounds by `cache_key`. The
original has no other per-play randomness: every play of a key is the same bytes. The only variation is the runner's
step counter (pitch variant `n % 4` of surface steps, and left/right foot). That counter is what `synth`'s `variant` means.

Primitives (`render.rs`, sound.gd 1084-1161), all with envelope `min(1, t / 9 ms) * (1 - progress)^2.25`:

- `tone(f, dur, vol, delay, wave, pan)`: sine / square / triangle (`asin(sin)`) / saw. Any other wave name renders as a
  sine. This includes `"sawtooth"`, which hunter_snore and hide_search use: they are sines in the original too.
- `thump(f, dur, vol, delay, pan)`: a sine gliding from 1.15 f to 0.72 f.
- `noise(dur, vol, f, delay, pan, rate)`: the LCG through a one-pole low-pass, `alpha = clamp(f * rate / 44100 * 5, .015, .55)`.
- Pan is equal-power: `L = cos((pan + 1) * pi / 4)`, `R = sin(...)`. A cue's length is the latest layer end + 25 ms (at least 50 ms).

## API

```rust
cues::SAMPLE_RATE                       // 44100 (sound.gd 10)
cues::synth(cue, variant)               // the usual call-site render (see mod.rs docs), Option<Vec<[f32; 2]>>
cues::render(cue, pan, intensity)       // full render_cue
cues::render_pcm16(..)                  // the same as i16 (the exact bytes)
cues::cache_key(cue, pan, intensity)    // the original's cache key
cues::cue_names()                       // 164 names: 96 plain, 36 nw_*, 32 step:<gait>:<surface>
cues::wav_bytes(&frames)                // for macroquad::audio::load_sound_from_bytes
cues::rules::*                          // what each Sound.<method>() plays, the throttles, and main_3d's pan/volume rules
cues::music_box::*                      // the tune (Night Watch's), the percentage slowdown + run buffer, the hum loop and positional gains
cues::ambience(region, seconds)         // silence: no bed plays on the finite route (see below)
```

`rules::Play { cue, pan, intensity, gain, bus }`: render `cue` at `pan` and `intensity` (`Play::frames()`, cache by
`Play::key()`), then play it at `gain` x the bus gain. `rules::SoundFront` holds the original's throttles and step
counter. Pass it a microsecond clock.

To compile this module, `src/audio/mod.rs` needs a `pub mod cues;` line (not added here). It is std only.

## Buses and volume

- The buses (`default_bus_layout.tres`) are Master, SFX (-10 dB) and UI (-12 dB). At boot Settings **overwrites** the SFX
  and UI levels with `linear_to_db(percent / 100)` (settings.gd 685-695, called at the end of `_load` either way). So at
  the default 100 % every bus is unity, and 0 % is -80 dB. `rules::bus_gain(bus, master, sfx, ui)` takes 0..1 gains;
  `Engine::bus_gain` is the only run-time call. `Bus::Master` (Master alone) is for the positional loops, see below.
- Every cue plays on **SFX**, except `menu` (the menu blip), which plays on **UI**. Runner steps use `Sound.footstep_bus`,
  which is SFX on the finite route (only LevelAmbience, on the infinite route only, moves it to its "Level" reverb bus).
- One-shots are non-positional `AudioStreamPlayer`s from a pool of 16 voices. When all 16 are busy, the oldest is cut
  off (quad-snd `mixer.rs`, `ONE_SHOT_VOICES`; loops are outside the pool). Positional feel comes only from the baked pan
  and the caller's volume.
- `_play_named` plays at 0 dB, with pan and intensity in the render. `_play_scaled` (hunter and teammate steps) renders
  once at intensity 1 with the pan snapped to 0.25, and puts the loudness on the player:
  `gain = clamp(intensity, .05, 4)`. Gains above 1 happen (hunter steps up to 1.8), so do not clip them to 1 in a mixer
  that allows more.
- While a loading screen is up, every world sound (SFX and Master) is muted through a gate bus. On entering the world it
  fades back in over 0.5 s (game/audio/world_audio_gate.gd). UI sounds are never gated.
- Underwater, `Sound.set_muffled(true)` puts a 650 Hz low-pass on Master. This only happens in a pool landing, which is
  not on the finite route.

## Cues the finite route plays

Line numbers are the cue's components in sound.gd, then the trigger sites.

| Cue | Def | Played when | Pan / intensity / volume |
| --- | --- | --- | --- |
| `footstep_walk` / `footstep_sprint` / `footstep_crouch` | 1050-1052 (method 430) | Every physics frame while moving on the floor (pc 2000), throttled to the gait's cadence. **The finite route never sets a floor surface** (`footstep_surface` is set only by infinite_route_test 1752), so these generic steps are what plays. | `rules::SoundFront::footstep(mode, "", ground_speed, now)`. Gap = stride / speed, clamped to 0.12-0.9 s. The strides are walk 1.34 m, sprint 1.28 m, crouch 1.15 m. Without a speed the gaps are fixed at .335 / .205 / .51 s. No step under 0.3 m/s. Feet alternate at pan +-0.045 (odd count = right). SFX. |
| `step:crouch:metal:<n>` | 817-832 + surface_sounds.gd | Each "thump" of the vent crawl (pc 3848, `footstep("crouch", "metal")`, fixed .51 s gap). | `n = count % 4`. Pitch `1 + (n - 1.5) * .035`. Same pan rule. |
| `step:<gait>:<surface>:<n>` | 817-832 | Generic surface steps (infinite route floors). `SURFACES` and `surface_for_floor(kind)` port SurfaceSounds. | Level by gait: walk .05, sprint .095, crouch .017, hunter .045. Length: .07 / .085 / .055 / .08 s. |
| `hunter_footstep` | 1033 (method 273) | main 1721-1735: one step per footfall of the **nearest non-idle active hunter**, cadence = its `footfall_rate` (`rules::HunterStepClock`). **Silent while the player is hidden.** Also on a hard-mode pounce launch (hv 1686), Thessine mimicking Haelith (main 2060), the "troll" fake step (pc 2425, every 2.5-6 s) and Pink's fake steps (void_player_effects.gd 134). | main: only within **9 cells**. `pan = clamp(offset . right / 4.5, -1, 1)`, where `right = (-facing.y, facing.x)`. `volume = .09 * (1 - d / 9)`, giving player `gain = clamp(volume / .05, .1, 2)` (`rules::hunter_step_pan_volume`). Pounce: `pan = side / max(d, 1)`, `volume = clamp(1.4 - d / 30, .3, 1.4) * .05`, within 30 m. Troll: `pan = rand(-1, 1)`, `volume = rand(.05, .12)`. Pink: `max(.09 * (1 - d / 9) * .55, .005)` (`rules::fake_step_volume`). Rendered at intensity 1, pan snapped to 0.25. |
| `heartbeat` | 1039-1049 (method 395) | Fed every frame (main 1720) and throttled. During a stim crash, every 0.8 s at 0.65 (pc 3348). | `danger = clamp(1 - (nearest active hunter cells - .5) / 9, 0, 1)`. At least .48 while chased, 0 in a safe zone (`rules::heartbeat_danger`). Silent under .04. Gap `max(.32, 1.08 - .73 * danger)` s. Intensity = danger. Pan 0. |
| `chase_start` | 960-964 | The frame this player becomes pursued: a hunter in CHASE, not stunned, targeting the local player (main 1712, edge-triggered). | Pan 0. |
| `collect` | 939-940 | Relay picked up (ri 747), battery (754), coin (759), tool pickup (766), locked cache opened with a tool (1048). | Ignores pan. |
| `recharge` | 941-943 | Battery used (pc 3371, `recharge(25)`, intensity 2.5). Safe zone (ri 793, `recharge(20)`, intensity 2). Stim boost lands (pc 3334, `recharge(100)`, clamped to 4). | `intensity = clamp(amount / 10, 0, 4)`. Pitch lift `min(120, 30 * i)` Hz. |
| `taser` | 965-968 | Taser fired (pc 2762). | Pan 0. |
| `axe_swing` | 981-982 | The rescue axe starts a swing, a hit or a whoosh at nothing (pc 3453). | Pan 0. |
| `axe_chop` | 983-986 | The axe lands a hit, after the impact delay (pc 3460). | Pan 0. |
| `door_breach` | 975-978 | Any other swinging tool (crowbar...) lands a hit (pc 3463). | Pan 0. |
| `tool_break` | 999-1001 | A tool's durability runs out (pc 3127). | Pan 0. |
| `interaction_fail` | 979-980 | A refused or failed interaction: the veto (pc 3881), activation failed (3885: inventory full, spot taken, "needs a revive kit"...), wrong tool for the target (3387), no durability (3399), impact failed (3410), stim still active (3317). | Pan 0. |
| `door_open` / `door_close` | 969-974 (method 209) | A room door toggled (ri 735, plays the new state). The vent hatch: entering opens (pc 3818), leaving closes (pc 3806). | Pan 0. **Lockers have no open/close cue**: entering and leaving a hiding spot is silent (ri 795-808, pc 707/764). |
| `light_switch` | 987 | The flashlight runs dead (pc 2414). Pressing F on an empty flashlight (pc 2437). | Pan 0. **A charged flashlight toggles silently.** Wall light switches are silent on the finite route (only infinite_route_test 5049/5650 plays this). |
| `music_box` | 945-947 | Winding Glyra's music box (ri 818). Thessine mimicking Glyra (main 2058). | Ignores pan. |
| `glyra_hum` | 954-958 | Loops on Glyra while she is active and her box is PLAYING, stops otherwise (hv 1546-1570). | Positional: AudioStreamPlayer3D, unit 4 m, max 14 m, +3 dB, 1.5 m up, inverse-distance (`music_box::HUM_EMITTER.gain(d)`). |
| `capture` | 1053 | A hunter grabs the player (main 989). | Pan 0. |
| `struggle` | 1054-1056 (method 451) | Each interact press during the escape struggle (main 1050). | `effort = escape_progress / max(1, escape_required)` is the intensity. Gap `.13 - .045 * effort` s. Pan -0.035 when the step counter is odd, else +0.035 (`SoundFront::struggle`). |
| `escape` | 1057 | The struggle won (main 1064). | Pan 0. |
| `nyxie_timer` | 1002 | Nyxie activates with a timer wake (main 1707, on her inactive-to-active edge). Thessine mimicking Nyxie (main 2056). | Pan 0. |
| `nyxie_empty` | 1003 | Nyxie activates by the empty-flashlight wake (main 1705). | Pan 0. |
| `mimic` / `mimic_rare` | 1037-1038 | Thessine: every 10 s a 50 % roll, and 8 % of hits are rare (thessine_gimmick.gd). Rare, or no other cueable hunter active, plays these. Otherwise it plays another hunter's cue (main 2041-2062, `rules::thessine_mimic`): nyxie gives `nyxie_timer`, glyra `music_box`, haelith `hunter_footstep(0, .09)`, ferryn / briselle / loxelle / ravelle `alert`. | Pan 0. |
| `alert` | 959 | Only via Thessine's mimic above. Every other alert call (main 338/385/1711/2036, void_player_effects 61) is gated by `HunterBrain.ANNOUNCE_HUNTER_EVENTS = false`. | Pan 0. |
| `nw_breath` | 875 (`_nw_components`) | A hunter lingering at a locker during an inspection, within 3.5 m of the camera (hv 1520-1539). It breathes once, or Nyxie twice 1.1 s apart. | `pan = clamp(side, -1, 1)`, where side is metres along the camera's right. Intensity `(1 - gap / 3.5 * .5) * (1.15 if two breaths)`. |
| `menu` | 944 (method 185) | Every button press in every menu (all BaseButtons are wired: sound.gd 804-812, except `silent_ui` touch buttons). Also sitting on a ride (pc 525), the start of a stim injection (pc 3321), "play" furniture (ri 780), and death-moment and chat captions (death_moment.gd 115, room_chat.gd 435). | **UI bus**. At most one per 45 ms. There is no separate hover, confirm or back sound: every press is this blip. |
| `sleep_settle` / `sleep_yawn` / `sleep_stir` / `sleep_jolt` | 988-993 | Settle: falling asleep (pc 620). Yawn: drowsy after 50 s lying idle (pc 664). Stir: a hunter within 12 m while asleep, at most every 3 s (pc 647), intensity `clamp(1 - d / 12, .25, 1)`. Jolt: woken by a hunter within 5 m (pc 633). | Pan 0. |
| `hunter_laugh` | 1004 | A hunter's noisy emote, within its radius (hv 1470). | `pan = clamp(side / max(radius / 2, 1))`. `intensity = clamp(.05 * (1 - d / radius) / .05, .1, 2)`. |
| `xivvy_screech` / `xivvy_wing` | 1005, 1009 | Xivvy taking off or closing in, and wing beats (hv 1306/1308), within 40 m. | `pan = side / max(d, 1)`. `loudness = clamp(1.5 - d / 30, .25, 1.4)`, x .8 for wings. |
| `system_offline` / `radar_static` / `signal_node` | 1007, 1006, 1008 | Void hunters' events (void_player_effects.gd 70, 77, 148): a system outage, the radar corrupted, each press of Gray's signal route. | signal_node: `intensity = clamp(.7 + progress, .5, 1.8)`, where `progress = 1 - nodes_left / nodes`. This scales the pitch. |
| `office_jumpscare` | 1073-1079 | The capture jumpscare (main 1122). **Disabled**: `HunterJumpscare.ENABLED = false`. | `intensity = 1 + (abs(djb2(hunter_id)) % 90) / 900`, then snapped to 0.05 (so 1.00 or 1.05). |

**Lift / descend: the original has no cue.** main_3d plays nothing when you take the service lift down (no Sound call in
the exit path), so the port's current `lift` sound has no source equivalent.

## The music box tune and its wind-down (`music_box.rs`)

- `music_box_tune`: **no longer the original's cue** (the golden in `tests.rs` was updated on purpose). It is Night
  Watch's Masked Hours tune (KyttusLab `games/night-watch/src/modes/mask/mask-data.js` `TUNE`, played by `mask.js`): 13
  sine notes, 659 587 523 587 659 659 659 587 587 587 659 784 784 Hz (E D C D E E E D D D E G G), 380 ms apart at full
  wind (16758 frames, so the spacing is exact), each ringing 0.5 s, the phrase restarting straight after the last note
  (no breath). `music_box::tune()` cuts the render to exactly 13 x 16758 frames and adds the last note's ring onto the
  start, so the loop has no click and no gap at the seam and every round is the same samples.
- The speed follows MusicBoxes' clock, as a share of the wound time (45 / 35 / 25 s standard / hard / nightmare):
  `music_box::slowdown_factor(left / wound, from, floor)` is 1 down to `from` (`slowdown_from_fraction`, 0.70), then a
  smoothstep down to `floor` (`slowdown_floor`, 0.40), and 0 at an empty box. Full wind and 70 % are normal speed, 35 %
  is about 0.7 (clearly slower and lower), 0 is stopped. The tune is read back at that rate (resampling, like Godot's
  `pitch_scale`), so tempo and pitch sink together, like Night Watch's tune slowing as its box runs down. (The old
  Godot formula, the last 15 s only, is gone; `winddown_seconds` stays for Glyra's mood and the HUD colour.)
- macroquad cannot repitch a playing sound, so `music_box::run_frames(wound, elapsed)` renders the whole wound time as
  ONE buffer, a pure function of the wound time and the data (the same on desktop and in the browser), and
  `world::music_box(left, wound)` plays it once when the box is wound, through `Engine::play_frames` (a looped voice on
  Master that stops itself when the stretch ends, so it follows Master live and `stop_all` stops it). There is no
  hand-over between a steady loop and a slowing one, which on the browser (where loops decode asynchronously) would
  have been a gap. It starts again when the box is wound again, or the game stood still for 0.4 s (paused).
  In the browser the timed end is counted from the moment the decoded channels start playing.
- In MusicBoxTune the sound is positional: unit 5 m, max 12 m, +6 dB, 0.4 m above the box; this port plays it
  non-positionally from the pack.
- **In the original, on the finite route the tune never plays** (this port plays it anyway, see above). MusicBoxTune is only attached when the interactable gets a `music_boxes`
  clock (ri 247), and only the streamed route view sets one (streamed_facility_view_3d.gd 4734). In main_3d the box
  gives the one-shot `music_box` phrase, and the music you hear is Glyra's `glyra_hum` while her box PLAYS.

## Ambience (`ambience.rs`)

The original has a full ambience generator: `game/audio/ambience_synth.gd`, with beds in `data/ambience/*.json` (room
tone, resonant hums, ticks, chirps, wind, thuds, creaks, groans, water, crackle, plus reverb, 10 s seamless loops seeded
by Godot's PCG32). There is also a level layer (`level_ambience.gd`, `emitter_loops.gd`, `falls_loop.gd`). **Neither runs
on the finite route.** `Sound.set_ambience` is only called by `infinite_route_test.gd` (1754, 7937), and LevelAmbience is
only created there (193). `PlatformProfile` only pre-renders beds and never plays one. So the README is right: there is no
continuous drone in main_3d. `ambience()` returns silence of the requested length. The only continuous sounds on the
route are `glyra_hum` (and `heartbeat`, which repeats).

## Not on the finite route (ported for completeness)

- **Region and infinite-route cues:**
  - carnival_* (CarnivalBooth3D, carousel_ride_3d)
  - spider_bark / spider_hiss / spider_skitter / inferno_crackle (creature_view)
  - pit_rumble (inferno_pit_rumble)
  - splash_big / water_drip (pool landing and wading)
  - ladder_rung and hunter_snore (infinite_route_test 3483, 3660)
  - hide_search (`play_search_tell`, only called from infinite_route_test 5656)
  - music_* (MusicProps)
- **teammate_footstep_\*** (co-op remote players): scaled with `gain = clamp(volume / .04, .08, 1.5)`.
- **office_\*** (Night Watch office queue: at most 180 ms ahead of real time, one after another) and **nw_\*** (the
  Night Watch minigame, `Sound.nw`), except `nw_breath` above.

## Measured against Godot (2026-11-01, Godot 4.7.2)

A copy of the original project was run with `--write-movie --fixed-fps 60` and a scene (`_measure/measure.tscn`, scratch
only) that calls the real `Sound` autoload and places a real AudioStreamPlayer3D. The mixed output was compared with
this module's renders (`%TEMP%\lr_audio_analyze`):

- **One-shots at a 44.1 kHz mix: bit-exact through the bus chain.** collect, menu_blip (UI bus), heartbeat(.5),
  hunter_footstep(.6, .07), footstep walk, recharge(25), taser, chase_start and struggle(.5) all match render x gain to
  within 1 LSB (SNR 58-90 dB). Fitted gains: 1.000 for `_play_named`, 1.400 for hunter_footstep(.6, .07) (`_play_scaled`,
  volume / .05). Both buses read 0 dB at default settings (Settings overwrites the layout's -10 / -12 dB), and
  there are no bus effects on Master / SFX / UI.
- **The output device rate.** In a real run Godot's driver is WASAPI at the device's mix rate (48000 Hz here) with a
  10 ms output latency. Godot resamples the 44.1 kHz cues with AudioStreamPlaybackResampled: Catmull-Rom cubic, read 2 source
  frames late (matches Godot's 48 kHz capture at 48-65 dB SNR; linear is 19-51 dB, a clean windowed-sinc 17-51 dB, so
  the resampler is audible on the square-wave cues: taser 17.5 dB).
- **Upstream quad-snd 0.2.8** opens WASAPI at a fixed 44100 Hz and lets Windows' SRC convert to the device rate (a
  different resampler), with a 4096-frame buffer (about 93 ms latency against Godot's 10 ms). `src/audio/quad-snd` is
  that crate patched: device rate, 10 ms buffer, Godot's cubic resampler in the mixer (matches Godot's 48 kHz output at
  48-67 dB SNR, the cue's own 16-bit noise floor). It needs `[patch.crates-io] quad-snd = { path = "src/audio/quad-snd" }`.
- **3D (Glyra's hum).** `positional.rs`: inverse distance + volume_db capped at max_db, times a linear fade to
  max_distance; stereo pan `sqrt((1 -+ 0.6 x) / 2)`. It matches Godot to 4 digits at 10 positions. The hum plays on
  **Master** (the 3D player's default bus), not SFX.

## Engine and API (src/audio)

- `sound::*`: one function per `Sound.<method>()`. `world::update(dt, &Listener, &[HunterAudio])` is main_3d
  `_update_local_audio` (danger heartbeat, chase_start edge, Nyxie's wake edge, hunter steps on the stepping hunter's real
  footfall rate from `stride.rs`, pan and volume as in main 1728-1734) plus Glyra's hum loop. `world::reset()` is called when a floor starts.
- Renders are made on first use and cached by `cache_key`. The common keys are prewarmed in `Sounds::new()` (20 heartbeats,
  the stepped feet, the 9 hunter-step pans...). Loading is synchronous: `load_sound_from_bytes` is ready on its first poll on native targets.
- Positional loops are two sounds (left-only, right-only) started together, with per-channel volume, because
  macroquad has no per-voice pan. A loop keeps its base gains and the engine applies its bus gain (`LoopGains`), so a
  Master change reaches a playing loop from `set_volumes`, not from the next game update.
- Not reproduced: Godot's per-buffer volume ramps, and
  the 3D shelf filter (under 0.1 dB on the hum).
## What could not be exact

- The 16-bit samples match the original's goldens bit for bit on this PC. Godot's own test file says the float
  synthesis may differ on another CPU architecture, and so may Rust's libm `sin` / `pow` / `asin` on another target. A
  difference there would be 1 LSB at most.
- The wind-down pitch is recomputed per output frame here. The original updates it once per `_process` frame (60 Hz
  steps), which is inaudibly different.
- Content-pack sound overrides (ContentRegistry "sounds" / "ambience") are not ported.
