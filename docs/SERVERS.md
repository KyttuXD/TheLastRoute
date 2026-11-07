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

## Open

- What `vps1` is for.
- Whether `lr_server` should sit behind Caddy, and the account forwarding variable `LASTROUTE_ACCOUNT_UPSTREAM` (TODO.md).
