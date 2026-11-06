# Admin menu

The in-run admin menu (`src/bin/last_route/ui/admin/`, key 0, Esc closes) and the title ADMIN panel
(`hub.rs`). Only the signed-in admin account gets it: `admin.enabled` follows the server's admin flag
(`backend.admin`), and `publish()` writes every switch at its default while it is off. If the flag drops
with the menu open, the next frame closes it (so the mouse is not left free).

## Tabs

STATUS, GENERAL, PLAYER, HUNTERS, NPCS, WORLD, STORY, VISUAL, DEBUG VIEW, FIELDS (`TABS` in `mod.rs`).

- Tab, Shift+Tab, Left and Right switch tabs and stop at the ends.
- Up, Down, PgUp, PgDn, Home and End scroll a tall page (the mouse wheel too); a thin bar at the right edge
  of the page shows where you are. On FIELDS, Up and Down move between the line edits while one has focus,
  and Enter runs the focused field.
- Dropdown rows are `(id, label)` lists from `data/admin/menu.json`. The button or list shows the label; the
  id is what the code matches. RUN SPEED and GAME SPEED captions end in `// NOW <speed>`, and their dropdowns
  start on the speed in force.
- RESET ALL SETTINGS (GENERAL, PLAYER, VISUAL, DEBUG VIEW) puts every switch, speed and text field back to its
  default and saves the defaults to admin.cfg.

## Data

`data/admin/menu.json` (`lr.admin_menu/1`): the lists above plus `npc_smarts` (exactly 4), `npc_spawn_max`
(the NPC count stepper stops there) and at least one run speed and one game speed. A bad file fails the tests.

## NPC commands

Live (`npc_live` in `menu.json`): TELEPORT TO ME, FOLLOW ME, STOP FOLLOWING, SMART = SPAWN SMART, REMOVE, and
ALL TO ME, ALL FOLLOW ME, ALL STOP FOLLOW, ALL SMART. FOLLOW and STOP FOLLOWING set the NPC's `joined` flag (an admin
NPC has no join radius, so a parked one stays put). Plus SPAWN NPC, REMOVE NEAREST and REMOVE ALL (sidebar).

A live button is drawn normally. It is gray only when the run has no NPC to act on (REMOVE NEAREST and REMOVE ALL too);
its hint line then says `NO NPCS ON THIS RUN // PRESS SPAWN NPC FIRST`. The buttons the build cannot do yet
(`npc_pending`, each with its reason) are always gray: GO TO ME, HUNTERS IGNORE / TARGET, CHAT ON / OFF, DOWN, REVIVE,
KILL, CLEAR, SPECTATE, FREEZE, RESUME and their ALL versions. A gray button still takes the click; while the mouse is
on one, the hint line under the menu says what is missing. Clicking it answers
`ADMIN // <LABEL> IS NOT IN THIS BUILD YET`. Each id must be in exactly one of the two lists (a test checks it), so
moving a command to `npc_live` is the whole change once the game can do it.

## Preferences

`admin.cfg` in the user folder (`prefs.rs`): no_target, god_mode, noclip, map_reveal, region_map,
show_collision, hunter_reveal, perf_overlay. In unit tests the preferences are kept in memory only, so a test
never edits the real file.

## Screenshots

`last_route --screenshot out.png` with `LR_SHOT_RUN=1 LR_SHOT_INF=1 LR_SHOT_ADMIN=<tab 0..9>` draws the menu on
a run (it forces the admin gate on for the screenshot only); add `LR_SHOT_SIZE=WxH` for a window size.
