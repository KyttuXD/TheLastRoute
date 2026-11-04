# Sewers: ladders, drops and waterfalls

How the sewer levels connect, as built for the continuous levels project (see [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md)). Godot reference: `game/world/generation/sewer_route_generator.gd`. Ramps, manholes, grates, elevators and flood valves are still open (see [GODOT_FEATURE_AUDIT.md](GODOT_FEATURE_AUDIT.md)).

## Ladders

- A ladder is a `Flight` of kind Ladder made by `ladder_flight`, so both levels agree on the cells.
- The climb moves your height smoothly; the world is not swapped.
- The step cell is the one clear floor cell beside the hatch where you step on and off. The rails stand on the side opposite the step cell.
- The hatch is a hole in the upper floor with a propped cover model beside it. The lower level has no ceiling over the shaft.
- Hunters take ladders too, at `ladder_speed_scale` (`data/hunters/flights.json`), over the same Ladder edge. There is no swap.
- One climber at a time. Jumping lets go and you fall to the ground below.

## Waterfalls

- The drop cells are open void: no floor and no ceiling; the sheet is drawn between the real heights.
- Walking in means a free fall of one storey (about 0.913 s at `PLAYER_GRAVITY` 12), with limited air control (`AIR_CONTROL`).
- Landing in the pool below gives the splash, swim and soaked rules from `data/swim/movement.json`. Landing anywhere else is a hard landing from `data/swim/fall.json`: stun, stamina loss, exposure and noise, all scaled by the drop height. There are no hit points.
- A waterfall is a real fall. Hunters never jump a drop unless `drop_allowed` is true in `data/hunters/flights.json` (it is false).
- The fall damage rule is `data/swim/fall.json` (see [DATA_FILES.md](DATA_FILES.md)).

## Water and hunters

- The sewer water field and valve floods reach hunters: `game/hunter_pipes.rs` and `hunter_pipe_cells` mark flooded cells, so hunters path around deep water, wade when chasing, and replan when a valve floods their route. Rules per hunter: `data/hunters/water.json`.
