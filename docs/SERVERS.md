# Servers: which VPS does what

Checked on the machines themselves on 2026-11-04 (read-only: service lists, Caddyfile, directory names). SSH aliases are in
`~/.ssh/config` (`vps1`, `vps2`, `vps3`, user `root`, key files per host). No keys, tokens or passwords belong in this file.

| Alias | Address | Role |
|---|---|---|
| `vps1` | 147.189.174.63 | Not recorded anywhere in the repo. Fill in. |
| `vps2` | 195.62.47.178 | Static files over Caddy: mods and skins. |
| `vps3` | 194.156.90.219 | Game servers (Rust `lr_server` and the Godot live server) and the update downloads. |

## VPS 3: game servers and updates

- `lastroute-rust.service`: the Rust port's `lr_server` (HTTP API on port 8105, UDP gameplay relay on port 1105).
  Binary: `/opt/lastroute-rust/current/lr_server` (the previous one is kept in `/opt/lastroute-rust/previous`).
  Data: `/var/lib/lastroute-rust/` (`saves`, `crash-reports`, `updates`, `update-manifest.json`, `moderation.json`,
  `reserved_rooms.json`, `guest_names.json`). Secrets: `/etc/lastroute-rust/lastroute-rust.env` (root only, never printed or committed).
- `last-route.service`: "LastRoute Live Server", the Godot version's server, still running beside it.
- Updates: the game's updater downloads builds from `/var/lib/lastroute-rust/updates`, described by `update-manifest.json`.
  CI publishes with `packaging/publish-vps.sh` (upload user `nightly-upload`), and local builds publish with
  `packaging/local/publish-local.sh publish` (host `vps3` by default).
- Deploying `lr_server`: stop the service, replace `/opt/lastroute-rust/current/lr_server`, check the SHA-256, start it. See TODO.md.

## VPS 2: mods and skins (static files)

Caddy (`caddy.service`) serves two sites from `/etc/caddy/Caddyfile`:

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
- if nobody remains the room is gone and the next runner to arrive opens a fresh one.

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
