import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { createSocket } from "node:dgram";
import {
  createEnetRelay,
  decodeRelayFrame,
  encodeRelayFrame,
  HANDOFF_FAILED_MESSAGE,
  RelayFrame,
} from "./enet_relay.mjs";

async function udpSocket() {
  const socket = createSocket("udp4");
  socket.bind(0, "127.0.0.1");
  await once(socket, "listening");
  return socket;
}

function receive(socket, timeoutMs = 1000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.off("message", onMessage);
      reject(new Error("UDP receive timed out"));
    }, timeoutMs);
    const onMessage = (message, remote) => {
      clearTimeout(timer);
      resolve({ frame: decodeRelayFrame(message), remote });
    };
    socket.once("message", onMessage);
  });
}

async function waitFor(check, timeoutMs = 1000) {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error("condition not reached");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

function send(socket, port, frame) {
  socket.send(frame, port, "127.0.0.1");
}

test("authenticated relay tunnels ENet datagrams through one public UDP port", async () => {
  const room = { code: "ABCDE", token: "host-secret" };
  const relay = createEnetRelay({
    roomByCode: (code) => code === room.code ? room : null,
    logger: { log() {}, error() {} },
  });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const relayPort = relay.address().port;
  const host = await udpSocket();
  const guest = await udpSocket();
  const attacker = await udpSocket();

  try {
    send(attacker, relayPort, encodeRelayFrame(
      RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from("wrong-secret")));
    assert.equal((await receive(attacker)).frame.type, RelayFrame.ERROR);

    send(host, relayPort, encodeRelayFrame(
      RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from(room.token)));
    assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);

    send(guest, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("guest-01")));
    const joined = (await receive(guest)).frame;
    assert.equal(joined.type, RelayFrame.GUEST_ACK);
    assert.notEqual(joined.sessionId, 0);

    // A retrying client may rotate its nonce, but its one UDP endpoint must
    // never consume another room slot for that retry.
    for (let index = 0; index < 20; index += 1) {
      send(guest, relayPort, encodeRelayFrame(
        RelayFrame.GUEST_JOIN, 0, room.code,
        Buffer.from(`retry-${String(index).padStart(3, "0")}`)));
      const retried = (await receive(guest)).frame;
      assert.equal(retried.type, RelayFrame.GUEST_ACK);
      assert.equal(retried.sessionId, joined.sessionId);
    }

    const request = Buffer.from("raw-enet-connect-packet");
    send(guest, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_DATA, joined.sessionId, room.code, request));
    const atHost = (await receive(host)).frame;
    assert.equal(atHost.type, RelayFrame.GUEST_DATA);
    assert.equal(atHost.sessionId, joined.sessionId);
    assert.deepEqual(atHost.payload, request);

    const response = Buffer.from("raw-enet-ack-packet");
    send(host, relayPort, encodeRelayFrame(
      RelayFrame.HOST_DATA, joined.sessionId, room.code, response));
    const atGuest = (await receive(guest)).frame;
    assert.equal(atGuest.type, RelayFrame.HOST_DATA);
    assert.deepEqual(atGuest.payload, response);

    const { counters, socketBuffers, ...status } = relay.relayStatus();
    assert.deepEqual(status, {
      enabled: true,
      listening: true,
      port: relayPort,
      hosts: 1,
      sessions: 1,
      error: undefined,
    });
    assert.equal(counters.forwardedGuest, 1);
    assert.equal(counters.forwardedHost, 1);
    assert.equal(counters.forwardedBytes, request.length + response.length);
    assert.ok(socketBuffers.receive > 0 && socketBuffers.send > 0, "buffer sizes are reported");

    // Every datagram the relay refuses is counted, so a deploy shows where co-op traffic goes.
    send(attacker, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_DATA, joined.sessionId, room.code, request));
    send(attacker, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_DATA, joined.sessionId ^ 1, room.code, request));
    send(attacker, relayPort, Buffer.from("not a relay packet"));
    await waitFor(() => {
      const seen = relay.relayStatus().counters;
      return seen.droppedWrongEndpoint === 1 && seen.droppedUnknownSession === 1 && seen.droppedInvalid === 1;
    });
    assert.equal(relay.relayStatus().counters.forwardedGuest, 1, "refused frames are not forwarded");
  } finally {
    host.close();
    guest.close();
    attacker.close();
    relay.close();
    await Promise.all([once(host, "close"), once(guest, "close"),
      once(attacker, "close"), once(relay, "close")]);
  }
});

test("relay protocol rejects malformed and oversized frames", () => {
  assert.equal(decodeRelayFrame(Buffer.from("not a relay packet")), null);
  assert.throws(() => encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, "ab!12"),
    /INVALID RELAY ROOM CODE/);
  const oversized = Buffer.alloc(1473);
  Buffer.from("LRR1").copy(oversized);
  oversized.write("ABCDE", 9, 5, "ascii");
  assert.equal(decodeRelayFrame(oversized), null);
});

test("a host handing over on purpose promotes the oldest guest; the other sessions survive", async () => {
  let timestamp = 0;
  const room = { code: "ABCDE", token: "host-secret" };
  const relay = createEnetRelay({
    roomByCode: (code) => code === room.code ? room : null,
    promoteRoomHost: () => {
      room.token = "migrated-host-secret";
      return { token: room.token };
    },
    now: () => timestamp,
    hostTtlMs: 100,
    sessionTtlMs: 1000,
    logger: { log() {}, error() {} },
  });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const relayPort = relay.address().port;
  const host = await udpSocket();
  const firstGuest = await udpSocket();
  const secondGuest = await udpSocket();

  try {
    send(host, relayPort, encodeRelayFrame(
      RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from("host-secret")));
    assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);
    send(firstGuest, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("first-guest")));
    const firstSession = (await receive(firstGuest)).frame.sessionId;
    timestamp += 1;
    send(secondGuest, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("second-guest")));
    const secondSession = (await receive(secondGuest)).frame.sessionId;

    timestamp = 50;
    const promotionPromise = receive(firstGuest);
    const migrationPromise = receive(secondGuest);
    relay.requestHostMigration(room.code);   // release-host: the only path to migration now
    const promotion = (await promotionPromise).frame;
    assert.equal(promotion.type, RelayFrame.HOST_PROMOTE);
    assert.equal(promotion.sessionId, firstSession);
    assert.equal(promotion.payload.toString("utf8"), "migrated-host-secret");
    assert.equal((await migrationPromise).frame.type, RelayFrame.HOST_MIGRATING);

    const acknowledgementPromise = receive(firstGuest);
    const rejoinPromise = receive(secondGuest);
    send(firstGuest, relayPort, encodeRelayFrame(
      RelayFrame.HOST_REGISTER, 0, room.code, promotion.payload));
    assert.equal((await acknowledgementPromise).frame.type, RelayFrame.HOST_ACK);
    // The new host registration sends another migration notice. Consume it
    // before proving that the old guest session now routes to the replacement.
    assert.equal((await rejoinPromise).frame.type, RelayFrame.HOST_MIGRATING);
    const request = Buffer.from("enet-reconnect-packet");
    const replacementPacketPromise = receive(firstGuest);
    send(secondGuest, relayPort, encodeRelayFrame(
      RelayFrame.GUEST_DATA, secondSession, room.code, request));
    const atReplacement = (await replacementPacketPromise).frame;
    assert.equal(atReplacement.type, RelayFrame.GUEST_DATA);
    assert.equal(atReplacement.sessionId, secondSession);
    assert.deepEqual(atReplacement.payload, request);
  } finally {
    host.close();
    firstGuest.close();
    secondGuest.close();
    relay.close();
    await Promise.all([once(host, "close"), once(firstGuest, "close"),
      once(secondGuest, "close"), once(relay, "close")]);
  }
});


async function handoffFixture(options = {}) {
  const clock = { t: 0 };
  const closed = [];
  const room = { code: "HNDOF", token: "host-secret" };
  let promotions = 0;
  const relay = createEnetRelay({
    roomByCode: (code) => (code === room.code && !closed.includes(code) ? room : null),
    onRoomClosed: (code, reason) => closed.push(code, reason),
    promoteRoomHost: () => {
      promotions += 1;
      room.token = `successor-secret-${promotions}`;
      return { token: room.token };
    },
    now: () => clock.t,
    hostTtlMs: 15_000,
    hostAliveTtlMs: 1_000,
    sessionTtlMs: 45_000,
    handoffTtlMs: 5_000,
    logger: { log() {}, error() {} },
    ...options,
  });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const port = relay.address().port;
  const host = await udpSocket();
  const successor = await udpSocket();
  const other = await udpSocket();
  send(host, port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from("host-secret")));
  assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);
  send(successor, port, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("successor-nonce")));
  const successorSession = (await receive(successor)).frame.sessionId;
  clock.t += 1;
  send(other, port, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("other-guest-nonce")));
  const otherSession = (await receive(other)).frame.sessionId;
  return {
    clock, closed, room, relay, port, host, successor, other, successorSession, otherSession,
    promotions: () => promotions,
    async close() {
      for (const socket of [host, successor, other, relay]) socket.close();
      await Promise.all([host, successor, other, relay].map((socket) => once(socket, "close")));
    },
  };
}

test("a prepared handoff promotes the NAMED session with an epoch; the others get a tagged ready notice", async () => {
  const f = await handoffFixture();
  try {
    // the host names the SECOND guest (not the oldest): the relay must follow the host's checkpoint choice
    const promotionPromise = receive(f.other);
    const waitingPromise = receive(f.successor);
    const result = f.relay.requestHostHandoff(f.room.code, f.otherSession, 7);
    assert.deepEqual(result, { migrating: true });
    const promotion = (await promotionPromise).frame;
    assert.equal(promotion.type, RelayFrame.HOST_PROMOTE);
    assert.equal(promotion.sessionId, f.otherSession);
    assert.equal(promotion.payload.toString("utf8"), `LRH1|7|${f.room.token}`);
    const waiting = (await waitingPromise).frame;
    assert.equal(waiting.type, RelayFrame.HOST_MIGRATING);
    assert.equal(waiting.sessionId, f.successorSession);
    assert.equal(waiting.payload.toString("utf8"), "LRH1|7", "the waiting notice never carries the token");

    // the same release again is idempotent: no second token rotation
    const again = receive(f.other);
    assert.deepEqual(f.relay.requestHostHandoff(f.room.code, f.otherSession, 7), { migrating: true });
    assert.equal((await again).frame.type, RelayFrame.HOST_PROMOTE);
    assert.equal(f.promotions(), 1);
    // a different epoch/candidate while one is in flight is refused
    assert.equal(f.relay.requestHostHandoff(f.room.code, f.successorSession, 8).migrating, false);

    // the old host's token is dead: its registration is refused
    const refused = receive(f.host);
    send(f.host, f.port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, f.room.code, Buffer.from("host-secret")));
    assert.equal((await refused).frame.type, RelayFrame.ERROR);

    // the successor registers from its guest socket with the promoted token
    const ack = receive(f.other);
    const ready = receive(f.successor);
    send(f.other, f.port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, f.room.code,
      Buffer.from(f.room.token)));
    assert.equal((await ack).frame.type, RelayFrame.HOST_ACK);
    const readyFrame = (await ready).frame;
    assert.equal(readyFrame.type, RelayFrame.HOST_MIGRATING);
    assert.equal(readyFrame.sessionId, f.successorSession);
    assert.equal(readyFrame.payload.toString("utf8"), "LRH1|7|ready");

    // the remaining guest's datagrams now reach the successor
    const routed = receive(f.other);
    send(f.successor, f.port, encodeRelayFrame(RelayFrame.GUEST_DATA, f.successorSession, f.room.code,
      Buffer.from("enet-connect")));
    const atSuccessor = (await routed).frame;
    assert.equal(atSuccessor.type, RelayFrame.GUEST_DATA);
    assert.equal(atSuccessor.sessionId, f.successorSession);
    assert.deepEqual(f.closed, []);
  } finally {
    await f.close();
  }
});

test("a handoff to an unknown or stale session is refused and the live host keeps its room", async () => {
  const f = await handoffFixture();
  try {
    assert.equal(f.relay.requestHostHandoff(f.room.code, 12345, 1).migrating, false);
    assert.equal(f.relay.requestHostHandoff(f.room.code, f.otherSession, 0).migrating, false);
    assert.equal(f.relay.requestHostHandoff(f.room.code, "x", 1).migrating, false);
    assert.equal(f.promotions(), 0, "a refused handoff never rotates the token");
    assert.equal(f.room.token, "host-secret");
    // the host still forwards guest traffic
    const forwarded = receive(f.host);
    send(f.other, f.port, encodeRelayFrame(RelayFrame.GUEST_DATA, f.otherSession, f.room.code, Buffer.from("still-live")));
    assert.equal((await forwarded).frame.type, RelayFrame.GUEST_DATA);
    // a session from long ago is stale
    f.clock.t += 60_000;
    send(f.host, f.port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, f.room.code, Buffer.from("host-secret")));
    await receive(f.host);
    assert.equal(f.relay.requestHostHandoff(f.room.code, f.otherSession, 2).migrating, false);
    assert.deepEqual(f.closed, []);
  } finally {
    await f.close();
  }
});

test("a successor that never registers closes the room after the handoff deadline: no guess election", async () => {
  const f = await handoffFixture();
  try {
    const promotion = receive(f.other);
    assert.equal(f.relay.requestHostHandoff(f.room.code, f.otherSession, 3).migrating, true);
    await promotion;
    f.clock.t += 4_000;
    f.relay.pruneNow();
    assert.deepEqual(f.closed, [], "still inside the deadline");
    // drop the re-announcement the 4 s prune sent, then look for the closure
    const seen = [];
    const onFrame = (message) => seen.push(decodeRelayFrame(message));
    f.successor.on("message", onFrame);
    f.clock.t += 2_000;
    f.relay.pruneNow();
    assert.deepEqual(f.closed, [f.room.code, HANDOFF_FAILED_MESSAGE]);
    await waitFor(() => seen.some((frame) => frame.type === RelayFrame.HOST_LEFT));
    f.successor.off("message", onFrame);
    const frame = seen.find((entry) => entry.type === RelayFrame.HOST_LEFT);
    assert.equal(frame.type, RelayFrame.HOST_LEFT);
    assert.equal(frame.payload.toString("utf8"), HANDOFF_FAILED_MESSAGE);
    assert.equal(f.promotions(), 1, "the room was not handed to another guest");
  } finally {
    await f.close();
  }
});

test("a host that quits or dies closes the room: guests get HOST_LEFT, the directory forgets the code", async () => {
  let timestamp = 0;
  const closed = [];
  const room = { code: "ABCDE", token: "host-secret" };
  const relay = createEnetRelay({
    roomByCode: (code) => (code === room.code && !closed.includes(code) ? room : null),
    onRoomClosed: (code, reason) => closed.push(code, reason),
    promoteRoomHost: () => ({ token: "never" }),
    now: () => timestamp,
    hostTtlMs: 15_000,
    hostAliveTtlMs: 1_000,
    logger: { log() {}, error() {} },
  });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const relayPort = relay.address().port;
  const host = await udpSocket();
  const keepAlive = await udpSocket();   // the host's background thread uses its own socket
  const guest = await udpSocket();
  try {
    send(host, relayPort, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from("host-secret")));
    assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);
    send(guest, relayPort, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("guest-nonce")));
    assert.equal((await receive(guest)).frame.type, RelayFrame.GUEST_ACK);

    // the main loop freezes for 5 s, but the keep-alive thread goes on every 250 ms: the room stays
    for (let ms = 250; ms <= 5000; ms += 250) {
      timestamp = ms;
      send(keepAlive, relayPort, encodeRelayFrame(RelayFrame.HOST_ALIVE, 0, room.code, Buffer.from("host-secret")));
      await new Promise((resolve) => setTimeout(resolve, 5));
      relay.pruneNow();
    }
    assert.deepEqual(closed, [], "a frozen host whose keep-alive kept going lost its room");
    assert.equal(relay.relayStatus().hosts, 1);
    // a forged keep-alive (wrong token) keeps nothing alive
    send(keepAlive, relayPort, encodeRelayFrame(RelayFrame.HOST_ALIVE, 0, room.code, Buffer.from("guess")));

    // killed: the keep-alive stops; within 1 s (plus one 250 ms prune) the room is closed
    const leftPromise = receive(guest);
    timestamp = 5000 + 1001;
    relay.pruneNow();
    const left = (await leftPromise).frame;
    assert.equal(left.type, RelayFrame.HOST_LEFT);
    assert.equal(left.payload.toString(), "HOST LEFT // ROOM CLOSED");
    assert.equal(closed[0], "ABCDE", "the directory was not told to forget the code");
    assert.equal(relay.relayStatus().sessions, 0);
    // a later join with that code finds nothing
    send(guest, relayPort, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, room.code, Buffer.from("guest-nonce-2")));
    const after = await receive(guest);
    assert.ok(after.frame.type === RelayFrame.ERROR || after.frame.type === RelayFrame.HOST_LEFT,
      `a join after the host left got frame ${after.frame.type}`);
  } finally {
    for (const socket of [host, keepAlive, guest]) socket.close();
    relay.close();
    await once(relay, "close");
  }
});

test("an older host (no keep-alive) keeps the long timeout, and closeRoom ends a room at once", async () => {
  let timestamp = 0;
  const closed = [];
  const room = { code: "ABCDE", token: "host-secret" };
  const relay = createEnetRelay({
    roomByCode: (code) => (code === room.code ? room : null),
    onRoomClosed: (code) => closed.push(code),
    now: () => timestamp, hostTtlMs: 15_000, hostAliveTtlMs: 1_000, logger: { log() {}, error() {} },
  });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const host = await udpSocket();
  try {
    send(host, relay.address().port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, room.code, Buffer.from("host-secret")));
    assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);
    timestamp = 5_000;
    relay.pruneNow();
    assert.deepEqual(closed, [], "an older build's host (registers every 3 s) was dropped on the 1 s timeout");
    assert.ok(relay.closeRoom("ABCDE"));
    assert.deepEqual(closed, ["ABCDE"]);
  } finally {
    host.close();
    relay.close();
    await once(relay, "close");
  }
});

test("server: an explicit quit invalidates the code at once; a killed relay host's code is gone within ~1 s", async () => {
  const { createLastRouteServer } = await import("./server.mjs");
  const { mkdtemp, rm } = await import("node:fs/promises");
  const { join } = await import("node:path");
  const { tmpdir } = await import("node:os");
  const root = await mkdtemp(join(tmpdir(), "host-left-"));
  const server = createLastRouteServer({ accountStorePath: join(root, "a.json"), moderationStorePath: join(root, "m.json"),
    accountRateLimitMax: 10_000, apiRateLimitMax: 10_000, logger: { log() {}, warn() {}, error() {} } });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const relay = server.startEnetRelay({ host: "127.0.0.1", port: 0 });
  await once(relay, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  const post = async (path, body) => {
    const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  };
  const host = await udpSocket();
  try {
    const first = await post("/api/enet/rooms/register", { code: "ABCDE", port: 7000 });
    assert.equal(first.status, 201);
    assert.equal((await post("/api/enet/rooms/unregister", { code: "ABCDE", token: first.body.token })).status, 200);
    assert.equal((await fetch(`${url}/api/enet/rooms/ABCDE`)).status, 404, "a quit room still resolved");

    const second = await post("/api/enet/rooms/register", { code: "BCDEF", port: 7001 });
    send(host, relay.address().port, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, "BCDEF", Buffer.from(second.body.token)));
    assert.equal((await receive(host)).frame.type, RelayFrame.HOST_ACK);
    send(host, relay.address().port, encodeRelayFrame(RelayFrame.HOST_ALIVE, 0, "BCDEF", Buffer.from(second.body.token)));
    await new Promise((resolve) => setTimeout(resolve, 300));
    assert.equal((await fetch(`${url}/api/enet/rooms/BCDEF`)).status, 200);
    const killedAt = Date.now();   // no more keep-alives from here: the process is gone
    let gone = 0;
    while (Date.now() - killedAt < 3000) {
      if ((await fetch(`${url}/api/enet/rooms/BCDEF`)).status === 404) { gone = Date.now() - killedAt; break; }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.ok(gone > 0 && gone <= 1400, `a killed host's room took ${gone || ">3000"} ms to drop (want about 1 s)`);
  } finally {
    host.close();
    server.close();
    await once(server, "close");
    await rm(root, { recursive: true });
  }
});
