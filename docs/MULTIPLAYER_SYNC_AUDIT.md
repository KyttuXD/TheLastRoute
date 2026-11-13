# Multiplayer sync audit (2026-11-12)

An audit of the host and guest sync of Last Route (the Rust port). The first pass made the fixes that fit without a
wire-format change (issues 1 to 8); the second pass (the same day) did the rest with new optional keys and new event
kinds that an older build ignores (issues 9 to 16, and part of 13). Read from the code, not from a play test: nothing
here was compiled or run by the audit (the build and test steps do that), and no live server was contacted.

Related: [SERVERS.md](SERVERS.md), [SERVER_SECURITY.md](SERVER_SECURITY.md), [DATA_FILES.md](DATA_FILES.md),
[SLEEP.md](SLEEP.md), [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md).

## The model in short

| What | Who decides | How it travels | How often |
|---|---|---|---|
| World layout | Each machine builds it from the room seed (`_receive_room`, `_receive_start`) | EVENTS, reliable | On join and on route start |
| Runner position, pose, held item, flashlight, locker, downed and revive state, waypoint | Each runner for itself | `_place` to the host, `_peer_place` from the host, STATE (unreliable, newest wins) | 30 Hz, early on a pose change; all state is in every sample, so a lost one heals itself |
| Hunters (place, level, state, stun, mechanic) | The host | `_hunters` (unreliable) or `_hunters_large` (reliable), numbered (`fr`) so an overtaken frame is dropped | 20 Hz, whole frame every time; a guest glides between frames |
| Hunter moments (state change, grab, door, locker, charge) | The host | `_event`, reliable | Once; only a notice on the guest |
| Sewer valves, burst pipes, safe-room status | The host (a guest asks with `_request_interaction`) | `_event`, reliable | Once, on change |
| NPC teammates | The host | `_xnpc_place` (unreliable), `_xnpc_remove` | 20 Hz; a guest drops one that is quiet for 1.5 s |
| Roster, difficulty, mods | The host | `_receive_room`, `_receive_packs` | On every change, to everyone |
| Late joiner | The host | `_receive_start`, profiles, `_snapshot` (hunters, sabotaged relay count), the replayed state events (with their `age`), the object table (`world_objects`) | On register |
| Relays, batteries, floor items and coins taken; room doors; room lights | The host (a guest asks with `_request_interaction`, `world_object_request`) | `_event` `world_object`, reliable; the whole table as `world_objects` to a late joiner | Once, on change. Kept per machine by level and absolute cell, and applied when that chunk is built |
| Lying on a bed, sitting on a seat | Each runner for itself | The `rest` key of its sample | In every sample while resting |
| Lockers' doors, items a runner dropped, instruments and arcade machines | Nobody: each machine keeps its own | Not sent | Never |

Runner samples and NPC places are absolute cells; each viewer moves them into its own streamed window.

## Issues

Severity: **high** = players see different worlds or a different place for the same thing; **medium** = wrong for a
while or under load; **low** = cosmetic or rare.

| # | Severity | Issue | Where | Fixed | Notes |
|---|---|---|---|---|---|
| 1 | High | A client host (the public route's stand-in) sent its hunters in its own window's cells with no window origin. A guest standing in another chunk window applied them unshifted, so every hunter was drawn a whole number of chunks away from where it hunts. A dedicated room already sent `origin`. | `net/world_sync.rs` `send_hunter_frame`, `snapshot_hunters`; `net/hunter_events.rs` `stamp_origin` | Yes | The host now adds `origin` to every entry that has none (the guest side already read it). The origin comes from `publish_npcs`, which the app calls every host frame. Optional key, ignored by an older guest. |
| 2 | High | A runner that joined (or rejoined) during a route was never told the valves turned, pipes burst and safe rooms breached before it came: the snapshot carries only hunters and the relay count. Its sewer gates and water differed from everyone else's for the rest of the route. | `net/world_sync.rs` `join_state`, `send_world_events`; `net/hunter_events.rs` `StateLog`, `state_key` | Yes | The host keeps the latest event per valve, flow and safe room (at most 1024, about 250 KB at the very most, cleared on route start and end) and replays them after the snapshot as ordinary `_event` messages. No new message. |
| 3 | High | A guest still loading the route dropped the world events that arrived meanwhile (the app has no game to apply them to), and they are sent once. | `net/world_sync.rs` `on_message` (`Message::Event`), `release_held_events` | Yes | A guest holds lasting-state events while its tick carries no sample and hands them over with the first sample's tick. Cleared on a new start or an end. |
| 4 | High | A hunter frame or a join snapshot longer than one UDP relay datagram (1452 bytes, about 6 hunters; the frame lists the resident hunters of every level) was not sent at all: `CoopSession::send` returns false and nothing reports it. Guests of a native client host then saw frozen hunters. | `net/world_sync.rs` `send_hunter_frame`, `join_state`, `frame_room`; `net/hunter_events.rs` `split_frame` | Yes | A frame that does not fit is sent as several `_hunters` messages that each fit (a guest applies entries one by one, by id). A snapshot that does not fit goes without hunters, followed by reliable `_hunters_large` parts. WebSocket rooms (65535 bytes) are unchanged. |
| 5 | Medium | A guest that said goodbye, was kicked or whose socket closed stayed in the session's peer table. The host kept sending it frames and resending every unacknowledged reliable message five times a second, for ever. Over the UDP relay each resend also refreshed the guest's relay session, so it never expired and counted against the room's 15 sessions. | `net/world_sync.rs` `forget_guest` | Yes | `forget_guest` now calls `CoopSession::forget_peer`. |
| 6 | Low | A stray late packet from a session that never registered made a nameless guest, and its timeout showed an empty `LEFT THE ROOM` notice. | `net/world_sync.rs` `prune` | Yes | Only a guest that registered gets a left notice. |
| 7 | Low | A runner that left stayed on the other guests' PLAYERS list as DISCONNECTED for 45 s, with its name, skin and waypoint kept. | `net/world_sync.rs` `on_message` (`Message::Room`) | Yes | A guest drops every runner the host's roster (`peers`) no longer lists. An older host sends no ids, and then nothing is dropped. |
| 8 | Medium | A runner on a flight of stairs snapped instead of gliding each time its floor label changed (the interpolation treated any `depth` change as a teleport), and its sample lost its velocity for that step. | `net/interp.rs` `same_floor`, `Track::shown`, `Track::velocity`; `net/world_sync.rs` `send_due` | Yes | With heights on both samples the height step decides (`snap_height_m`, 2.5 m, optional key of `data/net/sync.json`); a sample without a height (an older client) still snaps on a label change. |
| 9 | High | Doors, pickups taken, relays collected and lights were not synced at all. A door one runner opened was shut for the others; an item one took was still there for the others. | `net/world_objects.rs` (new); `net/world_sync.rs` `publish_object`, `accept_object_event`, `accept_object_request`, `join_state`; `game/guest_apply.rs` `sync_world_objects`, `host_object_request`, `apply_object_op`, `apply_object_table`; `game/interact.rs` `claim_relay`, `take_battery`, `take_pickup`, `press_door`, the coin and the light switch; `app/mod.rs` and `headless/dedicated.rs` (one call each) | Yes | The host decides. An object is named by kind, level and absolute cell (every machine builds the same world from the seed). A guest's press is a `world_object_request`; the host checks it against the guest's own sample (reach, level; the object must exist and be free while the host's window holds its chunk), applies it and publishes one `world_object` event with the peer that has it, so only that runner is given the item and nobody takes it twice. A door and a light switch change at once for the guest that pressed (prediction) and are put right by the host's answer; a close the host refuses (a body in the leaf's sweep) is answered to that guest alone. Each machine keeps the table (`ObjectTable`, at most 16384 entries, about 0.5 MB at the very most; only what differs from an untouched world) and applies it whenever its window is another one, so a chunk streamed after the fact is right. A late joiner is sent the table in parts that fit the link. The host also publishes the doors its hunters pulled open (compared with the table twice a second, only while a guest is in the room). Relays count for the whole room: the host's hunter population grows with every guest's relay, and a guest's own count follows the table. **Compatibility:** the host says it decides the objects with `"sync": 2` in its room message; a new guest in an older host's room sees no such key and keeps every object to itself as before; an older guest ignores the three new kinds (it takes its own pickups as before, which the host does not hear of). Not synced: an item a runner dropped, and lockers' doors. |
| 10 | Medium | A sleeping (in bed) or sitting runner was shown standing beside the furniture. | `game/bed.rs` `rest_wire`; `net/world_sync.rs` `RestPose`, `Sample::with_rest`, `remote_view`; `render/anim/runner.rs` `Pose::Sit`, `Pose::Lie`, `clip_name`; `ui/markers.rs`; `app/mod.rs` (the sample) | Yes | The sample carries `"rest": ["lie" or "sit", x, y, height, yaw]` (absolute cells, the height above the runner's feet) while the runner rests. A viewer draws the body there, in the rig's `sit` clip or lying (the clip a caught runner lies in, as its own pose `Lie`, so the markers do not read it as DOWN). An older peer ignores the key and shows the runner standing; an anchor further than 6 cells from the runner is not believed. Not checked in the picture: how the lying body sits on the mattress (the anchor is the bed's seat point and the look along the bed). |
| 11 | Medium | On a guest the hunters were placed straight from the 20 Hz frames with no interpolation, so they stepped at 20 Hz while runners glide. | `game/hunting.rs` `HunterTrack`, `note_hunter_place`, `glide_guest_hunters`; `net/world_sync.rs` `Sample::at` | Yes | Each hunter's places go through the runner smoothing (`net/interp.rs` `Track`), in absolute cells, and the guest's hunter step puts each hunter of its floor where the track says it is now. A jump longer than `snap_distance_m` or a height step above `snap_height_m` snaps. The frame still sets the place at once (the host's word); only what is drawn in between is smoothed. The tracks live in a thread-local map by hunter id (the game struct is not this pass's file). |
| 12 | Medium | A frame that grows past 1100 bytes moves from the unreliable HUNTERS channel to the reliable HUNTERS_LARGE channel. Around the threshold an older reliable frame could arrive after a newer unreliable one and be applied. | `net/world_sync.rs` `send_hunter_frame`, `frame_is_stale`, `stamp_frame_number`, `take_frame_number` | Yes | The host numbers its frames: the first entry of each message carries `"fr"`. A guest drops a frame older than one it applied and takes the number off before the game sees the entries. An older host sends no number (every frame passes); an older guest reads the entry by its other keys. |
| 13 | Medium | A client host simulates hunters only around its own window; a guest far from the host sees none near itself. A guest's taser or escape on another floor than the host's runner is dropped (`place_guest_event` finds no place). | `game/guest_apply.rs` `guest_escaped` | Partly | Fixed: an escape from a hunter the host named at the capture stuns that hunter on a resident floor too. Still open: (a) the window of a client host follows its own runner. Giving it the dedicated room's party anchor would take the host's own world away from under it; the answer is the dedicated room, which every online room but the public stand-in already is. (b) A taser from another floor needs that floor's walls for the line-of-sight test, and the host's game holds them only for resident floors, through the resident lists (not this pass's files). |
| 14 | Low | A late joiner's replayed burst pipe or flood started filling at the moment it was applied, not when it really started. | `net/hunter_events.rs` `StateLog::replay`, `event_age`; `game/guest_apply.rs` `apply_pipe_event` | Yes | The host remembers when each kept event happened and a replayed valve turn or burst older than half a second carries `"age"` (seconds). The joiner starts the flow that long ago. An older guest ignores the key; an event without it starts now. |
| 15 | Low | The dedicated room dropped the frame's world events while no guest was in the room. | `headless/dedicated.rs` `send_outputs` | Yes | The events always go to `send_world_events`: a lasting change is kept for the next joiner, a passing moment is sent to nobody. |
| 16 | Low | UDP relay: a guest that rejoins from the same address and port gets its old session id back, while the host still holds that id's packet counters (up to 45 s). The new connection's reliable messages were acknowledged and thrown away. | `net/session.rs` `joined_again`; `net/world_sync.rs` `store_remote` | Yes | No wire change: a reliable EVENTS packet with seq 0 that comes 5 s or more after the host took that session's first one is a new connection's first packet (an honest resend comes every 200 ms), so the host starts that peer's counters again. A runner silent past the draw timeout whose samples count from the start again is taken as new too. |
| 17 | Low | The relay closes a room 1 s after the host's last HOST_ALIVE. | `src/bin/lr_server/relay/enet_relay.rs` `host_alive_ttl_ms` | No | Left as it is. The keep-alive is sent every 250 ms from its own thread and socket, so a frame hitch or a long load of the host's game does not stop it: only an uplink that loses four in a row does, and then the handover to the next runner is what the 1 s is for. It is the Node original's number and a server tuning decision for KT. |
| 18 | Low | The relay allows 4 new guest sessions per address per room per 10 s. | `src/bin/lr_server/relay/enet_relay.rs` `allow_guest_allocation` | No | Left as it is: a hardening limit a normal game does not reach (five runners behind one address joining within ten seconds; the fifth gets in when the window ends, and the client retries). Forwarding itself has no rate limit. |

## Checked and found sound

- Reliable channels are acknowledged, resent and delivered in order (`net/session.rs`); unreliable ordered channels
  drop anything older than the newest seen; a sample older than the last kept is refused twice (session and track).
- Interpolation: the delay follows the measured interval and jitter inside fixed limits, extrapolation is capped at
  0.15 s and at the speed cap, a long silence holds instead of gliding, a burst after a hitch is spread out and its
  lead drains, and the picture never plays backwards.
- Downed and revive state, held item, flashlight, locker and waypoint ride in every sample, so they resync by
  themselves after loss or a late join.
- A guest never simulates hunters, never applies its own valve turn before the host publishes it, and never sends
  host-only messages (the session refuses them on both ends).
- NPC teammates: a lost remove cannot leave a ghost (1.5 s stale rule).
- Server: the relay forwards room data without a rate limit; the bounded tables are for joins and HTTP requests only.

## New on the wire (second pass)

All optional: an older build ignores each one, and nothing that existed changed.

| What | Where | Meaning |
|---|---|---|
| `"sync": 2` | the host's room message | this host decides the world objects and answers requests for them |
| `world_object` | `_event` | one object changed: `k` (kind), `l` (level), `x`, `y` (absolute cell), `on`, `by` (the peer that has it) |
| `world_objects` | `_event` | the table for a late joiner: `o`, a list of `[k, l, x, y, on]` |
| `world_object_request` | `_request_interaction` | a guest asks: `k`, `l`, `x`, `y`, `on` |
| `"rest"` | a runner's sample | `["lie" or "sit", x, y, height, yaw]` while it rests |
| `"fr"` | the first entry of a hunter frame message | the frame's number |
| `"age"` | a replayed `stream_valve` or `stream_pipe_burst` event | seconds since it happened |

## Tests added

First pass: `net/world_sync.rs` (`sync_audit_tests`), `net/hunter_events.rs` and `net/interp.rs`, one per fix, with
plain-sentence names. Two older tests were adjusted to the new rules (a floor change with the same height is no longer
a teleport; a guest outside a route holds a valve event).

Second pass: `net/world_objects.rs`, `net/world_sync.rs` (`sync_audit_tests`), `net/hunter_events.rs`, `net/session.rs`,
`net/runner_figure.rs`, `game/guest_apply.rs` (`world_object_tests`), `game/hunting.rs` (`hunter_glide_tests`),
`game/bed.rs`, `ui/markers.rs` and `headless/dedicated.rs` (two rooms with bot guests over the test hub). Each new key
and kind has a test of what an older build makes of it.
