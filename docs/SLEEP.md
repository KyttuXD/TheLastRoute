# Beds and sleeping

The runner can lie down on a bed and sleep in it. Port of the Godot runner sleep (`docs/REST_AND_SLEEP.md` and
`game/player/player_controller_3d.gd` in the Godot project: `use_furniture`, `toggle_sleep`, `sleep_disturb`,
`_update_sleep`). Code: `src/bin/last_route/game/bed.rs`, on top of `game/seating.rs` (the pose) and `game/sleep.rs`
(the sleeper and the drowsiness).

## Rules

- **Beds:** a prop whose type ends in `Bed`, or contains `bunk` (Godot `interaction_for`). `bedroomNightstand`,
  `bedroomWardrobe` and `bedroomBench` are not beds. A bunk that is also a hiding spot keeps its `[E] HIDE` target;
  the mattress is a separate target.
- **Lie down:** look at the mattress within `default_reach` of `data/seating.json` and press interact
  (`[E] LIE DOWN`). The view glides to the pillow and looks along the bed; the look is held around that line.
- **Lying:** the runner does not move and the flashlight is off. Interact gets up. Jump sleeps.
- **Asleep:** the screen darkens, stamina refills faster (the normal recovery plus `stamina_per_s`) and the
  drowsiness (the fatigue shown in the lying prompt) falls by `sleep_fall_per_s`.
- **Waking:** jump or a move key wakes (the runner still lies); interact wakes and gets up. The sleeper wakes
  `RESTED` when the drowsiness reaches `wake_at`. A hunter inside `disturb_radius`, a hunter chasing the runner, or a
  stun wakes it with a jolt: `jolt_exposure`, `jolt_noise` and `jolt_seconds` of frozen input. A hunter inside
  `stir_radius` gives the stir cue every `stir_cooldown_s`.
- **Not tired:** a runner at or below `wake_at` drowsiness cannot fall asleep. Drowsiness rises while awake
  (`awake_rise_per_s`, twice as fast standing still) and starts at 0 on every run and load.
- **Not safe:** sleeping is not hiding. Hunters see, hunt and catch a sleeper as any runner. A grab, going down, or
  the run ending takes the runner off the bed.
- **Time:** nothing is skipped. Hunters, the anti-idle pressure and the other runners carry on.
- **Body and saves:** only the eye moves onto the bed. The body stays where it stood beside the bed, so a save, a
  net sample and the hunters all read that spot, and a runner that quits asleep loads standing by the bed. The save
  format and the net sample are unchanged.

## Data

`src/bin/last_route/data/sleep.json` (`lr.sleep/1`):

| Key | Meaning |
| --- | --- |
| `sleep_fall_per_s`, `wake_at` | drowsiness lost a second asleep; the level the sleeper wakes at |
| `awake_rise_per_s`, `moving_rise_scale`, `still_rise_scale` | drowsiness gained a second awake |
| `disturb_radius`, `disturb_chase_always` | the hunter distance that jolts a sleeper (Godot 5 m); a chase always does |
| `jolt_seconds`, `jolt_exposure`, `jolt_noise`, `jolt_penalty` | what a jolt costs |
| `bed.prop_rules` | which prop types are beds, and their seat kind in `data/seating.json` |
| `bed.eye_above_mattress`, `bed.head_inset`, `bed.ease_seconds` | the pillow view |
| `bed.look_pitch`, `bed.look_yaw_limit_deg`, `bed.look_pitch_down_deg`, `bed.look_pitch_up_deg` | the look range |
| `bed.stamina_per_s` | extra stamina a second asleep (Godot 9) |
| `bed.dark_alpha`, `bed.dark_in_per_s` | how dark the screen gets, and how fast |
| `bed.stir_radius`, `bed.stir_cooldown_s` | the stir cue (Godot 12 m, 3 s) |
| `bed.text` | the prompts and messages; `{fatigue}` is the drowsiness in percent |

`data/seating.json` gives the bed kinds their label (`LIE DOWN`), mattress `height` and the `default_reach`.

## Hunter naps

Port of Godot `game/hunters/ai/hunter_naps.gd`. A nap is a wander activity, not a separate system: a `bed`
point of interest (any prop the bed rules call a bed) is one of the goals a wandering hunter picks, weighted in
`data/hunters/activities.json` (`kinds.bed`, weight 0.25; the activity `nap`, 25 to 45 seconds).

- **Walk and lie:** the hunter walks to the open cell nearest the bed, then naps on the mattress. The body lies on its
  back along the bed's long axis with the head toward the pillow end (`render/anim/hunter_activity_pose.rs`
  `nap_root`). The nap pose (`nap` in `data/render/hunter_activity_poses.json`) folds the arms and tilts the head.
- **Asleep:** the brain runs no perception, no movement and no catch while it naps. Only the nap clock runs
  (`hunters/brain.rs` `update`, the `napping` block).
- **Waking:** a runner within `nap_wake_distance` (3 m), a noisy runner (`noise` at least `nap_wake_noise`) within
  `nap_wake_noise_distance` (10 m), or a torch in the hunter's face within `nap_wake_torch_distance` (7 m) wakes it
  with a startle: a stun of `nap_startle_stun` seconds, then it investigates where the runner was (`hunters/naps.rs`).
  A hidden runner does not wake it. A stun wakes it too. When the duration runs out it wakes on its own and idles.
- **Guests:** a guest sees the nap through the activity the host sends (`act` index 10). The wire has no bed spot,
  so a guest draws a napping hunter standing, with the nap pose, not lying on the mattress.

**Not done:** Godot's per-difficulty chance (no naps on nightmare, a chance per second) is not wired: naps are plain
weights on the bed goal. The startle does not play a flinch emote. The meter (a runner in the room waking it slowly)
is not ported; the wake is the distance, noise and torch rules only.

## Not done

- Co-op: the others see a sleeper standing by the bed. The net sample has no lying or asleep pose besides `Down`,
  which means downed, so nothing is sent (Godot replicates `asleep` and draws "z Z").
- Passing out at full drowsiness and the tired slow-down (`pass_out`, `slow_below` in the data) are not wired.
- Accidental sleep (Godot solo: 50 s idle in a bed, then 8 s drowsy) and the getting-up glide.
- Player NPC teammates lying in beds (they walk past beds, sleep is not wired for them yet).
- A drowsiness bar on the HUD.
