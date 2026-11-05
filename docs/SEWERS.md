# Sewers: ladders, drops and waterfalls

How the sewer levels connect, as built for the continuous levels project (see [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md)). Godot reference: `game/world/generation/sewer_route_generator.gd`. Ramps, manholes, grates, elevators and flood valves are still open (see [GODOT_FEATURE_AUDIT.md](GODOT_FEATURE_AUDIT.md)).

## Ladders

- A ladder is a `Flight` of kind Ladder made by `ladder_flight`, so both levels agree on the cells.
- The climb moves your height smoothly; the world is not swapped.
- The step cell is the one clear floor cell beside the hatch where you step on and off. The rails stand on the side opposite the step cell.
- The hatch is a hole in the upper floor with a propped cover model beside it. The lower level has no ceiling over the shaft, and the shaft's walls, rails and back plate run up through the slab gap to the floor above (`render/seams.rs`).
- Hunters take ladders too, at `ladder_speed_scale` (`data/hunters/flights.json`), over the same Ladder edge. There is no swap.
- One climber at a time. Jumping lets go and you fall to the ground below.

## Waterfalls

- The drop cells are open void: no floor and no ceiling; the sheet is drawn between the real heights. The pool cells under the void have no ceiling either; their walls and the slab rims of their ceilinged neighbours close the opening up to the lip of the level above.
- Walking in means a free fall of one storey (about 0.913 s at `PLAYER_GRAVITY` 12), with limited air control (`AIR_CONTROL`).
- Landing in the pool below gives the splash, swim and soaked rules from `data/swim/movement.json`. Landing anywhere else is a hard landing from `data/swim/fall.json`: stun, stamina loss, exposure and noise, all scaled by the drop height. There are no hit points.
- A waterfall is a real fall. Hunters never jump a drop unless `drop_allowed` is true in `data/hunters/flights.json` (it is false).
- The fall damage rule is `data/swim/fall.json` (see [DATA_FILES.md](DATA_FILES.md)).

## Water and hunters

- The sewer water field and valve floods reach hunters: `game/hunter_pipes.rs` and `hunter_pipe_cells` mark flooded cells, so hunters path around deep water, wade when chasing, and replan when a valve floods their route. Rules per hunter: `data/hunters/water.json`.

## Heights and the water camera

- The sewer ceiling is 4.0 m (`geometry.wall_height` in `data/regions/sewer.json` and `sewer2.json`): Godot's `CEILING_HEIGHT` 3.2 times our 1.25. The storey stays 5.0 m, so a 1.0 m slab lies between the sewer ceiling and the floor above. Ladder shafts and stair flights are lined up through that slab to the floor above (`Site::shaft_top` in `render/stairs.rs`), so no void shows.
- The water has the same depth on every sewer storey: `data/sewer_water.json` (channel depth 1.0, surface -0.3, so 0.7 m over the bed). Level -2 no longer has its own depth; the drawn bed and the swim physics, hunters and landing agree. The numbers are Godot's unscaled values, which are player-relative there.
- The eye sinks in channel water, as if the body sank into it (`data/swim/camera.json`). The feet stay on the walkway, so ground, steps and falls are unchanged; `Player::water_sink` lowers `eye_y()` (and so the camera and the hunters' sight line to the runner). The target is the channel depth over a water cell, capped by `max_sink`, and never closer than `min_eye_above_surface` to the surface or `min_eye_above_floor` to the walkway; it eases at `ease_down_per_s` and `ease_up_per_s`. A ladder or a hiding spot lets it rise. A pipe flood over dry floor gives no sink with the shipped data. Godot never lowers the camera in wading water (only its plunge puts the eye just above the surface), so this is an owner decision, not parity.
- The Inferno floor (level -3) is flat, as in Godot; see `data/inferno.json` `floor`.
