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

## RESET PASSWORD (FIELDS tab, full admin owners)

The fourth FIELDS line edit sets another account's password through `POST /api/admin/account/reset-password`
(`net::api::admin_reset_password`, `ui/admin/password.rs`). It exists only while the signed-in account is a full admin
owner (KYTTU or AYTTU: `Admin::owner`, which the app sets from the server's `adminOwner` flag each frame); nobody else sees
the field or the button, and the server refuses a non-owner anyway. Type the account name (3 to 24 letters, numbers or `_`)
and press Enter or RESET PASSWORD. The server generates a temporary password, signs that account out everywhere and answers
with the password once; a message box shows it with COPY (clipboard) and CLOSE (Esc closes it too). Nothing stores or
logs the password: it is gone when the box closes. Your own account is refused (use the account panel). The field is the
last entry of `fields` in `data/admin/menu.json` (a test pins that, so hiding it moves nothing). Lost-password steps for
the live server, including the offline command, are in docs/SERVERS.md.

## Preferences

`admin.cfg` in the user folder (`prefs.rs`): no_target, god_mode, noclip, map_reveal, region_map,
show_collision, hunter_reveal, perf_overlay. In unit tests the preferences are kept in memory only, so a test
never edits the real file.

## Screenshots

`last_route --screenshot out.png` with `LR_SHOT_RUN=1 LR_SHOT_INF=1 LR_SHOT_ADMIN=<tab 0..9>` draws the menu on
a run (it forces the admin gate on for the screenshot only); add `LR_SHOT_SIZE=WxH` for a window size.

## Changes 2026-11-07

- **ADMIN ACTIVE badge removed.** The top-centre "ADMIN ACTIVE // SOLO // DEBUG RUN" banner is no longer drawn, in or out of the menu. The STATUS tab no longer shows "ROUTE DEBUG" or "DEBUG RUN // NOT RECORDED"; it still shows the offline line and the MAP REVEAL state.
- **MAP REVEAL undo.** Switching MAP REVEAL off now restores the map as it was before it was switched on: the discovered cells and level 0's seen bitmaps (`Game::map_reveal_snapshot` / `map_reveal_restore`). Before, the reveal's cells stayed discovered after the toggle was turned off. Discoveries made while the reveal was on are discarded on switch-off. RESET ALL and a new run also drop the saved snapshot.
- **Debug marking is unchanged.** A run is still marked as a debug run internally (roster and read-only rows opt out), so the report and records still show it; it is just no longer displayed.

## Changes 2026-11-07

- **ADMIN ACTIVE badge removed.** The top-centre "ADMIN ACTIVE // SOLO // DEBUG RUN" banner is no longer drawn, in or out of the menu. The STATUS tab no longer shows "ROUTE DEBUG" or "DEBUG RUN // NOT RECORDED"; it still shows the offline line and the MAP REVEAL state.
- **MAP REVEAL undo.** Switching MAP REVEAL off now restores the map as it was before it was switched on: the discovered cells and level 0's seen bitmaps (`Game::map_reveal_snapshot` / `map_reveal_restore`). Before, the reveal's cells stayed discovered after the toggle was turned off. Discoveries made while the reveal was on are discarded on switch-off. RESET ALL and a new run also drop the saved snapshot.
- **Debug marking is unchanged.** A run is still marked as a debug run internally (roster and read-only rows opt out), so the report and records still show it; it is just no longer displayed.

## STATUS and UPDATES (2026-11-07)

- **BUILD and SERVER lines:** the STATUS tab shows the running build (CI number and commit, or LOCAL) and the server the game talks to. The old "SERVER OFFLINE (SOLO, NO SERVER)" line is gone.
- **UPDATES:** the MACOS, WINDOWS and LINUX lines come from the public nightly release on GitHub (`last_route-macos-x86_64.zip`, `last_route-macos-arm64.zip`, `last_route-windows-x86_64.exe`, `last-route-linux-x86_64.tar.gz`), with the time CI last published each file. They no longer ask the old update server, which was stuck on old builds. PHONE and QUEST show NONE PUBLISHED, since there is no nightly file for them.

## Audit of ADMIN // STATUS (2026-11-07)

Checked line by line against `ui/admin/hub.rs` and the data each line reads. Status: OK, STALE (hard-coded or old text), WRONG (shows false information), GAP (not wired).

| Section | Line | Status | Finding | Fix |
|---|---|---|---|---|
| THIS BUILD | version | STALE | `VERSION 0.0.1.0` (menus.rs:132) does not match the shipped apps (Mac app 0.1.0) and does not show the CI build | show `updater::build_label()` (CI build and commit) |
| THIS BUILD | `BUILD RUST PORT (#0)` | STALE | `#0` is a literal, not a number | drop it |
| THIS BUILD | `CHANNEL LOCAL` | STALE | always LOCAL, even on CI builds | show nightly (CI) or LOCAL from the build number |
| THIS BUILD | `HALF-RES PIXEL VIEW` | STALE | the description is true only with the quality setting that uses half resolution | show the real render scale or drop it |
| ACCOUNT | `SCOPES NONE` | WRONG | always NONE, so owners (KYTTU, AYTTU) see no scopes; the real list is `adminPermissions` in the signed-in account (api.rs:213-236) | show the account's `admin_permissions` |
| ACCOUNT | `ADMIN` | WRONG | shown for everyone signed in, not only owners | show ADMIN OWNER / ADMIN / NOT ADMIN from the server's `adminOwner` flag |
| SERVER | `CHANNEL DEBUG` | STALE | `CHANNEL` is the old `debug` constant (hub.rs:42); updates now come from nightly | show the real update channel (nightly) |
| SERVER | `INSTANCE`, `SERVICE` | WRONG | these come from the vps2 backend (`vps2-godot-test`, `last-route-backend`), which is the Godot test instance the Rust web site shares; the line implies the Rust server | label it as the shared backend, or hide the instance name |
| SERVER | `ROOMS`, `WEBROOMS`, `RELAY` | OK | live from `/api/health` on the backend | none |
| UPDATES | MACOS / WINDOWS / LINUX | OK (fixed in e889006) | now from the nightly release on GitHub | none |
| UPDATES | PHONE / QUEST | OK | no nightly file, so NONE PUBLISHED | none |
| PUBLIC ROUTE | the whole line | OK | live from `/api/public/status` on the backend | none |
| SERVER NOTICES | `NONE THIS SESSION` | GAP | literal text; no notices are read from anywhere | read the server's notices, or hide the section |
| DEBUG TOGGLES | text | OK | static explanation | none |
| SKINS / MODERATION | `OWNERS ONLY` | GAP | shows the same line for everyone, not the real rights | show the real rights from the account |
| ROUTE TOOLS | text | OK | static explanation | none |

Summary: 4 WRONG or STALE items show false information (VERSION, CHANNEL, SCOPES, ADMIN), 2 items are GAPS (SERVER NOTICES, SKINS/MODERATION rights), and the rest are correct. The UPDATES and PUBLIC ROUTE data is fine; the build and account lines are the ones to fix first.

### Fixed (2026-11-07)

- THIS BUILD: shows `BUILD <CI number> (<commit>)` or `BUILD LOCAL (<commit>)` from the real build, plus the platform. The fixed "RUST PORT (#0)", "CHANNEL LOCAL", "HALF-RES" and "VERSION 0.0.1.0" text is gone.
- ACCOUNT: `NAME // OWNER|ADMIN|PLAYER // SCOPES ...` from the server's `adminOwner` and `adminPermissions`. Signed out reads `NOT SIGNED IN // NO ADMIN SCOPES`. Scopes the server did not report read `NOT REPORTED BY SERVER`.
- Still open from the audit: SERVER's `CHANNEL DEBUG` line, SERVER NOTICES, and SKINS / MODERATION rights.
