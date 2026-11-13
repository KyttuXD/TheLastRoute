# lr_server security review (2026-11-12)

Scope: `src/bin/lr_server/` (HTTP API on TCP 8105 through `tiny_http` 0.12, UDP gameplay relay on 1105). Deployment and
variables: [SERVERS.md](SERVERS.md). Skins: [SKIN_SYNC.md](SKIN_SYNC.md). Tests for this pass:
`server/security_tests.rs`, plus tests beside each changed module.

## Attack surface

Every POST body is JSON with its own size limit (`POST_ROUTES`, 64 KB unless noted). Secrets travel in the body or a
header, never in a cookie, so the open CORS policy (`*`) gives a web page nothing a script could not already do.

| Routes | Caller | Reads | Stores | Returns |
|---|---|---|---|---|
| `GET /api/health`, `/api/regions`, `/api/rooms/list`, `/api/public/status`, `/api/enet/rooms/<code>`, `/api/updates/<channel>/<platform>`, `/api/mods/packs*` | anyone | path, query | nothing | public status, room address or relay address, manifests |
| `GET /api/updates/files/<name>`, `/api/custom-maps/<name>.png`, static files | anyone (update files: `X-Download-Key` when set) | file name (pattern-checked, no `..`) | nothing | the file |
| `POST /api/account/register`, `login`, `session`, `logout`, `password`, `delete`, `rename`, `save`, `load` | anyone / session token | name, password, progress (32 KB) | `accounts.json` (scrypt N=2^14 r=8 p=1, sessions as sha256) | session token (192 random bits), progress |
| `POST /api/guest/name` | anyone | previous name, lease token | `guest_names.json` | name, lease token |
| `GET/PUT /api/v1/saves/<game>/<slot>` | session (`X-Session-Token`) | allow-listed game and slot, 64 KB JSON | `saves/<ACCOUNT>.json` | the caller's own save |
| `POST /api/v1/skins`, `/api/skins/upload`, `mine`, `clear`, `remove`; `GET /api/v1/skins/<id>` | session (remove: owner); GET anyone | 64 KB PNG, fully decoded and checked as 64x64 | `skins/<sha256>.png` | the id; the PNG with `nosniff` and a closed CSP |
| `POST /api/enet/rooms/register`, `heartbeat`, `unregister`, `release-host`, `admin-tickets/*` | anyone; later calls need the host token | code, port, flags | room directory (memory) | host token (once), room view |
| `POST /api/rooms/*` (web rooms) | anyone; later calls need the player's room token; host-only actions check the host | room code, player state, world snapshot | memory (512 rooms) | room state |
| `POST /api/public/join`, `state`, `leave` | anyone, moderation-checked | player state | public route store | route state |
| `POST /api/rooms/reserved/resolve` | invited account, owner, or room password | code, password | join ticket (memory) | relay address, ticket |
| `POST /api/moderation/*` | host token / session | reports, build hash, device id | `moderation.json` | status |
| `POST /api/v1/crash-reports` | anyone | 96 KB report, clipped and scrubbed | `crash-reports/*.json` (5000 kept) | the stored report |
| `POST /api/regions/report` | region agent (HMAC-SHA256, 60 s window, replay guard) | status | memory | queued actions |
| `POST /api/admin/*`, `/api/dev/*` | owner session (checked in one place for every `/api/admin/` path), setup password or dev token | varies | accounts, flags, mods, reserved rooms | admin data |
| UDP 1105 relay frames | host: room token (public route: vouched address); guest: nonce, or a join ticket for a reserved room | 14-byte header, payload up to 1458 bytes | sessions (memory) | forwarded datagrams, short errors |

## Findings

Severity is for a server reachable directly from the internet. "Fixed" is this pass.

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | high | A request that announced a huge `Content-Length` ended the process: when it was dropped, tiny_http allocated a buffer of the whole unread size (`server/mod.rs` `serve_one`). | Fixed: bodies over 8 MB are never read or dropped (`give_up_on`), bodies over 1 MB are answered 413 and drained on a helper thread. See open item A. |
| 2 | high | A sender that announced a body and then went silent held one of the 8 workers for good; 8 such connections stopped the API. | Fixed: bodies over 1 KB are read on a helper thread and the worker waits at most `LASTROUTE_BODY_TIMEOUT_SECONDS` (15 s). |
| 3 | high | A panic while the state lock was held poisoned it; `handle`, the relay thread and maintenance then panicked on every use. | Fixed: every lock recovers from poisoning, and workers, the relay handler and maintenance catch a panic and keep running. |
| 4 | medium | The room password of a reserved room was checked (scrypt, about 50 ms) under the global lock with only the general 600/min limit, so one caller could stall every request and guess passwords fast. | Fixed: 10 attempts per address and 60 per room per window, refused before the check. |
| 5 | medium | Sign-in was limited per address only, so one account could be guessed from many addresses. | Fixed: 30 wrong sign-ins per account per 5 minutes from all addresses (`LASTROUTE_LOGIN_FAILURE_LIMIT`), refused before the password check. Side effect: someone can keep an account's sign-in blocked; live sessions keep working. |
| 6 | medium | Limits were keyed by the full IPv6 address, so a caller with a /64 had a fresh allowance per address. | Fixed: IPv6 callers are counted by /64 (`rate_address`). |
| 7 | medium | One address could lease the whole guest-name space (about 108 000 names for 30 days), each grant rewriting the lease file under the lock. | Fixed: 20 new names per address per hour; renewals are free. The lease token is compared in constant time. |
| 8 | medium | Crash reports were limited per address and device id, and the device id is the sender's own word, so one address could push the 5000 kept reports out. | Fixed: 30 reports per address per hour whatever the device id. |
| 9 | medium | Any number of ENet room codes and all 512 web rooms could be taken from one address. | Fixed: 4096 codes in total, 32 per address (spawned dedicated hosts, reserved rooms and loopback are not counted per address); 20 new web rooms per address per window. |
| 10 | low | Every sign-in added a 30-day session and expired ones were only dropped when looked up, so `accounts.json` could grow without bound. | Fixed: 16 sessions per account (oldest ends), expired sessions are swept at each sign-in. |
| 11 | low | The limiter map and the relay's join-counter table grew with the number of distinct (or forged) addresses. | Fixed: 200 000 limiter keys and 50 000 join counters; a new key is refused while the table is full. |
| 12 | low | A crash-report write error was echoed to the client with the server's file path. | Fixed: a fixed message; the reason goes to the log. |
| 13 | low | `/api/regions/report` computed an HMAC over a body of up to 1 MB before any size check. | Fixed: the 64 KB limit comes first. |
| 14 | low | JSON and static replies had no `X-Content-Type-Options`. | Fixed: `nosniff` on every reply. |

Checked and found sound: tokens come from the OS random source (144 to 256 bits); host tokens, the setup password,
the download key and HMACs are compared in constant time; sign-in answers the same for an unknown name and a wrong password
(with a decoy scrypt); a password change and an admin reset end the account's other sessions; every `/api/admin/` route
is refused without an owner session; saves, skins, crash reports and update files cannot name a path outside their
folder; files are written to a temp name and renamed; `X-Forwarded-For` is read only from a loopback or `TRUST_PROXY`
peer, from the right; log lines are scrubbed of tokens and passwords; the `regex` crate has no backtracking; a guest
cannot send host frames on the relay (the source endpoint is checked per frame).

## Open

- **A. Giving up on a request leaks its connection.** tiny_http offers no way to close a connection whose announced
  body was not read. Each such request (finding 1, or a body while 1024 helper threads are busy) keeps one socket and
  one thread until the process ends. The count is logged, and at 256 the server stops on purpose so that systemd
  starts a clean one (before this pass one request was enough). Needed: a proxy in front that caps body size, or a
  listener of our own with socket timeouts.
- **B. Slow or huge request headers** (medium). tiny_http reads header lines with no size or time limit and uses one
  thread per connection. Needed: the proxy or firewall below.
- **C. One address can fill a relay room** (medium): 4 joins per 10 s per address against 15 guest slots that last
  45 s. A per-address cap would break several players behind one router; a join proof issued over HTTP would fix it
  (protocol change).
- **D. Relay error replies to a forged source address** are up to 3.5 times the request (low). Needed: reply only to
  addresses that completed a join or registration, or cap error replies per address.
- **E. Sign-up** is limited per address only (10/min by default); accounts never expire. Needed: an owner tool to list
  and remove unused accounts, or a proof of work.
- **F. scrypt parameters** are Node's defaults (N=2^14). Raising them needs a per-record parameter field and a rehash at
  sign-in.
- **G. Static files** are read whole into memory per request; a large update file served to many callers at once uses
  that much RAM each. Serve update files from Caddy instead.

## Limits now enforced

| Limit | Value | Where |
|---|---|---|
| Request body: hard cap / per route | 1 MB / 64 KB (mods 256 KB, crash 96 KB, skin 64 KB PNG, save 64 KB) | `RAW_BODY_HARD_CAP`, `POST_ROUTES` |
| Body arrival time | 15 s (`LASTROUTE_BODY_TIMEOUT_SECONDS`, 1 to 300) | `read_body` |
| Requests per address per 60 s | API 600, mods 120, account routes 30, sign-in and sign-up 10 | `LASTROUTE_*_RATE_LIMIT` |
| Wrong sign-ins per account | 30 per 5 min (`LASTROUTE_LOGIN_FAILURE_LIMIT`) | `LOGIN_FAILURE_WINDOW_MS` |
| Sessions per account | 16, each 30 days (`LASTROUTE_SESSION_TTL_HOURS`) | `MAX_SESSIONS_PER_ACCOUNT` |
| Reserved-room password attempts per 60 s | 10 per address, 60 per room | `ROOM_PASSWORD_ATTEMPTS_*` |
| New guest names | 20 per address per hour | `GUEST_NAMES_PER_ADDRESS` |
| Crash reports per hour | 6 per address and device, 30 per address; 5000 stored | `CRASH_LIMITS` |
| Skin uploads | 5 per account per 10 min | `UPLOAD_LIMIT` |
| ENet room codes | 4096 total, 32 per address | `MAX_ENET_ROOMS`, `ENET_ROOMS_PER_ADDRESS` |
| Web rooms | 512 total, 16 players each, 20 new per address per 60 s | `MAX_ROOMS`, `WEB_ROOMS_PER_ADDRESS` |
| Relay | 1472-byte datagrams, 15 guests per room, 4 joins per address per 10 s, 50 000 join counters | `EnetRelayOptions` |
| Limiter keys | 200 000 | `MAX_RATE_BUCKETS` |

A 429 from these limits has the same shape as before (`code: "RATE_LIMITED"`, `retryAfterSeconds`); the room cap
answers `TOO_MANY_ROOMS`.

## Operations (what the code cannot enforce)

- **TLS and a reverse proxy.** The server speaks plain HTTP. Put Caddy in front on the same machine
  (`reverse_proxy 127.0.0.1:8105`), bind the server to loopback (`HOST=127.0.0.1`) and let Caddy hold the certificate.
  In the site block set `request_body { max_size 1MB }`, and in the global `servers { timeouts { ... } }` options set
  `read_header 10s`, `read_body 20s` and `idle 2m`: this closes open items A and B.
- **Client address.** Leave `TRUST_PROXY` off with a proxy on the same machine (loopback is trusted by itself). Set it
  only for a proxy on another machine, with port 8105 firewalled to that machine, and set
  `LASTROUTE_TRUSTED_PROXY_HOPS` to the number of proxies that append to `X-Forwarded-For`. A wrong value makes every
  player share one allowance, or lets a client choose its own address.
- **Firewall.** Open only 80/443 (Caddy) and UDP 1105. Limit new TCP connections and UDP packets per source address
  (nftables `ct count` and `limit rate`); the relay runs under the same lock as the API, so a UDP flood slows both.
- **Unprivileged user.** Run the service as its own user with `NoNewPrivileges=yes`, `ProtectSystem=strict`,
  `ReadWritePaths=` the data folder only, `Restart=always`, `LimitNOFILE=65536`.
- **Secrets.** `LASTROUTE_ADMIN_PASSWORD`, `LASTROUTE_UPDATE_KEY`, `LASTROUTE_AGENT_TOKENS`, `LASTROUTE_SERVICE_TOKEN`
  and `LASTROUTE_MODERATION_SALT` live in `.env`: mode 600, owned by the service user, never in git. Use long random
  values (32+ characters) and change them when someone who knew them leaves. The setup password acts as an owner
  login: unset it once the owner accounts exist.
- **Data folder.** Mode 700. Back up `accounts.json`, `moderation.json`, `reserved_rooms.json`, `saves/` and `skins/`
  daily, off the machine, and test a restore. The backup holds password hashes: protect it like the original.
- **Logs.** The server writes to stdout and stderr; let journald keep them with a size cap
  (`SystemMaxUse=`). Watch for `Gave up on`, `panicked` and bursts of `outcome=RATE_LIMITED` or
  `outcome=INVALID_CREDENTIALS`.
- **Updates.** Rebuild after dependency fixes (`tiny_http`, `scrypt`, `sha2`, `hmac`, `serde_json`, `regex`,
  `socket2`); `Cargo.lock` pins them. Keep the OS and Caddy patched.
