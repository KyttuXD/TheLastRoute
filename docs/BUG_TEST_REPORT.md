# Bug test report

Read-only pass over the recently changed code. Nothing was fixed, built, or run (no cargo). Each item says whether it is confirmed by reading the code or only suspected.

Scope: `git log -n 12` covers the whole history (7 commits), so the focus was the two latest commits, `3ae870b` (login reset, scroll and spike fixes) and `1503df5` (server update key, pack check, safe-zone light, shader lamps). `world_mesh.rs` was not reviewed line by line.

The bug report that triggered this pass was cut off after "`lobby_players` in". The search covered `lobby_players` in `src/bin/last_route/ui/menus.rs`.

## Reported bug: guest lobby shows only its own row

Status: suspected, not reproduced.

Traced path: `app/mod.rs:538-549` (join sets `RoomMode::Guest`) → `world_sync.rs:577-586` (`Message::Room` sets `host_name` and `players`) → `world_sync.rs:337-362` (`guest_names`) → `app/mod.rs:911` → `menus.rs:1198-1227` (`lobby_players`). Each step reads correctly.

Likely cause to check at runtime: the host sends the roster only on `RegisterPeer` (`world_sync.rs:529`) and `PeerLeave` (`world_sync.rs:560`), with no resend. If the relay drops the host's `Room` message before the guest is linked, `host_name` and `players` stay empty until the next roster change. Nothing retries.

## Other findings

1. **Confirmed (by reading):** `src/bin/lr_server/pack_check.rs:45-46`. `collect_files` recurses on `path.is_dir()`, which follows symlinks, and has no visited set. A pack folder with a symlink back to a parent loops until the stack overflows, so `lr_server --check-pack` aborts. Trigger: a symlink to `..` inside the pack folder.
2. **Confirmed (by reading):** `pack_check.rs:54`. `std::fs::read` loads every file whole before any size check. A large file in a pack folder is read into memory. Severity: low.
3. **Confirmed (by reading):** `pack_check.rs:42`. `entries.flatten()` drops read_dir errors silently, so a file that fails to list is missing from the manifest with no message.
4. **Confirmed (minor):** `menus.rs:1201` and `:1215` compare and display the raw `settings.username`. The wire name is `clean_name()` (`app/mod.rs:1048, 1056`). A username with stripped characters shows a different name in row one than the roster does.
5. **Low (by reading):** `src/bin/lr_server/util.rs:468`. `timing_safe_equal` returns early on a length mismatch, which leaks the key length. Only relevant to the `/api/updates/files` key gate (`routes.rs:253-254`).

## Checked and not bugs

- `room_service.rs:931`: `% room.players.len()` cannot divide by zero, because `start_room_round` calls `authenticate(body, true)` first, which returns the caller's index in the room.
- `app/mod.rs:724-729`: relock settle is correct.
- `app/perf.rs` `SpikeLog::latest`: correct when full and when partly filled.
- `shader.rs:160-175`: nearest-lamp packing is correct. A lamp with `range` 0 is dropped silently, which is a design note, not a fault.
- `regions.rs` `safe_light`: bounds and key checks are correct.

Not checked: `world_mesh.rs` render code, `facility.rs` `unwrap` calls (lines 1438, 1459), and `settings_panel.rs` scroll clamps beyond the lines above.
