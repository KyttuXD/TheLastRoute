# Skins between players (server-synced skins)

A skin is a 64x64 PNG known on the wire by its id: the lowercase hex SHA-256 of the PNG bytes. A room carries the id
(`world_sync.rs` `profile_message`); everything else is fetched by that hash. Godot reference: `docs/PLAYER_SKINS.md` and
`backend/skins.mjs` of the Godot project.

## The pipeline

1. **Pick** (SKIN tab): the chosen file, URL, copied runner skin or catalogue picture is validated and re-encoded canonically
   (`skins::validate_skin`: the picked file goes through the one shared structural check in the shape `Local` (64x64 or the old
   64x32, interlaced allowed), then the fallible decoder, the old format upgraded to 64x64, the base layer made opaque). The id
   is the hash of those canonical bytes.
2. **Upload by hash** (signed in, not offline): `skins::upload_in_background` sends `POST /api/v1/skins {sessionToken, png}`
   (`net::api::upload_skin`). One upload per skin runs at a time and a failed one waits 60 s (`retry_secs`) before the next try.
   The server answers `{ok, id, hash}` (the Node backend's `POST /api/skins/upload`, the fallback when the v1 route is unknown,
   answers only `{ok, hash}`; the client accepts either); the client remembers that id, and only that one, in
   `skins-uploaded.json` (the website keeps one skin per account, so choosing an earlier skin again uploads it again). The browser
   build has no user folder, so it keeps that list in memory for the session (`skins::uploaded_ids`) and uploads again on a reload.
3. **Publish**: once the id is held by the server, the host publishes it in its profile; other clients see the id.
4. **Download by hash**: `skins::fetch_skin_in_background(id)`, tried in this order, first fully verified copy wins:
   1. `GET <backend>/api/v1/skins/<id>` (lr_server's own route, the store the uploads go to),
   2. `GET https://skins.kyttuslab.org/<id>.png` (static immutable copies behind Caddy on VPS 2),
   3. `GET <backend>/api/skins/files/<id>.png` (the Godot backend's route; lr_server answers it as well).
   A recorded skin is checked against the server the same way (the v1 route, then the Godot route) before it is trusted again.
   Nothing here needs a session: a guest fetches any skin it is told about. At most 4 downloads and uploads run at once, a skin that failed is left alone for 60 s, the timeout is 8 s, the size limit
   is the cap + 1 byte, the content type must be an image or a generic binary type.
5. **Validate** (both ends, the same code): `last_route_core::shared::skin_validate`. Its structural check (`check_png`, shapes
   `Remote` and `Local`, chunk limit from `data/skins/config.json` `png.max_chunks`) and its full check (`check_skin_png_with`,
   `Remote` only plus decode and the opaque base) are the only validators: the game uses them through `skins::verify_skin` (hash
   plus full check, for downloads, cache and disk files), `skins::validate_skin` (picked files) and the upload check in
   `api::upload_skin`, and `lr_server` through `skin_problem`. `shared/skin_png.rs` only decodes what that check accepted.
   Every chunk CRC, IHDR first and exactly 64x64, a legal colour type and depth, no unknown critical chunk, IEND last with
   nothing after it, no interlace, a zlib stream that inflates to exactly the promised number of bytes (bounded: a
   decompression bomb stops at that number) with a matching Adler-32, filter bytes 0 to 4, and every pixel outside the
   second-layer rectangles (`data/skins/rules.json` `layout.overlay_rects`) fully opaque. A header that merely looks right,
   the old check, let a hostile PNG panic the decoder on every teammate and a transparent one hide the runner.
   The file is **not** required to equal the game's canonical encoding (Godot clients hash their own encoder's bytes).
6. **Cache**: a verified download is written to `<user dir>/skins-cache/<id>.png` (temp file then rename) and never fetched again.
   At most `cache_max` (16, Godot `PEER_CACHE_MAX`) files are kept, the oldest go first, except a skin that is on the GPU or is the
   runner's own (`skins::mark_in_use`). Before the first download of a run the folder is cleaned: any file that fails the full check
   (a hostile one an older build stored) and any `<id>.tmp` older than `tmp_stale_secs` (60) are deleted; a younger temp file is a
   download running on another thread and is never touched. The runner's own worn skin is also kept in memory
   (`skins::remember_own`), so a pruned or wiped cache folder never takes it away from the hand, the body or the upload.
   A cached file is read at most cap + 1 bytes and only if it is a regular file, and it counts as cached only when it passes the
   full check (`fetch_needed`, `read_skin`).
   With no cache folder (the browser build, or a desktop without a user folder) a verified download is kept in memory instead
   (`skins::remember_peer`, the same `cache_max` bound) and `skins::read_skin` finds it there.
7. **Fallback**: no skin yet, a failed or refused download: the runner wears the default mannequin and the fetch is retried
   after the cool-down.

## Server (`lr_server`, `src/bin/lr_server/skins/skins.rs`)

| Route | Meaning |
|---|---|
| `POST /api/v1/skins` | JSON `{sessionToken, png: base64}`; body limit 88,408 bytes (the base64 of 64 KB plus JSON), other routes keep 64 KiB |
| `POST /api/skins/upload` | Godot route: raw PNG body, header `x-session-token` |
| `GET /api/v1/skins/<id>` and `GET /api/skins/files/<id>.png` | the PNG: `image/png`, `immutable` for a year, `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'`; 404 `SKIN_NOT_FOUND`, 400 `BAD_SKIN_ID` |
| `POST /api/v1/skins/mine`, `/clear` (and `/api/skins/...`) | the account's own skin id; remove it |
| `POST /api/v1/skins/remove` (and `/api/skins/remove`) | full admin only, by `hash` or `username`; the id is blocked from ever being uploaded again |

Rules (same as `skins.mjs`, except the file lifetime): cap 64 KB, one skin per account (`<data dir>/skins/index.json` keeps
owners, blocked ids and the time each unreferenced id was released). Files are content-addressed and are **not** deleted when an
account switches or clears: peers in a room still name the old id. A file nobody holds is kept for a grace period
(`LASTROUTE_SKIN_GRACE_SECONDS`, 7 days by default) and then collected; the release time is saved, so a restart does not reset it, a
re-upload of the id cancels it, and files an older build left unreferenced start their grace at the next start. Only an owner's
`remove` deletes a file at once. At most 5 uploads per account per 10 minutes
(a refused picture does not use a slot; this is separate from the per-address request limit), the account's moderation flags
apply, a stored file is served only while it is still valid and still hashes to its name, and it is never read past the cap.
With `LASTROUTE_ACCOUNT_UPSTREAM` the website stores the skins: upload (both the JSON route and the raw Godot route), read, `mine`
and `clear` are forwarded (an upload that fails the check is refused here first, and the raw route never writes the local
directory), and `remove` answers 503 (its owner check is the local account list). A client treats a 503 `SKINS_UNAVAILABLE` from
`clear` as "nothing held there" and forgets its uploaded id.

Tests: `cargo test --features server --bin lr_server skins` (includes upload, fetch and validate over a real loopback socket),
`cargo test --lib skin_validate`, `cargo test --bin last_route skin` and `net::http`.

## Arm style on the wire

The profile message carries `skin_slim` (boolean) with the hash, like Godot. The host relays it with each guest's profile.
The sender writes `skin_slim` and `skin_copy` with the id (`world_sync::SkinFlags`: the resolved ARMS setting and OTHERS MAY COPY);
a receiver uses the profile's value when the profile has one, else reads the style from the downloaded picture once it is in the
cache, else classic. The worn style comes from the validated file, except for the CUSTOM file (its canonical PNG has an opaque
base, so the layout read when it was picked is kept in the settings).

## Reset to DEFAULT

`net::api::clear_skin` (`POST /api/v1/skins/clear`) and `skins::clear_in_background` remove the account's stored skin and
forget the uploaded id. `app/mod.rs` calls it when the worn skin goes from a real one to DEFAULT and the account is signed in
(retried on later frames until accepted).

## PICK A PNG, in a run and on the web

**PICK A PNG** opens the system's own file chooser (`shared/file_pick.rs`), never a list drawn in the game:

- **Web**: the page's script (`packaging/web/index.html`, copied to `dist/web/index.html`) makes a hidden `<input type=file accept="image/png,.png">`
  and clicks it, so the browser shows its own file window. The chosen file is read into the page and the game polls for the answer
  (`lr_pick_state`: pending, done or cancelled); the bytes stay in the page until `lr_pick_read` copies one file into the game. The bridge is
  generic, so a mod can reuse it: `lr_pick_open(kind, multiple, directory)` returns a request id (`kind` is an HTML `accept` string, `directory`
  picks a folder and gives its files by relative path), `lr_pick_state(id)`, `lr_pick_count(id)`, `lr_pick_name(id, i, dst, cap)`,
  `lr_pick_size(id, i)`, `lr_pick_read(id, i, dst, cap)` and `lr_pick_release(id)`. It never throws into the game. A page without these
  functions cannot start the wasm (the imports are missing), so `index.html` and `last_route.wasm` are deployed together.
- **Windows and Linux**: the `rfd` crate (native targets only) shows the system open dialog on a thread of its own, starting in the Pictures
  folder, else Desktop, else the home folder. **macOS**: the open panel must run on the main thread, which is the macroquad loop's thread, so
  the synchronous dialog is called there on the frame after the press (the window waits while the panel is open). Linux without a desktop
  portal or zenity has no dialog: the status line says to drop a PNG on the window.
- The chosen file goes through the same checks as a dropped one (`.png` name, `skins::validate_skin`, the size cap). A closed chooser changes
  nothing. Tests use a fake dialog (`file_pick` tests) and hand outcomes to `skin_tab::take_picked`.

The browser has no skins folder (`paths::user_dir()` is `None` there), so APPLY keeps the skin in memory and in the page's localStorage
(`last_route.custom_skin`) and the game wears it from there (`skins::keep_custom_in_memory`, `worn_skin_in`).

**In a run**: the pause menu has a SKIN entry (between HUNTER DOSSIER and SETTINGS) that opens the settings overlay on its SKIN tab (the same
`ui/skin_tab.rs`). APPLY and DEFAULT change `Settings::skin`; the game loop resolves it again the same frame (`WornTracker`), so the first-person
hands change, the upload starts, and the open room gets the new id and arm style (`set_skin`, `set_skin_flags`).

## FROM URL and the skins host

They go through `net/http.rs` over https (`docs/NET_HTTPS.md`): `skins::fetch_https` with the rules' url checks, at most 3
redirects each re-checked, loopback, private and other non-public addresses refused after the name is resolved (IPv6 forms that
carry an IPv4 address, such as NAT64, 6to4, v4-compatible and Teredo, are judged by it), and DNS inside the request deadline. Tests:
FROM URL with a fake getter (`fetch_url_skin_via`: hosts and content types), a skin PNG over real TLS on loopback through the client's own limit
and content types (`net::http` tests), and an ignored live test (`cargo test --bin last_route live_ -- --ignored`).

## What another player's skin can and cannot do to your frame

Only a skin id (a 64 hex hash), the arm style and a copy flag cross the room's wire; the PNG itself never does (the server's skin
store holds it, 64 KB at most, and each client fetches, verifies and decodes it). Everything that can go wrong with a peer's skin ends in
the default mannequin atlas on the viewer's screen, never in a lost frame:

- `shared/skin_cache.rs` holds every decision about the skin textures (generic over the texture type, tested with no GPU): a pending skin, a
  failed download, a refused file (not a PNG, wrong size, 64x32, 128x128, 16-bit, palette, interlaced, trailing bytes, a hash that matches
  a hostile file) all answer `None` and the caller draws the atlas. A refused skin is noted once and never read again; only exactly
  64x64 RGBA reaches the GPU upload; at most `cache_max` skin textures are held (least recently used freed); at most 64 pending or refused ids
  are remembered (ids are announced by other players).
- Profiles are cleaned on receipt (`clean_skin`, `clean_name`, a boolean-only `skin_slim`), tested against hostile profiles in
  `net::world_sync::skin_wire_tests`.
- A skin that arrives late (or after the runner was first drawn) is picked up within 2 s; the slim body is drawn from the profile's
  `skin_slim` while the picture is still on its way.
- The release profile aborts on a panic (and a wasm panic stops the frame loop), so there is no `catch_unwind` net: the defence is that
  none of these paths can panic.

**Browser build.** It has no user folder, so a web client keeps its own skin in memory, does not fetch other runners' skins (it draws
them in the default atlas, with their arm style) and, before the fix, never recorded that the website holds its own skin: the skin was
uploaded again on every 60 s retry (a blocking request on the page's only thread) and never announced to the room. The session now remembers
what the website took (`skins::uploaded_ids_at`), so a web skin is uploaded once and announced.

## Room announce, retries and visible state

- A runner (host or guest, direct or relay) announces its skin in its wire profile only once the server holds it
  (`world_sync::published_skin`, `skins-uploaded.json`); until then peers see DEFAULT. The app re-checks every 2 s and republishes
  as soon as the upload is accepted, so peers fetch at once. Peers fetch, verify, cache and apply by id (`Assets::skin_texture`).
- Uploads and downloads that fail are retried with a growing wait: 5 s, 10 s, 20 s, 40 s, then the config's 60 s
  (`skins::backoff`; the old fixed 60 s made a skin that was only late to appear wait a full minute). The upload offer is checked every 5 s.
- Visible state: the SKIN tab's "Your skin" line adds `skins::ShareState::text()` (sign in so other runners see it, sharing...,
  `not shared yet: <why>`, e.g. "server needs an update" or "the server did not accept this sign-in"; "sharing" only while an upload
  job runs, "waiting to share" before the first try, "offline: ..." when the game is offline). The failure bookkeeping keeps at most
  64 failed download keys (peer-announced ids are untrusted); the runner's own upload is never dropped.
  Failed uploads and downloads also log one line to stderr.
- Test: `net::world_sync::tests::two_peers_announce_fetch_and_apply_a_skin` (two sessions over a fake relay: DEFAULT before the
  upload, the id after it, the other side sees it on the remote body, then a fetch that misses once, retries, verifies and decodes).

### Server deploy needed (VPS 3, lr_server at 194.156.90.219:8105)

**The real fix for skins in rooms is deploying `lr_server` to VPS 3.** Nothing in the client can make an old server store a skin:
until it is deployed, uploads fail and rooms show DEFAULT. The client only tells the truth meanwhile: it tries
`POST /api/v1/skins` and, on a 404, the Godot route `POST /api/skins/upload` (a supported contract of `lr_server`, see the table
above); if both answer 404 the SKIN tab says "not shared yet: server needs an update" and the retry waits the full 60 s.

The deployed build predates the skin routes, which is why no client could ever upload and every room showed DEFAULT. Deploy the
current `lr_server` with at least: `POST /api/v1/skins` (JSON `{sessionToken, png}`), `GET /api/v1/skins/<id>`,
`POST /api/v1/skins/mine` and `/clear`, and the Godot-compatible `POST /api/skins/upload` and `GET /api/skins/files/<id>.png`.
`LASTROUTE_ACCOUNT_UPSTREAM` must be unset or point at a reachable website, and player sessions must be valid on that server.
Check afterwards: `GET /api/v1/skins/<64 a's>` answers 404 `SKIN_NOT_FOUND` and `POST /api/v1/skins` with a bad token answers
401/403, not 404 `UNKNOWN_ENDPOINT`. Not deployed from here.

## Open

Nothing that can be done in code. Measuring the release binary size increase of rustls still needs a release build (`docs/NET_HTTPS.md`).
