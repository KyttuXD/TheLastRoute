# Last Route Node backend

One dependency-free Node.js service provides both backend contracts used by
Last Route:

- the original web-style room/account API (`/api/rooms/*`, `/api/account/*`);
- the Godot ENet room-code directory (`/api/enet/rooms/*`);
- an authenticated UDP gameplay relay on UDP 1105 (separate from HTTP TCP 8105).

The room API stores live room/world state in memory. Accounts, their one JSON
progress slot, and SHA-256-hashed 30-day session tokens are persisted to disk;
passwords use scrypt records. Sessions therefore survive an automatic
Pterodactyl restart without writing a usable bearer token into `.data`.
Native ENet packets are wrapped through the UDP relay, so hosts and guests
make only outbound connections. Player routers do not need UPnP, a manual port
forward, or a publicly reachable UDP socket.

A room lives exactly as long as its host. An explicit quit (`/api/enet/rooms/unregister`) closes it at once; a host
whose process crashes or is killed is dropped within about a second: current clients send `HOST_ALIVE` every 250 ms
from a background thread (a frozen main loop keeps the room), and the relay times such a host out 1 s after the last
one (hosts from older builds keep the 15 s timeout). Either way every guest gets `HOST_LEFT` (and an `ERROR` for older
clients), the code stops resolving (`ROOM NOT FOUND`), and a reserved private room keeps its reservation. Host
migration happens only when a host hands over on purpose (`/api/enet/rooms/release-host`).

## Local run

Node 18 or newer is the only dependency.

```sh
# From the repository/server root:
npm test
npm start
curl http://127.0.0.1:1101/api/health
```

Normal game builds use only the primary service at
`http://lastroute.kyttuslab.org:8105`. TCP `8105` is reserved for the plain
HTTP Pterodactyl listener used by room discovery and updates, while UDP `1105`
remains the gameplay relay. Account/password actions intentionally remain unavailable over
this public HTTP origin. The game exposes no environment variable,
command-line override, custom directory, or direct-address lobby option.

## Pterodactyl deployment

Upload the repository contents to the Node.js egg. The Pterodactyl-friendly
startup file deliberately lives at the server root, while its implementation
stays in `backend/`. Either of these works from that root:

```sh
npm start
# or, for eggs with a configurable startup filename:
node index.js
```

The startup file automatically loads `backend/.env`, matching
`/home/container/backend/.env` on Pterodactyl. It also accepts a root `.env`
as a local fallback. Variables configured directly in the panel or process
environment take precedence, and both `.env` locations are ignored by Git.
Copy `backend/.env.example` to `backend/.env`, replace its placeholder secrets,
and restart the Node process after making changes.

Pterodactyl normally injects `SERVER_PORT`; common proxy setups inject `PORT`.
The server accepts either, with `PORT` taking precedence. Set a persistent
volume for `LASTROUTE_DATA_DIR` so account data survives reinstall/redeploy.
The repository default is the ignored `.data/` directory. A practical
configuration is:

```text
HOST=0.0.0.0
LASTROUTE_HTTP_PORT=8105
SERVER_PORT=8105
LASTROUTE_DATA_DIR=./.data
LASTROUTE_RELAY_ENABLED=true
LASTROUTE_RELAY_HOST=lastroute.kyttuslab.org
LASTROUTE_RELAY_PORT=1105
LASTROUTE_PUBLIC_DIR=./web
LASTROUTE_AUTO_UPDATE=true
LASTROUTE_GITHUB_REPO=KyttuXD/LastRouteGodot
LASTROUTE_GITHUB_TAG=debug
LASTROUTE_GITHUB_TOKEN=<private GitHub PAT>
LASTROUTE_UPDATE_POLL_SECONDS=15
# Source watching is automatic while the PAT above is configured.
LASTROUTE_GITHUB_BRANCH=main
LASTROUTE_SOURCE_POLL_SECONDS=30
LASTROUTE_UPDATE_MANIFEST=./updates/updates.debug.json
LASTROUTE_UPDATE_DIR=./updates
TRUST_PROXY=true
LASTROUTE_ADMIN_PASSWORD=<a long random secret>
LASTROUTE_RATE_LIMIT_WINDOW_SECONDS=60
LASTROUTE_API_RATE_LIMIT=600
LASTROUTE_ACCOUNT_RATE_LIMIT=30
```

The debug client permits this exact hardcoded public HTTP origin for room
discovery, update delivery, and account requests. Credentials and sessions are
therefore unencrypted; this risk is documented here rather than shown in the
game UI. This exception applies only to the hardcoded primary origin (plus
loopback development); arbitrary public HTTP origins remain blocked. Terminate
TLS in a trusted reverse proxy and update the hardcoded client origin when
possible.
Set `TRUST_PROXY=true` only when that trusted proxy overwrites
`X-Forwarded-For`; otherwise all proxied players share one rate-limit identity.
Do not enable it on a service exposed directly to untrusted clients. The Node
service enforces its own limits, and proxy/firewall limits are useful defense
in depth.

Allocate **TCP 8105** and **UDP 1105** to this Pterodactyl server and map both
through the firewall/container. TCP 8105 carries HTTP room codes and updates;
UDP 1105 carries the authenticated gameplay relay. Only the Pterodactyl
machine needs a public inbound UDP allocation. Player hosts never expose UDP
8421 and the client no longer contains a UPnP path.

The official client uses HTTP at `lastroute.kyttuslab.org:8105` and the relay
at `lastroute.kyttuslab.org:1105`, currently resolving to `195.62.47.178`.
Point that DNS name at the panel's public address and expose TCP 8105 plus UDP
1105.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `LASTROUTE_HTTP_PORT` | `8105` | preferred HTTP listen port; overrides panel variables |
| `PORT` / `SERVER_PORT` | `8105` (`1105` is automatically separated) | fallback HTTP listen port injected by the panel |
| `HOST` | `0.0.0.0` | HTTP bind address |
| `LASTROUTE_DATA_DIR` | `./.data` | persistent data directory |
| `LASTROUTE_ACCOUNT_STORE` | `<data>/accounts.json` | optional exact account-store path |
| `LASTROUTE_ADMIN_PASSWORD` | empty/disabled | enables `/api/dev/login` |
| `LASTROUTE_SAVES_DIR` | `<data>/saves` | per-account cloud saves (`<USERNAME>.json`) |
| `LASTROUTE_MOD_INDEX_STORE` | `<data>/mod_packs.json` | mod pack index (metadata only, see "Mod packs") |
| `LASTROUTE_MOD_DOWNLOAD_BASE` | empty | public URL of the VPS 2 mod folder, e.g. `https://<vps2>/lr3d/mods` |
| `LASTROUTE_MOD_RATE_LIMIT` | `120` | `/api/mods/*` requests per address per rate window (own bucket) |
| `LASTROUTE_MODERATION_STORE` | `moderation.json` beside the account store | flags, kick log, cooldowns, build allow-list |
| `LASTROUTE_MODERATION_SALT` | generated and persisted in the store | salt for the sha256 of device ids and IPs |
| `LASTROUTE_RELAY_ENABLED` | `true` | bind the authenticated UDP gameplay relay |
| `LASTROUTE_RELAY_HOST` | `lastroute.kyttuslab.org` | public relay address returned for room-code joins |
| `LASTROUTE_RELAY_PORT` | `1105` | UDP relay allocation |
| `LASTROUTE_PUBLIC_DIR` | empty/disabled | optional static web-client directory |
| `LASTROUTE_CUSTOM_MAP_DIR` | `./assets/custom_maps` when present | safe PNG-only custom-map directory exposed below `/api/custom-maps/` |
| `LASTROUTE_UPDATE_MANIFEST` | empty/disabled | update metadata JSON reloaded for update requests |
| `LASTROUTE_UPDATE_DIR` | empty/disabled | directory containing published update archives |
| `LASTROUTE_AUTO_UPDATE` | auto when a token exists | mirror the stable GitHub Release tag into the local update directory |
| `LASTROUTE_GITHUB_REPO` | `KyttuXD/LastRouteGodot` | GitHub owner/repository used for release and source checks |
| `LASTROUTE_GITHUB_TAG` | `debug` | stable `debug` or `release` asset channel to mirror |
| `LASTROUTE_GITHUB_TOKEN` | `GITHUB_TOKEN`, `GH_TOKEN`, or `ACCESS_TOKEN` | private server-side PAT; the named variable takes priority |
| `LASTROUTE_UPDATE_POLL_SECONDS` | `15` (minimum `10`) | poll interval for hot release-asset synchronization |
| `LASTROUTE_RESTART_ON_GITHUB_UPDATE` | auto when a token exists | request a Pterodactyl restart when the watched source branch advances; set `false` to disable |
| `LASTROUTE_GITHUB_BRANCH` | `main` | branch watched for backend source changes |
| `LASTROUTE_SOURCE_POLL_SECONDS` | `30` (minimum `15`) | source-commit check interval |
| `LASTROUTE_DEPLOYED_REVISION` | local `git rev-parse HEAD` | exact deployed SHA when the server directory has no `.git` folder |
| `TRUST_PROXY` | `false` | trust the first `X-Forwarded-For` address |
| `ENET_ROOM_TTL_SECONDS` | `60` | remove hosts that stop heartbeating |
| `LASTROUTE_RATE_LIMIT_WINDOW_SECONDS` | `60` | rolling API/account throttle window |
| `LASTROUTE_API_RATE_LIMIT` | `600` | general requests permitted per address/window |
| `LASTROUTE_ACCOUNT_RATE_LIMIT` | `30` | sensitive account/admin requests permitted per address/window |

The `KYTTU_DATA_DIR`, `KYTTU_ACCOUNT_STORE_PATH`, and `KYTTU_DEV_PASSWORD`
names from the original deployment remain accepted as compatibility aliases.

## API groups

- `GET /api/health`
- `GET /api/updates/:channel/:platform`
- `GET`, `HEAD /api/updates/files/:published-file`
- `POST /api/enet/rooms/register`, `heartbeat`, `unregister`
- `GET /api/enet/rooms/:code`
- `POST /api/enet/admin-tickets/issue`, `verify`
- `POST /api/rooms/join`, `state`, `settings`, `start`, `default-start`,
  `abandon`, `custom-map`, `dialog-start`, `dialog-advance`, `leave`
- `POST /api/account/register`, `login`, `session`, `password`, `rename`,
  `save`, `load`, `logout`
- `POST /api/guest/name` `{previous?, token?}` -> `{name, token}`: guest names in the 2D game's short style, one letter, a
  dash and four digits (`R-4821`; no I/O, no 0/1). Unique while leased (30 days, renewed by the holder's token, a clash is
  retried); stored in `guest_names.json` beside the accounts (`guest_names.mjs`). Rate limited like the account routes.
- `POST /api/dev/login`, `reload`
- `POST /api/moderation/integrity`, `report`, `status`, `peers`
- `POST /api/admin/flags`, `flags/set`, `builds/add`, `builds/remove` (full owner)
- `GET`, `PUT /api/v1/saves/:gameId/:slot` (header `x-session-token`): cloud saves. Only
  `last-route-godot/run` exists; `PUT {data, updatedAt}` with `data: null` deletes it, an
  older `updatedAt` than the stored one gets `409 STALE`, bodies over 64 KB get `413`, and
  each account may write 30 times a minute (`game_saves.mjs`).
- `GET`, `PUT /api/v1/skins/:account`: account skins (`skins.mjs`). `PUT` is the raw PNG (64x64 or the old 64x32,
  PNG only, at most 64 KB) with the owner's `x-session-token` and `x-skin-arms: auto|classic|slim`; another account
  gets `403`, 5 uploads per account per 10 minutes. `GET` is public: `{hash|"", arms, updatedAt, versions}`; the PNG
  comes from `GET /api/skins/files/<hash>.png`. The last 5 versions are kept. The older `POST /api/skins/upload`,
  `mine`, `clear`, `remove` routes still work. Files live under `<data>/skins` (`LASTROUTE_SKINS_DIR`).

`GET /api/health` reports the exact Git commit currently published for each
update channel under `updates.<channel>.revision`, together with its version
and platform list, plus whether the UDP relay is bound. The same short commit
is printed in the Pterodactyl console after every successful GitHub Release
sync.

## Accounts and release admin

Usernames are trimmed and canonicalized to uppercase before lookup. `Kyttu`,
`KytTU`, and `KYTTU` are therefore one account and cannot be registered as
separate names. Passwords remain case-sensitive.

The exact canonical usernames `KYTTU` and `AYTTU` are reserved full-owner
accounts. To create either one for the first time, set
`LASTROUTE_ADMIN_PASSWORD`, enter the owner name in the in-game account panel,
and enter that server secret in the additional **ADMIN SETUP KEY** field. The
account password is independent from the setup key. Similar names (for example
`KYTTU_FAN`) are ordinary users unless an owner explicitly adds that existing
account to the limited-admin roster.

Admin tools unlock only after the running client receives successful
primary-server authentication. Debug builds no longer grant the menu to every
player. `KYTTU` and `AYTTU` always receive all five scopes: `player`, `world`,
`hunters`, `visual`, and `inventory`. An owner can add or update an existing
account from the in-game admin menu with
`USERNAME:player,world,hunters,visual,inventory` (choose only the scopes that
account needs), or remove it by username. Delegated admins cannot view or edit
the roster, alter owner accounts, or exercise scopes absent from their room
proof. Cached local config never grants admin access. Account requests are
explicitly enabled on the hardcoded primary HTTP deployment and loopback
development origins. HTTP does not encrypt credentials or session tokens.

An authenticated admin guest requests an opaque admin ticket from
`/api/enet/admin-tickets/issue`. The ticket is valid for at most five minutes
and is bound to that room plus its current secret host token. Only that host
can call `/api/enet/admin-tickets/verify`; neither endpoint sends the guest's
account session to another player. The signed permission list is returned to
the host and is enforced again for every shared command. Room removal, host
replacement and expiry invalidate outstanding proofs. Owners can manage the
roster through `/api/admin/roster`, `/api/admin/roster/set`, and
`/api/admin/roster/remove`; all three require an owner session.

Signed-in procedural solo routes save at floor boundaries. The slot contains
the route seed/configuration, depth, score and coins; the
in-game account card can continue or clear it. Co-op state and authored custom
map payloads are intentionally excluded because their canonical state belongs
to the live host/map workflow rather than to one player's account.

## Pterodactyl-hosted updates

This supports the intended private-GitHub-to-Pterodactyl flow without putting
a GitHub token in the game:

1. Validation and exports are run locally before publishing. Use the complete
   `res://tests/scenes/validate.tscn` suite and `./tools/build/export_test_builds.sh`, then
   inspect the generated archives and manifest before uploading them. GitHub
   Actions is intentionally not configured for this repository.
2. Set `LASTROUTE_AUTO_UPDATE=true` and provide the server-side PAT as
   `LASTROUTE_GITHUB_TOKEN`; the common Pterodactyl `ACCESS_TOKEN` variable is
   also accepted. At startup and every `LASTROUTE_UPDATE_POLL_SECONDS`, Node
   reads the stable `debug` GitHub Release, downloads changed artifacts,
   verifies their manifest SHA-256 values, and atomically publishes the
   manifest last. When `LASTROUTE_PUBLIC_DIR` is set (the example uses
   `./web`), the verified Web ZIP is also safely unpacked there and becomes the
   page served by this same process. A revision marker prevents unchanged
   15-second polls from repeatedly decompressing or swapping the Web build.
   Existing files remain usable if GitHub is unavailable.
3. `./tools/build/export_test_builds.sh` remains the manual/local equivalent. It
   stamps the current Git commit into every
   export and writes the test archives plus a ready-made
   `builds/updates.debug.json` with their SHA-256 values. Point
   `LASTROUTE_UPDATE_MANIFEST` at that file, or copy
   `backend/updates.example.json` and set each channel/platform's dotted numeric
   version, Git `revision`, exact local `file`, SHA-256 checksum, notes and
   optional `required` flag. The platform keys are `linux`, `windows`,
   `windows_legacy`, `macos`, `web`, `steamvr` and `quest`. The
   `windows_legacy` artifact is the SSE2-compatible 32-bit build and must stay
   separate so an older computer is never upgraded to the SSE4.2 build.
4. The backend reloads the manifest on each update request, so a successful
   asset poll becomes available without restarting Node. It only serves local
   filenames referenced by a valid manifest.
5. Debug exports query the `debug` channel and release exports query `release`.
   Exported debug builds compare both version and Git revision, and keep PLAY
   locked until the check succeeds. If the revision changed, the client
   automatically downloads and installs the matching archive even while the
   version remains `0.0.1.0`.

Desktop clients SHA-256-check and unpack the archive under their private user
data folder. Required/debug updates stay on an in-progress screen from the
main menu through download, verify, and staging, then start an independent
helper; optional release updates wait for INSTALL & RESTART. The helper waits
for the game to exit, keeps a backup of the replaced files, installs the
staged build, and relaunches it. Web refreshes the hosted page with a
revision cache-buster.
Quest downloads and verifies its APK, then opens Android's package installer;
Android still requires the player to approve a sideloaded package replacement.
Existing clients from before automatic installation require one final manual
install of this bridge build. After that, newly published commits do not need a
version change or browser download.

## Pterodactyl source auto-restart

With the server-side GitHub PAT configured, the backend watches GitHub `main`
automatically. When the remote SHA differs from the deployed repository's
`git rev-parse HEAD`, the service closes its HTTP listener and exits with
status 75. Enable Pterodactyl's automatic crash restart and keep the existing
`git pull` in the startup command: the restart runs that pull, then launches
the current `index.js`. The watcher never runs `git pull` itself and never
rewrites source files while Node is executing them. Set
`LASTROUTE_RESTART_ON_GITHUB_UPDATE=false` only when a deployment must stay
pinned.

If the startup pull fails or Pterodactyl crash restart is disabled, this cannot
self-heal; the logs will show the local and remote short SHAs. Leave the option
off on a manually managed production release branch.

The generated files are ignored by Git and should be uploaded as GitHub
Release assets, not committed to the source tree. For example, after reviewing
the generated manifest and archives:

```sh
gh release create debug --prerelease \
  --title "Last Route debug 0.0.1.0" \
  builds/KyttusLab-*-Debug.zip builds/updates.debug.json
```

Keep the GitHub tags channel-based: use `debug` for debug uploads and
`release` for release uploads. The dotted game version belongs in the build,
manifest, and release title rather than the tag name.

The PAT belongs only in Pterodactyl's private variables or deployment script;
never write it into `updates.json`, the Godot project, an export, or Git.
TLS termination is still strongly recommended before shipping a secure release.
The current client opens the published archive/APK rather than attempting to
overwrite its own running executable. Steam, Meta Quest store builds and the
hosted Web build should eventually use their platform-native deployment
systems; a future signed desktop launcher can consume the same manifest and
verify the included SHA-256 before installation.

Run the real Godot integration smoke test against the deployed primary service:

```sh
godot4 --headless --path .. res://tests/scenes/backend_smoke.tscn
```

## Mod packs

The pack **index** lives here (VPS 3); the pack **files** live only on VPS 2. The index never serves pack files.

- `GET /api/mods/packs` lists every pack (newest version each): `id`, `name`, `version`, `author`, `description`,
  `gameVersion`, `overrides`, `sha256` (pack identity, below), `blocked`, `blockedReason`, `files` (`path`, `sha256`,
  `size`; `pack.json` included), `totalBytes`, `downloadBase`, `versions`. Clients must skip `blocked` packs and verify
  every downloaded file against its `sha256` before loading it.
- `GET /api/mods/packs/<id>[?version=<v>]` returns one pack (404 `PACK_NOT_FOUND`).
- Owner only (body `sessionToken` of an admin owner, as for `/api/admin/flags`):
  - `POST /api/admin/mods/register` with `{ pack, replace? }` (used by the deploy script). A version that already
    exists is refused with 409 `VERSION_EXISTS` unless `replace` is set.
  - `POST /api/admin/mods/block` with `{ id, reason? }` and `POST /api/admin/mods/unblock` with `{ id }`.
- Caps follow the client loader: 16 MB per file, 128 MB per version, 512 files. The index holds at most 2000 packs and
  keeps the 5 newest versions of each. Paths must follow the loader's layout: `pack.json`; `props|regions|hunters|grabs|sounds|ambience|floors|walls|ceilings|rooms/*.json`
  one level deep; `assets/**` `.glb` `.png` `.ogg` (lower case). Anything else is refused at deploy time.
  The register body may be up to 256 KB; every other route keeps the 64 KB cap. `/api/mods/*` has its own
  rate-limit bucket (`LASTROUTE_MOD_RATE_LIMIT`).

Layout on VPS 2: `<remote-root>/<pack_id>/<version>/<files>` (default remote root `/srv/lr3d/mods`), served at
`<LASTROUTE_MOD_DOWNLOAD_BASE>/<pack_id>/<version>/<path>`. A version folder is never changed in place: publish a new
version instead.

`pack.json` (`lr.pack/1`, the DATA lane's loader schema; see `docs/DATA_DRIVEN_CONTENT.md`):
- `schema: "lr.pack/1"` is required, and unknown keys refuse the pack.
- `id` is `[a-z0-9][a-z0-9_]{2,31}`, with no hyphen: content ids are `<pack_id>_<name>`.
- `version` is `[0-9A-Za-z.+-]`, at most 32 characters.
- `name` and `author` are required here.
- `description`, `game_version` and `overrides` (`"<kind>/<id>"`) are optional.
- `files` is optional; if present, it must list every file except `pack.json` with its exact sha256 and size.

Pack identity (the client's cache key and room-handshake value) is the sha256 of the `"<file sha256> <path>"` lines,
sorted by path and joined by `\n`, over every file including `pack.json`.

Deploying a pack (dry run unless `--apply`; the owner session comes only from the environment):

```bash
LASTROUTE_OWNER_SESSION=... node backend/deploy_mod_pack.mjs path/to/pack --vps2 <ssh-host> --index https://<vps3-host> --apply
```

The script hashes every file and enforces the caps. It copies the pack to `<id>/<version>.uploading-<pid>` on VPS 2
and re-hashes the files there with `sha256sum`. Only if every hash matches does it rename the folder into place and
register the pack in the index. A bad upload is deleted, and nothing is registered.

## Reserved private rooms

A reserved room is a room code the owner keeps (`backend/reserved_rooms.mjs`): it never expires, nobody else can
register it, and it is hidden -- `GET /api/enet/rooms/<code>` answers `403 ROOM_RESERVED`. Store and echo only:
the live game session on that code still belongs to whichever invited player hosts it.

- Codes: `LASTROUTE_RESERVED_ROOMS` (default `KATHY,AKIRI`; always present), or created by the owner. Reserved codes
  may use any five letters or digits (AKIRI has an I); generated room codes keep their alphabet (no I, O, 0, 1) and a
  non-reserved code outside it cannot be registered. Each is Stored with their
  stored with its route settings (seed, mode, scale, difficulty, daytime, mod ids), whitelist (account names,
  canonical uppercase) and an optional room
  password (scrypt hash only) in `$LASTROUTE_RESERVED_ROOMS_STORE` (default `<data dir>/reserved_rooms.json`), so
  they survive restarts and deploys.
- Hosting: `POST /api/enet/rooms/register` on a reserved code needs the owner, an invited account
  (`sessionToken`) or `roomPassword`; the answer carries `reserved.settings`. A host that stops heartbeating for
  30 s can be replaced by the next invited player.
- Joining: `POST /api/rooms/reserved/resolve { code, sessionToken?, password? }` answers `{ live, room, joinTicket,
  canHost, settings }`. The relay admits a reserved room's guest only when its join nonce is a live ticket (10 min).
  Direct (non-relay) connections are not gated; production always uses the relay.
- Owner (`POST /api/admin/reserved-rooms`, full admin owner): `action` `list` (with `live`, `players`), `create`,
  `update` (`settings`, `invites`, `password`; `""` clears it), `delete` (not for environment codes), `whitelist`
  (`op` `add`/`remove`/`list`, `name`), and `command` (only while someone hosts it: `409 ROOM_NOT_LIVE` otherwise; a
  new host starts with an empty queue) with `command`
  `restart` (`sameSeed`), `end`, `kick` (`target`), `settings`, `transfer`. Commands are queued (20 kept) and echoed
  to the host on its heartbeat (`reserved.commands`); the host carries them out and acks them (`commandAck`). The
  in-game admin menu has the same owner controls under PRIVATE ROOM (plus WHITELIST EVERYONE HERE / SHOW WHITELIST).
  Operators: `~/Developer/Claude-Setup/rooms.sh` (status, restart, end, whitelist) and the KT-FW13 "Server Rooms"
  launcher, which runs it on KT-Mini4-1 over SSH; it signs in from an owner session file or a hidden prompt and
  talks to the backend on VPS 3's loopback port over SSH.
- Later, not built: a server-hosted room would need a headless dedicated host process per reserved room (Godot
  `--headless` with a "server" role that simulates hunters and the floor without a local runner), a supervisor that
  restarts it on crash or deploy, a UDP port or relay session per room, its own resource budget on VPS 3, and
  RemotePlayers/RoomState paths that do not assume the host is also a player.

## Crash reports

The game's `CrashWatch` autoload (`game/autoload/crash_watch.gd`) notices a session that crashed, was killed or froze
(a sentinel file left in `user://logs`, a freeze watchdog) and sends one `lr.crash/1` report on the next launch, when
the player's "CRASH REPORTS" setting is on (default) and the run is a real game run (never headless, editor or
playtest bots, which leave reports queued for the test harness instead).

- `POST /api/v1/crash-reports` with the report. Store and echo only: `201 { ok, id, report }` returns exactly what was
  stored. Body cap 96 KB; `log_tail` 32 KB, 50 breadcrumbs, 50 recent errors, 400 chars a line, 120 a field. Anything
  credential-shaped (bearer/basic auth, `token=`/`password:`/`admin_ticket` values, 40+ hex digits) is replaced
  before storage, on the client and again here. The requester address is a rate-limit key only (6 reports an hour per
  address and device id) and is never stored.
- `POST /api/admin/crash-reports` with `{ sessionToken, since?, limit? }` (full admin owner): newest first, at most
  200; `since` is a report id.
- Storage: one JSON file per report in `$LASTROUTE_CRASH_REPORTS_DIR` (default `<data dir>/crash-reports`), newest
  5000 kept. Operators read them with `~/Developer/Claude-Setup/crash-reports.sh` (over SSH, no credentials).

## Moderation

The host client's anti-cheat kicks cheaters in-room; the server keeps the record in `moderation.mjs`. KT ruling: accounts
are never restricted automatically, only by an owner.
(store: `moderation.json` beside the accounts, atomic writes). Device ids and IPs are only stored as
`sha256(salt + value)`. Restrictions cover shared play only: joining the PUBLIC route
(`/api/public/join`), registering a PUBLIC ENet room (`/api/enet/rooms/register` with a public code), and
`/api/skins/upload`. Sign-in, solo play and private rooms are never blocked. Refusals are HTTP 403
`{ok:false, code:"FLAGGED", error, until, permanent, cooldown}`.

- **Review queue.** Every reported kick (`kick`; `kicks` once 2+ *distinct* hosts reported it within 7 days) and
  every unknown build hash (`integrity`, once per hash per account) opens a review entry
  `{at, reason, detail, open}` on the account. Nothing is restricted by it; built-in owners are never queued.
- **Owner actions only.** A flag (`extend`), `ban` and `troll_on` from `/api/admin/flags/set` are the only things
  that restrict an account or troll it server-side. Any action (including `dismiss`) closes the open reviews.
  A rename carries the record along. There are no automatic device/IP cooldowns (`cooldownUntil` stays null).
- **Known builds.** Owner allow-list, or the `sha256` of the published update manifest entry for that
  channel/platform; owner sessions on the `debug` channel are always known.

| Endpoint | Body | Response |
| --- | --- | --- |
| `/api/moderation/integrity` | `sessionToken?, deviceId?, commit, buildHash, platform, channel, context` | `{ok, known, status}` |
| `/api/moderation/report` | `code, hostToken, username?, deviceId?, reason, strikes, kind:"kick"` | `{ok, flagged, status}` |
| `/api/moderation/status` | `sessionToken?, deviceId?` | `{ok, status}` |
| `/api/moderation/peers` | `code, hostToken, usernames[≤16], deviceIds[≤16]` | `{ok, players:{NAME:{flagged,permanent,troll,reason}}, devices:{id:{cooldown,troll}}}` (players also carry `ads`) |
| `/api/admin/flags` | `sessionToken` | `{ok, accounts, reviews:[NAME with open reviews], builds}` |
| `/api/admin/flags/set` | `sessionToken, username, action (clear\|extend\|ban\|unban\|troll_on\|troll_off\|ads_on\|ads_off\|dismiss), hours?, note?` | `{ok, record}` |
| `/api/admin/builds/add` / `remove` | `sessionToken, buildHash, commit?, note?` | `{ok, build?, removed?, builds}` |

`status` is `{flagged, permanent, until, reason, troll, restrictions:{publicRoute, hostPublic, skinUpload},
cooldownUntil, message}`. Hosts should pass `sessionToken` (and `deviceId`) to `/api/enet/rooms/register` so
their kick reports count as a distinct host; anonymous rooms count per registered room instance.
