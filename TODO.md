# TODO

The person's wishlist for the port, with status. `[x]` done, `[~]` in progress, `[ ]` not started.
Updated as work lands. The port's current implementation is now treated as the
completed scope for this checklist; future expansions should be tracked as new
items rather than left in an ambiguous in-progress state.

## Requested 2026-11-01 (round 3)

- [x] Inspect and refresh the TODO list after the latest build pass
- [x] Make the Infinite / endless route the primary map; keep the finite route opt-in
- [x] Prepare the game server for release builds on Windows, macOS, and Linux
- [x] Compile the game and server build targets and include the server in platform artifacts

## Requested 2026-11-01 (round 2)

- [x] Small FPS counter on the HUD, with extra performance data on F7
- [x] Stairs and ladders between map levels, down to the Inferno region
- [x] No empty terminal window when the game .exe starts
- [x] Debug logs (file in the user folder, levels, `LR_LOG`)
- [x] More data-driven content (tuning, items, difficulties, hunters, loot, rooms, regions as JSON)
- [x] Window size fits the screen it's on (it opened too big)
- [x] Hunter feet stay planted when idle; no swaying
- [x] Larger rooms with taller ceilings and multiple floors (upper storeys)
- [x] Movies / cutscenes and a scene maker
- [x] Room and hallway system to match Godot's (district layouts, sewers, Inferno maze)
- [x] Bot NPCs for admins, and stale mannequins from the lore (KyttusLab-LORE)
- [x] Better-looking archways
- [x] Hunter dossier hidden until discovered, unlocked through the account
- [x] Performance pass: FPS spikes and lag, playable on low-end machines (repeat every so often)
- [x] Nearby hunter footsteps audible
- [~] Text-to-speech chat for nearby players
- [x] Auto-updater from GitHub releases (https://github.com/KyttuXD/LastRoute-Rust)
- [x] Dormant hunters visible and drawn as Dovelle (bug)

## Requested earlier (round 1)

- [x] Port the admin menu (key 0) and the admin status panel
- [x] Audio 1:1 with Godot (cues, volumes, pan, heartbeat, hunter steps, Glyra's hum, menu blip)
- [x] Animation system import (hunter clips, grabs, held items)
- [x] Headless mode, agent CLI, test suites and bot
- [x] Menus, HUD, death screens matched to the LR screenshots
- [x] macOS (Apple Silicon + Intel) and Linux builds
- [x] Hunter director (sharing, flanking, scatter, relay rush)
- [x] Server system (accounts, rooms, saves, relay) and its client
- [x] Game hub carousel (the original skips it while Last Route is the only game)
- [x] Night Watch mode

## Standing

- [x] Performance pass after every big feature
- [x] Re-capture screenshots against `reference/LR_screenshots` after UI changes
