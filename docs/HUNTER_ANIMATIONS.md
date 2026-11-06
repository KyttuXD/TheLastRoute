# Hunter animations: audit and per-hunter profile

Question (KT): are the hunters' animations per hunter, or one generic humanoid animation? Short answer: **the clips are per
hunter, and so are the chase layer and the grab pool; what was shared is the view logic around them, and many hunters
share a plain chase posture.** This page records the audit (against 40f85a2, where this branch started) and the
data-driven profile added on top (`data/render/hunter_anims.json`).

## How a hunter is animated

| Layer | Where | Per hunter? |
| --- | --- | --- |
| Rig and clips (idle, walk, chase, sprint, crouch, charge, flight, jumpscare, climb, stun, stun_recover; Kathy also `scythe_*`) | `assets/hunters/anim/<id>.json`, extracted from the Godot GLBs by `render/anim/extract.py`; sampled by `render/anim/clip.rs` | **Yes.** Every one of the 38 humanoid rigs has its own walk, chase, idle and stun tracks (checked by hashing the tracks: no two hunters share a clip). The shapes come from the original's `tools/blocky_hunters/hunters/motion.py` personality profiles (tempo, stride, arm swing, bounce, lean, head pitch / roll, shoulder-in, idle amplitude / tempo) and its stun style table (base, stiff, topple, cover, tumble, shake, float, curl, flop). The bone count differs too (21 to 40: tails, ears, hair, wings, halos). |
| Stride match (clip speed = ground speed / the clip's measured reference speed) | `stride` table inside each anim json; `HunterView::choose` | **Yes**, measured per model (walk reference 1.14 to 2.76 m/s). The legs do not skate whatever the hunter's speed. |
| Clip choice (idle / walk / chase / sprint / stun / wake / flight / charge, the gait hysteresis, the 0.15 s travel blend) | `render/anim/hunter.rs` `update_animation` (Godot `_update_animation`) | Shared logic, as in Godot. The *clips it picks* are per hunter. |
| Upper-body chase layer | `chase_style` in `data/hunters/<id>.json`; `ChaseStyle::apply` (Godot `hunter_chase_style.gd`) | **Yes**, one of eight styles per hunter (findings below). Hunters in the same style share the arm pose, not the legs. |
| Head look, body yaw, faces, blink, stars, heavy-stun reel | `hunter.rs` | Shared, as in Godot (the blink and head-scan phase are seeded per hunter). |
| Grab (the hold on a caught runner) | `assets/hunters/grabs/<id>.json` pool, `render/anim/grab.rs` | **Yes**: a weighted pool of styles per hunter (wrap, tackle, neck_lift, wrist_yank, leg_grab, upside_down ...). The wolf spiders pounce. |
| Creature rigs (the two wolf spiders) | creature path in `hunter.rs`, `secret_creatures.json` | Own clips (walk, idle, bark, sprint, attack, death) and their own state machine. |
| **Runtime profile (new)** | `data/render/hunter_anims.json`, `render/anim/profile.rs` | **Yes, data-driven**: gait speed, stride, arm swing, lean, crouch, idle sway, head tilt, and named mechanic states. |

## Per-hunter table

Columns: bones in the rig; clips in the rig; walk / chase clip length (s); measured swing of the walk clip (left thigh peak-to-peak
degrees / left arm peak-to-peak degrees / pelvis bob in 1/100 of the model height); mean chest pitch in the chase clip
(degrees, negative = forward); chase style; stun style (Godot `STYLE_OF`); grab pool; the Godot motion profile that produced
the clips. Every hunter's walk, chase, idle and stun clips are unique to it. "Gaps" are in the findings below (the mechanic animations were a gap for every hunter that has a gimmick; they are added below).

| hunter | bones | clips | walk / chase len | thigh / arm / bob | chase lean | chase style | stun | grab pool | motion.py profile |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| aki | 24 | 11 | 1.11 / 0.56 | 54 / 19 / 2.8 | -7.8 | reach1 | cover | wrist_yank/collar_drag | tempo=0.9, stride=0.8, arm_swing=0.6, lean=7, head_pitch=9, shoulder_in=5, idle_amp=0.6 |
| briselle | 24 | 11 | 1.11 / 0.56 | 61 / 25 / 3.9 | -4.3 | reach1 | base | wrap/shoulder_carry | tempo=0.9, stride=0.9, arm_swing=0.8 |
| ceryth | 24 | 11 | 1.00 / 0.50 | 64 / 16 / 4.5 | -4.3 | glide | stiff | leg_grab/collar_drag/wrist_yank | tempo=1.0, stride=0.95, arm_swing=0.5, bounce=0.7, idle_amp=0.5 |
| dovelle | 21 | 11 | 1.00 / 0.50 | 67 / 32 / 5.2 | -4.3 | pump | base | shoulder_grab/wall_pin/tackle | (default) |
| elise | 27 | 11 | 0.83 / 0.42 | 74 / 32 / 6.5 | -6.8 | pump:fast | tumble | neck_lift/tackle | tempo=1.2, stride=1.1, lean=5, bounce=1.2 |
| ferryn | 24 | 11 | 0.91 / 0.45 | 54 / 22 / 2.8 | -6.8 | stopstart | shake | wrap/wrist_yank | tempo=1.1, stride=0.8, arm_swing=0.7, lean=5, head_pitch=6, shoulder_in=6, idle_amp=1.6, idle_tempo=1.4 |
| glyra | 26 | 11 | 1.18 / 0.59 | 61 / 32 / 3.9 | -4.3 | stalk | float | wrap/shoulder_carry/upside_down | tempo=0.85, stride=0.9, head_roll=7, idle_amp=1.4, idle_tempo=0.8 |
| haelith | 30 | 11 | 0.91 / 0.45 | 74 / 32 / 6.5 | -4.3 | pump:big | curl | tackle/shoulder_grab/wall_pin | tempo=1.1, stride=1.1, bounce=1.3, idle_amp=1.3, idle_tempo=1.2 |
| ilyra | 27 | 11 | 1.11 / 0.56 | 61 / 19 / 3.9 | -4.3 | lope | curl | leg_grab/upside_down/tackle | tempo=0.9, stride=0.9, arm_swing=0.6, bounce=0.7, head_roll=-4, idle_amp=0.5 |
| javelle | 27 | 11 | 0.91 / 0.45 | 74 / 38 / 6.5 | -6.8 | reach2 | topple | wrist_yank/collar_drag/shoulder_grab | tempo=1.1, stride=1.1, lean=5, arm_swing=1.2 |
| kathy | 24 | 16 | 1.00 / 0.50 | 67 / 32 / 5.2 | -4.3 | reach1 | topple | neck_lift/shoulder_carry | tempo=1.0, stride=1.0, bounce=1.15, head_roll=4, idle_amp=1.0 |
| loxelle | 30 | 11 | 1.05 / 0.53 | 61 / 32 / 3.9 | -4.3 | lope | float | shoulder_grab/wall_pin/tackle | tempo=0.95, stride=0.9, bounce=1.1, idle_amp=1.2, head_roll=2 |
| mavrine | 25 | 11 | 1.00 / 0.50 | 64 / 10 / 4.5 | -4.3 | glide | stiff | wrist_yank/wall_pin | tempo=1.0, stride=0.95, arm_swing=0.3, bounce=0.5, idle_amp=0.3 |
| nyxie | 25 | 11 | 1.00 / 0.50 | 77 / 32 / 7.1 | -4.3 | stalk | tumble | wrap/leg_grab/upside_down | tempo=1.0, stride=1.15, bounce=0.9, head_roll=5, idle_amp=1.3 |
| opalie | 30 | 11 | 0.91 / 0.45 | 67 / 41 / 5.2 | -4.3 | reach2 | tumble | shoulder_grab/wall_pin | tempo=1.1, stride=1.0, bounce=1.5, arm_swing=1.3, idle_amp=1.3, idle_tempo=1.2 |
| porcelle | 28 | 11 | 1.18 / 0.59 | 54 / 22 / 2.8 | -4.3 | glide | float | leg_grab/collar_drag/wrap | tempo=0.85, stride=0.8, arm_swing=0.7, shoulder_in=4, head_pitch=4, idle_amp=0.7 |
| quenelle | 27 | 11 | 1.11 / 0.56 | 61 / 16 / 3.9 | -4.3 | reach2 | cover | tackle/wrist_yank | tempo=0.9, stride=0.9, arm_swing=0.5, bounce=0.7, idle_amp=0.5, head_roll=-3 |
| ravelle | 24 | 11 | 0.91 / 0.45 | 71 / 32 / 5.8 | -4.3 | stopstart | tumble | neck_lift/leg_grab/upside_down | tempo=1.1, stride=1.05, bounce=1.4, head_roll=-6, idle_amp=1.4 |
| solyth | 23 | 11 | 0.91 / 0.45 | 81 / 41 / 7.9 | -6.3 | pump:fists | topple | wrist_yank/shoulder_grab/collar_drag | tempo=1.1, stride=1.2, arm_swing=1.3, lean=4 |
| thessine | 24 | 11 | 1.00 / 0.50 | 61 / 16 / 3.9 | -4.3 | stalk | stiff | shoulder_grab/wrap | tempo=1.0, stride=0.9, arm_swing=0.5, head_roll=6, idle_amp=0.7 |
| umbriel | 26 | 11 | 1.18 / 0.59 | 57 / 22 / 3.4 | -6.8 | lope | curl | neck_lift/wall_pin/upside_down | tempo=0.85, stride=0.85, arm_swing=0.7, lean=5, head_pitch=7, shoulder_in=4, idle_amp=0.7 |
| verrine | 27 | 11 | 1.11 / 0.56 | 61 / 13 / 3.9 | -7.3 | skitter | curl | tackle/leg_grab | tempo=0.9, stride=0.9, arm_swing=0.4, lean=6, head_pitch=5, shoulder_in=7, idle_amp=0.6 |
| wynelle | 30 | 11 | 1.11 / 0.56 | 64 / 25 / 4.5 | -4.3 | glide | float | neck_lift/wall_pin/upside_down | tempo=0.9, stride=0.95, arm_swing=0.8, bounce=0.8, idle_amp=0.8, idle_tempo=0.8 |
| xivvy | 25 | 11 | 0.80 / 0.40 | 81 / 44 / 7.9 | -2.8 | pump:wings | flop | neck_lift/upside_down/tackle | tempo=1.25, stride=1.2, arm_swing=1.4, bounce=1.7, lean=-3, head_roll=6, idle_amp=1.8, idle_tempo=1.5 |
| ysolde | 24 | 11 | 1.18 / 0.59 | 57 / 32 / 3.4 | -4.3 | reach1 | cover | shoulder_grab/wrist_yank | tempo=0.85, stride=0.85, head_roll=6, head_pitch=3, idle_amp=0.9 |
| zellia | 24 | 11 | 0.95 / 0.48 | 67 / 35 / 5.2 | -4.3 | pump | tumble | wrist_yank/shoulder_grab | tempo=1.05, stride=1.0, bounce=1.35, arm_swing=1.1, idle_amp=1.2 |

The twelve Void colours share one baseline (`_VOID`: tempo 1.15, stride 1.1, arm_swing 0.8, lean 5, head_pitch 3, idle_amp 0.9) with a nudge per colour, so they are
the most alike of all: their clips differ in tempo, swing and lean but have one silhouette.

| hunter | bones | clips | walk / chase len | thigh / arm / bob | chase lean | chase style | stun | grab pool | motion.py profile |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| void_black | 40 | 11 | 0.87 / 0.43 | 74 / 25 / 7.3 | -6.8 | lope | float | wrap/upside_down/shoulder_carry | `_VOID` base + nudge (see below) |
| void_blue | 40 | 11 | 1.00 / 0.50 | 64 / 16 / 5.1 | -6.8 | glide | stiff | shoulder_grab/wall_pin/shoulder_carry | `_VOID` base + nudge (see below) |
| void_brown | 40 | 11 | 0.95 / 0.48 | 81 / 38 / 8.8 | -8.3 | lope | topple | tackle/shoulder_grab | `_VOID` base + nudge (see below) |
| void_cyan | 40 | 11 | 0.87 / 0.43 | 74 / 25 / 7.3 | -6.8 | glide | shake | wrist_yank/tackle | `_VOID` base + nudge (see below) |
| void_gray | 40 | 11 | 1.05 / 0.53 | 74 / 6 / 7.0 | -5.3 | glide | stiff | shoulder_grab/wall_pin/collar_drag | `_VOID` base + nudge (see below) |
| void_green | 40 | 11 | 0.87 / 0.43 | 74 / 29 / 7.3 | -6.8 | lope | topple | leg_grab/collar_drag/upside_down | `_VOID` base + nudge (see below) |
| void_orange | 40 | 11 | 0.77 / 0.38 | 84 / 41 / 9.6 | -9.3 | reach2 | tumble | tackle/wrist_yank | `_VOID` base + nudge (see below) |
| void_pink | 40 | 11 | 1.05 / 0.53 | 74 / 25 / 7.3 | -6.8 | reach2 | float | wrist_yank/wrap | `_VOID` base + nudge (see below) |
| void_purple | 40 | 11 | 0.83 / 0.42 | 74 / 25 / 7.3 | -6.8 | lope | float | wrap/shoulder_carry | `_VOID` base + nudge (see below) |
| void_red | 40 | 11 | 0.77 / 0.38 | 84 / 44 / 9.6 | -8.8 | reach2 | shake | tackle/wall_pin/shoulder_grab | `_VOID` base + nudge (see below) |
| void_white | 40 | 11 | 1.05 / 0.53 | 74 / 25 / 7.3 | -6.8 | glide | cover | wrist_yank/upside_down | `_VOID` base + nudge (see below) |
| void_yellow | 40 | 11 | 0.83 / 0.42 | 77 / 35 / 8.0 | -6.8 | reach2 | shake | shoulder_grab/shoulder_carry | `_VOID` base + nudge (see below) |

Wolf spiders (`white_wolf_spider`, `black_wolf_spider`): creature rig, 37 to 38 bones, clips walk / idle / bark / sprint / attack / death, `pounce_pin` grab. Complete and unique per creature.

## Findings

1. **Not generic.** All 26 A-Z hunters, the 12 Voids and the 2 spiders have their own clips. The premise "one shared humanoid animation" does not hold at the clip level.
2. **Why it can still look alike.**
   - The stride match normalises leg speed to the ground, so a slow and a fast hunter at the same speed step the same way.
   - 18 of the 26 A-Z hunters have the base chase lean (-4.3 degrees): motion.py only sets `lean` for aki, elise, ferryn, javelle, solyth, umbriel, verrine and xivvy. The chase posture of the rest differs only by their chase style.
   - Hunters in the same chase style share the arm pose: reach1 (aki, briselle, kathy, ysolde), reach2 (javelle, opalie, quenelle), glide (ceryth, mavrine, porcelle, wynelle), lope (ilyra, loxelle, umbriel), stalk (glyra, nyxie, thessine), pump (dovelle, zellia; with fast / big / fists / wings variants elise, haelith, solyth, xivvy), stopstart (ferryn, ravelle), skitter (verrine).
   - Near twins by measured swing: dovelle and kathy (67 / 32 / 5.2 both), briselle / ilyra / quenelle / loxelle (61 thigh), haelith and elise (74 / 32 / 6.5 both).
3. **Gaps against Godot** (what the original shows that the port does not):
   - *Mechanic animations.* **Added** (the section below): one pose per gimmick, raised by the real event, replicated to guests.
   - *Climbing.* The port's ladder edges are walked "with no climb animation" (`hunters/brain.rs` `walk_edge`): the body rises while the horizontal speed is zero, so the gait fell back to the **idle** clip. Godot plays `climb`. **Fixed here** (state `climb`).
   - *Kathy's scythe.* The `scythe_idle / scythe_walk / scythe_swing / scythe_stun / scythe_stun_recover` clips are in her rig and her stride table, and Godot plays them (and mounts `kathy_scythe.glb` on `R-HandProp`) for the rare scythe carrier. The port has neither the prop nor the clip switch. Not done: needs the prop model and the carries-scythe roll (`HunterRoster.carries_scythe`).
   - *Hunter emotes.* `hunters/emotes.rs` picks them, `extract.py --emotes` can pull the `emote_*` clips, but the shipped anim json holds none and nothing plays them.
   - *Jumpscare.* The `jumpscare` clip exists in every rig and is unused (the port shows the grab instead). `crouch` is unused in Godot too.
   - *Night Watch poses* (corner peek left / right) are a lean in the port (`peek_lean`), not Godot's pose clips.
4. **Not a gap.** Godot's `HunterAddons` (data-driven tails / ears / hats with sine "motions") ships empty (`hunters: {}`, `shared: []`), so there is nothing to port.

## The profile (this change)

`data/render/hunter_anims.json`, schema `lr.hunter_anims/1`, read by `render/anim/profile.rs`, applied in `HunterView::update` (`render/anim/hunter.rs`).
The `default` row is the **identity**, so a hunter with no row looks exactly as before (a test compares the bones frame by frame against an explicit identity profile).
A row names only the keys it changes.

| key | effect | unit |
| --- | --- | --- |
| `walk_speed`, `chase_speed` | multiplies the playback speed of the walk, or of the chase and sprint clips, on top of the stride match (1 keeps the legs on the ground; above 1 the steps are quicker than the ground speed) | factor 0.25 to 4 |
| `stride` | how far the legs (thigh, calf, foot) swing from rest while a gait clip plays | factor 0 to 3 |
| `arm_swing` | the same for arms and forearms | factor 0 to 3 |
| `lean_deg`, `chase_lean_deg` | extra forward lean of belly and chest (the head keeps half back so the gaze stays level), walking / chasing | degrees, forward positive, -30 to 45 |
| `crouch`, `chase_crouch` | knees bent and hips lowered by exactly the leg's shortening, so the ankles stay on the floor; the chase one adds while chasing | 0 to 1 (1 = 40 degrees at the thigh) |
| `idle_sway` | how much the idle clip moves chest, belly, head and arms | factor 0 to 4 |
| `head_roll_deg` | constant head tilt while she is up and about | degrees |
| `states.<name>` | a mechanic animation (below) | |

The layers follow the clip playing (weights ease at 6 per second), run **under** the chase style (so a chase style still sets its arms) and
never apply to stunned, dormant, waking or held hunters or to creature rigs. The rows shipped are a first pass to tune by eye:

| hunter | row | why |
| --- | --- | --- |
| elise | chase_speed 1.06 | competitive, always racing |
| solyth | chase_speed 1.04, chase_lean_deg 4 | hot-tempered |
| haelith | chase_speed 1.04, chase_lean_deg 5 | eager wolf |
| quenelle | chase_lean_deg 4 | cold, then clingy wolf |
| nyxie | chase_crouch 0.1, chase_lean_deg 4 | bold stalker, low and forward |
| thessine | chase_crouch 0.05, chase_lean_deg 4 | stalk style, nitpicky |
| ilyra | chase_crouch 0.12, chase_lean_deg 5 | catlike lope |
| loxelle | chase_lean_deg 3 | calm lope with a little push |
| verrine | chase_speed 1.05, chase_crouch 0.16 | low skitter |
| void_orange, void_red | chase_speed 1.05 | the two most aggressive Voids |

KT asked for hunters that chase "slightly faster, animation wise": the global knob is `default.chase_speed` (1.0 now, to keep today's look); 1.05 to 1.1 should read as quicker
without the feet visibly sliding (not checked by eye). It scales the legs only; how fast a hunter actually moves is `chase` in `data/hunters/<id>.json`.

## Mechanic animations (one per gimmick)

Every hunter whose gimmick (`hunters/gimmicks/*.rs`) does something you can see now plays a pose for it, raised by the real gameplay event, working for host-simulated hunters and replicated to online guests. The pieces:

* **When (the host).** The gimmick decides, in its own hook, because only it knows. Two kinds of name (`hunters/mechanics.rs`):
  * a **held** name lasts as long as a gimmick state does: `Gimmick::mechanic(brain)` (`gimmicks/mod.rs`, one arm per gimmick calling the gimmick's `mechanic()`); examples: Ysolde within reach of a battery, Glyra while the box plays, Loxelle in the windup;
  * a **cue** is raised once, at an event, with `brain.raise_cue("name")` and the brain keeps it for `seconds` (`data/hunters/mechanic_cues.json`), then it clears by itself. A cue is never longer than the gameplay moment, never moves or freezes the hunter (the pose is laid over the walk), and a stun, `Brain::apply_stun`, drops it at once; the clock keeps running on a dormant or stunned hunter, so nothing is left behind when she gets up, when the floor changes or when the run ends (the brain is dropped with the run).
  * `Brain::mechanic()` picks one: the ladder climb, else the cue, else the held name. A guest's copy (`display_only`, or a `shadow` made from a host frame) runs no gimmick, so its idle gimmick must not speak (Glyra's sits at "box playing"): there only the name the host sent counts. The void hunters' event-driven cues are raised in one place from the events their hook just pushed (`Gimmick::raise_event_cues`); every other gimmick raises its own where it pushes the event.
* **What it looks like (data).** `states.<name>` in `data/render/hunter_anims.json`, under the hunter that owns the gimmick (Umbriel's `shapeshift` is global, because she plays it on whatever body she wears). Fields in the next section.
* **Online.** The host's hunter frame entry (`game/resident_hunters.rs frame_entry`, the same entry active, resident and dedicated-room hunters use) carries an optional `mech` string, absent when none. The guest (`apply_hunter_frame`) stores the name with a `net_ttl` of 0.6 s that its update runs down, so a host whose frames stop (paused, dropped link) cannot leave a guest hunter frozen in a pose; a frame without `mech` clears it at once; a name this build does not know is ignored, an older guest ignores the key. Frames come at 20 Hz (`data/net/hunter_events.json`), so no other wire field was needed (`net/world_sync.rs` just forwards the entry, `net/held_wire.rs` is only reused for how an item sits in a hand). The ladder climb now reaches guests the same way.
* **Props.** A state may put an item in the right hand (`prop`: `battery`, `music_box`, the `net/held_wire.rs` ids). It is drawn on the `R-Hand` bone with the same pose table as a runner's held item (`data/render/remote_held.json`), so it follows every clip and is gone the frame the state or the stun ends.

### What was added

Poses are first passes, set by reasoning about the rig's axes and checked by tests of the geometry (the hand ends in front / above / out to the side as written), **not looked at in game**; tune the numbers in the json.

| hunter | name | kind and trigger | pose |
| --- | --- | --- | --- |
| ysolde | `steal_battery` | held: within 1.9 cells of the battery she is heading for (and not fleeing), until the grab at 0.8 | bent over, knees soft, right arm reaching down and out |
| ysolde | `pocket_battery` | cue 0.9 s: the frame `TakeBattery` fires | the battery (real model) in her right hand, lifted to look at, then slid to her side pocket and put away at 0.5 s, a glance round |
| solyth | `burn_battery` | cue 1.1 s: the battery she burns every 30 s | battery crushed between both hands, then arms flared open and it is gone |
| solyth, nyxie | `pounce` | held: the Nightmare leap in the air | both arms stretched ahead and up, body bowed forward |
| solyth | `cold_recoil` | held: the cold source holds her off | both arms crossed up in front of her face |
| nyxie | `darkness_wake` | cue 1.2 s: she wakes (timer, dead lamp or empty lamp) | unfolds from a crouch, arms out |
| glyra | `music_box_hold` | held: the box plays and she is not chasing | the music box (real model) cradled at the chest, head bowed over it |
| glyra | `music_box_startle` | cue 1.2 s: winding down to stopped | box held out, head thrown up |
| glyra | `music_box_settle` | cue 2 s: the runner wound the box | takes it up again, then shoulders drop and head bows |
| mavrine | `survey` / `ping_mark` | held: the 0.9 s survey (she is already standing still for it) / the 0.45 s ping when the mark is crossed | arms out low, head turned / one arm pointing |
| aki | `fuse_lit` / `fuse_burst` | held: the 10 s fuse after a flash / cue 0.8 s at each of the three exposure bursts | hands pressed to her head / arms flung wide, head back |
| briselle | `relay_lock` | held: locked on a recovered relay (12 s) | one arm pointing out in front |
| ceryth | `locker_vigil` | held: waiting by her locker | arms folded, head turned to the door |
| elise, quenelle | `lock_on` | cue 1.5 s: the sprint counter / the 8 s of light reach the lock | head snapped round, fists clenched, shoulders forward |
| quenelle | `torch_frenzy` | held: locked on and the torch is lit | both arms reaching for the light |
| ferryn | `flash_recoil` | cue 1 s: a held beam sends her home | arm over her face |
| haelith | `relay_camp` | held: investigating and within 1.4 cells of the relay | low, forward, hands out |
| opalie | `guard_post` | held: on station | arms crossed, chin up |
| ilyra | `corner_watch` / `pounce_curse` | held: in her corner and not banished / cue 1.4 s when the 15 s curse stuns the runner | hands behind her back, peering / one arm thrust out |
| javelle | `locker_loom` | held: standing within 2 cells of the occupied locker (the 8 s) | leaning in, both hands out for the door |
| kathy | `beam_spot` | held: her search beam is on the runner this frame | one arm along the beam |
| loxelle | `charge_windup` / `charge_lunge` | held: the windup / the charge (the existing `charge` clip still plays under them) | coiled low, arms back / driving forward, arms swept back |
| ravelle | `lay_wire` / `tripwire_snap` | cue 1.4 s when her wires are re-laid / cue 1 s when the runner hits one | crouched, hands working low, then pulling it tight / fist pumped |
| thessine | `mimic_cue` | cue 1.4 s: the fake sound is sent | a hand cupped to her mouth, head back and turned |
| umbriel | `shapeshift` (global) | cue 1.2 s: she changes bodies | rigid, arms stiff out from her sides, head back, then let go |
| verrine | `vent_stalk` | held: the runner is in the vents | crawling low, hands forward |
| wynelle | `materialize` / `freeze_cast` | cue 1.2 s when she appears / held: the ten-second freeze | arms drawn up out of the cold, then open forward over the runner |
| zellia | `trade_offer` | held: the shop is open | both hands out, palms up |
| void_black | `watched_freeze` | held: watched (she is stopped) | rigid, head tilted too far |
| void_blue | `reveal_flinch` | cue 1 s: the beam reveals her | arm up against the light |
| void_brown | `jam_cast` | cue 1.2 s: she jams the hiding spots | both palms thrust out |
| void_cyan | `signal_lock` / `signal_ping` | held: sight building toward the ping / cue 1.4 s when it goes out | hand to her ear / both arms thrown up |
| void_green | `vent_surface` | cue 1.2 s: she comes back out of the vents | rises from a crouch, arms sweeping up |
| void_orange | `sabotage_cast` | cue 1.5 s: a system goes offline | both hands clawed and thrust out, then dropped |
| void_pink | `footstep_trick` | cue 1.2 s: fake footsteps | a hand cupped to her ear |
| void_purple | `static_cast` | cue 1.5 s: the radar is corrupted | both hands up, fingers spread |
| void_white | `beam_flinch` | held: the beam holds her | both arms crossed over her face |

Every row has a test in `hunters/gimmicks/mechanic_tests.rs` (the real event, driven through `Brain::update`, gives the name and it clears by itself or when the event ends), the brain's cue rules and the guest/ttl behaviour are tested there and in `game/hunting.rs` (`the_hunter_frame_carries_the_mechanic_animation...`), the data in `render/anim/profile.rs` (shipped names match what the code raises, a cue's second beat and prop fit inside the cue, the prop sits at the hand, the pose geometry follows this rig's axes, Ysolde end to end through the view).

### Deliberately skipped

* **Xivvy**: her flight is already animated (the `flight` clip, lift and sway); her lock-on at arm's length has no visible beat worth a pose.
* **Wolf spiders**: creature rigs with their own state machine and clips (bark, lunge, attack).
* **Void Gray**: dormant and unseen for most of the run; her prompt events are dropped (`before_perception`, "the route prompt is not ported"), so there is no event to hang an animation on yet.
* **Void Red** (corridor / room speed) and **Void Yellow** (speed gained while the map is open): no discrete event, only a speed value; their gait already shows it through `chase_speed`.
* **Dovelle, Porcelle**: no gimmick (Porcelle's radar mark is not a body action).
* **Zellia's death-on-trade, Ceryth's wrong-answer trap, Javelle's pull**: each ends in a runner effect, a stun on the hunter or her leaving, so the beat would be hidden by the stun (a stun drops a pose by design) or happen where she is not drawn.
* **Kathy's scythe, hunter emotes, the jumpscare clip**: unchanged (see the gaps above).
* **Time-varying poses** (a head that scans, a wrist that wiggles): a state is a static pose with one second beat (`then`); real motion needs a clip.

## Mechanic states: how to add an animation for a hunter's mechanic

A state is a name the code raises plus data that says what it looks like.

1. **Raise the name.**
   * An event: call `b.raise_cue("name")` (in the gimmick hook, next to the `ev.push`) and add the name to `CUES` in `hunters/mechanics.rs` and its length to `data/hunters/mechanic_cues.json`. Example (Ysolde, `gimmicks/ysolde.rs`): `ev.push(Event::TakeBattery(i)); b.raise_cue("pocket_battery");`.
   * A state: give the gimmick a `mechanic(&self) -> Option<&'static str>` returning the name while the state holds (Ysolde: `self.reaching.then_some("steal_battery")`), add an arm to `Gimmick::mechanic` in `gimmicks/mod.rs`, and the name to `HELD` in `hunters/mechanics.rs`.
   `Brain::mechanic()` (called every frame for every drawn hunter and for every hunter frame sent) takes it from there; `render/scene.rs` feeds it to `Input::mechanic` and `frame_entry` to guests. A name outside `CUES` / `HELD` is ignored on a guest and refused by the cue timing file, and the shipped-table test fails if a name has no state or a state has no name.
2. **Describe it in data** (`data/render/hunter_anims.json`), under one hunter's `states` (or globally under `states`; a hunter's entry replaces the global one of the same name, `null` removes it):
   ```json
   "ysolde": { "states": { "pocket_battery": {
       "mode": "set", "blend_in": 0.12, "blend_out": 0.3, "prop": "battery", "prop_until": 0.5,
       "pose": { "R-Arm": [58, 0, 10], "R-Forearm": [82, 0, 0], "Head": [-16, 0, 0] },
       "then": { "after": 0.45, "over": 0.2, "pose": { "R-Arm": [14, 0, 20], "R-Forearm": [24, 0, 0] } } } } }
   ```
   * `clip`: optional, a clip of **that hunter's rig**. It replaces the locomotion clip while the state is on (a stun or a grab still wins). A rig without it keeps its normal clip, so a state can name a clip only some hunters have. Looping clips loop; a one-shot is held on its last pose until the state ends. `speed`, `blend` (seconds of cross-fade).
   * `pose`: bone -> [x, y, z] degrees (Godot YXZ), eased in over `blend_in` and out over `blend_out`. `mode` `"set"` (what the shipped states use): the angles are relative to the bone's REST and replace what the clips and the chase style did to that bone, so the silhouette is the same whatever the walk is doing, and a bone you do not list keeps walking; `"add"` (the default) multiplies them onto the animated pose. Conventions in this rig (it faces -Z, her right is +X): arm X positive raises the arm forward, arm Z positive swings the RIGHT arm out (negative for the left), forearm X positive bends the elbow, chest / belly X negative bows forward, head X negative looks down (positive looks up), head Y positive turns to her left, head Z rolls. Bones the rig lacks are skipped.
   * `crouch`: 0..1 knee bend with the hips lowered so the feet stay down (1 = 40 degrees at the thigh).
   * `then`: a second beat, `after` seconds into the state, moving over `over` seconds to its own `pose` (bones it does not list keep the first beat) and `crouch`. The timeline starts again each time the state comes on.
   * `prop` / `prop_until`: an item model in the right hand, put away `prop_until` seconds in (omit to keep it while the state is on).
   * at least one of `clip`, `pose`, `crouch`, `prop`. New clips for a rig go into `assets/hunters/anim/<id>.json` (add them to `extract.py`'s clip list and re-extract, or edit the json), then name them in `clip`.
3. **Test it.** Add a test to `hunters/gimmicks/mechanic_tests.rs` with the `Bench` in `gimmicks/bench.rs` (one hunter on a generated floor, stepped at 30 fps against a scripted runner): drive the event, assert `x.mech()` is your name, step past it, assert it is `None`. The shipped-table tests in `render/anim/profile.rs` check the bones exist on the humanoid rigs, the names match the code, and a cue's second beat and prop fit inside the cue.

Not covered by states, by design: moving the body (a lunge's displacement stays the brain's job), faces (`desired_expression` in `hunter.rs`), Kathy's scythe (needs a model on `R-HandProp`).
