import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";

// Regions (docs/SERVER_HOSTING.md): each VPS runs a region agent (region_agent.mjs) that spawns and stops dedicated
// headless hosts. The directory (this backend, VPS 3) stays the one list of rooms; agents report to it every few
// seconds and pick up spawn/stop requests in the answer, so an agent needs no inbound port.
//
// Agent requests are signed, never carry the token: x-lr-region, x-lr-time (ms) and x-lr-signature =
// HMAC-SHA256(token, `${region}\n${time}\n${body}`) hex. Tokens come from LASTROUTE_AGENT_TOKENS on the directory
// ("virginia:<token>,texas:<token>") and LASTROUTE_AGENT_TOKEN on each agent -- never from the repo.

export const REGION_PATTERN = /^[a-z][a-z0-9-]{1,23}$/;
export const SIGNATURE_WINDOW_MS = 60_000;
export const REGION_STALE_MS = 20_000;
const ROOM_CODE = /^[A-Z0-9]{5}$/;

export function signAgentRequest(token, region, time, body) {
  return createHmac("sha256", String(token)).update(`${region}\n${time}\n${body}`).digest("hex");
}

export function parseAgentTokens(value) {
  const tokens = new Map();
  for (const pair of String(value || "").split(",")) {
    const [region, token] = pair.split(":").map((part) => String(part || "").trim());
    if (REGION_PATTERN.test(region) && token && token.length >= 24) tokens.set(region, token);
  }
  return tokens;
}

function safeHexEqual(left, right) {
  const a = Buffer.from(String(left), "hex");
  const b = Buffer.from(String(right), "hex");
  return a.length === b.length && a.length > 0 && timingSafeEqual(a, b);
}

export function createRegions({ tokens = new Map(), now = Date.now } = {}) {
  const regions = new Map();     // region -> last report
  const pending = new Map();     // region -> [{ id, action, code, options }]
  const seen = new Map();        // region -> last accepted x-lr-time (replay guard)

  // The region a signed request comes from, or "" when the signature, clock or region is wrong.
  function verify(headers, rawBody) {
    const region = String(headers["x-lr-region"] || "");
    const time = Number(headers["x-lr-time"]);
    const token = tokens.get(region);
    if (!token || !Number.isFinite(time) || Math.abs(now() - time) > SIGNATURE_WINDOW_MS) return "";
    if (time <= (seen.get(region) || 0)) return "";
    if (!safeHexEqual(headers["x-lr-signature"], signAgentRequest(token, region, time, rawBody))) return "";
    seen.set(region, time);
    return region;
  }

  // An agent's status; returns the actions it should carry out now (each handed out once).
  function report(region, body) {
    const rooms = (Array.isArray(body.rooms) ? body.rooms : []).slice(0, 64)
      .filter((room) => ROOM_CODE.test(String(room?.code || "")))
      .map((room) => ({ code: String(room.code), pid: Number(room.pid) || 0, uptime: Number(room.uptime) || 0,
        rssMb: Number(room.rssMb) || 0 }));
    const capacity = Math.max(0, Math.min(64, Number(body.capacity) || 0));
    regions.set(region, {
      region,
      label: String(body.label || region).slice(0, 40),
      relay: body.relay && typeof body.relay === "object"
        ? { address: String(body.relay.address || "").slice(0, 100), port: Number(body.relay.port) || 0 } : null,
      capacity,
      free: Math.max(0, capacity - rooms.length),
      load: Number(body.load) || 0,
      cores: Number(body.cores) || 0,
      memFreeMb: Number(body.memFreeMb) || 0,
      rooms,
      version: String(body.version || "").slice(0, 40),
      lastSeen: now(),
    });
    const actions = pending.get(region) || [];
    pending.delete(region);
    return actions;
  }

  function live(region) {
    const status = regions.get(region);
    return Boolean(status && now() - status.lastSeen <= REGION_STALE_MS);
  }

  // Public: where a client may host or ping (no load details beyond free slots).
  function publicList() {
    return [...regions.values()].filter((status) => live(status.region)).map((status) => ({
      region: status.region, label: status.label, relay: status.relay, free: status.free,
    }));
  }

  // Owner/backend: queue a dedicated host in a region. [status, body]
  function requestSpawn(region, code, options = {}) {
    if (!live(region)) return [409, { ok: false, code: "REGION_OFFLINE", error: `Region ${region} is not reporting` }];
    const status = regions.get(region);
    const queued = (pending.get(region) || []).filter((entry) => entry.action === "spawn").length;
    if (status.free - queued <= 0) return [409, { ok: false, code: "REGION_FULL", error: `Region ${region} has no free slot` }];
    if (!ROOM_CODE.test(code)) return [400, { ok: false, code: "INVALID_ROOM_CODE", error: "Room codes are 5 letters/digits" }];
    const action = { id: randomBytes(6).toString("hex"), action: "spawn", code,
      options: { seed: String(options.seed || "").slice(0, 40), difficulty: Number(options.difficulty) || 0,
        idleMinutes: Math.min(240, Math.max(1, Number(options.idleMinutes) || 10)), spawnTicket: options.spawnTicket || "",
        // which game build must host the room: "rust" (the Rust port's headless host) or "" (the agent's default host)
        build: options.build === "rust" ? "rust" : "" } };
    pending.set(region, [...(pending.get(region) || []), action]);
    return [202, { ok: true, queued: { ...action, options: { ...action.options, spawnTicket: undefined } } }];
  }

  function requestStop(region, code) {
    if (!regions.has(region)) return [404, { ok: false, code: "UNKNOWN_REGION", error: "Unknown region" }];
    const action = { id: randomBytes(6).toString("hex"), action: "stop", code: String(code) };
    pending.set(region, [...(pending.get(region) || []), action]);
    return [202, { ok: true, queued: action }];
  }

  // Owner view: everything the agents reported.
  function ownerList() {
    return [...regions.values()].map((status) => ({ ...status, live: live(status.region),
      pending: (pending.get(status.region) || []).map((entry) => ({ action: entry.action, code: entry.code })) }));
  }

  // The region currently running a dedicated host for `code`, or "".
  function regionOf(code) {
    for (const status of regions.values()) if (live(status.region) && status.rooms.some((room) => room.code === code)) return status.region;
    return "";
  }

  return { verify, report, publicList, requestSpawn, requestStop, ownerList, regionOf, live,
    hasTokens: () => tokens.size > 0 };
}
