# KYTTU: The Last Route

A first-person survival horror game set in an endless facility. Every run starts fresh, and
the facility keeps going down: district by district, with stairwells to deeper levels and
hunters that grow more dangerous the more relays you recover.

This build contains the game's own files, for the private use of the person who owns the game,
on their own computer. Please don't share or publish it.

## How a run works

- **Start:** choose **CREATE ROOM** on the title screen to begin an Infinite route run.
- **Explore:** the facility is generated as you walk. Each district is different, and the
  map (**M**) fills in as you discover it.
- **Recover relays:** relays are the objective. Each relay you recover brings more hunters
  into the facility.
- **Survive:** hunters can see, hear and chase you. Hide in lockers, beds and wardrobes, and
  keep your light and stamina in mind.
- **Find safe zones:** a safe zone is a place the hunters can't take you. Reach one when you
  need to rest or regroup.
- **Go deeper:** stairwells lead down to deeper, more dangerous levels.
- **Save:** a run saves itself at each level. Open the account menu from the title screen's
  username button to continue a saved run or clear it.

## Controls

These are the default bindings. You can change them in the settings.

| Action | Key |
| --- | --- |
| Move | W A S D |
| Look | Mouse (I J K L and the arrow keys also work) |
| Sprint / crouch / hop | Shift (hold) / C (hold) / Space |
| Use held item | Left click |
| Interact (relays, pickups, lockers, lift, stairs, safe zone) | E or right click |
| Flashlight / taser | F / G |
| Hotbar | 1-5, mouse wheel |
| Hunter specialty bar (shows only while the hunter is on the floor) | 6 music box (Glyra), 7 heat/cold item (Wynelle, Solyth; press 7 again or click to flip), 8 and 9 reserved |
| Drop held item | Q |
| Map | M |
| Pause | Escape |
| Hide HUD / fullscreen | F1 / F11 |

## Saves and settings

Your settings, saved runs and account data are kept in your user folder:

- **Windows:** `%APPDATA%\KYTTU\TheLastRoute`
- **macOS:** `~/Library/Application Support/KYTTU/TheLastRoute`
- **Linux:** `~/.local/share/KYTTU/TheLastRoute`

Deleting that folder resets everything.

## Getting the game

- **Windows:** `last_route.exe`.
- **macOS:** `Kyttu's Lab.app`, one app for both Apple Silicon and Intel Macs.
- **Linux:** the `last-route-linux-x86_64` archive, with a binary, desktop entry and icon.

Builds are private for now. The game needs a few system fonts (Courier New Bold and Impact on
Windows and macOS). On Linux, if those aren't installed, Liberation Mono and Liberation Sans
Bold are used instead.

## Credits

KYTTU: The Last Route was made by Kyttu (KyttuXD, Kyttu's Lab). The full list of the files and
sources the game uses is in [CREDITS.md](CREDITS.md).

## For developers

Build steps, the headless and test modes, the code layout and the AI-agent protocol are in
[docs/DEVELOPING.md](docs/DEVELOPING.md).
