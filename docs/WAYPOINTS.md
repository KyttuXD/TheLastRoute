# Waypoints

One waypoint per player. This is the Rust port's behaviour; the Godot reference is `docs/WAYPOINTS.md` in the Godot project
(`LastRoute Godot/KyttusLabApp`), with `game/hud/waypoint_overlay.gd`, `game/hud/teammate_waypoints.gd` and
`game/hud/waypoint_state.gd` as the source.

## Placement is map-only

A waypoint is placed and cleared on the fullscreen map (M): a double click, a right click or E at the map cursor
(`src/bin/last_route/ui/map.rs`, module doc). There is no in-world key.

The Godot doc is stale here. Its Design section (lines 25-27) says the `waypoint` key (B, gamepad R3) places at the crosshair and
`waypoint_clear` (V) clears, and line 57 repeats it. Godot's own test says otherwise: `tests/validate_waypoints.gd` lines 136-147
check that `waypoint` and `waypoint_clear` are gone from the input map, the rebind list and the controls tabs, so typing in
chat can never set one, and that the map's request still reaches the route. The Rust port follows the test, not the doc.

## Where each piece lives

| Piece | Rust |
|---|---|
| The runner's own waypoint, its route and its map pin | `src/bin/last_route/game/endless.rs` (`Waypoint`, `WaypointRoutes`) |
| Every player's waypoint (one each; a new one replaces the old; clearing removes it) | `src/bin/last_route/game/waypoint.rs` (`PeerWaypoint`, wire fields `wp` and `wl`) |
| Chevrons from a teammate toward their waypoint, edge arrows, name and distance | `src/bin/last_route/ui/teammate_markers.rs` |
| Chevron spacing, count, reach and label | `src/bin/last_route/data/hud/teammate_markers.json` |
| The TEAMMATE WAYPOINTS setting | `src/bin/last_route/app/settings.rs` (`teammate_waypoints`), shown in `ui/settings_panel.rs` |
| Pins on the minimap and fullscreen map | `src/bin/last_route/ui/map.rs` |

## Open

- The wire fields `wp` and `wl` are written for `net/world_sync.rs` but the sample hook is not applied there yet, so a teammate's
  waypoint does not reach other clients.
- The ground trail on the floor along the route stays the runner's own PATH guidance in `endless.rs`.
- Godot's floor line and edge arrow for the player's own pin, and the distance and chime settings, are not ported.
