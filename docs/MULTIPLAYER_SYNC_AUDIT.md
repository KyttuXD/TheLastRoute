# Multiplayer sync audit (2026-11-12)

An audit of the host and guest sync of Last Route (the Rust port), with the fixes that fit without a wire-format
change. Read from the code, not from a play test: nothing here was compiled or run by the audit (the build and
test steps do that), and no live server was contacted.

Related: [SERVERS.md](SERVERS.md), [SERVER_SECURITY.md](SERVER_SECURITY.md), [DATA_FILES.md](DATA_FILES.md),
[SLEEP.md](SLEEP.md), [STAIRS_CONTINUOUS_DESIGN.md](STAIRS_CONTINUOUS_DESIGN.md).

## The model in short

| What | Who decides | How it travels | How often |
|---|---|---|---|
| World layout | Each machine builds it from the room seed (`_receive_room`, `_receive_start`) | EVENTS, reliable | On join and on route start |
| Runner position, pose, held item, flashlight, locker, downed and revive state, waypoint | Each runner for itself | `_place` to the host, `_peer_place` from the host, STATE (unreliable, newest wins) | 30 Hz, early on a pose change; all state is in every sample, so a lost one heals itself |
| Hunters (place, level, state, stun, mechanic) | The host | `_hunters` (unreliable) or `_hunters_large` (reliable) | 20 Hz, whole frame every time |
| Hunter moments (state change, grab, door, locker, charge) | The host | `_event`, reliable | Once; only a notice on the guest |
| Sewer valves, burst pipes, safe-room status | The host (a guest asks with `_request_interaction`) | `_event`, reliable | Once, on change |
| NPC teammates | The host | `_xnpc_place` (unreliable), `_xnpc_remove` | 20 Hz; a guest drops one that is quiet for 1.5 s |
| Roster, difficulty, mods | The host | `_receive_room`, `_receive_packs` | On every change, to everyone |
| Late joiner | The host | `_receive_start`, profiles, `_snapshot` (hunters, sabotaged relay count) | On register |
| Doors, pickups, relays collected, lights, lockers' doors, beds and seats | Nobody: each machine keeps its own | Not sent | Never |

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
| 9 | High | Doors, pickups taken, relays collected and lights are not synced at all. A door one runner opened is shut for the others; an item one took is still there for the others. | `game/interact.rs`, `game/endless.rs` (claims), no net code | No | A design decision and new event kinds (`door`, `pickup`) plus a guest request for each, and the game hooks are in files other agents own. The Godot original sends these through `_request_interaction` and `_event` and puts them in the snapshot. The `StateLog` of issue 2 will carry them to late joiners once they exist: give the new kinds a `state_key`. |
| 10 | Medium | A sleeping (in bed) or sitting runner is shown standing beside the furniture. | `game/bed.rs`, `net/runner_figure.rs` `local_state`, the sample built in `app/mod.rs` | No | Needs a public accessor for the seat pose and anchor in `game/bed.rs`, a sit clip in `render/anim` (there is no sit pose), an optional sample key (for example `"rest": ["lie" or "sit", x, y, h, yaw]`, safe for older peers) set where the app builds the sample, and `ui/markers.rs` must stop reading the lying clip as DOWN. None of those files are in this pass. |
| 11 | Medium | On a guest the hunters are placed straight from the 20 Hz frames with no interpolation, so they step at 20 Hz while runners glide. | `game/hunting.rs` `apply_hunter_frame` | No | Needs a display buffer per hunter in the game (another agent's file). |
| 12 | Medium | A frame that grows past 1100 bytes moves from the unreliable HUNTERS channel to the reliable HUNTERS_LARGE channel. Around the threshold an older reliable frame can arrive after a newer unreliable one and be applied (one frame of rubber-band, 50 ms). | `net/world_sync.rs` `send_hunter_frame`, `net/session.rs` | No | A clean fix needs a frame number in the frame (a wire addition both sides must read) or one channel for both. |
| 13 | Medium | A client host simulates hunters only around its own window; a guest far from the host sees none near itself. A guest's taser or escape on another floor than the host's runner is dropped (`place_guest_event` finds no place). | `app/mod.rs`, `net/wiring.rs`, `game/guest_apply.rs` | No | Design: the dedicated room solves it with the party anchor. A client host would need the same anchor rules. |
| 14 | Low | A late joiner's replayed burst pipe or flood starts filling at the moment it is applied, not when it really started, so its water level lags the host's until both are full. | `game/pipes.rs` `start_flow`, `apply` | No | Needs the event's age on the wire and `game/pipes.rs` to read it. |
| 15 | Low | The dedicated room drops the frame's world events while no guest is in the room, so a state change with nobody there is not kept for the next joiner. | `headless/dedicated.rs` `send_outputs` | No | Not this pass's file. Only guests turn valves, so it needs a hunter-made change in an empty room. Fix: always call `send_world_events`. |
| 16 | Low | UDP relay: a guest that rejoins from the same address and port gets its old session id back, while the host still holds that id's packet counters (up to 45 s). The new connection's reliable messages are then acknowledged and thrown away. | `src/bin/lr_server/relay/enet_relay.rs` (session reuse), `net/session.rs` | No | A rejoin opens a new socket, so the port differs in practice. A real fix is a connection epoch in the session header: a wire change. |
| 17 | Low | The relay closes a room 1 s after the host's last HOST_ALIVE. A host whose uplink stalls for a second loses the room for everyone. | `src/bin/lr_server/relay/enet_relay.rs` `host_alive_ttl_ms` | No | The Node original's number (it makes host handoff fast). Changing it is a server tuning decision for KT. |
| 18 | Low | The relay allows 4 new guest sessions per address per room per 10 s. Five players behind one address joining at once: the fifth is refused until the window ends. | `src/bin/lr_server/relay/enet_relay.rs` `allow_guest_allocation` | No | Hardening limit; a normal busy game does not reach it. Data forwarding itself has no rate limit, and the HTTP limit (600 a minute per address) is far above a host's heartbeat (12 a minute). |

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

## Tests added

`net/world_sync.rs` (`sync_audit_tests`), `net/hunter_events.rs` and `net/interp.rs`, one per fix, with plain-sentence
names. Two older tests were adjusted to the new rules (a floor change with the same height is no longer a teleport;
a guest outside a route holds a valve event).
