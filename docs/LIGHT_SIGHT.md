# Mutual light sight

If the runner can see a hunter because the hunter stands in light, the hunter can see the runner, flashlight or not. This is
the Rust port of Godot's `game/look/light_field.gd` and its `docs/LIGHT_SIGHT.md`.

## The bug this fixed

`Brain::self_exposure` and `Brain::target_exposure` existed and `Brain::sight_limit` read them, but nothing on the host
ever set them. Every hunter therefore saw a runner with the flashlight off at a flat half of its sight (about 4 cells for
Aki), however well the building lit her, while the player saw a lamp-lit hunter from the lamp's full reach.

## Where it lives

* `src/bin/last_route/hunters/light_field.rs`: the pure rule. `exposure_at(point, ambient, sources, line_clear)` is 0..1:
  the ambient floor plus every lamp within `LAMP_REACH` (25) and its own range, each adding
  `0.9 * energy * (1 - d / range)^1.6` (Godot's falloff), unless a wall blocks the line (`Facility::has_line_of_sight`) or the
  contribution is under 0.02. `unlit_scale(exposure)` runs from `DARK_SIGHT_SCALE` (0.25) to `LIT_SIGHT_SCALE` (1.6).
* `src/bin/last_route/game/light_sight.rs`: the host. `Game::refresh_light_sight` runs in `update_hunters` before each
  brain's `update`, about every 0.2 s per hunter (`Brain::light_due`, staggered over the hunters), and sets both exposures.
* `Brain::sight_limit` is unchanged and is the Godot rule: with the flashlight off the range is
  `sight * unlit_scale(target_exposure)`; the flashlight keeps its own range; and a hunter whose own exposure is at least
  `VISIBLE_EXPOSURE` (0.36) sees at least her full sight (line of sight still required). Gimmicks scale the result.

## What counts as light

* The lamps that draw the floor (`Game::world.lamps`: room lights, prop lamps, the safe zone, stairwells), with the flicker
  of the moment applied, so a lamp that is dark in its flicker or a switched-off or dark room lights nothing.
* The ambient floor is the luminance of the render ambient at the runner's place (`data/render/ambient.json`), never below
  Godot's 0.02. The Rust render is brighter than Godot's near-black (ambient 0.35, not 0.035), and a hunter should not lose the
  runner before the player loses the hunter.
* Points are lit at body height (1.2 cells above the feet) in 3D, so a ceiling lamp counts by its real distance.
* Calibration: inside the inner half of a shipped room lamp's reach the exposure is at least 0.36 (test
  `the_inner_half_of_a_real_room_lamp_counts_as_visible`).

## When the rule is off (the old flat 0.5 scale)

* A guest (it runs no hunters).
* The admin MUTUAL LIGHT SIGHT switch, off (`shared::admin_flags::LIGHT_SIGHT`, published by the admin menu).
* A floor with no lamp data at all.
* The target's side only: when the target is on another level (the 3D `cross` sight answers), and the hunter's side when
  the hunter is not on the runner's floor.

## Not done

* Lamps of neighbouring levels (through stairwells) and the catwalk and splash lamps are not read; only the floor's own lamps.
* Resident hunters on other levels get no exposures (their target is always across levels).
* Bots as targets get their own exposure from the same lamps; the shared per-frame cache is keyed by the point asked for.
