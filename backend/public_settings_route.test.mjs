import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createLastRouteServer } from "./server.mjs";

async function open(options) {
  const server = createLastRouteServer(options);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return { url, close: async () => { server.close(); await once(server, "close"); } };
}

async function get(url, path) {
  const response = await fetch(`${url}${path}`);
  return { status: response.status, body: await response.json() };
}

async function post(url, path, body) {
  const response = await fetch(`${url}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: response.status, body: await response.json() };
}

test("public route settings: anyone reads them, only an owner changes them, and a bad value changes nothing", async () => {
  const dir = await mkdtemp(join(tmpdir(), "public-settings-route-"));
  const app = await open({
    now: () => 50_000,
    adminPassword: "server setup proof",
    publicRouteStorePath: join(dir, "godot_publicroom.json"),
    publicSettingsStorePath: join(dir, "public_settings.json"),
    apiRateLimitMax: 100,
    accountRateLimitMax: 100,
  });
  try {
    let result = await get(app.url, "/api/public/settings");
    assert.equal(result.status, 200);
    assert.equal(result.body.settings.join_per_minute, 20);
    assert.ok(result.body.specs.heartbeat_seconds, "the specs travel with the values");

    result = await post(app.url, "/api/admin/public/settings", { changes: { join_per_minute: 30 } });
    assert.equal(result.status, 403, "no owner proof, no change");
    assert.equal(result.body.code, "ADMIN_OWNER_REQUIRED");

    const login = await post(app.url, "/api/dev/login", { password: "server setup proof" });
    const devToken = login.body.devToken;

    result = await post(app.url, "/api/admin/public/settings", { devToken, changes: { join_per_minute: 0 } });
    assert.equal(result.status, 400);
    assert.equal(result.body.key, "join_per_minute");

    result = await post(app.url, "/api/admin/public/settings", { devToken, changes: { join_per_minute: 30, bogus: 1 } });
    assert.equal(result.status, 400, "one bad key refuses the whole change");
    assert.equal((await get(app.url, "/api/public/settings")).body.settings.join_per_minute, 20);

    result = await post(app.url, "/api/admin/public/settings", { devToken, changes: { join_per_minute: 30, client_host_fallback: false } });
    assert.equal(result.status, 200);
    assert.equal(result.body.settings.join_per_minute, 30);
    assert.equal((await get(app.url, "/api/public/settings")).body.settings.client_host_fallback, false);
  } finally {
    await app.close();
    await rm(dir, { recursive: true, force: true });
  }
});
