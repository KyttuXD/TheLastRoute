import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { randomBytes, createHash } from "node:crypto";
import http from "node:http";
import { createDirectoryServer } from "./server.mjs";

// A tiny RFC 6455 client (masked frames), so the tests need no dependency and no global WebSocket (Node 18 has none).
function frame(opcode, payload, { fin = true } = {}) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const mask = randomBytes(4);
  let header;
  if (body.length < 126) header = Buffer.from([(fin ? 0x80 : 0) | opcode, 0x80 | body.length]);
  else if (body.length <= 0xffff) {
    header = Buffer.alloc(4);
    header[0] = (fin ? 0x80 : 0) | opcode;
    header[1] = 0x80 | 126;
    header.writeUInt16BE(body.length, 2);
  } else throw new Error("test frame too large");
  const masked = Buffer.from(body);
  for (let i = 0; i < masked.length; i += 1) masked[i] ^= mask[i & 3];
  return Buffer.concat([header, mask, masked]);
}

async function connect(url, path = "/api/ws") {
  const key = randomBytes(16).toString("base64");
  const request = http.request(`${url}${path}`, {
    headers: { Connection: "Upgrade", Upgrade: "websocket", "Sec-WebSocket-Key": key, "Sec-WebSocket-Version": "13" },
  });
  request.end();
  const [response, socket, head] = await new Promise((resolve, reject) => {
    request.once("upgrade", (...args) => resolve(args));
    request.once("response", (res) => reject(new Error(`no upgrade: HTTP ${res.statusCode}`)));
    request.once("error", reject);
  });
  assert.equal(response.headers["sec-websocket-accept"],
    createHash("sha1").update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64"));
  const client = { socket, messages: [], closed: false, closeCode: 0, waiters: [], buffer: Buffer.alloc(0) };
  const push = (message) => {
    client.messages.push(message);
    for (const waiter of [...client.waiters]) waiter();
  };
  const parse = () => {
    for (;;) {
      const b = client.buffer;
      if (b.length < 2) return;
      const opcode = b[0] & 0x0f;
      let length = b[1] & 0x7f;
      let offset = 2;
      if (length === 126) { if (b.length < 4) return; length = b.readUInt16BE(2); offset = 4; }
      if (b.length < offset + length) return;
      const payload = b.subarray(offset, offset + length);
      client.buffer = b.subarray(offset + length);
      if (opcode === 1) push({ text: JSON.parse(payload.toString("utf8")) });
      else if (opcode === 2) push({ binary: Buffer.from(payload) });
      else if (opcode === 8) {
        client.closed = true;
        client.closeCode = payload.length >= 2 ? payload.readUInt16BE(0) : 1005;
        push({ close: client.closeCode });
      } else if (opcode === 9) socket.write(frame(10, payload));
    }
  };
  socket.on("data", (chunk) => { client.buffer = Buffer.concat([client.buffer, chunk]); parse(); });
  socket.on("close", () => { client.closed = true; for (const waiter of [...client.waiters]) waiter(); });
  client.send = (text) => socket.write(frame(1, JSON.stringify(text)));
  client.sendData = (target, payload, mode = 2, channel = 0) => {
    const header = Buffer.alloc(6);
    header.writeInt32LE(target, 0);
    header[4] = mode;
    header[5] = channel;
    socket.write(frame(2, Buffer.concat([header, Buffer.from(payload)])));
  };
  client.next = (predicate = () => true, timeoutMs = 2000) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => { cleanup(); reject(new Error("timed out waiting for a message")); }, timeoutMs);
    const check = () => {
      const index = client.messages.findIndex(predicate);
      if (index >= 0) { const [message] = client.messages.splice(index, 1); cleanup(); resolve(message); }
      else if (client.closed) { cleanup(); reject(new Error("socket closed while waiting")); }
    };
    const cleanup = () => { clearTimeout(timer); client.waiters = client.waiters.filter((w) => w !== check); };
    client.waiters.push(check);
    check();
  });
  client.close = () => socket.destroy();
  if (head.length) { client.buffer = Buffer.from(head); parse(); }
  return client;
}

async function fixture(options = {}) {
  const server = createDirectoryServer(options);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  return {
    server, url,
    close: async () => { server.close(); server.closeAllConnections?.(); await once(server, "close"); },
  };
}

async function registerWebRoom(url, code = "WSTST", transport = "ws") {
  const response = await fetch(`${url}/api/enet/rooms/register`, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ code, players: 1, transport }),
  });
  assert.equal(response.status, 201);
  return response.json();
}

async function hostAndGuest(url, code = "WSTST") {
  const { token } = await registerWebRoom(url, code);
  const host = await connect(url);
  host.send({ t: "hello", code, role: "host", token });
  assert.deepEqual(await host.next(), { text: { t: "welcome", id: 1, role: "host" } });
  const guest = await connect(url);
  guest.send({ t: "hello", code, role: "guest" });
  const welcome = (await guest.next()).text;
  assert.equal(welcome.t, "welcome");
  assert.ok(welcome.id >= 2);
  const joined = (await host.next()).text;
  assert.deepEqual(joined, { t: "peer", id: welcome.id, on: true });
  return { token, host, guest, guestId: welcome.id };
}

test("a web room is listed with the ws transport and no UDP address", async () => {
  const app = await fixture({ relayAddress: "relay.test", relayPort: 1105 });
  try {
    const registered = await registerWebRoom(app.url);
    assert.equal(registered.room.transport, "ws");
    assert.equal(registered.room.address, "");
    assert.equal(registered.room.port, 0);
    assert.equal(registered.room.wsPath, "/api/ws");
    const resolved = await (await fetch(`${app.url}/api/enet/rooms/WSTST`)).json();
    assert.equal(resolved.room.transport, "ws");
    const health = await (await fetch(`${app.url}/api/health`)).json();
    assert.equal(health.wsRelay.enabled, true);
    // An ordinary ENet room still needs a port.
    const bad = await fetch(`${app.url}/api/enet/rooms/register`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ code: "ENETX", port: 0 }),
    });
    assert.equal(bad.status, 400);
  } finally { await app.close(); }
});

test("host and guest exchange packets through the relay with the right sender ids", async () => {
  const app = await fixture();
  try {
    const { host, guest, guestId } = await hostAndGuest(app.url);
    guest.sendData(1, "hello host", 2, 3);
    const atHost = (await host.next((m) => m.binary)).binary;
    assert.equal(atHost.readInt32LE(0), guestId);
    assert.equal(atHost[4], 2);
    assert.equal(atHost[5], 3);
    assert.equal(atHost.subarray(6).toString(), "hello host");
    host.sendData(guestId, "hello guest");
    const atGuest = (await guest.next((m) => m.binary)).binary;
    assert.equal(atGuest.readInt32LE(0), 1);
    assert.equal(atGuest.subarray(6).toString(), "hello guest");
    host.sendData(0, "to everyone");
    assert.equal((await guest.next((m) => m.binary)).binary.subarray(6).toString(), "to everyone");
    host.close(); guest.close();
  } finally { await app.close(); }
});

test("a second guest is relayed through the host only for the target the sender names", async () => {
  const app = await fixture();
  try {
    const { host, guest, guestId } = await hostAndGuest(app.url);
    const second = await connect(app.url);
    second.send({ t: "hello", code: "WSTST", role: "guest" });
    const welcome = (await second.next()).text;
    await host.next((m) => m.text?.on === true);
    host.sendData(-welcome.id, "all but the second");
    assert.equal((await guest.next((m) => m.binary)).binary.subarray(6).toString(), "all but the second");
    await assert.rejects(second.next((m) => m.binary, 300));
    second.sendData(guestId, "guest to guest");
    assert.equal((await guest.next((m) => m.binary)).binary.readInt32LE(0), welcome.id);
    host.close(); guest.close(); second.close();
  } finally { await app.close(); }
});

test("hello is required, and a wrong token, unknown room or ENet room are refused", async () => {
  const app = await fixture();
  try {
    const { token } = await registerWebRoom(app.url);
    await registerWebRoom(app.url, "ENETR", "enet").catch(() => {});
    let client = await connect(app.url);
    client.send({ t: "hello", code: "WSTST", role: "host", token: `${token}x` });
    assert.equal((await client.next()).text.message, "INVALID HOST TOKEN");
    client = await connect(app.url);
    client.send({ t: "hello", code: "NOPE1", role: "guest" });
    assert.equal((await client.next()).text.message, "ROOM NOT FOUND");
    client = await connect(app.url);
    client.send({ t: "hello", code: "WSTST", role: "guest" });
    assert.equal((await client.next()).text.message, "HOST IS NOT CONNECTED YET");
    client = await connect(app.url);
    client.sendData(1, "no hello first");
    assert.equal((await client.next()).text.message, "EXPECTED HELLO");
    const enet = await fetch(`${app.url}/api/enet/rooms/register`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ code: "ENETR", port: 8421 }),
    });
    assert.equal(enet.status, 201);
    client = await connect(app.url);
    client.send({ t: "hello", code: "ENETR", role: "guest" });
    assert.equal((await client.next()).text.message, "ROOM DOES NOT USE THE WEB RELAY");
  } finally { await app.close(); }
});

test("when the host leaves, guests are told and disconnected; the host sees guests leave", async () => {
  const app = await fixture();
  try {
    const { host, guest, guestId } = await hostAndGuest(app.url);
    guest.close();
    assert.deepEqual((await host.next((m) => m.text?.t === "peer")).text, { t: "peer", id: guestId, on: false });
    const again = await connect(app.url);
    again.send({ t: "hello", code: "WSTST", role: "guest" });
    await again.next((m) => m.text?.t === "welcome");
    host.close();
    assert.deepEqual((await again.next((m) => m.text?.t === "host")).text, { t: "host", on: false });
    assert.equal((await again.next((m) => m.close)).close, 4001);
  } finally { await app.close(); }
});

test("unregistering the room closes its relay connections", async () => {
  const app = await fixture();
  try {
    const { token, host, guest } = await hostAndGuest(app.url);
    const response = await fetch(`${app.url}/api/enet/rooms/unregister`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ code: "WSTST", token }),
    });
    assert.equal((await response.json()).removed, true);
    assert.equal((await host.next((m) => m.close)).close, 4004);
    assert.equal((await guest.next((m) => m.close)).close, 4004);
  } finally { await app.close(); }
});

test("a forwarded address is trusted from a loopback proxy only", async () => {
  const app = await fixture({ apiRateLimitMax: 2 });
  try {
    // Two different client addresses behind the local proxy do not share one rate-limit bucket.
    const get = (address) => fetch(`${app.url}/api/health`, { headers: { "x-forwarded-for": address } });
    assert.equal((await get("203.0.113.1")).status, 200);
    assert.equal((await get("203.0.113.1")).status, 200);
    assert.equal((await get("203.0.113.1")).status, 429);
    assert.equal((await get("203.0.113.2")).status, 200);
  } finally { await app.close(); }
});

test("the host can remove a guest", async () => {
  const app = await fixture();
  try {
    const { host, guest, guestId } = await hostAndGuest(app.url);
    host.send({ t: "kick", id: guestId });
    assert.equal((await guest.next((m) => m.close)).close, 4005);
    host.close();
  } finally { await app.close(); }
});

test("a web room stays listed while its host holds the relay socket, and expires after it leaves", async () => {
  const app = await fixture({ roomTtlMs: 400 });
  try {
    const { token } = await registerWebRoom(app.url);
    const host = await connect(app.url);
    host.send({ t: "hello", code: "WSTST", role: "host", token });
    await host.next((m) => m.text?.t === "welcome");
    await new Promise((resolve) => setTimeout(resolve, 1000)); // well past the 400 ms listing TTL, no heartbeats sent
    assert.equal((await fetch(`${app.url}/api/enet/rooms/WSTST`)).status, 200);
    host.close();
    await new Promise((resolve) => setTimeout(resolve, 1200));
    assert.equal((await fetch(`${app.url}/api/enet/rooms/WSTST`)).status, 404);
  } finally { await app.close(); }
});

test("a reserved room admits only guests with a valid join ticket", async () => {
  const { createWsRelay } = await import("./ws_relay.mjs");
  const room = { code: "KATHY", token: "host-token", transport: "ws", admitGuest: (ticket) => ticket === "good-ticket" };
  const relay = createWsRelay({ roomByCode: (code) => (code === "KATHY" ? room : null) });
  const server = http.createServer((request, response) => { response.writeHead(404); response.end(); });
  server.on("upgrade", (request, socket, head) => { if (!relay.handleUpgrade(request, socket, head)) socket.destroy(); });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  try {
    const host = await connect(url);
    host.send({ t: "hello", code: "KATHY", role: "host", token: "host-token" });
    await host.next((m) => m.text?.t === "welcome");
    for (const ticket of [undefined, "", "wrong"]) {
      const guest = await connect(url);
      guest.send({ t: "hello", code: "KATHY", role: "guest", ...(ticket === undefined ? {} : { ticket }) });
      assert.equal((await guest.next()).text.message, "ROOM IS PRIVATE");
    }
    const guest = await connect(url);
    guest.send({ t: "hello", code: "KATHY", role: "guest", ticket: "good-ticket" });
    assert.equal((await guest.next()).text.t, "welcome");
    host.close(); guest.close();
  } finally { relay.close(); server.close(); server.closeAllConnections?.(); }
});
