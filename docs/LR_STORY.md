# KYTTU - LRS pack format and commands

KYTTU - LRS plays a story pack: a folder of JSON and PNG files that the engine reads, merges and plays
in first person. The binary, the source folder (`src/bin/lr_story`), the pack id `lr_story` and the save keys are still named
`lr_story`; only the title changed.

## Layout

```text
<pack>/
  pack.json           manifest (required)
  scenes/*.json       lr.story.scenes/1: dialogue scenes
  items/*.json        lr.story.items/1: item definitions
  routines/*.json     lr.story.routines/1: NPC daily routines (see Life and weather)
  maps/*.json         lr.story.map/1: a map's name and legend
  maps/*.png          the floor picture (a whole multiple of the mask) and the mask (one pixel per tile)
```

The core pack defaults to `src/bin/last_route/data/lr_story` (`--pack DIR` or `LR_STORY_PACK` replaces it). Mods are read
from `mods/` beside the executable, in the working folder and per-user, plus `--mod DIR`, merged in id order.

## pack.json

```json
{ "schema": "lr.pack/1", "id": "my_story", "name": "My story", "version": "0.1.0" }
```

| Field | Rule |
| --- | --- |
| `schema` | Must be `lr.pack/1`. |
| `id` | Required. 3 to 32 characters: lower-case letters, digits and `_`, starting with a letter. The folder name must equal it. |
| `name`, `version`, `author`, `description` | Optional. `name` defaults to the id, `version` to `0.0.0`. |
| `overrides` | Optional list of `scenes/<id>`, `items/<id>`, `maps/<id>` or `start`. See below. |
| `start` | `{"map", "spawn"}`. Required in the core pack. A mod may set it only if `overrides` lists `start`. |
| `start_scene` | Optional scene id that runs before play starts. |
| `story` | Optional block that makes the pack a playable story. See below. |

## Stories

A pack with a `story` block is one playable story. The launcher lists the stories sorted by `order`, then `title`.

```json
{
  "schema": "lr.pack/1",
  "id": "simulora",
  "story": {
    "title": "Simulora: Quiet Side",
    "era": "simulora",
    "order": 30,
    "player": { "name": "Linda", "role": "Reader", "speed": 1.0, "start_items": [], "start_flags": [] }
  },
  "start": { "map": "simulora_reading_room", "spawn": "start" }
}
```

| Field | Rule |
| --- | --- |
| `title` | Shown in the menu. |
| `era` | One of `sample`, `past`, `lastroute`, `simulora`, `future`. |
| `order` | Number used for sorting in the menu. |
| `player.name`, `role` | Shown as the speaker "You" in dialogue, and in the HUD. |
| `player.speed` | Multiplies walk and sprint speed. Default 1.0. |
| `player.start_items`, `start_flags` | Given to the player when the story starts. Each item must exist. |

A story pack starts at its own `start`, so it needs no `"start"` override. A pack without a `story` block may set `start`
only through `overrides`. The core pack without a `story` block is the `sample` story.

## Ids and overrides

- A mod's scene, item and map ids must start with `<pack id>_`, so they never collide. The core pack has no prefix rule.
- A mod may replace an id an earlier pack defined only if it lists `"<kind>/<id>"` in `overrides`. A second definition
  without that entry is an error, and so is an override of an id no pack defines.
- **Pilot exception (in force):** a scene id may appear twice within one pack only when that id is listed in
  `overrides` and the later copy is in a file whose name contains `overrides`. The files are read in name order, so the
  override file must sort after the file it replaces. Any other repeat is still an error. This exists for the Simulora
  pilot's choice override (`scenes/pilot_overrides.json`).
- Scene targets, item ids, exits and spawn names are all checked at load.

## Scenes (`lr.story.scenes/1`)

```json
{ "schema": "lr.story.scenes/1", "scenes": { "my_story_intro": {
    "once": true,
    "lines": ["Plain text is narration.", { "speaker": "Felix", "text": "Hello." }],
    "choices": [{ "text": "Take the card", "if": { "not_flag": "took_card" },
                  "do": [{ "op": "set_flag", "flag": "took_card" }], "next": "my_story_after" }],
    "next": "my_story_after" } } }
```

- `lines`: a string, or `{speaker, text}`. No speaker means narration. `once`: run only the first time (default false).
- `choices`: offered once the lines are done. Each has `text`, an optional `if` (hidden when false), `do` and `next`.
  Picking one runs its `do` and starts its `next`; with no `next` the scene ends. While choices are offered, the
  scene's own `do` and `next` are not used.
- `do` and `next`: when no choice is offered, the scene's `do` runs and `next` starts. A run of scenes stops after 32
  in a row without the player acting.

Conditions (`if`, `cond`): `{"flag": "f"}`, `{"not_flag": "f"}`, `{"has": "item"}`, `{"not": C}`,
`{"all": [C, ...]}`, `{"any": [C, ...]}`.

Ops (`do`): `{"op": "set_flag", "flag": "f", "value": true}` (value defaults to true), `{"op": "give", "item": "id"}`,
`{"op": "take", "item": "id"}`, `{"op": "exit", "map": "id", "spawn": "name"}`, `{"op": "toast", "text": "..."}`, `{"op": "weather", "kind": "rain", "intensity": 0.7}` (sets the current map's weather; see Life and weather).

Flags are kept between runs in `lr_story_flags.json` in the user folder, under the core pack's id.

## Items and maps

`items/*.json`: `{"schema": "lr.story.items/1", "items": {"keycard": {"name": "Keycard", "description": "..."}}}`.

Optional item fields: `"stack"` (1 to 99, the most copies one slot holds; default 1, or 9 when the item has a `use`) and `"icon"` (a plain PNG file name in the pack's `items/icons/` folder, at most 64x64; checked at load; without one the hotbar draws a coloured square with the first letter). A key item has stack 1, so a second copy takes another slot.

**Hotbar and inventory.** The player has a 9-slot hotbar (small, bottom centre, fades after a few seconds idle; hidden during dialogue, pause and panels) plus a bag that catches overflow, so nothing is lost. `give`, item pick-ups and `start_items` add one copy; `take` removes one. Keys 1 to 9 select a slot (digits still pick choices while a choice is open), the mouse wheel scrolls the selection and F uses the selected item. In the inventory screen (I or Tab) move the cursor with W/S/A/D or the arrows, press Enter on a usable item to use it, press 1 to 9 to move the item under the cursor to that hotbar slot (swapping with what was there), and Esc, I or Tab to close.

`maps/*.json`: `{"schema": "lr.story.map/1", "id", "name", "art", "mask", "legend", "outdoor"?, "ceiling"?, "sky"?, "walls"?, "floor"?}`. `id` defaults to the file name and
`mask` to the art. Each legend key is a 6-digit hex colour (or `clear`); a mask colour with no entry is a load error.

| Kind | Fields |
| --- | --- |
| `floor` | none |
| `wall` | `height` (metres, 0 to 16, default 3), `color` |
| `spawn` | `name`: where the player can arrive; optional `facing`: "north", "east", "south", "west" or degrees clockwise from north (north is up the map picture). The story start faces south without it; an exit or a reset keeps the current facing without it |
| `door` | `if` (opens when true), `locked` (message), `height`, `color` |
| `item` | `item`: picked up once when the player steps on it |
| `npc` | `name`, `scene` (talk with E), `routine` (optional routine id, see Life and weather) |
| `trigger` | `scene`, `once` (default true): runs when the player steps on it |
| `exit` | `map`, `spawn`: moves the player to another map |
| `prop` | `prop` (model id), `yaw`, optional `use` (the player and NPCs can use it, see Life and weather) |

## Life and weather

### Usable props

A `prop` legend entry may carry `"use"`:

```json
"0000ff": { "kind": "prop", "prop": "my_bench", "yaw": 0,
            "use": { "action": "sit", "text": "Sit", "scene": "my_bench_scene", "seat_height": 0.45 } }
```

- `action` (required): `sit`, `lean`, `work`, `draw_water`, `read`, `sleep`, `warm_hands` or `look`.
- `text`: the prompt word, shown as "[E] Sit". Defaults to the action's own word (for example "Sit", "Draw water").
- `scene`: a scene id that runs when the player uses the prop (must exist).
- `seat_height`: metres, 0 to 2, for `sit` and `sleep` (default 0.45).

Controls: facing a usable prop shows "[E] <text>". E on a `sit` or `sleep` prop seats the body: the camera drops to
the seat height and any movement key stands the body up ("[Move] Stand up" is shown while seated). The other actions play
a short use (a small camera nudge) and toast the text. If the prop has a `scene`, it starts. E on an NPC within 2 m
in front of the body talks to the nearest one (by position, not tile); otherwise E uses the prop in the facing tile.

### NPC routines (`lr.story.routines/1`)

An `npc` legend entry takes `"routine": "<routine id>"`. Routines live in `routines/*.json` and follow the same id
prefix and `routines/<id>` override rules as scenes:

```json
{ "schema": "lr.story.routines/1", "routines": { "my_story_baker": {
    "loop": true,
    "steps": [
      { "go": "prop:13,10", "do": "work", "secs": 14 },
      { "go": "prop:17,10", "do": "sit", "secs": 10 },
      { "go": "well", "do": "chat", "with": "Tilda", "secs": 12 },
      { "do": "wander", "radius": 3, "secs": 14 } ],
    "shelter": "market_awning", "flee": "west_gate" } } }
```

Routine fields:

- `loop`: start over after the last step (default true). With false the NPC stays on its last step.
- `steps` (required, not empty): done in order.
- `shelter`: spawn name the NPC runs to in rain, snow or storm, then idles under it until the weather clears.
- `flee`: spawn name the NPC runs to while a battle is on the map. Without it the NPC cowers in place.

Step fields:

- `go`: where to walk first: a spawn name on the map, or `prop:<x>,<y>`, a tile that holds a prop. Without it the NPC
  does the step where it stands.
- `do`: the action for `secs` (default `idle`):

| `do` | Pose |
| --- | --- |
| `idle` | stands; looks around now and then, small gestures |
| `sit` | sits (hips down, knees forward) |
| `lean` | leans back on a surface |
| `work` | swings an arm like hammering |
| `draw_water` | pulls with both arms |
| `read` | arms forward, head down |
| `sleep` | lies down |
| `warm_hands` | arms forward to a fire |
| `look` | turns the head from side to side |
| `wave` | one arm up, waving |
| `chat` | talk gestures; needs `with` |
| `wander` | walks to random open tiles within `radius` |

- `secs`: seconds, above 0 (default 5). `with`: the NPC name to talk to (`chat` only). `radius`: metres, above 0 (`wander`, default 3).

Checks at load: `steps` not empty, `secs` above 0, every `go`, `shelter` and `flee` target exists on each map an NPC with
the routine stands on (a prop target must hold a prop), and `with` names an NPC on the same map.

Behaviour:

- NPCs with a routine walk tile paths at 1.4 m/s and turn smoothly. NPCs without one stand at their tile and idle.
- Greeting: within 2.5 m of the player an idle or walking NPC stops, turns to face the player and waves once; it resumes
  2 s after the player leaves.
- Talk lock: pressing E to talk locks the NPC in place until the scene ends.
- Rain, snow and storm send NPCs with a `shelter` to it. A battle on the map sends NPCs with a `flee` to it.
- Chat: two NPCs whose steps name each other go to their `go` targets and face each other with talk gestures while both are there.
- NPCs with a routine do not block tiles. The body and NPCs are pushed apart by a 0.35 m radius, so the player cannot
  walk into an NPC and NPCs step around the player.

### Weather

An outdoor map (`"outdoor": true`) may carry `"weather"`; on an indoor map it is a load error:

```json
"weather": { "kind": "clear", "intensity": 0.6, "every_secs": 90,
             "changes": [{ "kind": "clear", "weight": 3 }, { "kind": "rain", "weight": 2 }] }
```

- `kind`: `clear`, `rain`, `snow`, `fog`, `storm` (rain plus lightning flashes) or `leaves` (tumbling leaves).
- `intensity`: 0 to 1 (default 0.6).
- `changes`: kinds the weather may change to, each with a `weight` above 0. The choice is deterministic (seeded from the map id).
- `every_secs`: seconds between possible changes, above 0 (default 90). Changes blend smoothly.

Each outdoor map keeps its own weather state when the player leaves it. The op `{"op": "weather", "kind", "intensity"?}`
sets the current map's weather (on an indoor map it does nothing). Outdoor maps show a small weather label under the map name.

## Stats, battles and quests

Fights are real time (action). There is no turn-based mode. The floor of every map is drawn as 1 m blocks (a tile is
2 m, so 2 by 2 blocks per tile) instead of one stretched picture.

**Player stats.** `story.player.stats` in `pack.json`: `{"max_hp": 30, "attack": 5, "defense": 2, "speed": 5}`. Every
field is optional (these are the defaults). The party (HP, max HP and the stats) starts from them. There is no XP and there are no levels.

**Item use.** An item may carry `"use": {"heal": N}` or `"use": {"damage": N}` (never both) and `"consumable": true|false`
(true by default when `use` is set, so the item is removed after use). Heal items restore HP. Damage items hurt the
enemy within 6 m of the player. Using works in and out of battle.

**battles/*.json** `{"schema": "lr.story.battles/1", "battles": {"<id>": {...}}}`. Fields:

| Field | Meaning |
| --- | --- |
| `name` | shown on the health bar (default: the id) |
| `boss` | true: big bar at the top centre; false: a small bar over the enemy |
| `max_hp`, `defense` | enemy HP (default 20) and defense (default 1) |
| `move_speed` | metres per second while chasing (default 3) |
| `skin` | a skin name from the pack's `skins/`; the enemy is drawn as the mannequin figure with it |
| `spawn` | required: a spawn name on the current map; the enemy appears there. Pick an open area with room to dodge |
| `phases` | HP percent marks, high to low, e.g. `[66, 33]`. Each mark passed raises the phase: +20% move speed, -15% cooldowns, and unlocks attacks with `min_phase` at or below it |
| `intro` | text shown when the fight starts |
| `win`, `lose` | optional scene ids started after the fight |
| `attacks` | at least one attack (below) |

Each attack: `name`, `kind`, `damage`, `range` (metres: the enemy only picks it when the player is this close),
`windup` (seconds of floor telegraph, above 0), `cooldown` (seconds), `weight` (likelihood among attacks in range and
off cooldown, at least 1), `min_phase` (0: from the start; at most the number of phase marks), `radius` (Slam area or Shot
size), `speed` (Shot and Charge metres per second) and `text` (shown when it starts). Kinds:

- `melee`: a 1.6 m arc in front of the enemy.
- `charge`: a lane toward where the player stood, then a dash along it.
- `shot`: a projectile fired at the player's position when the windup ends.
- `slam`: a circle at the player's position when the windup starts.

The floor shows each attack in red, filling as the hit approaches. Damage to the player is `max(1, damage - defense / 2)`.
After a hit the player has 0.6 s of invulnerability.

**quests/*.json** `{"schema": "lr.story.quests/1", "quests": {"<id>": {"title", "description", "show_if"?, "steps":
[{"text", "done_if": <condition>}]}}}`. A quest shows in the journal once `show_if` holds (always when omitted) or any step is
done. Steps tick off by their conditions (usually flags), so the quest follows the story.

**Ops.** `{"op": "battle", "battle": "<id>"}` starts a fight (it ends any talk), `{"op": "heal", "amount": N}` heals. There is no experience op (an old one is a load error). Battles and quests use the same `<pack id>_` prefix and `overrides`
(`battles/<id>`, `quests/<id>`) rules as scenes.

**During a fight** exits do not fire (toast "You can't leave now") and NPC talk is off; movement stays live. On a win the
flag `won_<battle id>` is set and the `win` scene runs. On a loss HP returns to full, the flag
`lost_<battle id>` is set, the player goes back to the start of the story and the `lose` scene runs.

**Controls.** Left mouse: attack (hits within 2 m, in a 100 degree arc in front). Right mouse held: block (60% less
damage, half speed). 1 to 9: select a hotbar slot (the old 1 to 4 battle hotkeys no longer use items; F does). Mouse wheel: scroll the hotbar. F: use the selected item, in and out of battle. J: journal. I or Tab: inventory (Enter uses, 1 to 9 moves an item to a hotbar slot). Escape: pause,
which pauses the fight.

**Saving.** The party (HP, max HP and stats) is saved per story under `"party"` in `lr_story_flags.json`; older files
without it still load, and old level and experience fields in a save are ignored.

## LORE choices (KyttusLab-LORE import)

`lore.rs` turns each LORE file into scenes. Each `Story` chapter becomes one scene, `<pack>_<file>_ch<N>`, chained with
`next`. A `???` speaker is narration. The text is copied as written; `Title` is not copied.

A chapter may add a `Choices:` list:

```yaml
    Chapter 1:
      Summary:
        - ???: The corridor splits.
      Choices:
        - Text: Go left
          Needs: [lantern]        # all listed flags must be set for the choice to show
          Sets: [went_left]       # flags set when it is picked
          Goto: Chapter 2         # optional; without it the choice goes to the next chapter
```

Map look fields (all optional):

| Field | Meaning |
| --- | --- |
| `outdoor` | `true`: no ceiling, a sky is drawn instead. Default `false` (an indoor map: it gets a ceiling, not a void). |
| `ceiling` | `{"height": metres (1 to 16), "color": "#rrggbb"}`. Indoor maps only (an outdoor map with one is an error). Height defaults to the tallest wall height in the legend, else 3. |
| `sky` | `{"top": "#rrggbb", "bottom": "#rrggbb"}`, a gradient. Needs `"outdoor": true`; defaults to a dusk gradient there. |
| `walls` | Wall texture: `lab` (default), `brick`, `concrete`, `plaster`, `timber` (lime-white plaster between dark oak beams) or `stone` (rough grey-brown ashlar). An indoor map with `timber` or `stone` walls gets a `beams` ceiling (dark planks, heavy cross beams). |
| `floor` | Floor texture: `blocks` (default, 1 m blocks), `asphalt`, `paving`, `tiles`, `cobble` (rounded uneven cobblestones) or `dirt` (packed earth with ruts, pebbles and straw). |

An unknown `walls` or `floor` name is a load error.

Flag names are lower-case letters, digits and `_`. `Goto` must name a chapter in the same file. A chapter with choices
has no `next` of its own, and the last chapter's choice without `Goto` ends the scene.

## Stories in this repo

Story packs imported from KyttusLab-LORE live in `mods/` at the repo root. That folder is gitignored and stays local.
Nothing under `mods/` is committed.

| Pack | Era | Player | Source |
| --- | --- | --- | --- |
| `lr_story` (core) | `sample` | none (the Dive Lab sample) | `src/bin/last_route/data/lr_story` |
| `past` | `past` | Owen, technician | LORE Book 0, `last-route-0-before-the-dive.yml` |
| `lastroute` | `lastroute` | Dia, test subject (X-101) | LORE Books 0-5 |
| `simulora` | `simulora` | Linda, reader | LORE side stories `the-unread-page` and `the-guarded-door` (pilot) |
| `future` | `future` | placeholder | none yet |
| `sample_greenhouse` | `sample` | Wren, gardener | sample pack for modders (key door, NPC skins, a planter prop) |
| `sample_lighthouse` | `sample` | Tamsin, keeper's apprentice | sample pack for modders (flag door, take ops, a lamp prop) |
| `sample_archive` | `sample` | Isa, cataloguer | sample pack for modders (all/any/not conditions, a locked nook, a shelf prop) |
| `sample_city` | `sample` | Jun, messenger | sample pack for modders (a medieval old town with a sky, timber and stone walls, cobbles and a dirt path, a barred smithy door, a stone smithy and a timber tavern, a cutpurse and a bandit captain boss in a dead-end alley) |

The four `sample_*` packs are not lore stories. Each has two or three maps with pixel-art floors, its own items, scenes, NPC skins and a prop model, and each is marked `"multiplayer": true`.
They are made by `node packaging/story-samples/make-samples.mjs`, which writes the PNGs and the `.bbmodel` files itself (node only, no dependencies) and rewrites those four folders in `mods/` on every run. Edit the script, not the generated files.
After a run, `lr_story check` should load them.

The other maps are placeholder rooms. The `simulora` pilot's choice pair lives in `mods/simulora/scenes/pilot_overrides.json`.
It uses the pilot exception under Ids and overrides and is not in the LORE text.

## Start menu and multiplayer

The start menu (`src/bin/lr_story/menu.rs`) opens with three choices: **Single player** (the story list, then the story),
**Multiplayer**, and **Quit**. W/S or the arrows move, Enter picks, Esc goes back (Esc on the title quits). The mouse
works too: hovering selects a row, a click picks it.

Multiplayer needs an identity on the story server:

- **Guest name**: `POST /api/guest/name`; the lease token the server returns proves it.
- **Sign in**: `POST /api/account/login` (the same routes as `lr_server`; the accounts are the story server's own, see below).

From there:

- **Create a room** lists the stories whose `story` block sets `"multiplayer": true`. A pack without that flag cannot host.
- **Join a room** takes a code. The menu first reads the room, checks that this computer has the story and that its files
  match the server's fingerprint (the same sha256 `lr_story pack` prints), and only then joins.
- **Lobby** shows the players (host, ready, offline), with Ready / Not ready, Pass the host (host only) and Leave. The
  lobby pings the server every two seconds, which also keeps the player's seat.

Playing the room together (shared world, votes, shared party save) is not built yet. When everyone is ready the lobby says
so and waits there.

The native client talks to `LR_STORY_SERVER` when that is set (for example `http://127.0.0.1:8307`), else to
`https://lastroutestory.games.kyttuslab.org`. The browser build uses the page's own origin, so the site's `/api/*` must
reach the story server (see below). Requests run on a worker thread natively (a dead server never freezes a frame) and
synchronously in the browser through the page's `lr_net` plugin (`packaging/story-web/index.html`).

## Story server (VPS 2)

`src/bin/lr_story_server` is the room server for multiplayer packs. It is a separate binary (build with
`--features server`) and copies the room layer of `lr_server`: accounts, guest names, the room API and the UDP relay.

| Setting | Default | Meaning |
| --- | --- | --- |
| `LR_STORY_HTTP_PORT` | 8307 | the HTTP API (`/api/...`, `GET /api/health`) |
| `LR_STORY_RELAY_PORT` | 1307 | the UDP relay (open in ufw) |
| `LR_STORY_HOST` | 0.0.0.0 | the address both listen on |
| `LR_STORY_DATA_DIR` | `.data/lr_story_server` | accounts and guest names |
| `LR_STORY_PACKS_DIR` | `mods` | the story packs (one folder per pack id) |
| `LR_STORY_TRUST_PROXY` | off | set on the server: Caddy is in front |

The server reads each pack itself: a client cannot claim a pack is multiplayer or pick its fingerprint. Rooms hold up to six
players, the host can pass the host, players ready up, vote on choices, and share one party save.

Deploy: `sh packaging/story-server/deploy-vps2.sh <linux binary> [mods folder]` (see the script header). It installs
`/opt/lr-story-server` (releases, the `mods` folder), the `lr-story` system user, `/etc/lr-story-server/lr-story-server.env`
(written once), the `lr-story-server` systemd unit, and the ufw rule for UDP 1307. Port 8307 is not opened to the world.

Caddy sends the site's API to the server, in the `lastroutestory.games.kyttuslab.org` block:

```text
lastroutestory.games.kyttuslab.org {
	root * /srv/lr-story-web
	handle /api/* {
		reverse_proxy 127.0.0.1:8307
	}
	handle {
		file_server {
			precompressed gzip
		}
	}
	encode gzip
	header Cache-Control "no-cache"
	header -Server
}
```

Accounts are the Last Route accounts. `LASTROUTE_ACCOUNT_UPSTREAM` (the same setting as `lr_server`) names the account
website; on VPS 2 it is the Last Route backend on loopback, `http://127.0.0.1:8205`, which also serves
`lastrouterust.games.kyttuslab.org/api`. Sign-in, sessions, registration, password and rename are answered there, so a
player signs in to story rooms with their Last Route account. Plain `http` only, so keep it on loopback. Unset, the story
server keeps its own `accounts.json`. Guest names stay on the story server.

## Browser build: load time

The browser build packs the game files (about 400: the pack folders and the assets) into one bundle, `vfs.bin`, with a
manifest that gives each file's offset (`packaging/story-web/assemble.mjs`). The page makes one request for the bundle
(`vfs.bin.gz` is what Caddy sends, and the browser decodes it), so loading no longer waits on hundreds of small requests.
The loading screen shows the download, then the unpacking. The bundle is about 12 MB, or 2.5 MB gzipped.

## Commands

Run from the repo root with `cargo run --bin lr_story -- <command>` (or the built binary). `--pack` and `--mod` apply
to every command.

```text
lr_story                              open the story menu (or the core story when there is no menu)
lr_story --story ID                   play one story directly, skipping the menu
lr_story --shot out.png [--scene ID]  save a few frames to out.png and exit
lr_story check                        load and validate (same as --check; status 1 on problems)
lr_story init <id> [parent]           write a starter mod to <parent or ./mods>/<id>, then check it
lr_story pack <dir>                   print the pack's id, version, file count, bytes and sha256
lr_story import-lore <files...> --id <id> [--into DIR]   LORE files to scenes (default DIR ./mods/<id>), then check
```

`check` prints one OK line when the pack loads, e.g. `OK: 'Dive Lab (sample pack)' - 2 map(s), 6 scene(s), 1 item(s)`.

`init` writes `pack.json`, `scenes/main.json` (one scene, `<id>_intro`, with one narration line) and an empty
`items/items.json`. It refuses an invalid id or an existing folder, then runs the check:

```text
$ lr_story init my_story
created mods\my_story
OK: 'Dive Lab (sample pack)' - 2 map(s), 6 scene(s), 1 item(s)
```

`pack` prints a fingerprint. The sha256 covers every file in sorted path order (`/` separators): for each file it hashes
the relative path, a NUL, the bytes and a NUL. Changing, adding or renaming any file changes it:

```text
$ lr_story pack mods/demo_mod
id: demo_mod
version: 0.1.0
files: 3
bytes: 316
sha256: 677e74e71c779a873a51bd41a09f36c2fc71bc065e3a90fe611b5a280fea5f8e
```

`import-lore` takes one or more `.yml` files and refuses an invalid id, suggesting a valid one:

```text
$ lr_story import-lore fork.yml --id my_fork
imported 2 scene(s) into mods\my_fork
```

## Settings, controls and skins

- Settings open from the main menu (SETTINGS) and from the pause menu. Tabs: Controls, Skins, Settings (click, Left/Right on the tab row, or PageUp/PageDown). Every change saves at once.
- Settings file: `lr_story_settings.json` in the user folder beside the flags file (schema `lr.story.settings/1`); in the browser it is the localStorage key `lr_story_settings`. Unknown or out-of-range values clamp or fall back to defaults; a failed save is ignored.
- Values: mouse sensitivity 0.25-3.0, invert Y, FOV 60-100, UI scale 0.8-1.3, flat textures (applies on the next story start unless `--flat-textures` is given), name tags, hotbar fade, player skin, key bindings.
- Controls: each action (Forward, Back, Left, Right, Sprint, Crouch, Jump, Interact, Use item, Inventory, Journal, View) has one rebindable key. Press Enter or click a row, then press the new key; Escape cancels. Binding a key already used by another action swaps the two. "Reset to defaults" restores W S A D, LeftShift, LeftControl, Space, E, F, I, J, V.
- Fixed keys: arrow keys also move, Tab also opens the inventory, Enter also interacts, Esc pauses, 1-9 pick slots, and the mouse buttons and wheel are fixed. Digits are not bindable.
- Skins: the Skins tab lists "mannequin" first, then every skin of the loaded packs by id; the chosen one is framed and used for the player's third-person figure.
