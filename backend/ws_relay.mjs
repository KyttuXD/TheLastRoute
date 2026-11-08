// WebSocket room relay (RFC 6455, zero dependencies) for browser builds.
//
// Browsers cannot open UDP sockets or listen for connections, so the ENet relay (enet_relay.mjs) is out of reach for a
// web build. This is the same idea over WebSocket: the room's host and every guest make an OUTBOUND connection to
// `/api/ws`, and this hub forwards Godot multiplayer packets between them. It never inspects or runs game logic.
//
// Protocol (one WebSocket per player):
//   1. client -> server, text:   {"t":"hello","code":"ABCDE","role":"host"|"guest","token":"<host token, host only>","ticket":"<join ticket, reserved rooms>"}
//   2. server -> client, text:   {"t":"welcome","id":N,"role":"host"|"guest"}   (host is always peer 1)
//      server -> host,   text:   {"t":"peer","id":N,"on":true|false}            (a guest joined / left)
//      server -> guest,  text:   {"t":"host","on":false}                        (then the socket is closed)
//      server -> client, text:   {"t":"error","message":"..."} then close
//   3. data, binary, both ways:  [int32 LE peer][uint8 mode][uint8 channel][payload]
//      client -> server: peer = target (1 host, N guest id, 0 everyone else, -N everyone except N)
//      server -> client: peer = the sender's id
//
// A room only exists here while its host is connected; the directory (server.mjs) still owns listing, tokens and codes.
// A guest cannot join a room whose host has not connected yet, and when the host leaves every guest is disconnected.
import { createHash, randomInt, timingSafeEqual } from "node:crypto";

const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
export const WS_PATH = "/api/ws";
export const MAX_PAYLOAD_BYTES = 65_535;
const HEADER_BYTES = 6;
const MAX_MESSAGE_BYTES = MAX_PAYLOAD_BYTES + HEADER_BYTES + 64;
const HELLO_TIMEOUT_MS = 5_000;
const PING_INTERVAL_MS = 25_000;
const IDLE_TIMEOUT_MS = 70_000;
const MAX_QUEUED_BYTES = 4 * 1024 * 1024;
const BUCKET_CAPACITY = 2_000;
const BUCKET_REFILL_PER_SECOND = 1_500;

const OP = { CONT: 0, TEXT: 1, BINARY: 2, CLOSE: 8, PING: 9, PONG: 10 };

function safeEqual(left, right) {
  const a = Buffer.from(String(left ?? ""));
  const b = Buffer.from(String(right ?? ""));
  return a.length === b.length && timingSafeEqual(a, b);
}

function frame(opcode, payload) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload ?? "");
  let header;
  if (body.length < 126) {
    header = Buffer.from([0x80 | opcode, body.length]);
  } else if (body.length <= 0xffff) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(body.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(body.length), 2);
  }
  return Buffer.concat([header, body]);
}

const DEBUG = /^(1|true|yes|on)$/i.test(String(process.env.LASTROUTE_WS_DEBUG || ""));

export function createWsRelay({
  roomByCode,
  maxPlayers = 16,
  now = Date.now,
  logger = console,
  path = WS_PATH,
} = {}) {
  /** @type {Map<string, {code: string, host: object|null, guests: Map<number, object>}>} */
  const hubs = new Map();
  const clients = new Set();
  let statsPackets = 0;
  let statsBytes = 0;

  const dbg = (...args) => { if (DEBUG) logger.log("[ws-relay]", ...args); };

  function sendText(client, object) {
    if (client.closed) return;
    client.socket.write(frame(OP.TEXT, JSON.stringify(object)));
  }

  function sendBinary(client, buffer) {
    if (client.closed) return;
    if (client.socket.writableLength > MAX_QUEUED_BYTES) {
      closeClient(client, 1013, "slow consumer");
      return;
    }
    client.socket.write(frame(OP.BINARY, buffer));
  }

  function closeClient(client, code = 1000, reason = "") {
    if (client.closed) return;
    dbg("close", client.role || "?", client.id, client.hub?.code || "-", code, reason);
    client.closed = true;
    clearTimeout(client.helloTimer);
    clients.delete(client);
    try {
      const reasonBytes = Buffer.from(String(reason).slice(0, 100));
      const body = Buffer.alloc(2 + reasonBytes.length);
      body.writeUInt16BE(code, 0);
      reasonBytes.copy(body, 2);
      client.socket.end(frame(OP.CLOSE, body));
    } catch { /* socket already gone */ }
    setTimeout(() => client.socket.destroy(), 1000).unref();
    detach(client);
  }

  function detach(client) {
    const hub = client.hub;
    if (!hub) return;
    client.hub = null;
    if (client.role === "host") {
      if (hub.host === client) {
        hub.host = null;
        hubs.delete(hub.code);
        for (const guest of [...hub.guests.values()]) {
          sendText(guest, { t: "host", on: false });
          closeClient(guest, 4001, "host left");
        }
        hub.guests.clear();
      }
    } else if (hub.guests.get(client.id) === client) {
      hub.guests.delete(client.id);
      if (hub.host) sendText(hub.host, { t: "peer", id: client.id, on: false });
    }
  }

  function newGuestId(hub) {
    for (let attempt = 0; attempt < 32; attempt += 1) {
      const id = randomInt(2, 0x7fffffff);
      if (!hub.guests.has(id)) return id;
    }
    return 0;
  }

  function onHello(client, message) {
    clearTimeout(client.helloTimer);
    let hello;
    try { hello = JSON.parse(message.toString("utf8")); } catch { hello = null; }
    if (!hello || hello.t !== "hello") return fail(client, "EXPECTED HELLO");
    const code = String(hello.code || "").trim().toUpperCase();
    const room = code ? roomByCode(code) : null;
    dbg("hello", hello.role, code, room ? `room ok transport=${room.transport}` : "no room");
    logger.log(`[ws] hello ${hello.role} ${code} ${room ? `room ok transport=${room.transport}` : "no room"}`);
    if (!room) return fail(client, "ROOM NOT FOUND");
    if (room.transport !== "ws") return fail(client, "ROOM DOES NOT USE THE WEB RELAY");
    if (hello.role === "host") {
      if (!safeEqual(hello.token, room.token)) return fail(client, "INVALID HOST TOKEN");
      const old = hubs.get(code);
      if (old?.host) closeClient(old.host, 4000, "replaced by a new host connection");
      const hub = { code, host: client, guests: new Map() };
      hubs.set(code, hub);
      client.hub = hub;
      client.role = "host";
      client.id = 1;
      sendText(client, { t: "welcome", id: 1, role: "host" });
      return;
    }
    if (hello.role !== "guest") return fail(client, "UNKNOWN ROLE");
    const hub = hubs.get(code);
    if (!hub?.host) return fail(client, "HOST IS NOT CONNECTED YET");
    // A reserved private room admits only guests holding a one-time join ticket (the same gate the ENet relay uses).
    if (typeof room.admitGuest === "function" && !room.admitGuest(String(hello.ticket || ""))) return fail(client, "ROOM IS PRIVATE");
    if (hub.guests.size + 1 >= maxPlayers) return fail(client, "ROOM IS FULL");
    const id = newGuestId(hub);
    if (!id) return fail(client, "COULD NOT ASSIGN A PEER ID");
    client.hub = hub;
    client.role = "guest";
    client.id = id;
    hub.guests.set(id, client);
    sendText(client, { t: "welcome", id, role: "guest" });
    sendText(hub.host, { t: "peer", id, on: true });
  }

  function fail(client, message) {
    sendText(client, { t: "error", message });
    closeClient(client, 4002, message);
  }

  function takeToken(client) {
    const timestamp = now();
    const elapsed = Math.max(0, timestamp - client.bucketAt) / 1000;
    client.bucketAt = timestamp;
    client.tokens = Math.min(BUCKET_CAPACITY, client.tokens + elapsed * BUCKET_REFILL_PER_SECOND);
    if (client.tokens < 1) return false;
    client.tokens -= 1;
    return true;
  }

  function route(client, data) {
    if (!client.hub || data.length < HEADER_BYTES) return;
    if (data.length - HEADER_BYTES > MAX_PAYLOAD_BYTES) return closeClient(client, 1009, "packet too large");
    if (!takeToken(client)) return closeClient(client, 1008, "rate limit");
    const hub = client.hub;
    const target = data.readInt32LE(0);
    statsPackets += 1;
    statsBytes += data.length;
    const out = Buffer.from(data);
    out.writeInt32LE(client.id, 0);
    const everyone = () => {
      const list = [];
      if (hub.host) list.push(hub.host);
      for (const guest of hub.guests.values()) list.push(guest);
      return list;
    };
    if (target === 0) {
      for (const peer of everyone()) if (peer !== client) sendBinary(peer, out);
    } else if (target < 0) {
      for (const peer of everyone()) if (peer !== client && peer.id !== -target) sendBinary(peer, out);
    } else if (target === 1) {
      if (client !== hub.host && hub.host) sendBinary(hub.host, out);
    } else {
      const peer = hub.guests.get(target);
      if (peer && peer !== client) sendBinary(peer, out);
    }
  }

  function onMessage(client, opcode, payload) {
    client.lastSeen = now();
    if (!client.hub && !client.helloDone) {
      client.helloDone = true;
      if (opcode !== OP.TEXT) return fail(client, "EXPECTED HELLO");
      return onHello(client, payload);
    }
    if (opcode === OP.BINARY) return route(client, payload);
    if (opcode !== OP.TEXT || client.role !== "host" || payload.length > 256) return; // keepalive pings and the like
    let control = null;
    try { control = JSON.parse(payload.toString("utf8")); } catch { /* not JSON */ }
    if (control?.t === "kick") {
      const guest = client.hub?.guests.get(Number(control.id));
      if (guest) closeClient(guest, 4005, "removed by host");
    }
  }

  function parse(client) {
    for (;;) {
      const buffer = client.buffer;
      if (buffer.length < 2) return;
      const first = buffer[0];
      const second = buffer[1];
      const fin = Boolean(first & 0x80);
      const opcode = first & 0x0f;
      const masked = Boolean(second & 0x80);
      let length = second & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (buffer.length < 4) return;
        length = buffer.readUInt16BE(2);
        offset = 4;
      } else if (length === 127) {
        if (buffer.length < 10) return;
        const big = buffer.readBigUInt64BE(2);
        if (big > BigInt(MAX_MESSAGE_BYTES)) return closeClient(client, 1009, "frame too large");
        length = Number(big);
        offset = 10;
      }
      if (!masked) return closeClient(client, 1002, "client frames must be masked");
      if (length > MAX_MESSAGE_BYTES) return closeClient(client, 1009, "frame too large");
      if (buffer.length < offset + 4 + length) return;
      const mask = buffer.subarray(offset, offset + 4);
      const payload = Buffer.from(buffer.subarray(offset + 4, offset + 4 + length));
      for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i & 3];
      client.buffer = buffer.subarray(offset + 4 + length);
      if (opcode >= 8) {
        if (opcode === OP.CLOSE) return closeClient(client, 1000, "bye");
        if (opcode === OP.PING) client.socket.write(frame(OP.PONG, payload));
        client.lastSeen = now();
        continue;
      }
      if (opcode !== OP.CONT) {
        client.fragments = [];
        client.fragmentOpcode = opcode;
        client.fragmentBytes = 0;
      } else if (!client.fragments) {
        return closeClient(client, 1002, "unexpected continuation");
      }
      client.fragments.push(payload);
      client.fragmentBytes += payload.length;
      if (client.fragmentBytes > MAX_MESSAGE_BYTES) return closeClient(client, 1009, "message too large");
      if (fin) {
        const message = client.fragments.length === 1 ? client.fragments[0] : Buffer.concat(client.fragments);
        const messageOpcode = client.fragmentOpcode;
        client.fragments = null;
        onMessage(client, messageOpcode, message);
        if (client.closed) return;
      }
    }
  }

  /** Handle an HTTP `upgrade`. Returns true when the request was for this relay (accepted or refused). */
  function handleUpgrade(request, socket, head) {
    let pathname = "";
    try { pathname = new URL(request.url || "/", "http://relay.invalid").pathname; } catch { /* bad URL */ }
    if (pathname !== path) return false;
    const key = request.headers["sec-websocket-key"];
    const upgrade = String(request.headers.upgrade || "").toLowerCase();
    if (upgrade !== "websocket" || !key || request.headers["sec-websocket-version"] !== "13") {
      socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
      return true;
    }
    const accept = createHash("sha1").update(`${key}${GUID}`).digest("base64");
    socket.write([
      "HTTP/1.1 101 Switching Protocols",
      "Upgrade: websocket",
      "Connection: Upgrade",
      `Sec-WebSocket-Accept: ${accept}`,
      "", "",
    ].join("\r\n"));
    socket.setNoDelay(true);
    socket.setTimeout(0);
    const client = {
      socket, buffer: Buffer.alloc(0), closed: false, helloDone: false, hub: null, role: "", id: 0,
      tokens: BUCKET_CAPACITY, bucketAt: now(), lastSeen: now(), fragments: null, fragmentOpcode: 0, fragmentBytes: 0,
      helloTimer: null,
    };
    clients.add(client);
    logger.log(`[ws] connection opened ${client.id ?? ""}`);
    client.helloTimer = setTimeout(() => { if (!client.helloDone) closeClient(client, 4003, "hello timeout"); }, HELLO_TIMEOUT_MS);
    client.helloTimer.unref?.();
    socket.on("data", (chunk) => {
      client.buffer = client.buffer.length ? Buffer.concat([client.buffer, chunk]) : chunk;
      if (client.buffer.length > MAX_MESSAGE_BYTES * 4) return closeClient(client, 1009, "buffer overflow");
      parse(client);
    });
    socket.on("error", () => closeClient(client, 1006, "socket error"));
    // HTTP server sockets allow half-open connections: a peer that just vanished only shows up as 'end'.
    socket.on("end", () => closeClient(client, 1006, "peer ended"));
    socket.on("close", () => { client.closed = true; clearTimeout(client.helloTimer); clients.delete(client); detach(client); });
    if (head?.length) socket.emit("data", head);
    return true;
  }

  const keepAlive = setInterval(() => {
    const timestamp = now();
    for (const client of [...clients]) {
      if (timestamp - client.lastSeen > IDLE_TIMEOUT_MS) closeClient(client, 1001, "idle");
      else if (!client.closed) client.socket.write(frame(OP.PING, Buffer.alloc(0)));
    }
  }, PING_INTERVAL_MS);
  keepAlive.unref();

  return {
    handleUpgrade,
    /** The directory dropped this room (unregistered or expired): disconnect everyone in it. */
    closeRoom(code) {
      const hub = hubs.get(code);
      if (!hub) return false;
      for (const guest of [...hub.guests.values()]) closeClient(guest, 4004, "room closed");
      if (hub.host) closeClient(hub.host, 4004, "room closed");
      hubs.delete(code);
      return true;
    },
    hasHost(code) {
      return Boolean(hubs.get(code)?.host);
    },
    status() {
      let guests = 0;
      for (const hub of hubs.values()) guests += hub.guests.size;
      return { enabled: true, path, rooms: hubs.size, guests, packets: statsPackets, bytes: statsBytes };
    },
    close() {
      clearInterval(keepAlive);
      for (const client of [...clients]) closeClient(client, 1001, "server closing");
      hubs.clear();
    },
  };
}
