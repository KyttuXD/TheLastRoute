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
    **Default:** keep the current speed until the swipe test is done.

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
