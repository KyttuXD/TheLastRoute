# Profiles and several instances

A profile is one player's own copy of everything the game keeps on disk. Several people can share a computer, and
one person can keep several game accounts, and run them side by side.

## What a profile holds

Everything that goes through `paths::user_dir` / `paths::user_file` (`src/shared/paths.rs`):

- `settings.json` (settings, key bindings, the worn skin, the local name), `admin.cfg`
- `checkpoint.json` (the saved run) and `profile.json` (hunter dossier)
- `last_route_account.cfg` (account sign-in) and `last_route_guest.cfg` (guest name lease)
- `skins/`, `skins-cache/`, `skins-uploaded.json`, `mods/`, `screenshots/`, `logs/`
- the graphics start marker and, on macOS, the update records

Not per profile: the program itself, mods beside the program, the updater's marker beside the program, the
Night Watch save (its own program, always the root folder), and the browser build (no folders: no profiles).

## Where the folders are

The root user folder is `%APPDATA%\KYTTU\TheLastRoute` (Windows), `~/Library/Application Support/KYTTU/TheLastRoute`
(macOS) or `$XDG_DATA_HOME/KYTTU/TheLastRoute`, by default `~/.local/share/KYTTU/TheLastRoute` (Linux).

- `DEFAULT` is the root folder itself. Nothing was moved: existing players keep every file where it was.
- Any other profile is `profiles/<NAME>/` under the root (a space in the name is `+` in the folder name).
- `profiles.json` in the root lists the profiles (name, created, last used) and the one used last.
- `profiles/.trash/<NAME>-<time>/` holds deleted profiles. DELETE only moves the folder; move it back by hand
  and add the profile again to restore it. `profiles/.picker/` is the profile screen's own log folder.

Names are 1 to 16 characters from A-Z, 0-9, space, `-` and `_`, upper-cased.

## Choosing a profile

| How | What happens |
| --- | --- |
| `LR_PROFILE_DIR=<folder>` | That folder is the profile. Wins over everything; no index, no screen. |
| `--profile NAME` | Starts on NAME, creating it when new. No screen. |
| `--profiles` | Shows the profile screen. |
| nothing, one profile | Starts as before, with no extra screen. |
| nothing, several profiles | Shows the profile screen, the last used profile selected. |
| SHIFT held as the title appears | Goes to the profile screen (press it while the game loads). |
| SWITCH PROFILE on the title | Goes to the profile screen. |

The profile screen lists the profiles with when each was last used. Up, Down and Enter (or the mouse) start one;
NEW PROFILE (N) asks for a name; DELETE (the Delete key) asks first and refuses DEFAULT.

Switching restarts the program: the profile screen runs as a short process of its own and then starts the game
with `--profile NAME`. The log, the graphics choice, the settings, the account, the skins and the save are all
read once at start, so a restart is the simple and safe way to change all of them together.

The automation modes (`--headless`, `--test-suite`, `--bot`, `--agent-window`, `--self-test`, `--screenshot`,
`--stress`, `--capture`, `--validate-pack`, `--dedicated-host`) never show the screen and take no lock; they use
the root folder unless `--profile` or `LR_PROFILE_DIR` says otherwise.

`--profile=NAME` (with `=`) also works, and as before also names the account file inside the folder
`last_route_account_NAME.cfg`. Prefer `--profile NAME`.

## Two instances at once

Two instances on different profiles share no file. Start them like this:

    last_route --profile KYTTU
    last_route --profile "PLAYER 2"

A running game keeps `instance.lock` (process id, start time, heartbeat) in its profile folder and rewrites it
every 5 seconds. A second instance asked for the same profile shows PROFILE IN USE on the profile screen: pick
another profile, or CONTINUE ANYWAY (the two windows then share one save and one settings file; last write wins).
The lock is removed on a normal exit. After a crash it is stale once the process is gone (Linux) or 20 seconds
after the last heartbeat (every system). Both times are in `data/ui/profiles.json`, with every string of the screen.

## Code

- `src/bin/last_route/shared/profiles.rs`: names, index, folders, trash, lock (pure, with tests)
- `src/bin/last_route/app/instance.rs`: the choice at start, the lock heartbeat, the restart
- `src/bin/last_route/ui/profile_screen.rs`: the screen
