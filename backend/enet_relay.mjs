import { randomBytes, timingSafeEqual } from "node:crypto";
import { createSocket } from "node:dgram";

// A tiny authenticated UDP tunnel for Godot's existing ENet transport.
// Both players make outbound UDP connections to this public socket, so a
// player hosting behind CGNAT (notably Starlink Residential) never needs an
// inbound port, UPnP, or router configuration.
export const RELAY_MAGIC = Buffer.from("LRR1", "ascii");
export const RELAY_HEADER_BYTES = 14;
export const RELAY_MAX_DATAGRAM_BYTES = 1472;
export const RelayFrame = Object.freeze({
  HOST_REGISTER: 1,
  HOST_ACK: 2,
  GUEST_JOIN: 3,
  GUEST_ACK: 4,
  GUEST_DATA: 5,
  HOST_DATA: 6,
  ERROR: 7,
  HOST_PROMOTE: 8,
  HOST_MIGRATING: 9,
  // The room is gone (its host quit, crashed or was killed): guests leave to the menu. Older clients also get ERROR.
  HOST_LEFT: 10,
  // The host is alive: payload = host token. Sent every ~250 ms from a background thread (EnetRelayTunnel), so a
  // frozen main loop does not drop a live room; a host that sends it is timed out after hostAliveTtlMs.
  HOST_ALIVE: 11,
});
export const HOST_LEFT_MESSAGE = "HOST LEFT // ROOM CLOSED";
export const HANDOFF_FAILED_MESSAGE = "HOST TRANSFER FAILED // ROOM CLOSED";
// Deliberate host handoff (task 010 phase 2). A host that already quiesced its route and delivered a checkpoint to one
// capable guest names that guest's relay session and a handoff epoch. HOST_PROMOTE then carries
// "LRH1|<epoch>|<token>" to that session only; HOST_MIGRATING carries "LRH1|<epoch>" while waiting and
// "LRH1|<epoch>|ready" once the successor registered. Legacy releases (no candidate) keep the old empty notices.
export const HANDOFF_PREFIX = "LRH1";

export function handoffPayload(epoch, tail = "") {
  return Buffer.from(tail ? `${HANDOFF_PREFIX}|${epoch}|${tail}` : `${HANDOFF_PREFIX}|${epoch}`, "utf8");
}

// any five letters/digits: generated codes and an owner's reserved codes (KATHY, AKIRI); unknown codes route nowhere
const ROOM_CODE_PATTERN = /^[A-Z0-9]{5}$/;

function safeEqual(leftValue, rightValue) {
  const left = Buffer.from(String(leftValue || ""));
  const right = Buffer.from(String(rightValue || ""));
  const length = Math.max(left.length, right.length, 1);
  const paddedLeft = Buffer.alloc(length);
  const paddedRight = Buffer.alloc(length);
  left.copy(paddedLeft);
  right.copy(paddedRight);
  return timingSafeEqual(paddedLeft, paddedRight) && left.length === right.length;
}

export function encodeRelayFrame(type, sessionId, code, payload = Buffer.alloc(0)) {
  const roomCode = String(code || "").trim().toUpperCase();
  if (!ROOM_CODE_PATTERN.test(roomCode)) throw new Error("INVALID RELAY ROOM CODE");
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const frame = Buffer.alloc(RELAY_HEADER_BYTES + body.length);
  RELAY_MAGIC.copy(frame, 0);
  frame.writeUInt8(type, 4);
  frame.writeUInt32LE(sessionId >>> 0, 5);
  frame.write(roomCode, 9, 5, "ascii");
  body.copy(frame, RELAY_HEADER_BYTES);
  return frame;
}

export function decodeRelayFrame(frame) {
  if (!Buffer.isBuffer(frame) || frame.length < RELAY_HEADER_BYTES
      || frame.length > RELAY_MAX_DATAGRAM_BYTES
      || !frame.subarray(0, 4).equals(RELAY_MAGIC)) return null;
  const code = frame.toString("ascii", 9, 14);
  if (!ROOM_CODE_PATTERN.test(code)) return null;
  return {
    type: frame.readUInt8(4),
    sessionId: frame.readUInt32LE(5),
    code,
    payload: frame.subarray(RELAY_HEADER_BYTES),
  };
}

function endpointKey(address, port) {
  return `${address}:${port}`;
}

function newSessionId(sessions) {
  for (;;) {
    const id = randomBytes(4).readUInt32LE(0);
    if (id !== 0 && !sessions.has(id)) return id;
  }
}

export function createEnetRelay({
  roomByCode,
  promoteRoomHost = null,
  now = Date.now,
  logger = console,
  hostTtlMs = 15_000,
  // KT: "make the room drop 1s or no delay". Hosts that send HOST_ALIVE (every 250 ms) are gone after this long;
  // older hosts (HOST_REGISTER every 3 s) keep hostTtlMs.
  hostAliveTtlMs = 1_000,
  // Called when a room's live session ends here (host quit, timed out, or closeRoom): the directory forgets it.
  onRoomClosed = null,
  sessionTtlMs = 45_000,
  maximumGuestsPerRoom = 15,
  guestAllocationWindowMs = 10_000,
  maximumGuestAllocationsPerAddress = 4,
  // A deliberate handoff whose named successor has not registered by then closes the room (no guess election).
  handoffTtlMs = 20_000,
  // A 4-player room moves ~400 KB/s through one host endpoint; the kernel default (~208 KB on Linux) leaves
  // little room for an event-loop pause. Linux caps this at net.core.rmem_max / wmem_max.
  socketBufferBytes = 4 * 1024 * 1024,
} = {}) {
  if (typeof roomByCode !== "function") throw new TypeError("roomByCode is required");

  const socket = createSocket({ type: "udp4", recvBufferSize: socketBufferBytes, sendBufferSize: socketBufferBytes });
  const hosts = new Map();
  const sessions = new Map();
  const guestSessions = new Map();
  const guestEndpoints = new Map();
  const guestAllocationRates = new Map();
  const migrations = new Map();
  let listening = false;
  let relayPort = 0;
  let lastError = "";
  // What happened to every datagram since start; relayStatus() reports it so a deploy shows real numbers.
  const counters = {
    forwardedGuest: 0,
    forwardedHost: 0,
    forwardedBytes: 0,
    droppedInvalid: 0,
    droppedUnknownSession: 0,
    droppedNoHost: 0,
    droppedWrongEndpoint: 0,
    sendErrors: 0,
    roomsClosed: 0,
  };

  function send(type, sessionId, code, payload, endpoint) {
    if (!endpoint || !listening) return;
    const frame = encodeRelayFrame(type, sessionId, code, payload);
    socket.send(frame, endpoint.port, endpoint.address, (error) => {
      if (error) counters.sendErrors += 1;
    });
  }

  function sendError(code, endpoint, message) {
    send(RelayFrame.ERROR, 0, code, Buffer.from(String(message).slice(0, 160)), endpoint);
  }

  function removeSession(sessionId) {
    const session = sessions.get(sessionId);
    if (!session) return;
    sessions.delete(sessionId);
    guestSessions.delete(session.guestKey);
    guestEndpoints.delete(session.endpointKey);
  }

  function allowGuestAllocation(code, address) {
    const timestamp = now();
    const key = `${code}|${address}`;
    const existing = guestAllocationRates.get(key);
    const rate = !existing || timestamp >= existing.resetAt
      ? { count: 0, resetAt: timestamp + guestAllocationWindowMs }
      : existing;
    rate.count += 1;
    guestAllocationRates.set(key, rate);
    return rate.count <= maximumGuestAllocationsPerAddress;
  }

  function removeHost(code) {
    hosts.delete(code);
    migrations.delete(code);
    for (const [sessionId, session] of sessions) {
      if (session.code === code) removeSession(sessionId);
    }
  }

  function migrationCandidates(code) {
    const timestamp = now();
    return [...sessions.entries()]
      .filter(([, session]) => session.code === code
        && timestamp - session.lastSeen <= sessionTtlMs)
      .sort((left, right) => left[1].joinedAt - right[1].joinedAt
        || left[0] - right[0]);
  }

  function announceMigration(code, migration) {
    for (const [sessionId, session] of sessions) {
      if (session.code !== code) continue;
      if (sessionId === migration.sessionId) {
        send(RelayFrame.HOST_PROMOTE, sessionId, code,
          migration.handoff ? handoffPayload(migration.epoch, migration.token) : Buffer.from(migration.token),
          session.guest);
      } else {
        send(RelayFrame.HOST_MIGRATING, sessionId, code,
          migration.handoff ? handoffPayload(migration.epoch) : Buffer.alloc(0), session.guest);
      }
    }
    migration.lastAnnounced = now();
  }

  function liveSession(code, sessionId) {
    const session = sessions.get(sessionId);
    return Boolean(session && session.code === code && now() - session.lastSeen <= sessionTtlMs);
  }

  // A prepared handoff: the host chose `candidateSession` (it reported that session over ENet after the checkpoint
  // barrier). An unknown or stale candidate refuses WITHOUT touching the live host: the room stays as it was.
  function beginHandoff(code, candidateSession, epoch) {
    const existing = migrations.get(code);
    if (existing) {
      if (existing.handoff && existing.sessionId === candidateSession && existing.epoch === epoch) {
        announceMigration(code, existing);   // an idempotent retry of the same release
        return { migrating: true };
      }
      return { migrating: false, error: "HOST TRANSFER ALREADY IN PROGRESS" };
    }
    if (!hosts.has(code)) return { migrating: false, error: "ROOM HOST HAS NOT REACHED THE RELAY" };
    if (!liveSession(code, candidateSession)) return { migrating: false, error: "HANDOFF CANDIDATE IS NOT CONNECTED" };
    if (typeof promoteRoomHost !== "function") return { migrating: false, error: "HOST TRANSFER UNAVAILABLE" };
    const promoted = promoteRoomHost(code);
    const token = String(promoted?.token || "");
    if (!token) return { migrating: false, error: "HOST TRANSFER UNAVAILABLE" };
    // committed: the old token is dead from here on, so the old host can never take the room back
    const migration = { sessionId: candidateSession, token, epoch, handoff: true, startedAt: now(), lastAnnounced: 0 };
    hosts.delete(code);
    migrations.set(code, migration);
    announceMigration(code, migration);
    return { migrating: true };
  }

  function beginHostMigration(code) {
    if (migrations.has(code)) {
      announceMigration(code, migrations.get(code));
      return true;
    }
    const candidates = migrationCandidates(code);
    if (!candidates.length || typeof promoteRoomHost !== "function") {
      removeHost(code);
      return false;
    }
    const promoted = promoteRoomHost(code);
    const token = String(promoted?.token || "");
    if (!token) {
      removeHost(code);
      return false;
    }
    const [sessionId] = candidates[0];
    const migration = { sessionId, token, startedAt: now(), lastAnnounced: 0 };
    hosts.delete(code);
    migrations.set(code, migration);
    announceMigration(code, migration);
    return true;
  }

  function hostTtl(host) {
    return host?.alive ? hostAliveTtlMs : hostTtlMs;
  }

  // The live session is over: every guest is told (HOST_LEFT, plus ERROR for older clients) and dropped, and the
  // directory forgets the room. Host migration happens only when a host hands over on purpose (release-host).
  function closeRoom(code, reason = HOST_LEFT_MESSAGE) {
    let told = 0;
    for (const [sessionId, session] of sessions) {
      if (session.code !== code) continue;
      send(RelayFrame.HOST_LEFT, sessionId, code, Buffer.from(reason), session.guest);
      sendError(code, session.guest, reason);
      removeSession(sessionId);
      told += 1;
    }
    const known = hosts.has(code) || migrations.has(code);
    hosts.delete(code);
    migrations.delete(code);
    counters.roomsClosed += 1;
    if (typeof onRoomClosed === "function") onRoomClosed(code, reason);
    return known || told > 0;
  }

  function prune() {
    const timestamp = now();
    for (const [code, host] of hosts) {
      if (!roomByCode(code)) removeHost(code);
      else if (timestamp - host.lastSeen > hostTtl(host)) closeRoom(code);
    }
    for (const [sessionId, session] of sessions) {
      if (!roomByCode(session.code) || timestamp - session.lastSeen > sessionTtlMs) {
        removeSession(sessionId);
      }
    }
    for (const [key, rate] of guestAllocationRates) {
      if (timestamp >= rate.resetAt) guestAllocationRates.delete(key);
    }
    for (const [code, migration] of migrations) {
      if (!roomByCode(code)) {
        removeHost(code);
        continue;
      }
      const candidate = sessions.get(migration.sessionId);
      if (migration.handoff) {
        // the named successor holds the checkpoint; nobody else may be guessed into authority
        if (!candidate || candidate.code !== code || timestamp - migration.startedAt > handoffTtlMs) {
          closeRoom(code, HANDOFF_FAILED_MESSAGE);
        } else if (timestamp - migration.lastAnnounced >= 2_000) {
          announceMigration(code, migration);
        }
        continue;
      }
      if (!candidate || candidate.code !== code
          || timestamp - candidate.lastSeen > sessionTtlMs) {
        migrations.delete(code);
        beginHostMigration(code);
      } else if (timestamp - migration.lastAnnounced >= 2_000) {
        announceMigration(code, migration);
      }
    }
  }

  socket.on("message", (packet, remote) => {
    const frame = decodeRelayFrame(packet);
    if (!frame) {
      counters.droppedInvalid += 1;
      return;
    }
    const endpoint = { address: remote.address, port: remote.port };
    const room = roomByCode(frame.code);

    if (frame.type === RelayFrame.HOST_REGISTER) {
      if (!room || !safeEqual(frame.payload.toString("utf8"), room.token)) {
        sendError(frame.code, endpoint, "HOST AUTHORIZATION FAILED");
        return;
      }
      hosts.set(frame.code, { ...endpoint, lastSeen: now(), alive: Boolean(hosts.get(frame.code)?.alive) });
      const migration = migrations.get(frame.code);
      if (migration) {
        removeSession(migration.sessionId);
        migrations.delete(frame.code);
        for (const [sessionId, session] of sessions) {
          if (session.code === frame.code) {
            if (migration.handoff) {
              // each guest learns the successor is up, tagged with its own session so a stray copy is ignored
              send(RelayFrame.HOST_MIGRATING, sessionId, frame.code,
                handoffPayload(migration.epoch, "ready"), session.guest);
            } else {
              send(RelayFrame.HOST_MIGRATING, 0, frame.code,
                Buffer.alloc(0), session.guest);
            }
          }
        }
      }
      send(RelayFrame.HOST_ACK, 0, frame.code, Buffer.alloc(0), endpoint);
      return;
    }

    if (frame.type === RelayFrame.HOST_ALIVE) {
      // from the host's keep-alive thread, on its own socket: proves the token, never moves the host endpoint
      const host = hosts.get(frame.code);
      if (!room || !host || !safeEqual(frame.payload.toString("utf8"), room.token)) return;
      host.lastSeen = now();
      host.alive = true;
      return;
    }

    if (frame.type === RelayFrame.GUEST_JOIN) {
      const host = hosts.get(frame.code);
      if (!room || !host || now() - host.lastSeen > hostTtl(host)) {
        sendError(frame.code, endpoint, "ROOM HOST HAS NOT REACHED THE RELAY");
        return;
      }
      if (frame.payload.length < 8 || frame.payload.length > 32) {
        sendError(frame.code, endpoint, "INVALID GUEST NONCE");
        return;
      }
      // A reserved private room admits only guests whose nonce is a join ticket from /api/rooms/reserved/resolve.
      if (typeof room.admitGuest === "function" && !room.admitGuest(frame.payload)) {
        sendError(frame.code, endpoint, "PRIVATE ROOM // ACCESS REQUIRED");
        return;
      }
      const guestKey = `${frame.code}|${endpointKey(remote.address, remote.port)}|${frame.payload.toString("hex")}`;
      const endpointSessionKey = `${frame.code}|${endpointKey(remote.address, remote.port)}`;
      let sessionId = guestSessions.get(guestKey);
      // A retrying client may regenerate its nonce, but one UDP endpoint is
      // still one guest. Rebind that endpoint's existing session instead of
      // consuming another room slot for every nonce.
      if (!sessionId) sessionId = guestEndpoints.get(endpointSessionKey);
      if (!sessionId) {
        if (!allowGuestAllocation(frame.code, remote.address)) {
          sendError(frame.code, endpoint, "TOO MANY RELAY JOIN ATTEMPTS");
          return;
        }
        let roomGuests = 0;
        for (const session of sessions.values()) if (session.code === frame.code) roomGuests += 1;
        if (roomGuests >= maximumGuestsPerRoom) {
          sendError(frame.code, endpoint, "ROOM RELAY IS FULL");
          return;
        }
        sessionId = newSessionId(sessions);
        guestSessions.set(guestKey, sessionId);
        guestEndpoints.set(endpointSessionKey, sessionId);
        sessions.set(sessionId, {
          code: frame.code,
          guest: endpoint,
          guestKey,
          endpointKey: endpointSessionKey,
          lastSeen: now(),
          joinedAt: now(),
        });
      } else {
        const session = sessions.get(sessionId);
        guestSessions.delete(session.guestKey);
        session.guestKey = guestKey;
        guestSessions.set(guestKey, sessionId);
        session.guest = endpoint;
        session.lastSeen = now();
      }
      send(RelayFrame.GUEST_ACK, sessionId, frame.code, Buffer.alloc(0), endpoint);
      return;
    }

    const session = sessions.get(frame.sessionId);
    if (!session || session.code !== frame.code || !room) {
      counters.droppedUnknownSession += 1;
      return;
    }

    if (frame.type === RelayFrame.GUEST_DATA) {
      if (session.guest.address !== remote.address || session.guest.port !== remote.port) {
        counters.droppedWrongEndpoint += 1;
        return;
      }
      const host = hosts.get(frame.code);
      if (!host || now() - host.lastSeen > hostTtl(host)) {
        counters.droppedNoHost += 1;
        return;
      }
      session.lastSeen = now();
      counters.forwardedGuest += 1;
      counters.forwardedBytes += frame.payload.length;
      send(RelayFrame.GUEST_DATA, frame.sessionId, frame.code, frame.payload, host);
    } else if (frame.type === RelayFrame.HOST_DATA) {
      const host = hosts.get(frame.code);
      if (!host || host.address !== remote.address || host.port !== remote.port) {
        counters.droppedWrongEndpoint += 1;
        return;
      }
      host.lastSeen = now();
      session.lastSeen = now();
      counters.forwardedHost += 1;
      counters.forwardedBytes += frame.payload.length;
      send(RelayFrame.HOST_DATA, frame.sessionId, frame.code, frame.payload, session.guest);
    }
  });

  function safeBufferSizes() {
    try {
      return { receive: socket.getRecvBufferSize(), send: socket.getSendBufferSize() };
    } catch {
      return undefined;
    }
  }

  socket.on("listening", () => {
    listening = true;
    relayPort = socket.address().port;
    lastError = "";
    logger.log(`ENet gameplay relay: UDP ${relayPort}`);
  });
  socket.on("error", (error) => {
    lastError = error.message;
    logger.error(`ENet gameplay relay error: ${error.message}`);
  });
  socket.on("close", () => {
    listening = false;
    hosts.clear();
    sessions.clear();
    guestSessions.clear();
    guestEndpoints.clear();
    guestAllocationRates.clear();
    migrations.clear();
  });

  // every 250 ms: a killed host's room must be gone within about a second (hostAliveTtlMs + one tick)
  const pruneTimer = setInterval(prune, Math.max(100, Math.min(250, hostAliveTtlMs / 4)));
  pruneTimer.unref();
  socket.once("close", () => clearInterval(pruneTimer));

  socket.relayStatus = () => ({
    enabled: true,
    listening,
    port: relayPort,
    hosts: hosts.size,
    sessions: sessions.size,
    counters: { ...counters },
    socketBuffers: listening ? safeBufferSizes() : undefined,
    error: lastError || undefined,
  });
  socket.requestHostMigration = (code) => beginHostMigration(String(code || "").trim().toUpperCase());
  // Deliberate handoff to a named relay session: {migrating, error}. Never closes or demotes a live host on refusal.
  socket.requestHostHandoff = (code, candidateSession, epoch) => {
    const session = Number(candidateSession);
    const handoffEpoch = Number(epoch);
    if (!Number.isInteger(session) || session <= 0 || session > 0xffffffff
        || !Number.isInteger(handoffEpoch) || handoffEpoch <= 0 || handoffEpoch > 0x7fffffff) {
      return { migrating: false, error: "INVALID HANDOFF REQUEST" };
    }
    return beginHandoff(String(code || "").trim().toUpperCase(), session, handoffEpoch);
  };
  socket.closeRoom = (code, reason) => closeRoom(String(code || "").trim().toUpperCase(), reason);
  socket.pruneNow = prune;
  return socket;
}
