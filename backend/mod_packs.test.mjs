import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { createHash } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { FALLBACK_PACK_KINDS, MOD_LIMITS, MOD_PACK_KINDS, packIdentity, validatePackManifest } from "./mod_packs.mjs";
import { readPackKindsFromRepo } from "./pack_kinds.mjs";
import { buildPack, deployPack, localTransport } from "./deploy_mod_pack.mjs";

const quiet = { error() {}, log() {}, warn() {} };
const sha = (text) => createHash("sha256").update(text).digest("hex");

async function fixture(options = {}) {
  const root = await mkdtemp(join(tmpdir(), "mod-packs-"));
  const make = () => createLastRouteServer({
    accountStorePath: join(root, "accounts.json"),
    moderationStorePath: join(root, "moderation.json"),
    modPackStorePath: join(root, "mod_packs.json"),
    modDownloadBase: "https://files.example.test/lr3d/mods/",
    adminPassword: "server setup proof",
    accountRateLimitMax: 10_000, apiRateLimitMax: 10_000, modRateLimitMax: 10_000,
    logger: quiet, ...options,
  });
  let server = make();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const app = {
    root,
    url: `http://127.0.0.1:${server.address().port}`,
    async restart() {
      server.close(); await once(server, "close");
      server = make(); server.listen(0, "127.0.0.1"); await once(server, "listening");
      app.url = `http://127.0.0.1:${server.address().port}`;
    },
    async close() { server.close(); await once(server, "close"); await rm(root, { recursive: true }); },
  };
  return app;
}

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

async function get(url, path) {
  const response = await fetch(url + path);
  return { status: response.status, body: await response.json() };
}

async function signUp(url, username, extra = {}) {
  const result = await post(url, "/api/account/register", { username, password: "correct horse battery", ...extra });
  assert.equal(result.status, 200, JSON.stringify(result.body));
  return result.body.sessionToken;
}

function manifest(extra = {}) {
  return {
    schema: "lr.pack/1", id: "neon_halls", name: "Neon Halls", version: "1.0.0", author: "KT",
    files: [
      { path: "pack.json", sha256: sha("{}"), size: 2 },
      { path: "props/hall.json", sha256: sha("hall"), size: 4 },
    ],
    ...extra,
  };
}

test("manifest validation follows the client loader (lr.pack/1) and refuses traversal, bad ids and oversized packs", () => {
  assert.ok(validatePackManifest(manifest()).pack);
  assert.equal(validatePackManifest(manifest({ id: "../etc" })).error, "INVALID_PACK_ID");
  assert.equal(validatePackManifest(manifest({ id: "neon-halls" })).error, "INVALID_PACK_ID", "no hyphen: content ids are <pack_id>_<name>");
  assert.equal(validatePackManifest(manifest({ id: "a".repeat(33) })).error, "INVALID_PACK_ID", "32 characters at most");
  assert.equal(validatePackManifest(manifest({ schema: "lr.pack/2" })).error, "INVALID_PACK_SCHEMA");
  assert.equal(validatePackManifest(manifest({ scripts: ["x.gd"] })).error, "UNKNOWN_PACK_FIELD");
  assert.ok(validatePackManifest(manifest({ overrides: ["props/officePrinter"], game_version: "0.9", description: "d" })).pack);
  assert.equal(validatePackManifest(manifest({ overrides: ["../x"] })).error, "INVALID_PACK_OVERRIDES");
  for (const kind of MOD_PACK_KINDS) {
    const ok = manifest({ overrides: [`${kind}/x`], files: [{ path: "pack.json", sha256: sha("{}"), size: 2 }, { path: `${kind}/a.json`, sha256: sha("a"), size: 1 }] });
    assert.ok(validatePackManifest(ok).pack, `${kind}/*.json is a pack data folder`);
  }
  assert.deepEqual(MOD_PACK_KINDS, readPackKindsFromRepo(), "the index takes its pack folders from the game");
  // only ambience override ids may contain ":" (not at either end)
  assert.ok(validatePackManifest(manifest({ overrides: ["ambience/room:chapel"] })).pack);
  for (const bad of ["props/room:chapel", "ambience/:chapel", "ambience/room:", "ceilings/a b", "basements/x"]) {
    assert.equal(validatePackManifest(manifest({ overrides: [bad] })).error, "INVALID_PACK_OVERRIDES", bad);
  }
  for (const path of ["props/sub/x.json", "assets/a.GLB", "maps/x.json", "assets/a.gd", "props/x.png"]) {
    const bad = manifest({ files: [{ path: "pack.json", sha256: sha("{}"), size: 2 }, { path, sha256: sha("x"), size: 1 }] });
    assert.equal(validatePackManifest(bad).error, "INVALID_FILE_PATH", path);
  }
  assert.equal(validatePackManifest(manifest({ version: "1/../2" })).error, "INVALID_PACK_VERSION");
  for (const path of ["../x.gd", "/abs.gd", "a/../b.gd", "a\\b.gd", ".hidden", "a//b"]) {
    const bad = manifest({ files: [{ path: "pack.json", sha256: sha("{}"), size: 2 }, { path, sha256: sha("x"), size: 1 }] });
    assert.equal(validatePackManifest(bad).error, "INVALID_FILE_PATH", path);
  }
  assert.equal(validatePackManifest(manifest({ files: [{ path: "props/a.json", sha256: sha("a"), size: 1 }] })).error, "PACK_JSON_MISSING");
  assert.equal(validatePackManifest(manifest({ files: [{ path: "pack.json", sha256: "nothex", size: 2 }] })).error, "INVALID_FILE_SHA256");
  const dupe = manifest({ files: [{ path: "pack.json", sha256: sha("{}"), size: 2 }, { path: "props/a.json", sha256: sha("a"), size: 1 }, { path: "props/A.json", sha256: sha("a"), size: 1 }] });
  assert.equal(validatePackManifest(dupe).error, "DUPLICATE_FILE_PATH");
  const big = manifest({ files: [{ path: "pack.json", sha256: sha("{}"), size: MOD_LIMITS.maxFileBytes + 1 }] });
  assert.equal(validatePackManifest(big).error, "FILE_TOO_LARGE");
  const many = Array.from({ length: 9 }, (_, i) => ({ path: `assets/f${i}.png`, sha256: sha(String(i)), size: MOD_LIMITS.maxFileBytes }));
  assert.equal(validatePackManifest(manifest({ files: [{ path: "pack.json", sha256: sha("{}"), size: 2 }, ...many] })).error, "PACK_TOO_LARGE");
});

test("pack identity: sha256 of sorted '<sha256> <path>' lines over every file including pack.json (DATA's cache/handshake key)", () => {
  const files = [{ path: "props/hall.json", sha256: sha("hall"), size: 4 }, { path: "pack.json", sha256: sha("{}"), size: 2 }];
  const expected = sha(`${sha("{}")} pack.json\n${sha("hall")} props/hall.json`);
  assert.equal(packIdentity(files), expected);
  assert.equal(validatePackManifest(manifest()).pack.sha256, expected);
});

test("index: anyone lists and reads; only an owner registers, blocks and unblocks; data survives a restart", async () => {
  const app = await fixture();
  try {
    assert.deepEqual((await get(app.url, "/api/mods/packs")).body, { ok: true, packs: [], downloadConfigured: true });
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const player = await signUp(app.url, "PLAYER");

    assert.equal((await post(app.url, "/api/admin/mods/register", { pack: manifest() })).status, 403);
    assert.equal((await post(app.url, "/api/admin/mods/register", { sessionToken: player, pack: manifest() })).status, 403);
    const registered = await post(app.url, "/api/admin/mods/register", { sessionToken: owner, pack: manifest() });
    assert.equal(registered.status, 200, JSON.stringify(registered.body));
    assert.equal(registered.body.pack.downloadBase, "https://files.example.test/lr3d/mods/neon_halls/1.0.0/");

    const again = await post(app.url, "/api/admin/mods/register", { sessionToken: owner, pack: manifest() });
    assert.equal(again.status, 409);
    assert.equal(again.body.code, "VERSION_EXISTS");
    await post(app.url, "/api/admin/mods/register", { sessionToken: owner, pack: manifest({ version: "1.1.0", name: "Neon Halls+" }) });

    const listed = await get(app.url, "/api/mods/packs");
    assert.equal(listed.body.packs.length, 1);
    assert.equal(listed.body.packs[0].version, "1.1.0", "the newest version is listed");
    assert.deepEqual(listed.body.packs[0].versions, ["1.1.0", "1.0.0"]);
    assert.equal(listed.body.packs[0].files.find((f) => f.path === "props/hall.json").sha256, sha("hall"));
    assert.ok(listed.body.packs[0].files.some((f) => f.path === "pack.json"), "pack.json is in the index files list");
    assert.match(listed.body.packs[0].sha256, /^[a-f0-9]{64}$/);
    assert.equal((await get(app.url, "/api/mods/packs/neon_halls?version=1.0.0")).body.pack.name, "Neon Halls");
    assert.equal((await get(app.url, "/api/mods/packs/nope_pack")).status, 404);
    assert.equal((await get(app.url, "/api/mods/packs/..%2Fetc")).status, 404);

    assert.equal((await post(app.url, "/api/admin/mods/block", { sessionToken: player, id: "neon_halls" })).status, 403);
    const blocked = await post(app.url, "/api/admin/mods/block", { sessionToken: owner, id: "neon_halls", reason: "crashes hosts" });
    assert.equal(blocked.body.pack.blocked, true);
    assert.equal(blocked.body.pack.blockedReason, "crashes hosts");

    await app.restart();
    const after = (await get(app.url, "/api/mods/packs/neon_halls")).body.pack;
    assert.equal(after.blocked, true, "block survives a restart");
    assert.equal(after.version, "1.1.0");
    const unblocked = await post(app.url, "/api/admin/mods/unblock", { sessionToken: owner, id: "neon_halls" });
    assert.equal(unblocked.body.pack.blocked, false);
    assert.equal(unblocked.body.pack.blockedReason, "");
  } finally {
    await app.close();
  }
});

test("mod reads have their own rate-limit bucket and the big register body is owner-route only", async () => {
  const app = await fixture({ modRateLimitMax: 3 });
  try {
    for (let i = 0; i < 3; i += 1) assert.equal((await get(app.url, "/api/mods/packs")).status, 200);
    const limited = await fetch(app.url + "/api/mods/packs");
    assert.equal(limited.status, 429);
    assert.ok(Number(limited.headers.get("retry-after")) >= 1);
    assert.equal((await get(app.url, "/api/health")).status, 200, "gameplay/API calls are not starved by mod polling");

    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const files = [{ path: "pack.json", sha256: sha("{}"), size: 2 },
      ...Array.from({ length: 499 }, (_, i) => ({ path: `assets/textures/highres/part_${String(i).padStart(3, "0")}_variant.png`, sha256: sha(String(i)), size: 10 }))];
    const body = { sessionToken: owner, pack: manifest({ files }) };
    assert.ok(JSON.stringify(body).length > 64 * 1024, "the manifest is over the normal 64 KB body cap");
    assert.equal((await post(app.url, "/api/admin/mods/register", body)).status, 200);
    assert.equal((await post(app.url, "/api/admin/flags", { ...body })).status, 413, "other routes keep the 64 KB cap");
  } finally {
    await app.close();
  }
});

test("deploy script: dry run touches nothing; apply copies, verifies on the file server, registers; versions are not overwritten", async () => {
  const app = await fixture();
  const work = await mkdtemp(join(tmpdir(), "mod-deploy-"));
  try {
    const owner = await signUp(app.url, "KYTTU", { adminProof: "server setup proof" });
    const packDir = join(work, "neon_halls");
    mkdirSync(join(packDir, "props"), { recursive: true });
    writeFileSync(join(packDir, "pack.json"), JSON.stringify({ schema: "lr.pack/1", id: "neon_halls", name: "Neon Halls", version: "2.0.0", author: "KT" }));
    writeFileSync(join(packDir, "props", "hall.json"), "hall");
    writeFileSync(join(packDir, ".DS_Store"), "junk");
    const files = join(work, "vps2");
    const logs = [];

    const dry = await deployPack({ directory: packDir, transport: localTransport(), remoteRoot: files, indexUrl: app.url,
      sessionToken: owner, log: (l) => logs.push(l) });
    assert.equal(dry.applied, false);
    assert.equal(existsSync(files), false, "dry run copies nothing");
    assert.equal((await get(app.url, "/api/mods/packs")).body.packs.length, 0, "dry run registers nothing");
    assert.deepEqual(dry.pack.files.map((f) => f.path), ["pack.json", "props/hall.json"], "dotfiles are not part of a pack");

    await assert.rejects(deployPack({ directory: packDir, transport: localTransport(), remoteRoot: files, indexUrl: app.url,
      sessionToken: "", apply: true, log() {} }), /LASTROUTE_OWNER_SESSION/);

    const done = await deployPack({ directory: packDir, transport: localTransport(), remoteRoot: files, indexUrl: app.url,
      sessionToken: owner, apply: true, log() {} });
    assert.equal(done.applied, true);
    assert.deepEqual(readdirSync(join(files, "neon_halls", "2.0.0")).sort(), ["pack.json", "props"]);
    assert.deepEqual(readdirSync(join(files, "neon_halls")), ["2.0.0"], "no staging folder left behind");
    const listed = (await get(app.url, "/api/mods/packs/neon_halls")).body.pack;
    assert.equal(listed.files.find((f) => f.path === "props/hall.json").sha256, sha("hall"));

    await assert.rejects(deployPack({ directory: packDir, transport: localTransport(), remoteRoot: files, indexUrl: app.url,
      sessionToken: owner, apply: true, log() {} }), /already exists/);

    // a file server that hands back different bytes must stop the deploy before anything is promoted or registered
    writeFileSync(join(packDir, "pack.json"), JSON.stringify({ schema: "lr.pack/1", id: "neon_halls", name: "Neon Halls", version: "2.0.1", author: "KT" }));
    const corrupt = { ...localTransport(), hashes: (_dir, paths) => new Map(paths.map((p) => [p, "0".repeat(64)])) };
    await assert.rejects(deployPack({ directory: packDir, transport: corrupt, remoteRoot: files, indexUrl: app.url,
      sessionToken: owner, apply: true, log() {} }), /upload check failed/);
    assert.deepEqual(readdirSync(join(files, "neon_halls")), ["2.0.0"], "the bad upload was discarded");
    assert.deepEqual((await get(app.url, "/api/mods/packs/neon_halls")).body.pack.versions, ["2.0.0"]);
  } finally {
    await rm(work, { recursive: true });
    await app.close();
  }
});

test("buildPack: a pack.json files list must cover every other file with exact hash and size", async () => {
  const work = await mkdtemp(join(tmpdir(), "mod-build-"));
  try {
    mkdirSync(join(work, "props"));
    writeFileSync(join(work, "props", "a.json"), "{}");
    const write = (files) => writeFileSync(join(work, "pack.json"), JSON.stringify({ schema: "lr.pack/1", id: "tiny_pack", name: "Tiny", version: "0.1", author: "KT", files }));
    write([{ path: "props/a.json", sha256: sha("something else"), size: 2 }]);
    assert.throws(() => buildPack(work), /does not match/);
    write([{ path: "props/a.json", sha256: sha("{}"), size: 3 }]);
    assert.throws(() => buildPack(work), /wrong size/);
    write([]);
    assert.throws(() => buildPack(work), /does not list props\/a.json/);
    write([{ path: "props/a.json", sha256: sha("{}"), size: 2 }]);
    assert.equal(buildPack(work).files.length, 2, "pack.json itself is added to the index list");
    writeFileSync(join(work, "notes.txt"), "x");
    writeFileSync(join(work, "pack.json"), JSON.stringify({ schema: "lr.pack/1", id: "tiny_pack", name: "Tiny", version: "0.1", author: "KT" }));
    assert.throws(() => buildPack(work), /INVALID_FILE_PATH/, "files the loader would refuse stop the deploy");
  } finally {
    await rm(work, { recursive: true });
  }
});


test("pack folders come from the game: ContentRegistry.PACK_KINDS plus data/kinds extensions", async () => {
  const root = await mkdtemp(join(tmpdir(), "pack-kinds-"));
  try {
    assert.equal(readPackKindsFromRepo(root), null, "not a checkout");
    mkdirSync(join(root, "src", "config"), { recursive: true });
    writeFileSync(join(root, "src", "config", "content_registry.gd"),
      'const PACK_KINDS := ["props", "regions", "hunters", "grabs", "sounds", "ambience", "floors", "walls", "ceilings", "rooms",\n\t"deaths", "emotes"]\n');
    mkdirSync(join(root, "data", "kinds"), { recursive: true });
    writeFileSync(join(root, "data", "kinds", "night_watch.json"), JSON.stringify({ nw_offices: {}, nw_calls: {}, props: {}, "Bad-Kind": {} }));
    writeFileSync(join(root, "data", "kinds", "broken.json"), "{ not json");
    assert.deepEqual(readPackKindsFromRepo(root), [...FALLBACK_PACK_KINDS, "nw_calls", "nw_offices"],
      "multi-line built-ins, then extension kinds sorted; duplicates, bad names and broken files ignored");
  } finally {
    await rm(root, { recursive: true });
  }
});
