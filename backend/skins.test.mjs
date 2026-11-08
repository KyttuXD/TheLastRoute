import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { createHash } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { deflateSync } from "node:zlib";
import { createLastRouteServer } from "./server.mjs";
import { SKIN_MAX_BYTES, validateSkinPng } from "./skins.mjs";

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let k = 0; k < 8; k += 1) crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const head = Buffer.alloc(4);
  head.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(body));
  return Buffer.concat([head, body, tail]);
}

// an RGBA 8-bit PNG of `size` x `size` filled with one colour (a different `seed` byte makes a different file)
function png({ size = 64, height = size, seed = 1, colour = 6, depth = 8, extra = [], raw = null } = {}) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(height, 4);
  header[8] = depth;
  header[9] = colour;
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[colour] || 1;
  const row = Math.ceil((size * channels * depth) / 8);
  const pixels = raw || Buffer.concat(Array.from({ length: height }, () => Buffer.concat([Buffer.from([0]), Buffer.alloc(row, seed)])));
  return Buffer.concat([SIGNATURE, chunk("IHDR", header), ...extra, chunk("IDAT", deflateSync(pixels)), chunk("IEND", Buffer.alloc(0))]);
}

async function fixture(options = {}) {
  const root = await mkdtemp(join(tmpdir(), "skins-"));
  const server = createLastRouteServer({ accountStorePath: join(root, "accounts.json"), skinDirectory: join(root, "skins"),
    adminPassword: "server setup proof", logger: { error() {}, log() {} }, ...options });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return { url, root, close: async () => { server.close(); await once(server, "close"); await rm(root, { recursive: true }); } };
}

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

async function signUp(url, username, extra = {}) {
  const result = await post(url, "/api/account/register", { username, password: "correct horse battery", ...extra });
  assert.equal(result.status, 200);
  return result.body.sessionToken;
}

async function upload(url, token, bytes) {
  const response = await fetch(`${url}/api/skins/upload`, { method: "POST", body: bytes,
    headers: { "content-type": "application/octet-stream", ...(token ? { "x-session-token": token } : {}) } });
  return { status: response.status, body: await response.json() };
}

test("validateSkinPng accepts a well-formed 64x64 PNG and refuses everything else", () => {
  assert.equal(validateSkinPng(png()), "");
  assert.equal(validateSkinPng(png({ colour: 2 })), "");
  assert.equal(validateSkinPng(png({ colour: 0, depth: 8 })), "");
  assert.match(validateSkinPng(Buffer.alloc(0)), /empty/);
  assert.match(validateSkinPng(Buffer.from("GIF89a....................................")), /not a PNG/);
  assert.match(validateSkinPng(png({ size: 32 })), /64x64/);
  assert.match(validateSkinPng(png({ size: 128 })), /64x64/);
  assert.equal(validateSkinPng(png({ size: 64, height: 32 })), "", "the old 64x32 layout is accepted");
  assert.match(validateSkinPng(png({ size: 64, height: 48 })), /64x64/);
  const good = png();
  assert.match(validateSkinPng(good.subarray(0, good.length - 20)), /truncated|corrupt/);
  const corrupted = Buffer.from(good);
  corrupted[good.length - 30] ^= 0xff;
  assert.match(validateSkinPng(corrupted), /corrupt|truncated/);
  assert.match(validateSkinPng(Buffer.concat([good, Buffer.from("trailing")])), /after the end|truncated/);
  assert.match(validateSkinPng(png({ raw: Buffer.alloc(10) })), /corrupt/, "a pixel stream of the wrong length");
  assert.match(validateSkinPng(png({ colour: 5 })), /unsupported/);
  assert.match(validateSkinPng(Buffer.concat([SIGNATURE, Buffer.from("extends Node\nfunc _ready(): OS.execute('x', [])")])), /not a PNG|truncated/);
  const bomb = png({ raw: Buffer.alloc(64 * 64 * 4 * 50) });
  assert.match(validateSkinPng(bomb), /corrupt/, "an inflate bomb is refused without being expanded");
  assert.match(validateSkinPng(Buffer.alloc(SKIN_MAX_BYTES + 1, 1)), /KB/);
});

test("one skin per account: upload, fetch by hash, replace, clear", async () => {
  const app = await fixture();
  try {
    const token = await signUp(app.url, "skin_one");
    assert.equal((await upload(app.url, "", png())).status, 401, "uploads need a session");
    assert.equal((await upload(app.url, "bad-token", png())).status, 401);
    const first = png({ seed: 7 });
    const uploaded = await upload(app.url, token, first);
    assert.equal(uploaded.status, 200);
    assert.equal(uploaded.body.hash, createHash("sha256").update(first).digest("hex"));
    assert.equal((await post(app.url, "/api/skins/mine", { sessionToken: token })).body.hash, uploaded.body.hash);
    let file = await fetch(`${app.url}/api/skins/files/${uploaded.body.hash}.png`);
    assert.equal(file.status, 200);
    assert.equal(file.headers.get("content-type"), "image/png");
    assert.match(file.headers.get("cache-control"), /immutable/);
    assert.ok(Buffer.from(await file.arrayBuffer()).equals(first), "the served bytes are the uploaded bytes");
    assert.equal((await fetch(`${app.url}/api/skins/files/${"0".repeat(64)}.png`)).status, 404);
    assert.equal((await fetch(`${app.url}/api/skins/files/..%2Faccounts.json`)).status, 404, "no path walking");
    assert.equal((await fetch(`${app.url}/api/skins/files/ABC.png`)).status, 404);
    const second = png({ seed: 9 });
    const replaced = await upload(app.url, token, second);
    assert.equal(replaced.status, 200);
    assert.equal((await fetch(`${app.url}/api/skins/files/${uploaded.body.hash}.png`)).status, 200, "the replaced skin is kept as a version");
    assert.equal(readdirSync(join(app.root, "skins", "files")).length, 2, "the current skin plus its previous version");
    assert.equal((await post(app.url, "/api/skins/clear", { sessionToken: token })).status, 200);
    assert.equal((await post(app.url, "/api/skins/mine", { sessionToken: token })).body.hash, "");
    assert.equal((await fetch(`${app.url}/api/skins/files/${replaced.body.hash}.png`)).status, 404);
    assert.equal((await fetch(`${app.url}/api/skins/files/${uploaded.body.hash}.png`)).status, 404, "clearing drops the versions too");
  } finally {
    await app.close();
  }
});

test("refused uploads store nothing", async () => {
  const app = await fixture();
  try {
    const token = await signUp(app.url, "skin_bad");
    for (const bytes of [png({ size: 32 }), Buffer.from("not a png at all, just text"), Buffer.alloc(0), png().subarray(0, 100)]) {
      const result = await upload(app.url, token, bytes);
      assert.ok(result.status === 400 || result.status === 413, `refused with ${result.status}`);
    }
    const huge = await upload(app.url, token, Buffer.alloc(SKIN_MAX_BYTES + 1000, 1));
    assert.equal(huge.status, 413);
    assert.equal((await post(app.url, "/api/skins/mine", { sessionToken: token })).body.hash, "");
    assert.equal(readdirSync(join(app.root, "skins", "files")).length, 0);
  } finally {
    await app.close();
  }
});

test("uploads are rate limited per account", async () => {
  const app = await fixture({ skinOptions: { uploadLimit: 3 } });
  try {
    const token = await signUp(app.url, "skin_rate");
    for (let i = 0; i < 3; i += 1) assert.equal((await upload(app.url, token, png({ seed: i + 1 }))).status, 200);
    assert.equal((await upload(app.url, token, png({ seed: 50 }))).status, 429);
    const other = await signUp(app.url, "skin_other");
    assert.equal((await upload(app.url, other, png({ seed: 51 }))).status, 200, "another account is not affected");
  } finally {
    await app.close();
  }
});

test("owners can remove a skin and it cannot come back; others cannot", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const player = await signUp(app.url, "skin_player");
    const bytes = png({ seed: 21 });
    const hash = (await upload(app.url, player, bytes)).body.hash;
    assert.equal((await post(app.url, "/api/skins/remove", { sessionToken: player, hash })).status, 403, "players cannot remove skins");
    assert.equal((await post(app.url, "/api/skins/remove", { hash })).status, 403);
    assert.equal((await fetch(`${app.url}/api/skins/files/${hash}.png`)).status, 200);
    const removed = await post(app.url, "/api/skins/remove", { sessionToken: owner, username: "skin_player" });
    assert.equal(removed.status, 200);
    assert.equal(removed.body.hash, hash);
    assert.equal((await fetch(`${app.url}/api/skins/files/${hash}.png`)).status, 404);
    assert.equal((await post(app.url, "/api/skins/mine", { sessionToken: player })).body.hash, "");
    assert.equal((await upload(app.url, player, bytes)).status, 403, "a removed skin is blocked from re-upload");
    assert.equal((await upload(app.url, player, png({ seed: 22 }))).status, 200, "a different skin is fine");
    assert.equal((await post(app.url, "/api/skins/remove", { sessionToken: owner, hash: "nope" })).status, 404);
  } finally {
    await app.close();
  }
});

test("skins survive a server restart and a tampered file is never served", async () => {
  const root = await mkdtemp(join(tmpdir(), "skins-restart-"));
  const options = { accountStorePath: join(root, "accounts.json"), skinDirectory: join(root, "skins"), logger: { error() {}, log() {} } };
  let server = createLastRouteServer(options);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  let url = `http://127.0.0.1:${server.address().port}`;
  const token = await signUp(url, "skin_keep");
  const hash = (await upload(url, token, png({ seed: 33 }))).body.hash;
  server.close();
  await once(server, "close");
  server = createLastRouteServer(options);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  url = `http://127.0.0.1:${server.address().port}`;
  try {
    assert.equal((await fetch(`${url}/api/skins/files/${hash}.png`)).status, 200);
    const { writeFileSync } = await import("node:fs");
    writeFileSync(join(root, "skins", "files", `${hash}.png`), png({ seed: 99 }));
    // the in-memory copy may still serve the original bytes; a fresh server must refuse the swapped file
    server.close();
    await once(server, "close");
    server = createLastRouteServer(options);
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    url = `http://127.0.0.1:${server.address().port}`;
    assert.equal((await fetch(`${url}/api/skins/files/${hash}.png`)).status, 404, "a file that is not its own hash is not served");
  } finally {
    server.close();
    await once(server, "close");
    await rm(root, { recursive: true });
  }
});


async function putSkin(url, account, token, bytes, arms) {
  const response = await fetch(`${url}/api/v1/skins/${account}`, { method: "PUT", body: bytes,
    headers: { "content-type": "application/octet-stream", ...(token ? { "x-session-token": token } : {}), ...(arms ? { "x-skin-arms": arms } : {}) } });
  return { status: response.status, body: await response.json() };
}

async function getSkin(url, account) {
  const response = await fetch(`${url}/api/v1/skins/${account}`);
  return { status: response.status, body: await response.json() };
}

test("account skins: the owner PUTs, anyone GETs, others are refused", async () => {
  const app = await fixture();
  try {
    const owner = await signUp(app.url, "SKIN_OWNER");
    const other = await signUp(app.url, "SKIN_OTHER");
    assert.deepEqual((await getSkin(app.url, "SKIN_OWNER")).body.hash, "", "no skin yet");
    assert.equal((await putSkin(app.url, "SKIN_OWNER", "", png())).status, 401, "a PUT needs a session");
    assert.equal((await putSkin(app.url, "SKIN_OWNER", other, png())).status, 403, "another account cannot write this skin");
    const bytes = png({ seed: 9 });
    const stored = await putSkin(app.url, "skin_owner", owner, bytes, "slim");
    assert.equal(stored.status, 200, "the account name is case-insensitive");
    assert.equal(stored.body.hash, createHash("sha256").update(bytes).digest("hex"));
    assert.equal(stored.body.arms, "slim");
    const found = await getSkin(app.url, "SKIN_OWNER");
    assert.equal(found.body.hash, stored.body.hash);
    assert.equal(found.body.arms, "slim");
    assert.ok(found.body.updatedAt > 0 && found.body.versions[0] === stored.body.hash);
    assert.equal((await getSkin(app.url, "SKIN_OTHER")).body.hash, "", "accounts do not see each other's skins");
    const file = await fetch(`${app.url}/api/skins/files/${stored.body.hash}.png`);
    assert.ok(Buffer.from(await file.arrayBuffer()).equals(bytes), "the PNG is served by hash");
    assert.equal((await putSkin(app.url, "SKIN_OWNER", owner, png({ seed: 3 }), "weird")).body.arms, "auto", "an unknown arms value falls back to auto");
    assert.equal((await getSkin(app.url, "not a name!")).status, 404, "a malformed account name is not routed");
  } finally {
    await app.close();
  }
});

test("account skins: format checks, the 64x32 layout, size cap and the upload rate limit", async () => {
  const app = await fixture({ skinOptions: { uploadLimit: 3, uploadWindowMs: 60_000 } });
  try {
    const token = await signUp(app.url, "SKIN_RULES");
    assert.equal((await putSkin(app.url, "SKIN_RULES", token, Buffer.from("GIF89a"))).status, 400, "only PNGs");
    assert.equal((await putSkin(app.url, "SKIN_RULES", token, png({ size: 48 }))).status, 400, "wrong dimensions");
    assert.equal((await putSkin(app.url, "SKIN_RULES", token, png({ size: 64, height: 32 }))).status, 200, "64x32 is allowed");
    assert.equal((await putSkin(app.url, "SKIN_RULES", token, Buffer.alloc(SKIN_MAX_BYTES + 10, 1))).status, 413, "over the size cap");
    for (const seed of [20, 21]) assert.equal((await putSkin(app.url, "SKIN_RULES", token, png({ seed }))).status, 200);
    assert.equal((await putSkin(app.url, "SKIN_RULES", token, png({ seed: 22 }))).status, 429, "uploads are rate limited");
  } finally {
    await app.close();
  }
});

test("account skins: a few versions are kept, older ones are dropped", async () => {
  const app = await fixture({ skinOptions: { uploadLimit: 100, uploadWindowMs: 60_000 } });
  try {
    const token = await signUp(app.url, "SKIN_VERSIONED");
    const hashes = [];
    for (let seed = 30; seed < 38; seed += 1) {
      const result = await putSkin(app.url, "SKIN_VERSIONED", token, png({ seed }));
      assert.equal(result.status, 200);
      hashes.push(result.body.hash);
    }
    const found = (await getSkin(app.url, "SKIN_VERSIONED")).body;
    assert.deepEqual(found.versions, hashes.slice(-5).reverse(), "the newest five versions, newest first");
    assert.equal(found.hash, hashes.at(-1));
    assert.equal((await fetch(`${app.url}/api/skins/files/${hashes[2]}.png`)).status, 404, "a version that fell off is deleted");
    assert.equal((await fetch(`${app.url}/api/skins/files/${hashes[3]}.png`)).status, 200, "a kept version is still served");
    assert.equal((await post(app.url, "/api/skins/clear", { sessionToken: token })).status, 200);
    assert.deepEqual((await getSkin(app.url, "SKIN_VERSIONED")).body.versions, [], "clearing drops the versions");
    assert.equal((await fetch(`${app.url}/api/skins/files/${hashes[3]}.png`)).status, 404);
  } finally {
    await app.close();
  }
});
