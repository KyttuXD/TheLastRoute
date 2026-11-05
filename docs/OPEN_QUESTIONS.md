# Open questions

Short answers are fine. Where there is a default, the work goes ahead with it if there is no answer.

## Loading card

1. The loading card still seems to stay on screen. When it hangs, what is on screen: the card at a
   fixed percent, the title with the card over it, or nothing at all? (Reply with the percent or a photo.)
2. First status line: keep "CONNECTING TO RELAY..." (web wording), or use "PREPARING..."?
   **Default:** keep the web wording.
3. Failure text: "RESTART THE GAME" (desktop) or the web's "REFRESH"?
   **Default:** "RESTART THE GAME".

## Web UI port

4. Settings tabs: match the web's four tabs (GAMEPLAY, AUDIO, VIDEO, CONTROLS), with SKIN, VOICE and
   MODS placed under GAMEPLAY? **Default:** yes.
5. Office mode (the web's office screens): in scope for this project, or leave it out?
   **Default:** leave it out for now.
6. Save slots in the lobby (AUTOSAVE, SLOT 1-3): OK to change the save format so each slot is kept?
   Current saves must still load. **Default:** yes, with old saves still loading.
7. Phase order for the web UI: keep the phases in the inventory order (settings, then pause and end
   screens, then HUD, then the full map, then lobby saves, then admin tools)?
   **Default:** yes.

## Character files (dossier)

8. Category headers: the web never locks a category, but Rust does. Should a category with no
   discovered hunter stay locked on both?
   **Default:** yes, keep the Rust rule.
9. Locked hunters: fully blank (name, description, stats) on both. Confirmed?
   **Default:** yes.

## Lobby and account

10. Account screen: the web says "ACCOUNT" when signed in; the Rust build shows the username.
    Keep "ACCOUNT" (done), or show the username?
    **Default:** ACCOUNT.
11. Account username hint: "3-24 CHARS: A-Z 0-9" (screenshot) or "3-24 chars: A-Z 0-9 _" (repo)?
    **Default:** the screenshot wording.
12. Which admin screen does "MANAGE ACCOUNTS (ADMIN)" open?
    **Default:** the existing Rust admin screen.

## NPC teammates

13. Hunters ignore the player bots (NPC teammates) for now. Is that right for this stage?
    **Default:** yes.
14. Which NPC commands come next: teleport, follow, freeze, down and revive, chat, or spectate?
    **Default:** teleport and remove-per-NPC first.

## Map and HUD

15. Relay colour on the map: orange (the new colour) or teal-blue (the old label)?
    **Default:** orange, and the legend text is updated to match.
16. Last Route focus outlines: amber, as on the web (not teal)?
    **Default:** amber.
17. Primary button labels: light cream, as on the web, not dark?
    **Default:** light cream.

## Title

18. PUBLIC ROUTE: the button shows "not available in this build yet". Is that the right wording?
    **Default:** yes.

## 3D world (Godot parity)

19. Which comes first after the web UI: the Godot 3D world look, or the rest of the web UI?
    **Default:** finish the web UI phases first.
20. Mouse look: can you do one identical physical swipe in the Godot build and the Rust build, with the
    same sensitivity? That settles whether the current speed is right.
    **Default:** the Windows multiplier is unchanged (0.583); macOS now uses 1/3 (Godot's k = 3, points) from
    `data/player/look.json` `mouse_multiplier_macos`, derived and not yet measured. The same swipe test settles it.

## Doors and props

21. Chairs: do they now face their tables in a restaurant? (The change is in; the capture was too small
    to confirm.)
22. Doors: do the door leaves now fill their frames? The wall above a door is still a gap in some rooms.
    Fill the gap now, or later?
    **Default:** later.

## Multiplayer

23. A two-window room test needs a person at the screen (host in one window, guest in another).
    When can you do that?
24. Should the room test also check that the hunter's stats stay hidden for other players?
    **Default:** yes.

## Repository

25. Commit and push the checkpoint branch (`checkpoint/2026-11-03`) to GitHub? Merge it into `main`?
    **Default:** push the checkpoint branch only, not `main`.
26. CI: the build workflows were failing because of artifact storage and the self-test. Should the
    artifacts keep 3 days, or go back to the default 90?
    **Default:** 3 days.

## Stairs project decisions D1-D8

The stairs questions D1 to D8 in the old list are answered in the Decisions table of [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md) and implemented (phases 0-9).

## Stairs: flights or Godot ramps (owner call)

27. Godot links levels -1 to -2 and -2 to -3 with 12-cell RAMP tunnels (`sewer_route_generator.gd` 134-186, `docs/SEWERS.md` 12-16), and
    its maze-to-sewer wells use `Stairwells.CHANCE[0] = .12`, `SPIRAL_SHARE[0] = .5` and the 8 `MAZE_AREAS` (`stairwells.gd` 20-37), vetoed
    by `_maze_stairwell_fits` (`streamed_route_generator.gd` 567). Rust links -1 to -2 and -2 to -3 with walked flights
    (`infinite::stair_link`, `STAIR_CHANCE` 0.3) and has maze wells in about 7 to 10 % of level-0 chunks (pinned by
    `level0_maze_wells_are_sparse_like_godot`), so many level-0 windows have no stairwell and the way down is mostly the sewer ladders. When
    you say "stairs do not link to the other floors", do you mean (a) a flight you walked that did not connect (then run the protocol command
    `stairs_report` standing there and send the seed, the level and its output; `docs/AGENT_CLI.md`), (b) that there are too few
    stairs on level 0, or (c) that you expect Godot's ramps below the first sewer? Keep the flights, or port the ramps?
    **Default:** keep the flights (decision D1 chose the three shapes); no ramps.

## Inferno terrain (owner override of Godot parity)

28. **Decided by the owner (2026-11-04): "add noisy floor height and ceiling height in the Inferno caverns".** Godot's Inferno floor is flat, and the heights track made it flat here too; this overrides that parity. Level -3 now has a smooth noisy floor (up to 0.5 m up or down) and ceiling (sagging up to 1.2 m under the slab line, never closer than 2.6 m to the floor, never above `STOREY - slab`). The player, hunters, bots, props, doors, pickups and decorations stand on it; the flat-floor contract is replaced by `world/inferno_terrain.rs` and its tests. The numbers are the `terrain` block of `data/inferno.json`. Stairwells keep a flat floor and a full-height ceiling (a pad that eases out over 8 cells). Open question: the noise covers the whole of level -3 (the maze rooms and halls too), not only the carved caves, because the terrain is a function of the world cell alone. **Default:** keep it everywhere; say if halls and rooms should stay flat.
