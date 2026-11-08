import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createLastRouteServer } from "./server.mjs";
import {
  createPublicRoute, PUBLIC_ITEM_RESPAWN_MS, publicHunterTargetCount, sanitizeInventory, sanitizePosition, utcDayKey,
} from "./public_route.mjs";

const KEY_A = "a".repeat(24);
const KEY_B = "b".repeat(24);

test("hunter target scales with players: 1 + floor((n-1)/2), capped at 8", () => {
  assert.deepEqual([1, 2, 3, 4, 5, 6, 15, 16, 40].map(publicHunterTargetCount), [1, 1, 2, 2, 3, 3, 8, 8, 8]);
});

test("UTC day key rolls over at 00:00 UTC", () => {
  assert.equal(utcDayKey(Date.UTC(2026, 9, 26, 23, 59, 59)), "2026-10-26");
  assert.equal(utcDayKey(Date.UTC(2026, 9, 27, 0, 0, 0)), "2026-10-27");
});

test("positions and inventories are clamped and sanitised", () => {
  assert.equal(sanitizePosition({ x: "NaN", z: 1 }), null);
  assert.equal(sanitizePosition({ x: 5e9, z: 1 }), null);
  assert.deepEqual(sanitizePosition({ x: 3.14159, z: -2 }), { x: 3.14, z: -2 });
  assert.deepEqual(sanitizeInventory([{ id: "flashlight", durability: 5000 }, { id: "BAD ID" }, null]),
    [{ id: "flashlight", durability: 1000, slot: 0 }]);
});

test("join returns the stable seed and a resume record after a crash, within the TTL", () => {
  let clock = 1_000_000;
  const route = createPublicRoute({ now: () => clock });
  const first = route.join({ playerKey: KEY_A, name: "Aki" }, { address: "1.1.1.1" }).body;
  assert.equal(first.code, "PUBLC");
  assert.equal(first.resume, null);
  clock += 1_000;
  assert.equal(route.post({ sessionId: first.sessionId, position: { x: 12.5, z: -40 }, inventory: [{ id: "taser", durability: 3 }] }).status, 200);
  clock += 5_000; // the client crashes: no leave call, the session just goes quiet
  clock += 30_000;
  const second = route.join({ playerKey: KEY_A, name: "Aki" }, { address: "1.1.1.1" }).body;
  assert.equal(second.seed, first.seed);
  assert.deepEqual(second.resume.position, { x: 12.5, z: -40 });
  assert.equal(second.resume.inventory[0].id, "taser");
  clock += 7 * 60 * 60 * 1000; // resume TTL is six hours
  const third = route.join({ playerKey: KEY_A, name: "Aki" }, { address: "1.1.1.1" }).body;
  assert.equal(third.resume, null);
});

test("only the ENet host-token holder can post snapshots and claims; items respawn, relays do not", () => {
  let clock = 5_000_000;
  const route = createPublicRoute({ now: () => clock });
  const host = route.join({ playerKey: KEY_A, name: "A" }, { address: "x" }).body;
  const guest = route.join({ playerKey: KEY_B, name: "B" }, { address: "y" }).body;
  const claims = [{ id: "1:0:0:PICKUP:RELAY:0" }, { id: "1:0:0:PICKUP:BATTERY:3" }, { id: "not an id" }];
  clock += 1000;
  route.post({ sessionId: guest.sessionId, snapshot: "forged", claims }, { hostTokenValid: false });
  assert.equal(route.status().relaysCollected, 0, "a guest must not forge claims");
  clock += 1000;
  const posted = route.post({ sessionId: host.sessionId, snapshot: "(snap)", claims }, { hostTokenValid: true }).body;
  assert.equal(posted.relaysCollected, 1);
  assert.equal(posted.hunterTargetCount, 1);
  clock += 1000;
  assert.equal(route.status().playersOnline, 2);
  assert.ok(route._state.snapshot === "(snap)");
  // the authority keeps posting (every few seconds); respawns come back on the first post after the timer
  let again;
  const respawned = [];
  for (let elapsed = 0; elapsed <= PUBLIC_ITEM_RESPAWN_MS + 10_000; elapsed += 5_000) {
    clock += 5_000;
    again = route.post({ sessionId: host.sessionId }, { hostTokenValid: true }).body;
    respawned.push(...again.respawned);
  }
  assert.deepEqual(respawned, ["1:0:0:PICKUP:BATTERY:3"]);
  assert.equal(again.relaysCollected, 1);
  const lateJoin = route.join({ playerKey: "c".repeat(24), name: "C" }, { address: "z" }).body;
  assert.deepEqual(lateJoin.claimed, ["1:0:0:PICKUP:RELAY:0"]);
  assert.equal(lateJoin.snapshot, "(snap)");
});

test("state posts are rate limited per session and joins per address", () => {
  let clock = 1;
  const route = createPublicRoute({ now: () => clock });
  const joined = route.join({ playerKey: KEY_A }, { address: "r" }).body;
  clock += 1000;
  assert.equal(route.post({ sessionId: joined.sessionId, position: { x: 1, z: 1 } }).status, 200);
  assert.equal(route.post({ sessionId: joined.sessionId, position: { x: 2, z: 2 } }).status, 429);
  let blocked = 0;
  for (let i = 0; i < 30; i += 1) {
    if (route.join({ playerKey: KEY_B }, { address: "spam" }).status === 429) blocked += 1;
  }
  assert.ok(blocked > 5);
  assert.equal(route.post({ sessionId: "nope" }).status, 404);
});

test("hunter target is sticky and grows with the party", () => {
  let clock = 1;
  const route = createPublicRoute({ now: () => clock });
  const keys = ["1", "2", "3", "4", "5"].map((n) => n.repeat(24));
  for (const key of keys) route.join({ playerKey: key }, { address: key });
  assert.equal(route.status().hunterTargetCount, 3);
});

test("state persists to disk and survives a server restart", async () => {
  const dir = await mkdtemp(join(tmpdir(), "public-route-"));
  try {
    const store = join(dir, "lastroute", "godot_publicroom.json");
    const first = createPublicRoute({ storePath: store });
    await first.load();
    const joined = first.join({ playerKey: KEY_A }, { address: "p" }).body;
    first.post({ sessionId: joined.sessionId, position: { x: 7, z: 8 }, snapshot: "s", claims: [{ id: "2:0:3:PICKUP:RELAY:1" }] }, { hostTokenValid: true });
    first.leave({ sessionId: joined.sessionId });
    await first.flush();
    assert.ok(JSON.parse(await readFile(store, "utf8")).claims.length === 1);
    const second = createPublicRoute({ storePath: store });
    await second.load();
    const back = second.join({ playerKey: KEY_A }, { address: "p" }).body;
    assert.deepEqual(back.resume.position, { x: 7, z: 8 });
    assert.deepEqual(back.claimed, ["2:0:3:PICKUP:RELAY:1"]);
    assert.equal(back.snapshot, "s");
    assert.equal(back.seed, joined.seed);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("HTTP: PUBLIC is a registered ENet code, its host token authorises the snapshot", async () => {
  const server = createLastRouteServer({ relayAddress: "127.0.0.1", relayPort: 11999 });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const call = async (path, body) => (await fetch(`${base}${path}`, {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
  })).json();
  try {
    const first = await call("/api/public/join", { playerKey: KEY_A, name: "one" });
    assert.equal(first.authorityLive, false);
    const registered = await call("/api/enet/rooms/register", { code: "PUBLC", port: 8421, players: 1 });
    assert.equal(registered.ok, true);
    assert.equal((await call("/api/enet/rooms/register", { code: "PUBLC", port: 8422 })).ok, false, "second host is refused");
    const second = await call("/api/public/join", { playerKey: KEY_B, name: "two" });
    assert.equal(second.authorityLive, true);
    const forged = await call("/api/public/state", { sessionId: second.sessionId, hostToken: "wrong", claims: [{ id: "1:0:0:PICKUP:RELAY:0" }] });
    assert.equal(forged.relaysCollected, 0);
    const real = await call("/api/public/state", { sessionId: first.sessionId, hostToken: registered.token, claims: [{ id: "1:0:0:PICKUP:RELAY:0" }], snapshot: "(x)" });
    assert.equal(real.relaysCollected, 1);
    assert.equal(real.authority, true);
    const status = await (await fetch(`${base}/api/public/status`)).json();
    assert.equal(status.playersOnline, 2);
    assert.equal(status.relaysCollected, 1);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

test("HTTP: each game has its own public route (CTYPR), state and ENet listing", async () => {
  const server = createLastRouteServer({ relayAddress: "127.0.0.1", relayPort: 11998 });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const call = async (path, body) => (await fetch(`${base}${path}`, {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
  })).json();
  try {
    const city = await call("/api/public/join", { playerKey: KEY_A, name: "one", game: "city" });
    assert.equal(city.code, "CTYPR");
    const lastRoute = await call("/api/public/join", { playerKey: KEY_B, name: "two" });
    assert.equal(lastRoute.code, "PUBLC");
    assert.notEqual(city.seed, lastRoute.seed);
    const registered = await call("/api/enet/rooms/register", { code: "CTYPR", port: 8421, players: 1 });
    assert.equal(registered.ok, true);
    assert.equal(registered.room.game, "city");
    const cityAgain = await call("/api/public/join", { playerKey: KEY_B, name: "two", game: "city" });
    assert.equal(cityAgain.authorityLive, true);
    const lastRouteAgain = await call("/api/public/join", { playerKey: KEY_A, name: "one" });
    assert.equal(lastRouteAgain.authorityLive, false, "the City listing must not make Last Route's route live");
    const real = await call("/api/public/state", { sessionId: city.sessionId, game: "city", hostToken: registered.token, claims: [{ id: "0:0:0:PICKUP:RELAY:0" }], snapshot: "(c)" });
    assert.equal(real.relaysCollected, 1);
    const cityStatus = await (await fetch(`${base}/api/public/status?game=city`)).json();
    const lastRouteStatus = await (await fetch(`${base}/api/public/status`)).json();
    assert.equal(cityStatus.relaysCollected, 1);
    assert.equal(lastRouteStatus.relaysCollected, 0);
    assert.equal(lastRouteStatus.code, "PUBLC");
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
