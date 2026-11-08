import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { deflateSync } from "node:zlib";
import { createLastRouteServer } from "./server.mjs";

const HOUR = 60 * 60 * 1000;
const BUILD = "a".repeat(64);
const RELEASE_HASH = "b".repeat(64);
const quiet = { error() {}, log() {}, warn() {} };

async function fixture(options = {}) {
  const root = await mkdtemp(join(tmpdir(), "moderation-"));
  const clock = { t: Date.parse("2026-10-28T12:00:00Z") };
  const server = createLastRouteServer({
    accountStorePath: join(root, "accounts.json"),
    moderationStorePath: join(root, "moderation.json"),
    skinDirectory: join(root, "skins"),
    adminPassword: "server setup proof",
    accountRateLimitMax: 10_000, apiRateLimitMax: 10_000,
    enetRoomTtlMs: 10 * 24 * HOUR,
    updates: { release: { windows: { version: "1.0.0", file: "w.zip", sha256: RELEASE_HASH, build: 3, revision: "abcdef1" } } },
    now: () => clock.t, logger: quiet, ...options,
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return { url, root, clock, close: async () => { server.close(); await once(server, "close"); await rm(root, { recursive: true }); } };
}

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

async function signUp(url, username, extra = {}) {
  const result = await post(url, "/api/account/register", { username, password: "correct horse battery", ...extra });
  assert.equal(result.status, 200, JSON.stringify(result.body));
  return result.body.sessionToken;
}

let nextCode = 0;
async function host(url, sessionToken = "") {
  const code = ["RMAAA", "RMAAB", "RMAAC", "RMAAD", "RMAAE", "RMAAF", "RMAAG", "RMAAH"][nextCode++ % 8];
  const result = await post(url, "/api/enet/rooms/register", { code, port: 9000, sessionToken });
  assert.equal(result.status, 201, JSON.stringify(result.body));
  return { code, hostToken: result.body.token };
}

const kick = (url, room, extra) => post(url, "/api/moderation/report", { ...room, kind: "kick", reason: "speed", strikes: 3, ...extra });
const join_ = (url, extra) => post(url, "/api/public/join", { playerKey: "0123456789abcdef", name: "RUNNER", ...extra });

function png() {
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const body = Buffer.concat([Buffer.from(type), data]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(body)); return Buffer.concat([len, body, c]); };
  const header = Buffer.alloc(13); header.writeUInt32BE(64, 0); header.writeUInt32BE(64, 4); header[8] = 8; header[9] = 6;
  const rows = Buffer.concat(Array.from({ length: 64 }, () => Buffer.concat([Buffer.from([0]), Buffer.alloc(256, 7)])));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}

test("kicks never restrict an account; they open review entries, 2 distinct hosts marked as the stronger signal", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const player = await signUp(app.url, "CHEATER");
    const roomA = await host(app.url, await signUp(app.url, "HOST_A"));
    const roomB = await host(app.url, await signUp(app.url, "HOST_B"));
    for (const room of [roomA, roomA, roomB, roomB]) {
      const result = await kick(app.url, room, { username: "cheater" });
      assert.equal(result.body.flagged, false, "no automatic flag");
      assert.deepEqual(result.body.status.restrictions, { publicRoute: false, hostPublic: false, skinUpload: false });
    }
    const status = await post(app.url, "/api/moderation/status", { sessionToken: player });
    assert.equal(status.body.status.flagged, false);
    assert.equal(status.body.status.message, "");
    assert.equal((await join_(app.url, { sessionToken: player })).status, 200, "kicked accounts still join the Public Route");
    const list = await post(app.url, "/api/admin/flags", { sessionToken: owner });
    assert.deepEqual(list.body.reviews, ["CHEATER"]);
    const entry = list.body.accounts.find((account) => account.username === "CHEATER");
    assert.equal(entry.reviewOpen, 4);
    assert.deepEqual(entry.reviews.map((review) => review.reason), ["kick", "kick", "kicks", "kicks"]);
    assert.equal(entry.kicks.length, 4);
    assert.equal(entry.flagged, false);
  } finally { await app.close(); }
});

test("owner accounts are never auto-flagged; unknown accounts record nothing", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const roomA = await host(app.url, await signUp(app.url, "HOST_A"));
    const roomB = await host(app.url, await signUp(app.url, "HOST_B"));
    await kick(app.url, roomA, { username: "KYTTU" });
    const result = await kick(app.url, roomB, { username: "KYTTU" });
    assert.equal(result.body.flagged, false);
    const integrity = await post(app.url, "/api/moderation/integrity", { sessionToken: owner, buildHash: BUILD, channel: "release", platform: "windows", context: "signin" });
    assert.equal(integrity.body.known, false);
    assert.equal(integrity.body.status.flagged, false);
    assert.equal((await kick(app.url, roomB, { username: "NOBODY_HERE" })).body.flagged, false);
  } finally { await app.close(); }
});

test("integrity: an unknown build opens one review per hash and never flags; manifest, allow-list and owner debug pass", async () => {
  const app = await fixture();
  try {
    const player = await signUp(app.url, "MODDER");
    const owner = await signUp(app.url, "AYTTU", { adminProof: "server setup proof" });
    let result = await post(app.url, "/api/moderation/integrity", { buildHash: BUILD, channel: "release", platform: "windows" });
    assert.deepEqual([result.status, result.body.known, result.body.status.flagged], [200, false, false], "no session: just reports");
    result = await post(app.url, "/api/moderation/integrity", { sessionToken: player, buildHash: RELEASE_HASH, channel: "release", platform: "windows", commit: "abcdef1" });
    assert.equal(result.body.known, true, "manifest sha256 is known");
    result = await post(app.url, "/api/moderation/integrity", { sessionToken: player, buildHash: RELEASE_HASH, channel: "release", platform: "linux" });
    assert.equal(result.body.known, false, "manifest match is per channel/platform");
    assert.equal(result.body.status.flagged, false, "an unknown build is never an automatic flag");
    await post(app.url, "/api/moderation/integrity", { sessionToken: player, buildHash: RELEASE_HASH, channel: "release", platform: "linux" });
    const list = await post(app.url, "/api/admin/flags", { sessionToken: owner });
    const entry = list.body.accounts.find((account) => account.username === "MODDER");
    assert.deepEqual(entry.reviews.map((review) => review.reason), ["integrity"], "one review per distinct hash");
    assert.equal(entry.integrity.hash, RELEASE_HASH);
    result = await post(app.url, "/api/moderation/integrity", { sessionToken: owner, buildHash: BUILD, channel: "debug", platform: "macos" });
    assert.equal(result.body.known, true, "owner debug builds are always known");
    result = await post(app.url, "/api/admin/builds/add", { sessionToken: owner, buildHash: BUILD, commit: "1234567", note: "test" });
    assert.equal(result.body.ok, true);
    result = await post(app.url, "/api/moderation/integrity", { sessionToken: player, buildHash: BUILD, channel: "release", platform: "windows" });
    assert.equal(result.body.known, true, "allow-listed");
    result = await post(app.url, "/api/admin/builds/remove", { sessionToken: owner, buildHash: BUILD });
    assert.equal(result.body.removed, true);
    assert.equal((await post(app.url, "/api/moderation/integrity", { buildHash: "zz", channel: "release" })).status, 400);
  } finally { await app.close(); }
});

test("owner-flagged accounts cannot join or host PUBLIC or upload skins; private rooms still work", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const player = await signUp(app.url, "MODDER");
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "MODDER", action: "extend", hours: 24 });
    let result = await join_(app.url, { sessionToken: player });
    assert.equal(result.status, 403);
    assert.equal(result.body.code, "FLAGGED");
    assert.equal(result.body.permanent, false);
    assert.equal(result.body.until, app.clock.t + 24 * HOUR);
    assert.match(result.body.error, /Public Route/);
    result = await post(app.url, "/api/enet/rooms/register", { code: "PUBLC", port: 9000, sessionToken: player });
    assert.equal(result.status, 403);
    assert.equal(result.body.code, "FLAGGED");
    const response = await fetch(`${app.url}/api/skins/upload`, { method: "POST", body: png(), headers: { "x-session-token": player } });
    assert.equal(response.status, 403);
    assert.equal((await response.json()).code, "FLAGGED");
    result = await post(app.url, "/api/enet/rooms/register", { code: "PRVAT", port: 9000, sessionToken: player });
    assert.equal(result.status, 201, "private room hosting is still allowed");
    result = await post(app.url, "/api/rooms/join", { roomCode: "PRIV1", name: "MODDER" });
    assert.notEqual(result.status, 403);
    assert.equal((await join_(app.url, {})).status, 200, "an unflagged guest still joins");
  } finally { await app.close(); }
});

test("kicks never put a device or IP on a public-join cooldown", async () => {
  const app = await fixture();
  try {
    const room = await host(app.url);
    await post(app.url, "/api/moderation/status", { deviceId: "device-1" });
    for (let i = 0; i < 4; i++) {
      const result = await kick(app.url, room, { deviceId: "device-1" });
      assert.equal(result.body.status.cooldownUntil, null);
    }
    assert.equal((await join_(app.url, { deviceId: "device-1" })).status, 200);
    assert.equal((await join_(app.url, { deviceId: "fresh-device" })).status, 200, "nor the IP it last used");
  } finally { await app.close(); }
});

test("peers and report require the room's host token; troll and flags come only from owners", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    await signUp(app.url, "PRANKER");
    const room = await host(app.url);
    assert.equal((await post(app.url, "/api/moderation/peers", { code: room.code, hostToken: "wrong", usernames: ["PRANKER"] })).status, 403);
    assert.equal((await post(app.url, "/api/moderation/report", { code: room.code, kind: "kick", username: "PRANKER" })).status, 403);
    assert.equal((await post(app.url, "/api/moderation/peers", { code: "ZZZZZ", hostToken: room.hostToken })).status, 403);
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "PRANKER", action: "troll_on" });
    await kick(app.url, room, { deviceId: "dev-x" });
    const result = await post(app.url, "/api/moderation/peers", { ...room, usernames: ["pranker", "KYTTU"], deviceIds: ["dev-x", "dev-y"] });
    assert.equal(result.status, 200);
    assert.deepEqual(result.body.players.PRANKER, { flagged: false, permanent: false, troll: true, ads: true, reason: "" });
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "PRANKER", action: "ads_off" });
    const quiet = await post(app.url, "/api/moderation/peers", { ...room, usernames: ["PRANKER"] });
    assert.equal(quiet.body.players.PRANKER.ads, false, "owners can turn troll ads off per player");
    assert.equal(quiet.body.players.PRANKER.troll, true);
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "PRANKER", action: "ads_on" });
    assert.equal(result.body.players.KYTTU.troll, false);
    assert.deepEqual(result.body.devices, { "dev-x": { cooldown: false, troll: false }, "dev-y": { cooldown: false, troll: false } },
      "a kicked device is never trolled or cooled down by the server");
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "PRANKER", action: "troll_off" });
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "PRANKER", action: "extend", hours: 2 });
    const flagged = await post(app.url, "/api/moderation/peers", { ...room, usernames: ["PRANKER"] });
    assert.equal(flagged.body.players.PRANKER.troll, true, "owner-flagged accounts are trolled too");
    const many = await post(app.url, "/api/moderation/peers", { ...room, deviceIds: Array.from({ length: 30 }, (_, i) => `d${i}`) });
    assert.equal(Object.keys(many.body.devices).length, 16);
  } finally { await app.close(); }
});

test("admin endpoints require a full owner and every action is logged", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const player = await signUp(app.url, "TARGET");
    for (const path of ["/api/admin/flags", "/api/admin/flags/set", "/api/admin/builds/add", "/api/admin/builds/remove"]) {
      assert.equal((await post(app.url, path, { sessionToken: player, username: "TARGET", action: "ban", buildHash: BUILD })).status, 403, path);
      assert.equal((await post(app.url, path, {})).body.code, "ADMIN_OWNER_REQUIRED", path);
    }
    const set = (action, extra = {}) => post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "target", action, ...extra });
    let result = await set("extend", { hours: 5, note: "manual" });
    assert.equal(result.body.record.flaggedUntil, app.clock.t + 5 * HOUR);
    result = await set("extend");
    assert.equal(result.body.record.flaggedUntil, app.clock.t + 29 * HOUR, "extends from the current until");
    assert.equal((await set("extend", { hours: 24 * 91 })).status, 400);
    result = await set("clear");
    assert.equal(result.body.record.flagged, false);
    result = await set("ban");
    assert.equal(result.body.record.permanent, true);
    let joined = await join_(app.url, { sessionToken: player });
    assert.deepEqual([joined.status, joined.body.permanent, joined.body.until], [403, true, null]);
    assert.match(joined.body.error, /banned/);
    assert.equal((await post(app.url, "/api/account/session", { sessionToken: player })).status, 200, "a ban never blocks sign-in");
    result = await set("unban");
    assert.equal(result.body.record.permanent, false);
    assert.equal((await join_(app.url, { sessionToken: player })).status, 200);
    await set("troll_on");
    result = await set("troll_off");
    assert.deepEqual(result.body.record.flags.map((flag) => flag.reason),
      ["admin:extend", "admin:extend", "admin:clear", "admin:ban", "admin:unban", "admin:troll_on", "admin:troll_off"]);
    assert.ok(result.body.record.flags.every((flag) => flag.by === "KYTTU"));
    assert.equal(result.body.record.flags[0].detail, "manual");
    assert.equal((await set("explode")).status, 400);
    const room = await host(app.url);
    await kick(app.url, room, { username: "TARGET" });
    result = await set("dismiss", { note: "lag" });
    assert.equal(result.body.record.reviewOpen, 0, "an owner action closes the open reviews");
    assert.equal(result.body.record.reviews[0].closedBy, "KYTTU");
    assert.equal(result.body.record.flagged, false, "dismiss restricts nothing");
    assert.equal((await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "GHOST", action: "ban" })).status, 404);
    await post(app.url, "/api/admin/builds/add", { sessionToken: owner, buildHash: BUILD });
    result = await post(app.url, "/api/admin/flags", { sessionToken: owner });
    assert.equal(result.body.accounts[0].username, "TARGET");
    assert.equal(result.body.builds[0].buildHash, BUILD);
  } finally { await app.close(); }
});

test("renaming keeps the owner flag and reviews and the store holds no raw IP or device id", async () => {
  const app = await fixture();
  try {
    const player = await signUp(app.url, "MODDER");
    const room = await host(app.url);
    await post(app.url, "/api/moderation/status", { deviceId: "secret-device-id-42" });
    await kick(app.url, room, { deviceId: "secret-device-id-42", username: "MODDER" });
    await post(app.url, "/api/moderation/integrity", { sessionToken: player, deviceId: "secret-device-id-42", buildHash: BUILD, channel: "release", platform: "windows" });
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    await post(app.url, "/api/admin/flags/set", { sessionToken: owner, username: "MODDER", action: "extend", hours: 24 });
    await post(app.url, "/api/account/rename", { sessionToken: player, username: "NEW_NAME" });
    const status = await post(app.url, "/api/moderation/status", { sessionToken: player });
    assert.equal(status.body.status.flagged, true);
    const stored = await readFile(join(app.root, "moderation.json"), "utf8");
    assert.ok(!stored.includes("secret-device-id-42"));
    assert.ok(!stored.includes("127.0.0.1"));
    assert.ok(stored.includes("NEW_NAME"));
  } finally { await app.close(); }
});
