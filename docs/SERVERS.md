# Servers: which VPS does what

Checked on the machines themselves on 2026-11-04 (read-only: service lists, Caddyfile, directory names). SSH aliases are in
`~/.ssh/config` (`vps1`, `vps2`, `vps3`, user `root`, key files per host). No keys, tokens or passwords belong in this file.

| Alias | Address | Role |
|---|---|---|
| `vps1` | 147.189.174.63 | Not recorded anywhere in the repo. Fill in. |
| `vps2` | 195.62.47.178 | Static files over Caddy: the web build and the signed native builds (`lastrouterust.games.kyttuslab.org`), mods and skins. |
| `vps3` | 194.156.90.219 | Game servers (Rust `lr_server` and the Godot live server) and the update downloads. |

## VPS 3: game servers and updates

- `lastroute-rust.service`: the Rust port's `lr_server` (HTTP API on port 8105, UDP gameplay relay on port 1105).
  Binary: `/opt/lastroute-rust/current/lr_server` (the previous one is kept in `/opt/lastroute-rust/previous`).
  Data: `/var/lib/lastroute-rust/` (`saves`, `crash-reports`, `updates`, `update-manifest.json`, `moderation.json`,
  `reserved_rooms.json`, `guest_names.json`; the running server also holds `accounts.json.lock`, which stops a second
  `lr_server` on the same data and the offline password reset, see "Lost password / admin reset"). Secrets: `/etc/lastroute-rust/lastroute-rust.env` (root only, never printed or committed).
- `last-route.service`: "LastRoute Live Server", the Godot version's server, still running beside it.
- Updates (the old route): the game's updater downloads builds from `/var/lib/lastroute-rust/updates`, described by `update-manifest.json`.
  Since the launcher ([LAUNCHER.md](LAUNCHER.md)) the in-game updater asks the website on vps2 first and uses this route only as the
  fallback. CI publishes with `packaging/publish-vps.sh` (upload user `nightly-upload`), and local builds publish with
  `packaging/local/publish-local.sh publish` (host `vps3` by default).
- Deploying `lr_server`: stop the service, replace `/opt/lastroute-rust/current/lr_server`, check the SHA-256, start it. See TODO.md.

## Lost password / admin reset

Accounts live in `/var/lib/lastroute-rust/accounts.json` (an `{username, salt, hash}` record per account; scrypt, so a
password can only be replaced, never read back). `lr_server` keeps the whole file in memory and rewrites it on every change.
That decides how a reset has to work: a hand-edit of the file while the service runs is overwritten by the next save, and
the old password keeps working until then. There are three ways to reset a password, simplest first. No password is ever
logged, and none comes back in an API answer except the one temporary password a reset generated for you, once.

**1. Nobody can sign in as an admin: the offline command (shell access only).** (The subcommand and the lock exist only in an `lr_server` built from this change: until that binary is deployed, stopping the service first is what makes the file safe to edit, and the old binary has no `reset-password`; edit with the old node script instead.) Stop the service, run the command as the
service user, start the service:

```sh
systemctl stop lastroute-rust
sudo -u lastroute-rust /opt/lastroute-rust/current/lr_server reset-password --list --data-dir /var/lib/lastroute-rust
sudo -u lastroute-rust /opt/lastroute-rust/current/lr_server reset-password AYTTU --data-dir /var/lib/lastroute-rust
systemctl start lastroute-rust
```

- The second command prints a one-time temporary password (16 characters from an alphabet without `0 O 1 I`, like
  `K7QF-2MXD-H9TA-WP4C`) on **stdout, once**; everything else it says goes to stderr. Capture it with
  `PW=$(sudo -u lastroute-rust ... reset-password AYTTU --data-dir ...)` or just read it off the screen, give it to the
  player, and tell them to change it after signing in. Clear the terminal scrollback afterwards.
- To choose the password instead: `--password-stdin` (reads the first line of standard input; keeps it out of the process
  list and shell history) or `--password NEWPASS` (6 to 200 characters, the same rule as register; visible in `ps` and in
  the shell history, so only for a throwaway). Neither is echoed back.
- `USERNAME` is any case and is stored in UPPERCASE. The account must exist: this command never creates accounts. Use
  `--list` (names only, no hashes) to find the exact name.
- It writes `accounts.json.<UTC time>.bak` (a copy of the old file, owner-only mode `0600`) next to the file, then replaces the file
  atomically (temp file, then rename, same mode and owner), with the new salt and hash and **all of that account's
  sessions removed**. Every other account, session and field is left exactly as it was. Delete old `.bak` files when you
  are done: they hold the old hashes.
- **The server must be stopped.** It holds an exclusive lock on `accounts.json.lock` while it runs; the command refuses
  (exit status 2) while that lock is held. `--force` edits anyway, but the running server then still has the old password
  and sessions in memory and overwrites the change on its next save, so restart it at once. Run the command as
  `lastroute-rust`, not as root, so the new file keeps its owner (the command copies the owner when it can).
- Without `--data-dir` it resolves the file like the server does (`LASTROUTE_DATA_DIR` / `KYTTU_DATA_DIR`,
  `LASTROUTE_ACCOUNT_STORE` / `KYTTU_ACCOUNT_STORE_PATH`, else `./.data`), reading `.env` from the working folder. The
  live env file `/etc/lastroute-rust/lastroute-rust.env` is root-only, so pass `--data-dir` on the live server. With
  `LASTROUTE_ACCOUNT_UPSTREAM` set the website owns the accounts and `accounts.json` is not used: the command says so and
  stops (unless `--data-dir` is given).
- Exit status: 0 done, 1 failed (unknown account, weak password, no file, bad usage), 2 the server is running.

**2. An owner is signed in: the admin endpoint.** `POST /api/admin/account/reset-password`

```json
{ "sessionToken": "<owner session>", "username": "RUNNER", "newPassword": "optional, 6-200 characters" }
```

- Only a full admin owner (`KYTTU`, `AYTTU`; `adminOwner` in the session, not a limited admin, not the setup password)
  may call it. No or an ended session answers 401 `NO_SESSION`; a signed-in non-owner 403 `ADMIN_OWNER_REQUIRED`; an unknown
  account 404 `ACCOUNT_NOT_FOUND`; a bad name 400 `INVALID_USERNAME`; a weak `newPassword` 400 `WEAK_PASSWORD`; your own account
  400 `USE_PASSWORD_CHANGE` (use the normal password change; an owner can reset the other owner).
- Without `newPassword` the server generates a temporary one and answers
  `{"ok":true,"username":"RUNNER","generated":true,"sessionsRevoked":2,"temporaryPassword":"K7QF-2MXD-H9TA-WP4C"}`: this is
  the only time it exists in plain text. A password you typed is never sent back (`generated:false`).
- **Every session of that account ends** (it is signed out everywhere), and any room admin ticket it held stops working.
- It shares the login rate limit (`LASTROUTE_ACCOUNT_RATE_LIMIT`, 30 per minute per address) and writes one audit line,
  `account admin-reset-password at <time> user=<target> actor=<owner> outcome=<ok|CODE> from=<address>`, never the body.
- With `LASTROUTE_ACCOUNT_UPSTREAM` set the request is forwarded to the website like the other account routes.

**3. In the game.** Signed in as `KYTTU` or `AYTTU` (the server's `adminOwner` flag), open the admin menu in a run (key `0`),
go to **FIELDS**, type the account name in **RESET PASSWORD** and press Enter or the RESET PASSWORD button. A message box
shows the temporary password once with a **COPY** button (the clipboard) and CLOSE. The field and the button are not shown
to anyone else. It calls the endpoint above. See docs/ADMIN_MENU.md.

## VPS 2: the game's website, mods and skins (static files)

`lastrouterust.games.kyttuslab.org` serves `/srv/lr-rust-web` (checked read-only on 2026-11-06: `/api/*` goes to a Node backend, everything
else is static with `Cache-Control: no-cache`, ETags and byte ranges work):

- the web build (`index.html`, `last_route.wasm`, ..., `version.json` written last), deployed by `packaging/web/deploy-vps2.sh`;
- the signed native builds under `native/<platform>/` (`latest.json`, `builds/<N>/manifest.json`, `manifest.json.sig` and the
  compressed files), published by `packaging/publish-native-vps2.sh` and installed by the launcher. The web deploy never deletes
  them. Nothing was published there as of this writing. Layout, signing and publishing: [LAUNCHER.md](LAUNCHER.md). No Caddy change is needed
  for them (there is no `precompressed` option and the launcher sends `Accept-Encoding: identity`).

Caddy (`caddy.service`) also serves two more sites from `/etc/caddy/Caddyfile`:

- `mods.kyttuslab.org` serves `/srv/lr3d/mods` (cache one hour). Layout `<pack_id>/<version>/pack.json` plus files, and an
  `index.json` at the root written by `tools/mods/publish-index.mjs` (copy it over by hand after review).
  The server reads the public URL from `LASTROUTE_MODS_BASE` (see docs/MODDING_SPEC.md section 4.7).
- `skins.kyttuslab.org` serves `/srv/skins` (cache one year, `immutable`, CORS `*`, `nosniff`). Because of `immutable`, a skin file
  must never be replaced under the same name: publish a new name (a content hash).
  On 2026-11-04 the folder was empty. Nothing uploads there: `lr_server` stores uploaded skins itself
  (`<data dir>/skins/<hash>.png`, `GET /api/v1/skins/<hash>`), and the game asks `lr_server` first, then this host over https
  (`https://skins.kyttuslab.org/<hash>.png`, a copy someone puts there by hand), then the legacy `/api/skins/files/<hash>.png`
  route. Every answer is checked by the full shared skin check before it is used, so a wrong or hostile file in this folder
  cannot hurt a player. A skin file nobody holds any more is kept for `LASTROUTE_SKIN_GRACE_SECONDS` (default 604800, 7 days)
  before it is collected, so peers that still name an old id can fetch it. See docs/SKIN_SYNC.md.

Other things on VPS 2 (not part of this project's docs): `/srv/osm`, `/opt/lastroute-host`, `/opt/node`.

## Public route: no host token, but not open to everyone

The public codes (`PUBLC`, `CTYPR`; `is_public_code`) are hostless: a relay `HOST_REGISTER` on them carries no host token.
Reserved codes (even a reserved public code) keep the full token check. What replaces the token:

- **Vouching.** An address that passed `check_host_public` (the moderation ban and flag check) on an HTTP register is
  vouched for 5 minutes (`HOSTLESS_VOUCH_MS`); one that was refused is refused for the same time. The register check now
  runs before the "already registered" 409. A UDP hello cannot name an account, so the relay uses the address instead:
  a refused address gets `HOST AUTHORIZATION FAILED`, and only a vouched address may create a missing entry or become
  the relay host (the address that made a relay entry keeps that right). A vouched host's token heartbeats keep its vouch.
- **Rate limit.** At most 5 relay-created entries per source address per minute (`HOSTLESS_CREATE_LIMIT`).
- **No address from the datagram.** The UDP source is the relay socket, not a game address, so a relay-created entry has
  address `""` and port 0; guests reach it through the relay. It remembers the creating address (`relay_made`).
- **Keep-alive.** `HOST_ALIVE` on a public code counts only from the current relay host's endpoint. The HTTP heartbeat
  without a token is accepted only from the address that hosts the entry (its HTTP register, or the relay host that made
  it) and only refreshes the expiry; players, `sessionActive` and `daytime` need the token.
- **Release.** The relay host that made an entry may unregister it over HTTP without a token (same address). The entry is
  also forgotten when the relay host goes quiet (about 1 s after its last keep-alive) or its session closes.
- **Second registrant.** If a live host (fresh by the relay's timers) holds the code, a registrant from another address gets
  frame 12 `HOST_JOIN_AS_GUEST` and joins as a guest. The same address on a new port (a new NAT mapping, a restart) replaces
  the holder. An HTTP register from the address that made the entry replaces it too (no 409).

Tradeoff: the vouch is by address, so a flagged account behind a shared address that someone else vouched can still host;
anyone who can send from the holder's address can replace it. The route is a public room, so nothing private is exposed.


## Guest names

A runner who is not signed in plays under `GUEST_` and four digits (`GUEST_4829`). The client draws the number once (1000 to
9999), keeps it (`last_route_guest.cfg` on a desktop, the page's localStorage key `last_route.guest` on the web) and sends
it as its name in the room (`RegisterPeer` and the wire profile), so the lobby, the players list, the nametag, the map pin
and the join message all show the same name. Solo play still shows RUNNER, since nobody else sees it.

- **Same number twice in a room.** The room's host keeps the earlier guest's name and gives the later guest a free
  `GUEST_####`, sent back as a profile for that guest's own session; the guest keeps the new number for its next run.
  Signed-in names are never changed.
- **The server's lease is optional.** `POST /api/guest/name` still hands out `R-4823` style names (and `lr_server` also
  accepts `GUEST_4829` as `previous`), but the client stores that lease apart and never shows it. The Node backend on vps2
  keeps issuing `R-4823`, which therefore never reaches a screen.
- **No account may look like a guest.** `lr_server` refuses to register or rename an account to `GUEST_` followed only by
  digits or the hex letters A-F (`RESERVED_USERNAME`), and the client's CREATE ACCOUNT refuses it first. The Node backend
  has no such rule yet.
- **Older clients** send `RUNNER` (or `R-4823`); the host accepts both and leaves them alone.

## Public route: no player host (2026-11-06)

The public route (`PUBLC`) is meant to have no player host. The world (hunters, timers, meters) is simulated by a server
process and runners come and go freely: no owner, no kick, no route command, and a runner leaving never ends the room.
What the report "the first joining member kicks everyone out of a public room when they leave" was, and what is built:

**The cause.** Before this change the first runner to find no room (`GET /api/enet/rooms/PUBLC` answering 404) registered
it and was the host (`world_sync::join_public_room`): it simulated the hunters, held the route seed and carried the relay
link. Both relays close a room the moment its host goes: the UDP relay (`HOST_LEFT` to every guest, from `close_room`, and
`on_room_closed` makes `lr_server` forget the directory entry), and the WebSocket hub, Node `ws_relay.mjs` `detach`
(`{"t":"host","on":false}`, then close 4001 on every guest). The game then took `HostLeft` as "the room is closed" and sent
every guest to the title. The relay cannot be told to promote a guest: the UDP relay's `HOST_PROMOTE` is only started by the
host's own `release-host` HTTP call, and the Node WebSocket hub has no promotion at all. The public route never calls the
Node `/api/public/*` routes; its hostless state lives in the directory entry and the relay (the section above).
Reproduced by `net/world_sync/public_tests.rs` `reproduction_the_first_joiner_leaves_and_the_relay_closes_the_room_on_everyone`
(A first, B and C join, A leaves: B and C both get `HostLeft`; this stays true after the fix, because it is the relay).

**The design: a server process holds the room (preferred).** `last_route --dedicated-host PUBLC` (headless/dedicated.rs) is
now a public room when the code is the public code: it registers the code itself over the WebSocket relay (no spawn ticket,
no owner; `register_public`), refuses every owner command (the session answers `THE PUBLIC ROUTE HAS NO OWNER`; the
directory is never asked), never closes for being empty, takes the room again with a fresh token whenever its link is lost, and
waits (retrying every 2 s) while a runner holds the code. Its guests never become hosts: `Room` carries `dedicated: true`, and
`data/net/public_route.json` `client_host_fallback: false` turns the stand-in off for good.

**What the deployment needs (not done; nothing on any server was changed).**
1. A Linux build of `last_route` on vps2 beside the Node directory (the process talks to the hub with `ws://` on loopback).
   `/opt/lastroute-rust` does not exist on vps2 today; vps2 only has the Godot export for the private always-on room
   (`lastroute-room@AKIRI.service`).
2. A systemd unit on vps2, patterned on `lastroute-room@.service`, `Restart=always`, `RestartSec=5`, its own `StateDirectory`
   and `HOME` (the device id for the directory's moderation check is stored there):
   `ExecStart=/opt/lastroute-rust/current/last_route --headless -- --dedicated-host PUBLC --backend=http://127.0.0.1:8205 --ws-rooms`
   (`--backend` names the loopback directory, which is also the relay; `--seed TEXT` keeps one world across restarts;
   `--idle-minutes` is ignored for the public room). No host token and no region-agent change: the region agent only spawns hosts
   for hosted codes that hold a spawn ticket, and the public code never has one.
   The unit and an env example are in the repo (not installed): [`packaging/public-route/lastroute-public-route.service`](../packaging/public-route/lastroute-public-route.service),
   service user `lr-publicroute`; the numbered steps are in [DEPLOY_ROOM_HOST.md](DEPLOY_ROOM_HOST.md). The binary is the one from the room host deployment
   (same libraries, same commit rule, see "CREATE ROOM says BUILD MISMATCH").
3. After the unit has been seen holding `PUBLC` (`GET /api/enet/rooms/PUBLC` lists it), set `client_host_fallback` to `false`
   and ship a client build. Until then the stand-in below keeps the route working.
4. `lr_server` on vps3 has no WebSocket hub, so a server process can only be reached through the Node hub on vps2; the UDP
   relay path keeps the stand-in only.

**The stand-in (until it is deployed, and whenever no server process is up).** A runner that finds no room opens it, as before,
but holds it only as a stand-in: it has no host controls (`NetSession::host_controls`: KICK, END ROUTE, RESTART, the owner tab
and the lobby's difficulty row are gone; the PLAYERS list shows no HOST tag; the difficulty is the route's own, `STANDARD`).
When it leaves, the others hand the room over (`net/public_reform.rs`), with no change to either relay:
- every guest keeps playing; the hunters stand where the old host's last frame left them;
- from the host's last room message each guest knows its place in line (lowest session id first: the oldest joiner on the
  WebSocket relay, whose ids count up; on the UDP relay ids are random, which is as deterministic);
- the first in line looks the code up, finds none and registers it (after about 0.3 s, so the relay and directory have let go
  of the old host); the others look it up every second and join whoever holds it; a runner whose place in line has come
  (1.5 s per place) opens the room if it is still free, so a first in line that never came costs one step;
- two that open at the same moment cannot both win: only one registration of a code succeeds (HTTP 409 for the other,
  which then joins) and the UDP relay answers a second public host `HOST_JOIN_AS_GUEST`;
- a room the directory still lists after its host died (the 15 s entry TTL) never links; it is dropped after 6 s and the
  next try is made; a runner gives up after 30 s past its turn and goes to the title;
- when the host that left was a server process, the stand-in waits 45 s before it may open the room (a restart takes seconds);
- if nobody remains the room is gone and the next runner to arrive opens a fresh one;
- a runner that joined as the host left, before the host's room message told it who is in the room, goes second in line
  (it never saw the world, so it must not win the room from the runners playing in it);
- a stand-in host whose own relay link ends for good (a dropped connection, a second host replacing it) lets go of the dead
  room (unregistering it, so its heartbeat cannot keep a listing no one can join alive) and comes back through the same
  handover behind the guests it had. The browser build's `WsLink` and the native WebSocket link report this; the UDP
  `RelayLink` cannot tell yet, so a native stand-in on the UDP relay (`lr_server`) is not covered by this.

What the new host knows: its own game (the world is the route's seed, which every runner builds itself) and the hunters it last
saw (position, facing, state, level, stun, slow, disguise). What the old host's brains knew beyond that (paths, search memory,
timers) is not on the wire, so a hunter restarts from that place with a clean brain. The relays and pickups each runner took
are that runner's own, as they always were (see headless/dedicated.rs, Limits). The gap (about a second) shows as hunters
standing still and a notice; nobody is sent to the title.

**Verified by tests** (`cargo test --lib --bin last_route`): the first joiner holds the room only as a stand-in with no host
controls; a server-held room is never opened by a joiner; the relay closes the room on B and C when A leaves (reproduction);
the lowest id opens it again and the others rejoin it, once, with the seed and the Start the old host sent, nobody kicked; a
single remaining runner keeps the room and later runners join it; an empty room closes and the next arrival opens a fresh
one; two simultaneous openers produce one host; a dead listing is dropped and retried; a server process leaving does not
make a runner open the room in a hurry; owner commands are refused; the dedicated public room never idles out, refuses owner
commands, and survives runners coming and going; a guest promoted to host keeps and simulates the last frame's hunters.

**Needs a live deployment:** the real Node directory and WebSocket hub (409 on the second registration, 4001 on host leave, the
15 s entry TTL), the real UDP relay under `lr_server`, a browser (the web build opens its handover host over `WsLink`; the
native tests do not run that path, and `cargo check --target wasm32-unknown-unknown` only proves it builds), more than
three runners, and the systemd unit itself.

## CREATE ROOM says BUILD MISMATCH on the web build (found 2026-11-07)

**Update 2026-11-14:** on vps2 the Node patch and `LASTROUTE_RUST_BIN` were already in place (since 2026-11-07/08). The mismatch came from the installed host binary (build of 2026-11-08) drifting from the served web build. On 2026-11-14 the nightly Linux build 148 (commit d134eef, the commit of the live web build) was installed with `packaging/room-host/deploy-vps2.sh`; nothing was restarted. Whenever the wire changes, install the new nightly the same way. Status of each step: [DEPLOY_ROOM_HOST.md](DEPLOY_ROOM_HOST.md). Still open: the end-to-end CREATE ROOM check by KT, and the public route process (`PUBLC` is shared with the Godot site on the same directory).

**Report:** pressing CREATE ROOM (online) on https://lastrouterust.games.kyttuslab.org shows `BUILD MISMATCH // HOST ... YOU ...`
(or `HOST UNVERSIONED`) and the room is not entered.

**Cause (evidence, read-only on vps2):** the room is not hosted by the client. `POST /api/rooms/hosted/create` makes the Node
directory (`/opt/lastroute-godot/current/backend/server.mjs`, port 8205, the `lastroute-godot-test` unit) queue a spawn; the region
agent (`lastroute-region-agent`, `region_agent.mjs`) starts `LASTROUTE_GODOT_BIN` from `/etc/lastroute-agent/agent.env`, which is
`/opt/lastroute-room/current/LastRoute-Dedicated.x86_64`: the **Godot** dedicated host. That host registers the room with the Godot
build's own `NetProtocol` fingerprint. The directory stores whatever 8-hex `protocol` the registrant sends (server.mjs, `register`),
and the Rust client compares it with its own `net::protocol::fingerprint()` before joining (`net/world_sync.rs` `join_found`), so it
refuses a room that was made for it. Live on 2026-11-07: the four rooms KYTTU created at 02:21 to 02:25 (`X6XX4`, `P4XZQ`, `M7AEZ`,
`P9ZJW`) are listed `{"dedicated":true,"region":"vps2","owner":"KYTTU","transport":"ws","protocol":"8645f524"}`, the directory log
shows `hosted/create 202`, two `GET /api/enet/rooms/CODE 404`, the host's `register 201`, then `GET ... 200` and **no** WebSocket guest
join (the client stopped there), and the agent log shows each spawn of the Godot export. The Rust fingerprint is a different value
(`09923496` at the time of writing; derived from the RPC table, see below), by design: the Godot host speaks Godot's SceneMultiplayer
RPCs and the Rust client could not talk to it even if the check passed.

It is not (checked and ruled out): the directory rewriting the field (it stores the registrant's value); an empty or wrong value from
the web build (`RegisterRoom::default()` sends `fingerprint()` on every target, and the fingerprint is a pure function of the RPC table
and `RUST_WIRE_VERSION`: no build id, no environment, no target, so a cached older web page differs from a newer host only after a wire
change, and the creator is a *guest* of the server's room, never its host); a case or format difference (lower-case hex both sides; the
client now also ignores case).

The always-on private room AKIRI (`lastroute-room@AKIRI`, the same Godot binary) is Godot's too, so a Rust client joining it gets the
same refusal. The Rust host cannot take a reserved code yet (`headless/dedicated.rs` registers only a spawn-ticket code or `PUBLC`).

**What the client does now** (`net/api.rs` `room_mismatch`, `app/mod.rs`, `net/hosted.rs`): for a room a server process hosts the
refusal reads `BUILD MISMATCH // SERVER ROOM HOST 8645F524 YOU 09923496 // UPDATE THE GAME OR TELL THE SERVER OWNER` (a player's own
room keeps the plain `HOST x YOU y // UPDATE`); the wait stops at the first check (it was never retried); CREATE ROOM goes back to the
CREATE ROOM dialog with the line written in it instead of a 4 second flash on the title; and the create request now says
`"build": "rust"` so a directory that can run both kinds of host can pick the right one. A directory that ignores the field behaves as before.

**What the server needs (not done; nothing on any server was changed).** The one directory on vps2 also serves the Godot web build
(`lastroutegodot.games.kyttuslab.org`, same port 8205, same agent), so pointing `LASTROUTE_GODOT_BIN` at the Rust binary would fix this
site and break that one. The patch [patches/node-hosted-build.patch](patches/node-hosted-build.patch) (against
`/opt/lastroute-godot/current/backend`, with tests) lets one agent run both. It applies with `patch -p1` (the first `region_agent.mjs` hunk needs fuzz 1) to the Godot repo's
`backend/`, the source of the live copy; it does **not** apply with `git apply`, and this repo's own `backend/` already contains it. Use
[`packaging/room-host/apply-node-patch.sh`](../packaging/room-host/apply-node-patch.sh) (a dry run on a copy, tests on the copy, a swap script that is
written but not run). The live copy was last read 2026-11-07 and may have drifted:

1. `server.mjs` passes `build: "rust"` from the create body on to `regions.requestSpawn`; `regions.mjs` keeps it (only the exact value
   `rust`) in the queued spawn options; `region_agent.mjs` runs `LASTROUTE_RUST_BIN` for such a room (no `--path`, and `--difficulty 0`
   is passed, which the Godot branch drops, so a RELAXED room is RELAXED) and the Godot host for every other room. With
   `LASTROUTE_RUST_BIN` unset nothing changes.
2. Put a Linux x86_64 build of the Rust game on vps2, e.g. `/opt/lastroute-rust/current/last_route` (mode 755, readable by `lastroute-godot`;
   [`packaging/room-host/deploy-vps2.sh`](../packaging/room-host/deploy-vps2.sh), dry run by default). The binary is dynamically linked to
   `libasound.so.2` (plus libgcc_s, libm, libc) and the CI build (ubuntu-22.04) needs **glibc 2.35 or newer**: check with `ldd` on vps2 first
   (read-only); installing `libasound2` (`libasound2t64` on Ubuntu 24.04) is a package change KT must approve. No build is needed on vps2: take the
   CI tarball `last-route-linux-x86_64.tar.gz` from the nightly release on KT's PC (the repo's token stays off vps2) and copy it over. The agent's command line (`--headless -- --dedicated-host CODE --port P --idle-minutes M --ws-rooms
   [--seed S] [--difficulty D] [--backend=http://127.0.0.1:8205]`, ticket and region in the environment) is exactly what
   `headless/dedicated.rs` reads (test `the_godot_agents_command_line_is_understood`).
3. Apply and test (on a copy, with `patch -p1`, never `git apply`): `cd /opt/lastroute-godot/current && cp -a backend backend.new && (cd backend.new && ...)`, which `apply-node-patch.sh` does, then swap the directories;
   add `LASTROUTE_RUST_BIN=/opt/lastroute-rust/current/last_route` to `/etc/lastroute-agent/agent.env`, restart `lastroute-godot-test` and
   `lastroute-region-agent` (a directory restart drops every room and bumps `reloadId`: pick a quiet moment).
4. Check: sign in, CREATE ROOM, then `curl -H 'Accept: application/json' https://lastrouterust.games.kyttuslab.org/api/enet/rooms/CODE`
   must show `"dedicated":true` and the Rust client's `protocol` (the 8 digits in `YOU ...` of any mismatch line).

**Things that go with this deployment** (found by the 2026-11-13 audit):
- **Same commit as the web build.** The room host binary must be built from the same commit as the wasm that is served: the protocol
  fingerprint depends on the RPC table and `RUST_WIRE_VERSION`. After any wire change the next web deploy brings BUILD MISMATCH back until the
  host binary is replaced too. The binary cannot print its fingerprint yet; a `last_route --print-protocol` flag (a source change) is a follow-up
  so a script can compare it with a live room.
- **`client_host_fallback` cannot be flipped at run time on vps2.** The live backend is the Godot repo's and has no `/api/public/settings` route
  (this repo's `backend/` has `public_settings.mjs`; the patch does not add the route), so the client's settings fetch fails there and only the
  shipped `data/net/public_route.json` applies. Set it to `false` by a client data change (after `PUBLC` is seen held by the process) or by deploying
  this repo's backend as a separate instance (KT_BUG_LIST 11).
- **A merge to `main` restarts vps3.** `.github/workflows/deploy-server.yml` deploys `lr_server` on every push to `main` that touches `src/**`,
  `crates/**` or the Cargo files (the previous binary stays in `/opt/lastroute-rust/previous`; `lr-server-swap` rolls back a failed health check).
  Connected players on vps3 are dropped. Merge when KT is ready. The room host binary has no such auto-deploy.

Also seen: each refused create leaves a Godot host holding one of the region's 9 slots until it idles out (10 minutes; `free` was 5 of 9
with four of them), so repeated tries hit `NO_FREE_REGION` for a while. `lr_server` (vps3) has no `/api/rooms/hosted/create` at all
(TODO.md), so CREATE ROOM against it answers 404; the register/lookup logic it shares with Node is now pinned by tests
(`a_rooms_protocol_is_listed_as_the_host_sent_it...`, `a_spawned_dedicated_host_is_listed_dedicated...`).

**Tests:** `net::api::build_mismatch_round_trips` (register then resolve against an in-process directory, `net/test_directory.rs`: a
client's own room, a spawned Rust host, a Godot-fingerprint host, no/malformed protocol), `net::hosted::create_round_trip_tests` (create
request body, the creator's wait ends at the first check with the new line, a not-yet-listed room is waited for),
`headless::dedicated` `the_spawned_host_lists_its_room_with_this_builds_fingerprint_and_the_creator_can_join_it`,
`net::protocol` (`fingerprint_derivation_is_pinned_and_depends_on_nothing_else`: FNV-1a over `rust;wire<N>;<rpc>=<sender>/<channel>...`
pinned on a fixed table), and the Node tests inside the patch.

## Accounts, sessions and the rest of the lr_server API (2026-11-08)

All of this is `lr_server` (vps3); with `LASTROUTE_ACCOUNT_UPSTREAM` set the website answers the account routes instead and the
server only forwards them. Tunables are environment variables, read once at start.

| Setting | Default | What it does |
|---|---|---|
| `LASTROUTE_SESSION_TTL_HOURS` | 720 (30 days) | How long a session token lives. Sign-out removes the token at once, on every request that follows. |
| `LASTROUTE_AUTH_RATE_LIMIT` | 10 | Sign-in, sign-up, password change and account deletion per address per window. The 429 says `TOO MANY SIGN-IN ATTEMPTS. TRY AGAIN IN N SECONDS.` |
| `LASTROUTE_ACCOUNT_RATE_LIMIT` | 30 | The other account and admin routes, per address per window. |
| `LASTROUTE_API_RATE_LIMIT`, `LASTROUTE_MOD_RATE_LIMIT` | 600, 120 | Everything else, and `/api/mods/*`. |
| `LASTROUTE_RATE_LIMIT_WINDOW_SECONDS` | 60 | The window of all of the above. |

- **Password change** `POST /api/account/password {sessionToken, currentPassword, password}`: the current password is required (a stolen
  token alone cannot change it). Every other session of the account ends; the one that asked stays.
- **Account deletion** `POST /api/account/delete {sessionToken, currentPassword[, confirmToken]}`: the first call checks the password
  and answers a one-time `confirmToken` (valid 5 minutes); the second call with that token deletes the account, its sessions and its
  cloud saves. The built-in owner accounts (`KYTTU`, `AYTTU`) cannot be deleted this way. The client screens are TODO.md (account settings).
- **Saves** `GET`/`PUT /api/v1/saves/:gameId/:slot`: only the game and slot pairs listed in `ALLOWED_SAVES` (`saves/game_saves.rs`)
  exist; anything else is 404 `UNKNOWN_SAVE` before a session is even looked at. The website's own slot list was not available to
  compare, so these are the pairs the clients use today (`run`, `run-world-1`, `progress`).
- **Admin**: every `/api/admin/*` route is refused with 403 `ADMIN_OWNER_REQUIRED` unless the session behind the token is one of the
  built-in owners; the roster and password-reset routes also check inside the account service (401 without a session).
- **Crash reports** (owner only): `POST /api/admin/crash-reports {sessionToken[, since, limit]}` lists, `POST /api/admin/crash-reports/view
  {sessionToken, id}` reads one. Clients upload with `POST /api/v1/crash-reports` (rate limited per address and device).
- **Mod packs**: `GET /api/mods/packs`, `GET /api/mods/packs/<id>[?version=]`, and `GET /api/mods/packs/<id>/download[?version=]`, which
  answers 403 `PACK_BLOCKED` (with the owner's reason and no file list) for a blocked pack and 503 when no download base is set.
- **Migration**: an `accounts.json` written by the Node server loads as it is. Names become canonical UPPER CASE; a session stored
  in the old plaintext layout (`token`) still works and is rewritten as a hash when the file loads. Pinned by `accounts/account_tests.rs`
  and `accounts/testdata/node_accounts.json`.
- **Logs**: the audit line of an account event holds the time, the event, a cleaned account name, the outcome and the address, never
  the request body. `redact_log_line` (`server/support.rs`) scrubs passwords, tokens and keys from it once more. Log rotation on the
  VPS is not set up from this repo (see Open).
- **Body limits**: `POST_ROUTES` in `server/routes.rs` lists every POST route with its own limit; `server/route_tests.rs` fails when a
  route is added without an entry.

### Security review (2026-11-12)

The attack surface, the findings, every limit the server enforces and the setup advice (proxy, firewall, secrets,
backups) are in [SERVER_SECURITY.md](SERVER_SECURITY.md). New variables from that pass: `LASTROUTE_BODY_TIMEOUT_SECONDS`
(15; how long a request body may take to arrive) and `LASTROUTE_LOGIN_FAILURE_LIMIT` (30; wrong sign-ins per account
per 5 minutes, from all addresses together).

### Client address, room kicks and room depth (2026-11-08)

**Client address** (rate limits, account audit lines, moderation, the ENet room address): `requester_address` in
`server/support.rs`, mirrored by `requesterAddress` in `backend/server.mjs`.

| Setting | Default | What it does |
|---|---|---|
| `TRUST_PROXY` | off | Treat every peer as a trusted proxy. Only for a proxy on another machine with the port firewalled to it. |
| `LASTROUTE_TRUSTED_PROXY_HOPS` | 1 (1 to 8) | How many trusted proxies sit in a row in front of the server. |

- A loopback peer (a proxy on the same machine, such as Caddy with `reverse_proxy 127.0.0.1:8105`) is always treated as a
  trusted proxy; a remote caller cannot connect from loopback. Any other peer, with `TRUST_PROXY` off, gets its own
  address and its `X-Forwarded-For` is ignored.
- Caddy (and the Node `/godot2-api` forwarder) **appends** the address it saw to whatever `X-Forwarded-For` the client
  sent. So the server reads the header from the right: the client is the entry `LASTROUTE_TRUSTED_PROXY_HOPS` places from
  the right (`spoofed, 203.0.113.9` from Caddy gives `203.0.113.9`). A header shorter than that was written by
  proxies only, so its leftmost entry is used. An empty header, or a chosen entry that is not a plain IP address, falls
  back to the peer address. Before this change the leftmost entry was used, which let a client behind the proxy pick
  its own address (and so its own rate-limit bucket).
- Set the hop count to the number of proxies that each append one entry. With a CDN in front of Caddy that is 2; a
  proxy that **replaces** the header instead of appending still works with 1.
- Whether `lr_server` on vps3 runs behind Caddy is still open (Open below); a server reached directly is not affected,
  because only loopback or `TRUST_PROXY` peers are read at all. `lr_story_server` already reads the rightmost entry.

**Room KICK / BAN** (`POST /api/rooms/moderate {room, playerId, sessionToken, action, targetId | targetName[, reason]}`):
only the room's host (its own room session token) can call it. The kick log and review entry go to the removed
player's **own account**, the one the server verified from that player's `accountToken` (or account `sessionToken`)
when they joined the room (`Player::account`). A `username` in the body is ignored, so a caller who mints a fresh room
code cannot get another account flagged; a guest player is removed but nothing is recorded. The host is counted as
`user:<account>` when the host joined signed in, so one account behind several fresh room codes is one host for the
"kicked by 2 different hosts" signal. A guest host (no account at join) can mint codes freely, so its kicks are logged
with host `guest` (`moderation::GUEST_HOST`) and open a plain `kick` review, but never count toward
`KICK_FLAG_THRESHOLD`. An owner account is protected by the account behind a player as well as by the name shown.

**ENet kick reports** (`POST /api/moderation/report {code, hostToken, username, ...}`, both servers): the server never
sees who is in an ENet room, so `username` is the host's word. A kick is recorded only when the room's host has an
identity the caller cannot reset: a host that registered signed in (`user:<account>`, one host however many codes it
registers) or a dedicated host the owner spawned (`dedicated:<code>`). A guest-hosted room's report records nothing
and answers `{ok, flagged: false, recorded: false, status}`, so registering fresh codes as a guest can no longer log
kicks against any account or open the "different hosts" review. `/api/moderation/peers` is unchanged.

**Room depth** (`POST /api/rooms/state` `claimDepth`): only the host's claim moves the room's `activeDepth` (a guest's
claim is ignored with no state change and no error, so older clients keep working). Once the room has a depth, one claim
may move it at most `MAX_DEPTH_CLAIM_STEP` = 3 floors (`rooms/room_service.rs`, same constant in
`backend/room_service.mjs`): the game's lift goes one floor at a time, and 3 leaves room for lifts taken between two state
posts. A room at depth 0 (new, or reset by a route start or abandon) takes the host's first claim at any depth up to
9 999, which is where a resumed run starts. A larger jump is ignored, so the items, the host's hunter snapshot and the
discovery are kept.

**Tests:** `server/tests.rs` `a_room_kick_is_recorded_against_the_removed_players_verified_account_never_a_body_username`,
`a_guest_hosted_enet_rooms_report_records_nothing_however_many_fresh_codes_it_registers`; `ops/moderation.rs`
`guest_host_kicks_are_logged_but_never_count_as_different_hosts`; `backend/moderation.test.mjs` "a guest-hosted ENet room's
report records nothing";
`the_client_address_is_the_entry_the_trusted_proxy_appended_and_a_direct_client_cannot_pick_one`; `rooms/room_tests.rs`
`a_kick_reports_the_accounts_the_server_verified_at_join_never_a_name_from_the_body`,
`only_the_host_advances_the_depth_and_a_jump_past_the_step_is_ignored`; `backend/room_service.test.mjs` and
`backend/server.test.mjs` (`forwardedClient`).

### Data directory: `.data`

`lr_server` keeps `accounts.json`, `saves/`, `skins/` and the public route in `.data` beside where it starts, unless
`LASTROUTE_DATA_DIR` (or `KYTTU_DATA_DIR`) names another folder. The Node server used `data/`. Decision for now: **keep `.data`**
(it is already what the code and the tests use, and it stays out of the repo listing). To move an existing Node install, point
`LASTROUTE_DATA_DIR` at its `data/` folder; nothing is copied or renamed automatically. This is KT's call and stays open (Open below).

### Health check

`scripts/health-check.sh [host] [http_port] [relay_port]` (defaults `127.0.0.1 8105 1105`; `HEALTH_HOST`, `HEALTH_HTTP_PORT`,
`HEALTH_RELAY_PORT` also work) asks `GET /api/health` and prints one line for HTTP and one for the ENet relay. The relay is UDP, so the
server's own report (`relay.listening`) decides, and `nc -u -z` is only a hint when installed. Exit code 0 = both fine, 1 = HTTP
down, 2 = HTTP fine but the relay is off. It sends and prints no secrets. It has not been run against any server from this repo's checks.

## Open

- What `vps1` is for.
- Whether `lr_server` should sit behind Caddy, and the account forwarding variable `LASTROUTE_ACCOUNT_UPSTREAM` (TODO.md).
  Behind Caddy on the same host the client address already comes out right (see "Client address" above).
- `POST /api/moderation/report` (ENet rooms) still takes `username` from a **signed-in** host, so one account can log
  kicks against any account (one host, so never the "different hosts" signal on its own; two accounts can reach it).
  The relay does not track which account each peer is (`EnetRoom` has only a player count), so the name cannot be
  checked against the room's peers yet. It needs the relay or the join to tell the server which account each peer is.
- Default data folder: `.data` (kept) or the old Node `data/`. Stays open for KT (see "Data directory" above).
- Log rotation for the `lr_server` service log on the VPS (journald or logrotate) is a server change and is not done from this repo.
- The website's slot list for `/api/v1/saves` could not be compared; `ALLOWED_SAVES` is the Rust side's list.
