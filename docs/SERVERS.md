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

## Open

- What `vps1` is for.
- Whether `lr_server` should sit behind Caddy, and the account forwarding variable `LASTROUTE_ACCOUNT_UPSTREAM` (TODO.md).
