import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";

async function mapFixture({ tamper = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), "city-maps-"));
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const folder = join(root, "live", "test-town", "1");
  await mkdir(folder, { recursive: true });
  const chunk = Buffer.from("LRC1-chunk-bytes");
  const manifest = Buffer.from(JSON.stringify({ format: "LRCM1", id: "test-town", name: "Test Town", version: 1,
    chunks_xz: [1, 1], attribution: "Map data (c) OpenStreetMap contributors, ODbL 1.0", licence: "ODbL-1.0",
    files: { "c_0_0.bin": createHash("sha256").update(chunk).digest("hex") } }));
  await writeFile(join(folder, "manifest.json"), manifest);
  await writeFile(join(folder, "manifest.sig"), sign(null, manifest, privateKey));
  await writeFile(join(folder, "c_0_0.bin"), tamper ? Buffer.from("changed") : chunk);
  await writeFile(join(root, "live", "index.json"), JSON.stringify({ maps: [{ id: "test-town", version: 1 }] }));
  const server = createLastRouteServer({ cityMaps: { directory: root, ticketSecret: "test-secret",
    publicKeyPem: publicKey.export({ type: "spki", format: "pem" }) }, logger: { error() {}, log() {} } });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return { url, chunk, close: async () => { server.close(); await once(server, "close"); await rm(root, { recursive: true }); } };
}

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

test("city maps need a session, a ticket for the same map, and serve verified bytes", async () => {
  const app = await mapFixture();
  try {
    assert.equal((await post(app.url, "/api/city-maps/list", {})).status, 401);
    const account = await post(app.url, "/api/account/register", { username: "map_runner", password: "correct horse" });
    const sessionToken = account.body.sessionToken;
    const listed = await post(app.url, "/api/city-maps/list", { sessionToken });
    assert.equal(listed.status, 200);
    assert.deepEqual(listed.body.maps.map((map) => `${map.id}/${map.version}`), ["test-town/1"]);
    assert.match(listed.body.maps[0].attribution, /OpenStreetMap/);
    assert.equal((await post(app.url, "/api/city-maps/ticket", { sessionToken, id: "other", version: 1 })).status, 404);
    assert.equal((await post(app.url, "/api/city-maps/ticket", { sessionToken, id: "test-town", version: 1,
      roomCode: "ABCDE" })).status, 409, "a ticket for a room needs that room to be registered");
    const ticket = (await post(app.url, "/api/city-maps/ticket", { sessionToken, id: "test-town", version: 1 })).body.ticket;
    assert.ok(ticket);

    let response = await fetch(`${app.url}/api/city-maps/files/test-town/1/c_0_0.bin`);
    assert.equal(response.status, 401, "no ticket, no bytes");
    response = await fetch(`${app.url}/api/city-maps/files/test-town/1/c_0_0.bin`,
      { headers: { "x-city-map-ticket": `${ticket.split(".")[0]}.forged` } });
    assert.equal(response.status, 401);
    response = await fetch(`${app.url}/api/city-maps/files/test-town/1/c_0_0.bin`, { headers: { "x-city-map-ticket": ticket } });
    assert.equal(response.status, 200);
    assert.deepEqual(Buffer.from(await response.arrayBuffer()), app.chunk);
    assert.equal(response.headers.get("cache-control"), "private, max-age=600");
    response = await fetch(`${app.url}/api/city-maps/files/test-town/1/manifest.sig`, { headers: { "x-city-map-ticket": ticket } });
    assert.equal(response.status, 200);
    response = await fetch(`${app.url}/api/city-maps/files/test-town/2/c_0_0.bin`, { headers: { "x-city-map-ticket": ticket } });
    assert.equal(response.status, 403, "a ticket is bound to one map version");
  } finally {
    await app.close();
  }
});

test("a map whose files do not match the signed manifest is never listed", async () => {
  const app = await mapFixture({ tamper: true });
  try {
    const account = await post(app.url, "/api/account/register", { username: "map_runner", password: "correct horse" });
    const listed = await post(app.url, "/api/city-maps/list", { sessionToken: account.body.sessionToken });
    assert.deepEqual(listed.body.maps, []);
  } finally {
    await app.close();
  }
});

test("without a configured map directory the routes answer 503", async () => {
  const server = createLastRouteServer({});
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  try {
    const result = await post(`http://127.0.0.1:${server.address().port}`, "/api/city-maps/list", {});
    assert.equal(result.status, 503);
  } finally {
    server.close();
  }
});
