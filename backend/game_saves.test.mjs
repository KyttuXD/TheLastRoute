import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { SAVE_MAX_BYTES } from "./game_saves.mjs";

const PATH = "/api/v1/saves/last-route-godot/run";

async function fixture(options = {}) {
  const root = await mkdtemp(join(tmpdir(), "saves-"));
  const server = createLastRouteServer({ accountStorePath: join(root, "accounts.json"), saveDirectory: join(root, "saves"),
    logger: { error() {}, log() {} }, ...options });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return { url, root, close: async () => { server.close(); await once(server, "close"); await rm(root, { recursive: true }); } };
}

async function signUp(url, username) {
  const response = await fetch(`${url}/api/account/register`, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password: "correct horse battery" }) });
  assert.equal(response.status, 200);
  return (await response.json()).sessionToken;
}

async function call(url, method, path, token, body) {
  const response = await fetch(url + path, { method,
    headers: { "content-type": "application/json", ...(token ? { "x-session-token": token } : {}) },
    ...(body === undefined ? {} : { body: typeof body === "string" ? body : JSON.stringify(body) }) });
  return { status: response.status, body: await response.json() };
}

const run = (level) => ({ format: 2, saved_at: 100, place: `LEVEL ${level}`, payload: { seed_text: "S", level } });

test("a signed-in account stores, reads and deletes its run; accounts never see each other's", async () => {
  const server = await fixture();
  try {
    const kt = await signUp(server.url, "KYTTU_SAVER");
    const other = await signUp(server.url, "OTHER_SAVER");
    assert.deepEqual((await call(server.url, "GET", PATH, kt)).body.data, null);
    const put = await call(server.url, "PUT", PATH, kt, { data: run(-2), updatedAt: 100 });
    assert.equal(put.status, 200);
    assert.equal(put.body.updatedAt, 100);
    const got = await call(server.url, "GET", PATH, kt);
    assert.equal(got.status, 200);
    assert.deepEqual(got.body.data, run(-2));
    assert.equal(got.body.updatedAt, 100);
    assert.equal((await call(server.url, "GET", PATH, other)).body.data, null, "another account read KT's run");
    assert.ok(existsSync(join(server.root, "saves", "KYTTU_SAVER.json")));
    assert.ok(!readdirSync(join(server.root, "saves")).some((name) => name.endsWith(".tmp")), "a temp file was left behind");
    const cleared = await call(server.url, "PUT", PATH, kt, { data: null, updatedAt: 150 });
    assert.equal(cleared.status, 200);
    assert.equal((await call(server.url, "GET", PATH, kt)).body.data, null, "a cleared run came back");
  } finally {
    await server.close();
  }
});

test("newest wins: an older upload is refused with the stored time", async () => {
  const server = await fixture();
  try {
    const kt = await signUp(server.url, "NEWEST_WINS");
    await call(server.url, "PUT", PATH, kt, { data: run(-5), updatedAt: 500 });
    const stale = await call(server.url, "PUT", PATH, kt, { data: run(-1), updatedAt: 400 });
    assert.equal(stale.status, 409);
    assert.equal(stale.body.code, "STALE");
    assert.equal(stale.body.updatedAt, 500);
    assert.deepEqual((await call(server.url, "GET", PATH, kt)).body.data, run(-5));
    assert.equal((await call(server.url, "PUT", PATH, kt, { data: run(-6), updatedAt: 500 })).status, 200, "an equal time is accepted");
  } finally {
    await server.close();
  }
});

test("beta generation slot is isolated from the preserved old run", async () => {
  const server = await fixture();
  const current = "/api/v1/saves/last-route-godot/run-world-1";
  try {
    const kt = await signUp(server.url, "WORLD_SAVER");
    const old = run(-2);
    const fresh = { ...run(-3), world_generation: 1 };
    assert.equal((await call(server.url, "PUT", PATH, kt, { data: old, updatedAt: 100 })).status, 200);
    assert.equal((await call(server.url, "PUT", current, kt, { data: fresh, updatedAt: 200 })).status, 200);
    assert.deepEqual((await call(server.url, "GET", PATH, kt)).body.data, old);
    assert.deepEqual((await call(server.url, "GET", current, kt)).body.data, fresh);
    assert.equal((await call(server.url, "PUT", current, kt, { data: null, updatedAt: 300 })).status, 200);
    assert.deepEqual((await call(server.url, "GET", PATH, kt)).body.data, old);
    assert.equal((await call(server.url, "PUT", current, kt, { data: fresh, updatedAt: 400 })).status, 200);
    assert.equal((await call(server.url, "PUT", PATH, kt, { data: null, updatedAt: 500 })).status, 200);
    assert.deepEqual((await call(server.url, "GET", current, kt)).body.data, fresh);
    for (const slot of ["run-world-0", "run-world-2", "run-world-999", "run-world-"]) {
      const path = `/api/v1/saves/last-route-godot/${slot}`;
      assert.equal((await call(server.url, "GET", path, kt)).status, 404);
      assert.equal((await call(server.url, "PUT", path, kt, { data: fresh, updatedAt: 600 })).status, 404);
    }
  } finally {
    await server.close();
  }
});

test("saves need a session, an allow-listed game/slot, a well-formed body and fit the size cap", async () => {
  const server = await fixture();
  try {
    const kt = await signUp(server.url, "GUARDED_SAVES");
    assert.equal((await call(server.url, "GET", PATH, "")).status, 401);
    assert.equal((await call(server.url, "PUT", PATH, "not-a-session", { data: run(-1) })).status, 401);
    assert.equal((await call(server.url, "GET", "/api/v1/saves/last-route-godot/slot2", kt)).status, 404);
    assert.equal((await call(server.url, "PUT", "/api/v1/saves/other-game/run", kt, { data: run(-1) })).status, 404);
    assert.equal((await call(server.url, "GET", "/api/v1/saves/..%2F..%2Faccounts/run", kt)).status, 404);
    assert.equal((await call(server.url, "PUT", PATH, kt, "{not json")).status, 400);
    assert.equal((await call(server.url, "PUT", PATH, kt, { updatedAt: 1 })).status, 400, "a body without data");
    assert.equal((await call(server.url, "PUT", PATH, kt, { data: [1, 2] })).status, 400, "an array is not a save");
    const huge = { data: { blob: "x".repeat(SAVE_MAX_BYTES) }, updatedAt: 1 };
    const tooLarge = await call(server.url, "PUT", PATH, kt, huge);
    assert.equal(tooLarge.status, 413);
    assert.equal((await call(server.url, "GET", PATH, kt)).body.data, null, "an oversized save was stored");
  } finally {
    await server.close();
  }
});

test("writes are rate-limited per account", async () => {
  const server = await fixture({ saveOptions: { writeLimit: 3, writeWindowMs: 60_000 } });
  try {
    const kt = await signUp(server.url, "RATE_SAVES");
    for (let i = 1; i <= 3; i += 1) {
      assert.equal((await call(server.url, "PUT", PATH, kt, { data: run(-i), updatedAt: i })).status, 200);
    }
    const limited = await call(server.url, "PUT", PATH, kt, { data: run(-4), updatedAt: 4 });
    assert.equal(limited.status, 429);
    assert.equal(limited.body.code, "SAVE_RATE_LIMIT");
    assert.equal((await call(server.url, "GET", PATH, kt)).status, 200, "reads are not write-limited");
  } finally {
    await server.close();
  }
});

test("without a save directory the route answers 503, not a crash", async () => {
  const server = await fixture({ saveDirectory: null });
  try {
    const kt = await signUp(server.url, "NO_SAVE_DIR");
    assert.equal((await call(server.url, "GET", PATH, kt)).status, 503);
  } finally {
    await server.close();
  }
});
