import test from "node:test";
import assert from "node:assert/strict";
import { once, EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { createRegions, parseAgentTokens, signAgentRequest } from "./regions.mjs";
import { canSpawn, capacity, createAgent } from "./region_agent.mjs";

const quiet = { log() {}, warn() {}, error() {} };
const TOKEN = "virginia-test-token-0123456789abcdef";

test("capacity follows the VPSes as measured (2026-10-28): one core kept, 700 MB a room plus 1 GB spare", () => {
  assert.equal(capacity({ cores: 12, totalMb: 32094 }), 11, "vps1 Virginia");
  assert.equal(capacity({ cores: 4, totalMb: 7941 }), 3, "vps2 Texas");
  assert.equal(capacity({ cores: 4, totalMb: 3072 }), 2, "vps3 California (relay only by configuration)");
  assert.equal(capacity({ cores: 12, totalMb: 32094, maxRooms: 4 }), 4, "LASTROUTE_MAX_ROOMS caps it");
  assert.equal(canSpawn({ running: 2, cap: 3, load1: 1.0, cores: 4, freeMb: 2000, roomMb: 700 }), true);
  assert.equal(canSpawn({ running: 3, cap: 3, load1: 0, cores: 4, freeMb: 9000, roomMb: 700 }), false, "full");
  assert.equal(canSpawn({ running: 0, cap: 3, load1: 3.5, cores: 4, freeMb: 9000, roomMb: 700 }), false, "busy CPU");
  assert.equal(canSpawn({ running: 0, cap: 3, load1: 0, cores: 4, freeMb: 500, roomMb: 700 }), false, "low RAM");
});

test("agent requests are signed: the token never travels, a bad, replayed or stale signature is refused", () => {
  let clock = 1_000_000;
  const regions = createRegions({ tokens: parseAgentTokens(`virginia:${TOKEN},bad:short`), now: () => clock });
  const sign = (body, time = clock, token = TOKEN) => ({ "x-lr-region": "virginia", "x-lr-time": String(time),
    "x-lr-signature": signAgentRequest(token, "virginia", time, body) });
  const body = JSON.stringify({ capacity: 3 });
  assert.equal(regions.verify(sign(body), body), "virginia");
  assert.equal(regions.verify(sign(body), body), "", "a replayed request (same time) is refused");
  clock += 1;
  assert.equal(regions.verify(sign(body, clock, "wrong-token-0123456789abcdef"), body), "", "wrong token");
  assert.equal(regions.verify(sign(body), body + " "), "", "a changed body breaks the signature");
  assert.equal(regions.verify(sign(body, clock - 120_000), body), "", "a stale clock is refused");
  assert.equal(parseAgentTokens("bad:short").size, 0, "short tokens are ignored");
});

test("agent <-> directory: report, owner spawn, ticketed register of a reserved code, stop", async () => {
  const root = await mkdtemp(join(tmpdir(), "regions-"));
  const server = createLastRouteServer({ accountStorePath: join(root, "a.json"), moderationStorePath: join(root, "m.json"),
    reservedRoomStorePath: join(root, "r.json"), reservedRoomCodes: ["KATHY"], agentTokens: parseAgentTokens(`virginia:${TOKEN}`),
    adminPassword: "proof", accountRateLimitMax: 10_000, apiRateLimitMax: 10_000, logger: quiet });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  const post = async (path, body) => {
    const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  };
  const spawned = [];
  const fakeSpawn = (bin, args, options) => {
    const child = new EventEmitter();
    child.pid = 4242 + spawned.length;
    child.kill = (signal) => { child.killed = signal; setImmediate(() => child.emit("exit", null, signal)); };
    spawned.push({ bin, args, env: options.env, child });
    return child;
  };
  const agent = createAgent({ LASTROUTE_REGION: "virginia", LASTROUTE_REGION_LABEL: "US East", LASTROUTE_AGENT_TOKEN: TOKEN,
    LASTROUTE_DIRECTORY_URL: url, LASTROUTE_GODOT_BIN: "/opt/lastroute/game.x86_64", LASTROUTE_RELAY_ADDRESS: "vps1.example",
    LASTROUTE_RELAY_PORT: "1105", LASTROUTE_MAX_ROOMS: "2" }, { spawnProcess: fakeSpawn, log() {},
    resources: () => ({ cores: 12, totalMb: 32094, freeMb: 30000, load1: 0.1 }) });
  try {
    assert.deepEqual((await (await fetch(`${url}/api/regions`)).json()).regions, []);
    await agent.reportOnce();
    const listed = (await (await fetch(`${url}/api/regions`)).json()).regions;
    assert.equal(listed.length, 1);
    assert.equal(listed[0].region, "virginia");
    assert.equal(listed[0].relay.address, "vps1.example");

    const owner = (await post("/api/account/register", { username: "KYTTU", password: "correct horse battery", adminProof: "proof" })).body.sessionToken;
    const player = (await post("/api/account/register", { username: "PLAYER", password: "correct horse battery" })).body.sessionToken;
    assert.equal((await post("/api/admin/regions", { sessionToken: player, action: "list" })).status, 403);
    const queued = await post("/api/admin/regions", { sessionToken: owner, action: "spawn", region: "virginia", code: "KATHY",
      seed: "KT SEED", idleMinutes: 15 });
    assert.equal(queued.status, 202, JSON.stringify(queued.body));
    assert.equal(queued.body.queued.options.spawnTicket, undefined, "the owner never sees the spawn ticket");
    assert.equal((await post("/api/admin/regions", { sessionToken: owner, action: "spawn", region: "texas", code: "AB2CD" })).body.code,
      "REGION_OFFLINE");

    await agent.reportOnce();   // the answer carries the spawn
    assert.equal(spawned.length, 1);
    const { args, env } = spawned[0];
    assert.deepEqual(args.slice(args.indexOf("--dedicated-host"), args.indexOf("--dedicated-host") + 2), ["--dedicated-host", "KATHY"]);
    assert.ok(args.includes("--seed") && args.includes("KT SEED") && args.includes("15"));
    assert.ok(env.LASTROUTE_SPAWN_TICKET && !args.join(" ").includes(env.LASTROUTE_SPAWN_TICKET), "the ticket is not on the command line");
    assert.equal(env.LASTROUTE_AGENT_TOKEN, "", "the agent's token is not handed to the game");

    // the spawned host registers the reserved code with its ticket -- once
    assert.equal((await post("/api/enet/rooms/register", { code: "KATHY", port: 18100, spawnTicket: "guess" })).status, 403);
    const hosted = await post("/api/enet/rooms/register", { code: "KATHY", port: 18100, spawnTicket: env.LASTROUTE_SPAWN_TICKET });
    assert.equal(hosted.status, 201, JSON.stringify(hosted.body));
    await post("/api/enet/rooms/unregister", { code: "KATHY", token: hosted.body.token });
    assert.equal((await post("/api/enet/rooms/register", { code: "KATHY", port: 18100, spawnTicket: env.LASTROUTE_SPAWN_TICKET })).status,
      403, "a spawn ticket works once");

    await agent.reportOnce();
    const ownerView = (await post("/api/admin/regions", { sessionToken: owner, action: "list" })).body.regions[0];
    assert.deepEqual(ownerView.rooms.map((room) => room.code), ["KATHY"]);
    assert.equal(ownerView.capacity, 2);
    assert.equal((await post("/api/admin/regions", { sessionToken: owner, action: "stop", region: "virginia", code: "KATHY" })).status, 202);
    await agent.reportOnce();
    assert.equal(spawned[0].child.killed, "SIGTERM", "stop signals exactly the spawned child");
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(agent.hosts.size, 0);
  } finally {
    server.close();
    await once(server, "close");
    await rm(root, { recursive: true });
  }
});

test("a spawn request carries the asked-for build, rust or nothing", () => {
  const regions = createRegions({ tokens: new Map([["virginia", "t"]]) });
  regions.report("virginia", { label: "US East", capacity: 4, relay: null, rooms: [], version: "test" });
  const queued = (build) => regions.requestSpawn("virginia", "ABCDE", { spawnTicket: "x", build });
  assert.equal(queued("rust")[1].queued.options.build, "rust");
  assert.equal(queued("godot")[1].queued.options.build, "");
  assert.equal(queued(undefined)[1].queued.options.build, "");
});
