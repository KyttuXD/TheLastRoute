# Skins between players (server-synced skins)

A skin is a 64x64 PNG known on the wire by its id: the lowercase hex SHA-256 of the PNG bytes. A room carries the id
(`world_sync.rs` `profile_message`); everything else is fetched by that hash. Godot reference: `docs/PLAYER_SKINS.md` and
`backend/skins.mjs` of the Godot project.

## The pipeline

1. **Pick** (SKIN tab): the file, URL, Minecraft name or catalogue picture is validated and re-encoded canonically
   (`skins::validate_skin`: 64x64, 64x32 upgraded, base layer made opaque). The id is the hash of those canonical bytes.
2. **Upload by hash** (signed in, not offline): `skins::upload_in_background` sends `POST /api/v1/skins {sessionToken, png}`
   (`net::api::upload_skin`). One upload per skin runs at a time and a failed one waits 60 s (`retry_secs`) before the next try.
   The server answers `{ok, id, hash}`; the client remembers the id in `skins-uploaded.json`.
3. **Publish**: once the id is held by the server, the host publishes it in its profile; other clients see the id.
4. **Download by hash**: `skins::fetch_skin_in_background(id)`, tried in this order, first fully verified copy wins:
   1. `GET <backend>/api/v1/skins/<id>` (lr_server's own route, the store the uploads go to),
   2. `GET https://skins.kyttuslab.org/<id>.png` (static immutable copies behind Caddy on VPS 2),
   3. `GET <backend>/api/skins/files/<id>.png` (the Godot backend's route; lr_server answers it as well).
   At most 4 downloads and uploads run at once, a skin that failed is left alone for 60 s, the timeout is 8 s, the size limit
   is the cap + 1 byte, the content type must be an image or a generic binary type.
5. **Validate** (both ends, the same code): `last_route_core::shared::skin_validate::check_skin_png`, used by the game
   (`skins::verify_download`, the upload check in `api::upload_skin`) and by `lr_server` (`skin_problem`).
   Every chunk CRC, IHDR first and exactly 64x64, a legal colour type and depth, no unknown critical chunk, IEND last with
   nothing after it, no interlace, a zlib stream that inflates to exactly the promised number of bytes (bounded: a
   decompression bomb stops at that number) with a matching Adler-32, filter bytes 0 to 4, and every pixel outside the
   second-layer rectangles (`data/skins/rules.json` `layout.overlay_rects`) fully opaque. A header that merely looks right,
   the old check, let a hostile PNG panic the decoder on every teammate and a transparent one hide the runner.
   The file is **not** required to equal the game's canonical encoding (Godot clients hash their own encoder's bytes).
6. **Cache**: a verified download is written to `<user dir>/skins-cache/<id>.png` (temp file then rename) and never fetched again.
   At most `cache_max` (16, Godot `PEER_CACHE_MAX`) files are kept, the oldest go first. Before the first download of a run the
   folder is cleaned: half-written files and any file that fails the full check (a hostile one an older build stored) are deleted.
   A cached file is read at most cap + 1 bytes and only if it is a regular file, and it counts as cached only when it passes the
   full check (`fetch_needed`, `read_skin`).
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

Rules (same as `skins.mjs`): cap 64 KB, one skin per account (`<data dir>/skins/index.json` keeps owners and blocked ids),
replacing or clearing deletes the old file unless another account holds it, at most 5 uploads per account per 10 minutes
(a refused picture does not use a slot; this is separate from the per-address request limit), the account's moderation flags
apply, a stored file is served only while it is still valid and still hashes to its name, and it is never read past the cap.
With `LASTROUTE_ACCOUNT_UPSTREAM` the website stores the skins: upload and read are forwarded (an upload that fails the
check is refused here first), and `mine`, `clear` and `remove` answer 503.

Tests: `cargo test --features server --bin lr_server skins` (includes upload, fetch and validate over a real loopback socket),
`cargo test --lib skin_validate`, `cargo test --bin last_route skin` and `net::http`.

## Arm style on the wire

The profile message carries `skin_slim` (boolean) with the hash, like Godot. The host relays it with each guest's profile.
A sender writes it when it knows the style (the skin was validated here this run, `skins::skin_is_slim`); a receiver uses the
profile's value, else reads the style from the downloaded picture once it is in the cache, else classic.

## Reset to DEFAULT

`net::api::clear_skin` (`POST /api/v1/skins/clear`) and `skins::clear_in_background` remove the account's stored skin and
forget the uploaded list.

## Open

See TODO.md "Server-synced skins, still open": the calls that live in `app/mod.rs` and `ui/` (calling
`clear_in_background` when a player picks DEFAULT, a re-APPLY uploading by hash, COPY SKIN with `skin_copy`, evicting GPU
textures in `shared/assets.rs`) belong to the skins-client track, which owns those files.
