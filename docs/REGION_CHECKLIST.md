# Region visual check list

A check list for reviewing each region in a run before it ships (TODO.md, "A visual check list per region"). Review one region at a
time in a real run, on Standard, with the flashlight on and then off. Tick the common list for every region, then the region's own
notes. Record what failed as an issue against the region, not as a general complaint.

Region data lives in `src/bin/last_route/data/regions/` (layout and rules) and `src/bin/last_route/data/region_rooms/` (room
sets); flat textures are chosen by `src/bin/last_route/data/region_textures.json`. The flat look rules are in
[OCD_TEXTURE_PLAN.md](OCD_TEXTURE_PLAN.md), the map overhaul notes in [MAP_OVERHAUL_AUDIT.md](MAP_OVERHAUL_AUDIT.md), and the sewer in
[SEWERS.md](SEWERS.md).

## Common list (every region)

- [ ] Floor: reads as the region at a glance; no stretched, flickering or mismatched tiles; seams between rooms and corridors are clean.
- [ ] Walls and ceiling: the colours are 2-3 tones and 3-6 colours per the flat texture rules; no z-fighting, no gaps showing the void.
- [ ] Doors and doorways: frames line up with the walls; doors open and close without clipping; the opening fits the player.
- [ ] Light: the flashlight cone reads on every surface; dark corners are dark, not black holes; light colours differ between areas.
- [ ] Props: placed on the floor, not floating or sunk; nothing blocks a door or traps the player; props fit the region.
- [ ] Landmarks: at least one place the player can recognise and use as a waypoint.
- [ ] Hiding spots: lockers and other hides match the region and are reachable; the camera does not clip inside.
- [ ] Stairs, ladders and vents (where the region has them): the walk is continuous, no snap, no stuck spot, the space around spiral stairs is open enough to turn in.
- [ ] Map: the minimap and the fullscreen map show the region's rooms and corridors correctly, and the colours match the floor.
- [ ] Hunters: a hunter can path through every room and corridor; its model is not clipped by walls.
- [ ] Sound: the region has its own ambience; footsteps match the floor.
- [ ] Performance: frame time stays steady while turning in the busiest room; no hitch when a new chunk loads.
- [ ] No text on screen overlaps, and the region name or label (if shown) is upper case.

## Regions

### Apartment

Data: `apartment.json`, rooms `region_rooms/apartment.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Beach

Data: `beach.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Carnival

Data: `carnival.json` (also the arcade and circus names).

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Gothic

Data: `gothic.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Greenhouse

Data: `greenhouse.json`, rooms `region_rooms/greenhouse.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Hospital

Data: `hospital.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Hotel

Data: `hotel.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Ice

Data: `ice.json`, rooms `region_rooms/ice.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Inferno

Data: `inferno0.json, inferno_caves.json`, rooms `region_rooms/inferno0.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Lab

Data: `lab.json`, rooms `region_rooms/lab.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Mall

Data: `mall.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Mansion

Data: `mansion.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Maze

Data: `maze.json, maze_boiler.json, maze_courtyard.json, maze_library.json, maze_warehouse.json`, rooms `region_rooms/maze.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Offices

Data: `offices.json`, rooms `region_rooms/offices.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Prison

Data: `prison.json`, rooms `region_rooms/prison.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Sewer

Data: `sewer.json, sewer2.json, sewer_pipes.json, sewer_rooms.json` (see docs/SEWERS.md).

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Stalls

Data: `stalls.json`, rooms `region_rooms/stalls.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Tunnels

Data: `tunnels.json`, rooms `region_rooms/tunnels.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

### Utility

Data: `utility.json`, rooms `region_rooms/utility.json`.

- [ ] Common list passed.
- [ ] The look is distinct from the regions on either side of it on the route.
- [ ] Region notes: ____

## After the review

Fix what failed in data first (`data/regions`, `data/region_rooms`, `data/region_textures.json`); code only when the data cannot say it.
Run the headless suites again after a region change, and tick the region here only after the run, not after the edit.
