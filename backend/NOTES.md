# Backend notes (task 046)

Tests only. Production code was not changed.

## Oversized crash reports leave the HTTP connection open

`readJson()` in `server.mjs` throws `REQUEST BODY TOO LARGE` as soon as the buffered body passes the cap, without destroying the request. On Node 22 the existing `crash_reports.test.mjs` HTTP case then hangs in `server.close()`: the socket is still open, undici is not holding the event loop, and `node:test` cancels the test with "Promise resolution is still pending but the event loop has already resolved". The assertions themselves succeed (the route returns 413). The test teardown now calls `server.closeAllConnections()` so `npm test` can finish. The server still does not destroy that socket.

## Username length is clipped instead of rejected

`account_service.mjs` `usernameOf()` uppercases and slices to 24 characters before `USERNAME_PATTERN` runs. A 25-character name therefore registers as its 24-character prefix. `AAAA…A` + `B` and `AAAA…A` + `C` collide with `USERNAME_TAKEN`, even though the error text says a username must be 3-24 characters. Covered by `account_service.test.mjs` ("long usernames are clipped to 24…").

## Room codes longer than 8 characters collapse together

`room_service.mjs` `cleanRoom()` strips non-alphanumerics and slices to 8. `ABCDEFGH` and `ABCDEFGHI` join the same room and share a world seed. The join error says codes must be 5-8 characters, but longer codes are not rejected. Covered by `room_service.test.mjs` ("nine-character codes are clipped…").

## Hide & Seek never leaves `hiding`

`startRoomRound()` sets `phase` to `"hiding"` and records `hideEndsAt` (30s). `released` for the seeker flips from that timestamp. The only transition to `"complete"` runs when `phase === "seeking"`, and nothing in this repo sets `"seeking"`. Tagging every hider leaves the round in `"hiding"`. Covered by `room_service.test.mjs` ("hide and seek starts in hiding…").

## Launch / abandon clears a custom map without bumping `customMapId`

`setCustomMap()` increments `customMapId` so a peer can see a change. `resetRuntime()` (launch and abandon) sets `customMapFile` to `""` and resets dialog, but leaves `customMapId` where it was. No in-repo client reads `customMapId`; a peer that reloads only when the id changes would keep the previous map. Covered by the launch/abandon test in `room_service.test.mjs`.

## `pack.json` listed in its own `files` array gets a misleading error

`buildPack()` compares the manifest list to every file except `pack.json`. Including `pack.json` in that list throws `pack.json "files" lists pack.json, which is not in the pack`. The refusal matches the loader rule (the list covers every other file), but the message is wrong. Covered by `deploy_mod_pack.test.mjs`.
