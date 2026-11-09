import test from "node:test";
import assert from "node:assert/strict";
import { createRoomService, DEFAULT_ROOM_SETTINGS, MAX_DEPTH_CLAIM_STEP, depthClaimAllowed, ITEM_TOTAL_LIMIT, ITEM_LIMIT } from "./room_service.mjs";

function serviceAt(now = 1_000_000, limits = {}) {
  let clock = now;
  const service = createRoomService({ now: () => clock, ...limits });
  return { service, at: () => clock, set: (value) => { clock = value; }, advance: (ms) => { clock += ms; } };
}

function join(service, body) {
  const [status, payload] = service.joinRoom(body);
  assert.equal(status, 200, JSON.stringify(payload));
  return payload;
}

test("default room settings are the standard last-route lobby", () => {
  assert.deepEqual(DEFAULT_ROOM_SETTINGS, {
    mapScale: "standard",
    difficulty: "standard",
    mode: "lastRoute",
    fakeRunners: false,
  });
});

test("join creates a host room, rejects bad input, and rejoins only with the session", () => {
  const { service } = serviceAt();
  assert.equal(service.joinRoom({ room: "abcd", playerId: "p1", name: "Ann" })[1].code, "INVALID_ROOM");
  assert.equal(service.joinRoom({ room: "HELLO", playerId: "", name: "Ann" })[1].code, "INVALID_ROOM");
  assert.equal(service.joinRoom({ room: "HELLO", playerId: "p1", name: "!!!" })[1].code, "INVALID_NAME");

  const host = join(service, { room: "he-llo!", playerId: "p 1", name: " ann " });
  assert.equal(host.room, "HELLO");
  assert.equal(host.isHost, true);
  assert.equal(host.spawnIndex, 0);
  assert.equal(host.roomSize, 1);
  assert.equal(host.maxPlayers, 16);
  assert.match(host.worldSeed, /^ROOM-HELLO-[A-F0-9]{20}$/);
  assert.equal(host.player.role, "runner");
  assert.equal(host.player.name, "ANN");
  assert.deepEqual(host.settings, DEFAULT_ROOM_SETTINGS);

  const guest = join(service, { room: "hello", playerId: "p2", name: "Bob" });
  assert.equal(guest.isHost, false);
  assert.equal(guest.spawnIndex, 1);
  assert.equal(guest.roomSize, 2);
  assert.equal(guest.players.length, 1);
  assert.equal(guest.players[0].name, "ANN");
  assert.equal(guest.players[0].isHost, true);
  assert.equal(JSON.stringify(guest.players).includes(host.sessionToken), false);

  assert.equal(service.joinRoom({ room: "HELLO", playerId: "p3", name: "ann" })[1].code, "NAME_TAKEN");
  const rejected = service.joinRoom({ room: "HELLO", playerId: "p1", name: "Ann", sessionToken: "nope" });
  assert.equal(rejected[0], 403);
  assert.equal(rejected[1].code, "INVALID_SESSION");
  const renamed = join(service, { room: "HELLO", playerId: "p1", name: "Ada", sessionToken: host.sessionToken });
  assert.equal(renamed.player.name, "ADA");
  assert.equal(renamed.sessionToken, host.sessionToken);
  assert.equal(renamed.spawnIndex, 0);
});

test("nine-character codes are clipped to eight and land in the same room", () => {
  const { service } = serviceAt();
  const first = join(service, { room: "ABCDEFGH", playerId: "p1", name: "Ann" });
  const second = join(service, { room: "ABCDEFGHI", playerId: "p2", name: "Bob" });
  assert.equal(second.room, "ABCDEFGH");
  assert.equal(second.roomSize, 2);
  assert.equal(second.worldSeed, first.worldSeed);
});

test("updates clamp state, share host world and items, and page discovery", () => {
  const clock = serviceAt();
  const host = join(clock.service, { room: "HELLO", playerId: "host", name: "Ann" });
  const guest = join(clock.service, { room: "HELLO", playerId: "guest", name: "Bob" });
  const auth = { room: "HELLO", playerId: "host", sessionToken: host.sessionToken };

  assert.equal(clock.service.updateRoom({ room: "NOPE", playerId: "host", sessionToken: host.sessionToken })[1].code, "ROOM_NOT_FOUND");
  assert.equal(clock.service.updateRoom({ ...auth, playerId: "missing" })[1].code, "PLAYER_NOT_JOINED");
  assert.equal(clock.service.updateRoom({ ...auth, sessionToken: "nope" })[1].code, "INVALID_SESSION");
  assert.equal(clock.service.updateRoom({ ...auth, name: "Eve" })[1].code, "NAME_LOCKED");

  const claims = Array.from({ length: 520 }, (_, index) => `C${index}`);
  const [status, updated] = clock.service.updateRoom({
    ...auth,
    state: {
      depth: -4, x: 5000, y: "nope", facingX: 4, facingY: -0.2, mode: "fly", hidden: true,
      flashlightOn: false, frame: 9, persona: "eirys", appearanceId: "My Skin!", noise: 400,
      travelSpeed: 3, vx: 1, vy: -2,
    },
    world: {
      depth: 0,
      hunters: [{ id: "h1", x: 9999, y: 1, facingX: 8, state: "chase", alerted: 1, active: false }, { id: "" }],
      mannequins: [{ id: "m1", name: "N".repeat(80), x: 2, y: 3 }],
      rooms: [{ id: "kitchen", temperature: 500 }, { temperature: 10 }],
    },
    itemClaims: [
      { kind: "relay", id: "r1", depth: 0, changes: { collected: true, sabotaged: true } },
      { kind: "battery", id: "b1", depth: 0, changes: { collected: true } },
      { kind: "prop", id: "p1", depth: 0, changes: { interactionState: "open", fired: true, cooldownUntil: 50, x: 3, y: 4 } },
      { kind: "nope", id: "x", depth: 0, changes: { collected: true } },
      { kind: "relay", id: "late", depth: 3, changes: { collected: true } },
    ],
    discoveryClaims: claims,
  });
  assert.equal(status, 200);
  assert.equal(updated.player.depth, 0);
  assert.equal(updated.player.x, 1000);
  assert.equal(updated.player.y, 0);
  assert.equal(updated.player.facingX, 1);
  assert.equal(updated.player.mode, "idle");
  assert.equal(updated.player.flashlightOn, false);
  assert.equal(updated.player.frame, 3);
  assert.equal(updated.player.persona, "elise");
  assert.equal(updated.player.appearanceId, "MySkin");
  assert.equal(updated.player.noise, 100);
  assert.equal(updated.world.hunters[0].x, 1000);
  assert.equal(updated.world.hunters[0].facingX, 1);
  assert.equal(updated.world.hunters[0].alerted, true);
  assert.equal(updated.world.hunters[0].active, false);
  assert.equal(updated.world.mannequins[0].name.length, 48);
  assert.equal(updated.world.rooms[0].temperature, 100);
  assert.equal(updated.world.rooms.length, 1);
  assert.equal(updated.items["relay:r1"].sabotaged, true);
  assert.equal(updated.items["battery:b1"].collected, true);
  assert.equal(updated.items["prop:p1"].stimulusSeq, 1);
  assert.equal(updated.items["prop:p1"].cooldownUntil, 50);
  assert.equal(updated.items["relay:late"], undefined);
  assert.equal(updated.discoveredTotal, 512);
  assert.equal(updated.discoveredReset, true);

  const guestWorld = clock.service.updateRoom({
    room: "HELLO", playerId: "guest", sessionToken: guest.sessionToken,
    world: { depth: 0, hunters: [{ id: "guest-hunter", x: 1, y: 1 }] },
  });
  assert.equal(guestWorld[1].world.hunters[0].id, "h1");

  const paged = clock.service.updateRoom({
    ...auth,
    discoveryEpoch: updated.discoveredEpoch,
    discoverySince: updated.discoveredTotal,
    discoveryClaims: ["1234567890123", "C0", "NEXT", 4],
  })[1];
  assert.deepEqual(paged.discovered, ["NEXT"]);
  assert.equal(paged.discoveredReset, false);
  assert.equal(paged.discoveredTotal, 513);

  const reset = clock.service.updateRoom({ ...auth, discoveryEpoch: 99, claimDepth: 2 })[1];
  assert.equal(reset.activeDepth, 2);
  assert.deepEqual(reset.items, {});
  assert.deepEqual(reset.world.hunters, []);
  assert.equal(reset.discoveredReset, true);
  assert.equal(reset.discoveredTotal, 0);
});

test("only the host advances the depth, and by a bounded step once the room has one", () => {
  const { service } = serviceAt();
  const host = join(service, { room: "DEPTH", playerId: "host", name: "Ann" });
  const guest = join(service, { room: "DEPTH", playerId: "guest", name: "Bob" });
  const hostAuth = { room: "DEPTH", playerId: "host", sessionToken: host.sessionToken };
  const guestAuth = { room: "DEPTH", playerId: "guest", sessionToken: guest.sessionToken };
  service.updateRoom({ ...hostAuth, claimDepth: 1, world: { depth: 1, hunters: [{ id: "h1", x: 1, y: 1 }] },
    itemClaims: [{ kind: "battery", id: "b1", depth: 1, changes: { collected: true } }] });
  // a guest's claim (old clients still send one) changes nothing and is not an error
  const [status, guestView] = service.updateRoom({ ...guestAuth, claimDepth: 9_999 });
  assert.equal(status, 200);
  assert.equal(guestView.activeDepth, 1);
  assert.equal(guestView.world.hunters[0].id, "h1");
  assert.equal(guestView.items["battery:b1"].collected, true);
  // too large a jump from the host is ignored too
  assert.equal(service.updateRoom({ ...hostAuth, claimDepth: 1 + MAX_DEPTH_CLAIM_STEP + 1 })[1].activeDepth, 1);
  // the next floor still works, and the host's world for it is kept
  const next = service.updateRoom({ ...hostAuth, claimDepth: 2, world: { depth: 2, hunters: [{ id: "h2", x: 1, y: 1 }] } })[1];
  assert.equal(next.activeDepth, 2);
  assert.equal(next.world.hunters[0].id, "h2");
  assert.equal(depthClaimAllowed(0, 40), true, "a room with no depth yet takes the host's starting depth");
  assert.equal(depthClaimAllowed(5, 5), false);
});

test("only the host changes settings, and an invalid patch leaves them alone", () => {
  const { service } = serviceAt();
  const host = join(service, { room: "HELLO", playerId: "host", name: "Ann" });
  const guest = join(service, { room: "HELLO", playerId: "guest", name: "Bob" });
  const auth = { room: "HELLO", playerId: "host", sessionToken: host.sessionToken };
  assert.equal(service.updateRoomSettings({ room: "HELLO", playerId: "guest", sessionToken: guest.sessionToken, settings: { difficulty: "nightmare" } })[1].code, "HOST_ONLY");
  assert.equal(service.updateRoomSettings({ ...auth, settings: [] })[1].code, "INVALID_SETTINGS");
  assert.equal(service.updateRoomSettings({ ...auth, settings: { nope: true } })[1].code, "INVALID_SETTINGS");
  const applied = service.updateRoomSettings({ ...auth, settings: { difficulty: "nightmare", fakeRunners: true } });
  assert.equal(applied[0], 200);
  assert.equal(applied[1].settings.difficulty, "nightmare");
  assert.equal(applied[1].settings.fakeRunners, true);
  assert.equal(applied[1].settings.mode, "lastRoute");
  assert.equal(service.updateRoomSettings({ ...auth, settings: { difficulty: "easy" } })[0], 400);
  assert.equal(service.joinRoom({ room: "HELLO", playerId: "host", name: "Ann", sessionToken: host.sessionToken })[1].settings.difficulty, "nightmare");
});

test("hide and seek starts in hiding, holds the seeker until the timer, and tagging does not complete it", () => {
  const clock = serviceAt(5_000);
  const { service } = clock;
  const host = join(service, { room: "HIDE1", playerId: "host", name: "Ann" });
  const hostAuth = { room: "HIDE1", playerId: "host", sessionToken: host.sessionToken };
  assert.equal(service.startRoomRound(hostAuth)[1].code, "WRONG_MODE");
  assert.equal(service.updateRoomSettings({ ...hostAuth, settings: { mode: "hideAndSeek" } })[0], 200);
  assert.equal(service.startRoomRound(hostAuth)[1].code, "NOT_ENOUGH_PLAYERS");

  const guest = join(service, { room: "HIDE1", playerId: "guest", name: "Bob" });
  assert.equal(service.startRoomRound({ room: "HIDE1", playerId: "guest", sessionToken: guest.sessionToken })[1].code, "HOST_ONLY");
  const started = service.startRoomRound(hostAuth);
  assert.equal(started[0], 200);
  assert.equal(started[1].round.phase, "hiding");
  assert.equal(started[1].round.roundId, 1);
  assert.equal(started[1].round.hideEndsAt, clock.at() + 30_000);
  assert.ok(["host", "guest"].includes(started[1].round.seekerId));
  const seekerIsHost = started[1].player.role === "seeker";
  const seekerAuth = seekerIsHost
    ? hostAuth
    : { room: "HIDE1", playerId: "guest", sessionToken: guest.sessionToken };
  const hiderAuth = seekerIsHost
    ? { room: "HIDE1", playerId: "guest", sessionToken: guest.sessionToken }
    : hostAuth;
  assert.equal(service.updateRoom({ ...seekerAuth, state: { roundId: 1 } })[1].player.released, false);
  assert.equal(service.updateRoom({ ...hiderAuth, state: { roundId: 1, tagged: true } })[1].player.released, true);
  clock.advance(29_999);
  assert.equal(service.updateRoom({ ...seekerAuth, state: { roundId: 1 } })[1].player.released, false);
  clock.advance(1);
  const afterTimer = service.updateRoom({ ...seekerAuth, state: { roundId: 1, tagged: true } })[1];
  assert.equal(afterTimer.player.released, true);
  assert.equal(afterTimer.round.phase, "hiding");
  const finished = service.updateRoom({ ...hostAuth, state: { roundId: 1, tagged: true } })[1];
  assert.equal(finished.round.phase, "hiding");

  const again = service.startRoomRound(hostAuth)[1];
  assert.equal(again.round.roundId, 2);
  assert.equal(again.player.tagged, false);
  assert.equal(again.players[0].tagged, false);
});

test("a solo hider against an NPC seeker is enough to start", () => {
  const { service } = serviceAt();
  const host = join(service, { room: "SOLO1", playerId: "host", name: "Ann" });
  const auth = { room: "SOLO1", playerId: "host", sessionToken: host.sessionToken };
  service.updateRoomSettings({ ...auth, settings: { mode: "hideAndSeek", seekerNpcs: 1 } });
  const started = service.startRoomRound({ ...auth, role: "hider" });
  assert.equal(started[0], 200, JSON.stringify(started[1]));
  assert.equal(started[1].round.seekerId, "npc:seeker:0");
  assert.equal(started[1].round.seekerSpawnIndex, -1);
  assert.equal(started[1].player.role, "hider");
});

test("launch, abandon, custom map and dialog follow the host", () => {
  const { service } = serviceAt();
  const host = join(service, { room: "OFFIC1", playerId: "host", name: "Ann" });
  const guest = join(service, { room: "OFFIC1", playerId: "guest", name: "Bob" });
  const auth = { room: "OFFIC1", playerId: "host", sessionToken: host.sessionToken };
  service.updateRoom({
    ...auth,
    itemClaims: [{ kind: "battery", id: "b1", depth: 0, changes: { collected: true } }],
  });
  assert.equal(service.startOfficeSolo, undefined, "Office Mode launch was removed (task 018)");
  const launched = service.startDefaultRoute(auth);
  assert.equal(launched[0], 200);
  assert.equal(launched[1].launchId, 1);
  assert.deepEqual(launched[1].items, {});
  assert.equal(service.startDefaultRoute(auth)[1].launchId, 2);

  // The removed Office Mode no longer locks the room or drops guests: an old client's
  // "officeSurvivor" falls back to Last Route and everyone stays in.
  service.updateRoomSettings({ ...auth, settings: { mode: "hideAndSeek" } });
  const legacy = service.updateRoomSettings({ ...auth, settings: { mode: "officeSurvivor" } });
  assert.equal(legacy[0], 200);
  assert.equal(legacy[1].settings.mode, "lastRoute");
  assert.equal(service.startDefaultRoute(auth)[1].launchId, 3);
  assert.equal(service.updateRoom({ room: "OFFIC1", playerId: "guest", sessionToken: guest.sessionToken })[0], 200);
  join(service, { room: "OFFIC1", playerId: "late", name: "Cam" });
  const rejoined = join(service, { room: "OFFIC1", playerId: "host", name: "Ann", sessionToken: host.sessionToken });
  assert.equal(rejoined.sessionToken, host.sessionToken);

  assert.equal(service.setCustomMap({ ...auth, filename: "../x.png" })[1].code, "INVALID_FILENAME");
  const mapped = service.setCustomMap({ ...auth, filename: "Floor.PNG" });
  assert.equal(mapped[0], 200);
  assert.equal(mapped[1].customMapFile, "Floor.PNG");
  assert.equal(mapped[1].customMapId, 1);
  assert.equal(service.startDialog({ ...auth, file: "../x.yaml" })[1].code, "INVALID_FILENAME");
  assert.equal(service.startDialog({ room: "OFFIC1", playerId: "late", sessionToken: service.rooms.get("OFFIC1").players.find((player) => player.id === "late").sessionToken, file: "intro.yaml" })[1].code, "HOST_ONLY");
  const dialog = service.startDialog({ ...auth, file: "intro.yaml#start" });
  assert.equal(dialog[1].dialog.file, "intro.yaml#start");
  assert.equal(dialog[1].dialog.dialogId, 1);
  assert.equal(service.advanceDialog({ ...auth, dialogId: 9, nodeId: "a", ended: true })[1].code, "STALE_DIALOG");
  const step = service.advanceDialog({ ...auth, dialogId: 1, nodeId: " line ", ended: true });
  assert.equal(step[1].dialog.nodeId, "line");
  assert.equal(step[1].dialog.ended, true);
  assert.equal(step[1].dialog.stepId, 1);

  const abandoned = service.abandonRoute(auth);
  assert.equal(abandoned[0], 200);
  assert.equal(abandoned[1].abandonId, 1);
  assert.equal(abandoned[1].customMapFile, "");
  assert.equal(abandoned[1].customMapId, 1);
  assert.equal(abandoned[1].dialog.dialogId, 0);
  assert.deepEqual(abandoned[1].items, {});
  join(service, { room: "OFFIC1", playerId: "after", name: "Dee" });
});

test("leaving hands the room to the next player, and the last one out closes it", () => {
  const { service } = serviceAt();
  assert.equal(service.leaveRoom({ room: "NONE", playerId: "p", sessionToken: "x" })[1].ok, true);
  const host = join(service, { room: "LEAVE1", playerId: "host", name: "Ann" });
  const guest = join(service, { room: "LEAVE1", playerId: "guest", name: "Bob" });
  assert.equal(service.leaveRoom({ room: "LEAVE1", playerId: "host", sessionToken: "nope" })[1].code, "INVALID_SESSION");
  assert.equal(service.leaveRoom({ room: "LEAVE1", playerId: "host", sessionToken: host.sessionToken })[1].ok, true);
  const remaining = service.updateRoom({ room: "LEAVE1", playerId: "guest", sessionToken: guest.sessionToken });
  assert.equal(remaining[0], 200);
  assert.equal(remaining[1].isHost, true);
  assert.equal(remaining[1].roomSize, 1);
  service.leaveRoom({ room: "LEAVE1", playerId: "guest", sessionToken: guest.sessionToken });
  assert.equal(service.rooms.has("LEAVE1"), false);

  const fresh = join(service, { room: "LEAVE1", playerId: "host", name: "Ann" });
  assert.equal(fresh.roomSize, 1);
});

test("item records are capped per room, but existing keys still update and a new depth clears them", () => {
  const { service } = serviceAt();
  const host = join(service, { room: "ITEMS", playerId: "host", name: "Ann" });
  const auth = { room: "ITEMS", playerId: "host", sessionToken: host.sessionToken };
  let next = 0, items = {};
  while (Object.keys(items).length < ITEM_TOTAL_LIMIT) {
    const claims = [];
    for (let i = 0; i < ITEM_LIMIT; i++) claims.push({ kind: "battery", id: "b" + (next + i), depth: 0, changes: { collected: true } });
    next += ITEM_LIMIT;
    items = service.updateRoom({ ...auth, itemClaims: claims })[1].items;
  }
  assert.equal(Object.keys(items).length, ITEM_TOTAL_LIMIT);
  const afterExtra = service.updateRoom({ ...auth, itemClaims: [{ kind: "battery", id: "b" + next, depth: 0, changes: { collected: true } }] })[1];
  assert.equal(Object.keys(afterExtra.items).length, ITEM_TOTAL_LIMIT);
  assert.equal(afterExtra.items["battery:b" + next], undefined);
  assert.equal(service.updateRoom({ ...auth, itemClaims: [{ kind: "battery", id: "b0", depth: 0, changes: { collected: true } }] })[1].items["battery:b0"].collected, true);
  const advanced = service.updateRoom({ ...auth, claimDepth: 1, itemClaims: [{ kind: "battery", id: "b0", depth: 1, changes: { collected: true } }] })[1];
  assert.equal(advanced.activeDepth, 1);
  assert.equal(Object.keys(advanced.items).length, 1);
});

test("idle players and rooms expire, and the process stops at 512 rooms", () => {
  const clock = serviceAt(0, { playerTtlMs: 1_000, roomTtlMs: 5_000 });
  const host = join(clock.service, { room: "HELLO", playerId: "host", name: "Ann" });
  clock.advance(900);
  const guest = join(clock.service, { room: "HELLO", playerId: "guest", name: "Bob" });
  clock.advance(101);
  const pruned = clock.service.updateRoom({ room: "HELLO", playerId: "guest", sessionToken: guest.sessionToken });
  assert.equal(pruned[1].roomSize, 1);
  assert.equal(pruned[1].isHost, true);
  assert.equal(pruned[1].player.name, "BOB");

  const roomClock = serviceAt(0, { playerTtlMs: 10_000, roomTtlMs: 5_000 });
  const seeded = join(roomClock.service, { room: "WORLD1", playerId: "host", name: "Ann" });
  roomClock.advance(5_001);
  roomClock.service.cleanup();
  const recreated = join(roomClock.service, { room: "WORLD1", playerId: "host", name: "Ann" });
  assert.notEqual(recreated.worldSeed, seeded.worldSeed);

  const busy = serviceAt();
  for (let index = 0; index < 512; index += 1) {
    const code = `R${String(index).padStart(4, "0")}`;
    join(busy.service, { room: code, playerId: "p", name: "Ann" });
  }
  assert.equal(busy.service.joinRoom({ room: "EXTRA1", playerId: "p", name: "Ann" })[1].code, "SERVER_BUSY");
  const stillThere = join(busy.service, { room: "R0000", playerId: "q", name: "Bob" });
  assert.equal(stillThere.roomSize, 2);
  assert.equal(host.room, "HELLO");
});
