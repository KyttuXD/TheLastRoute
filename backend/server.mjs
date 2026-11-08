import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { createAccountService } from "./account_service.mjs";
import { CITY_MAP_TICKET_HEADER, createCityMaps } from "./city_maps.mjs";
import { SKIN_ACCOUNT_PATH, SKIN_ARMS_HEADER, SKIN_MAX_BYTES, SKIN_TOKEN_HEADER, createSkins } from "./skins.mjs";
import { SAVE_MAX_BYTES, SAVE_PATH, createGameSaves } from "./game_saves.mjs";
import { createGuestNames } from "./guest_names.mjs";
import { createEnetRelay } from "./enet_relay.mjs";
import { createWsRelay, WS_PATH } from "./ws_relay.mjs";
import { createRoomService } from "./room_service.mjs";
import { createPublicRoute, PUBLIC_CODES } from "./public_route.mjs";
import { createPublicSettingsStore, PUBLIC_SETTING_SPECS } from "./public_settings.mjs";
import { createModeration } from "./moderation.mjs";
import { createModPacks } from "./mod_packs.mjs";
import { CRASH_MAX_BODY_BYTES, createCrashReports } from "./crash_reports.mjs";
import { createReservedRooms } from "./reserved_rooms.mjs";
import { createRegions, parseAgentTokens } from "./regions.mjs";

// Which deployment this is (production, test): shown in /api/health so nobody mistakes one for the other.
const instanceName = String(process.env.LASTROUTE_INSTANCE || "production").replace(/[^A-Za-z0-9_-]/g, "").slice(0, 32) || "production";
// Generated room codes skip I, O, 0 and 1; an owner's reserved private room may use any five letters or digits
// (KATHY, AKIRI), so lookups take both and registration of a non-reserved code still needs the generated alphabet.
const ENET_CODE_PATTERN = /^[A-Z0-9]{5}$/;
// Temporary display probe for the web build's ?diag=1 overlay: logged to the journal, never stored, at most once a second.
const diagProbe = { last: 0 };
const GENERATED_CODE_PATTERN = /^[A-HJ-NP-Z2-9]{5}$/;
// The PUBLIC host posts to the server every second or so; if it vanishes, its
// listing must lapse quickly so a guest can take over.
const PUBLIC_ENET_TTL_MS = 15_000;
const MAX_BODY_BYTES = 64 * 1024;
// a pack manifest lists up to 500 files with a sha256 each; only the owner-only register call may send this much
const MOD_REGISTER_MAX_BODY_BYTES = 256 * 1024;
const MOD_PACK_PATH = /^\/api\/mods\/packs\/([a-z0-9][a-z0-9_]{2,31})$/;
const DEFAULT_ADMIN_TICKET_TTL_MS = 5 * 60 * 1000;
const OWNER_TICKET_TTL_MS = 10 * 60 * 1000;
const DEFAULT_RATE_LIMIT_WINDOW_MS = 60 * 1000;
const DEFAULT_DEV_TOKEN_TTL_MS = 12 * 60 * 60 * 1000;
const ACCOUNT_RATE_LIMIT_PATHS = new Set([
  "/api/guest/name",
  "/api/account/register",
  "/api/account/login",
  "/api/account/session",
  "/api/account/password",
  "/api/account/rename",
	"/api/admin/roster",
	"/api/admin/roster/set",
	"/api/admin/roster/remove",
  "/api/dev/login",
  "/api/dev/reload",   // accepts adminProof (the setup password): must not be a cheap guessing oracle
  "/api/enet/admin-tickets/issue",
  "/api/enet/owner-tickets/issue",
  "/api/moderation/integrity",
  "/api/admin/flags",
  "/api/admin/flags/set",
  "/api/admin/builds/add",
  "/api/admin/builds/remove",
]);
const UPDATE_CHANNELS = ["debug", "release"];
const UPDATE_PLATFORMS = ["linux", "windows", "windows_legacy", "macos", "web", "steamvr", "quest", "phone"];
const MIME_TYPES = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".ico", "image/x-icon"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".mp3", "audio/mpeg"],
  [".ogg", "audio/ogg"],
  [".png", "image/png"],
  [".svg", "image/svg+xml; charset=utf-8"],
  [".wasm", "application/wasm"],
  [".wav", "audio/wav"],
  [".webm", "video/webm"],
]);

function responseHeaders() {
  return {
    "cache-control": "no-store",
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET, HEAD, POST, PUT, OPTIONS",
    "access-control-allow-headers": `content-type, ${CITY_MAP_TICKET_HEADER}, ${SKIN_TOKEN_HEADER}, ${SKIN_ARMS_HEADER}`,
  };
}

function json(response, status, value, extraHeaders = {}) {
  if (status === 204) {
    response.writeHead(status, { ...responseHeaders(), ...extraHeaders });
    response.end();
    return;
  }
  const body = JSON.stringify(value);
  response.writeHead(status, {
    ...responseHeaders(),
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    ...extraHeaders,
  });
  response.end(body);
}

async function readJson(request, maxBytes = MAX_BODY_BYTES) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > maxBytes) throw Object.assign(new Error("REQUEST BODY TOO LARGE"), { status: 413 });
    chunks.push(chunk);
  }
  if (!chunks.length) return {};
  try {
    const parsed = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") throw new Error();
    return parsed;
  } catch {
    throw Object.assign(new Error("INVALID JSON BODY"), { status: 400 });
  }
}

function normalizeEnetCode(value) {
  const code = String(value || "").trim().toUpperCase();
  return ENET_CODE_PATTERN.test(code) ? code : "";
}

function cleanAddress(value) {
  const address = String(value || "").trim();
  if (address.startsWith("::ffff:")) return address.slice(7);
  if (address === "::1") return "127.0.0.1";
  return address;
}

function requesterAddress(request, trustProxy) {
  // A proxy running on this machine (the site's same-origin /godot2-api forwarder) is trusted for the real client
  // address: only local processes can connect from loopback, so a remote caller cannot forge the header.
  const direct = cleanAddress(request.socket.remoteAddress);
  if (trustProxy || direct === "127.0.0.1") {
    const forwarded = String(request.headers["x-forwarded-for"] || "").split(",")[0].trim();
    if (forwarded) return cleanAddress(forwarded);
  }
  return cleanAddress(request.socket.remoteAddress);
}

// Which game a room belongs to ("last_route" or "city"); servers and clients that predate the field mean "last_route".
function publicGameId(value) {
  const game = String(value ?? "").trim().toLowerCase();
  return Object.hasOwn(PUBLIC_CODES, game) ? game : "last_route";
}

function cleanGame(value) {
  const text = String(value ?? "").trim().toLowerCase();
  return /^[a-z][a-z0-9_]{0,15}$/.test(text) ? text : "last_route";
}

function publicEnetRoom(room, timestamp, relayAddress = "", relayPort = 0) {
  const common = {
    code: room.code,
    game: room.game || "last_route",
    players: room.players,
    sessionActive: room.sessionActive,
    daytime: Boolean(room.daytime),
    protocol: room.protocol || "",   // the host build's NetProtocol fingerprint ("" from older builds)
    dedicated: Boolean(room.dedicated),
    region: room.region || "",
    expiresIn: Math.max(0, Math.ceil((room.expiresAt - timestamp) / 1000)),
    ...(room.owner ? { owner: room.owner } : {}),   // hosted rooms: the signed-in creator who may run the room
  };
  if (room.transport === "ws") {
    // Browser-capable room: players connect to this server's WebSocket relay over the same origin they already use.
    return { ...common, transport: "ws", address: "", port: 0, wsPath: WS_PATH };
  }
  const relayed = Boolean(relayAddress && relayPort);
  return {
    ...common,
    transport: relayed ? "relay" : "direct",
    address: relayed ? relayAddress : room.address,
    port: relayed ? relayPort : room.port,
  };
}

function publicUpdateManifest(updates, channel, platform) {
  const entry = updates?.[channel]?.[platform];
  if (!entry || typeof entry !== "object") return null;
  const version = String(entry.version || "").trim().replace(/^v/i, "");
  const url = String(entry.url || "").trim();
  const file = String(entry.file || "").trim();
  const sha256 = String(entry.sha256 || "").trim().toLowerCase();
  const revision = String(entry.revision || "").trim().toLowerCase();
  const relativeUrl = /^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$/.test(file)
    ? `/api/updates/files/${encodeURIComponent(file)}`
    : "";
  if (!/^\d+(?:\.\d+){2,3}$/.test(version) || (!url.startsWith("https://") && !relativeUrl)) return null;
  if (!/^[a-f0-9]{64}$/.test(sha256)) return null;
  if (revision && !/^[a-f0-9]{7,40}$/.test(revision)) return null;
  // Phone and Quest APKs carry the publish build number (git rev-list --count, the Android versionCode); clients only
  // install a higher one, so a missing or invalid number withholds the entry rather than offering an unusable update.
  const mobile = platform === "phone" || platform === "quest";
  const build = Number.isSafeInteger(entry.build) && entry.build > 0 ? entry.build : undefined;
  // phones/Quest and (since the 2026-10 rework) macOS / Windows / Linux clients compare build numbers: withhold entries
  // without one rather than offer an update no client will take. The web build refreshes by revision only.
  if (platform !== "web" && (!build || !revision)) return null;
  return {
    version,
    revision,
    build,
    url: url.startsWith("https://") ? url : relativeUrl,
    file,
    bytes: Number.isSafeInteger(entry.bytes) && entry.bytes > 0 ? entry.bytes : undefined,
    sha256,
    notes: String(entry.notes || "").slice(0, 2000),
    required: Boolean(entry.required),
  };
}

function publicUpdateStatus(updates) {
  const status = {};
  for (const channel of UPDATE_CHANNELS) {
    const published = [];
    for (const platform of UPDATE_PLATFORMS) {
      const update = publicUpdateManifest(updates, channel, platform);
      if (update) published.push({ platform, version: update.version, revision: update.revision });
    }
    if (!published.length) continue;
    const versions = [...new Set(published.map((entry) => entry.version))];
    const revisions = [...new Set(published.map((entry) => entry.revision).filter(Boolean))];
    status[channel] = {
      version: versions.length === 1 ? versions[0] : "mixed",
      revision: revisions.length === 1 ? revisions[0] : (revisions.length ? "mixed" : "unreported"),
      platforms: published.map((entry) => entry.platform),
    };
  }
  return status;
}

function publishedUpdatePath(updates, encodedName) {
  let name;
  try {
    name = decodeURIComponent(encodedName);
  } catch {
    return "";
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$/.test(name)) return "";
  for (const channel of UPDATE_CHANNELS) {
    for (const platform of UPDATE_PLATFORMS) {
      const entry = updates?.[channel]?.[platform];
      if (entry?.file === name && publicUpdateManifest(updates, channel, platform)) {
        return `/${name}`;
      }
    }
  }
  return "";
}

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

function tryStaticFile(request, response, publicDirectory, pathname) {
  if (!publicDirectory || !new Set(["GET", "HEAD"]).has(request.method)) return false;
  let decoded;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return false;
  }
  const candidate = resolve(publicDirectory, `.${decoded === "/" ? "/index.html" : decoded}`);
  const relativePath = relative(publicDirectory, candidate);
  if (!relativePath || relativePath.startsWith(`..${sep}`) || relativePath === "..") return false;
  let filePath = candidate;
  try {
    if (statSync(filePath).isDirectory()) filePath = join(filePath, "index.html");
    if (!statSync(filePath).isFile()) return false;
  } catch {
    return false;
  }
  response.writeHead(200, {
    ...responseHeaders(),
    "content-type": MIME_TYPES.get(extname(filePath).toLowerCase()) || "application/octet-stream",
    "content-length": statSync(filePath).size,
  });
  if (request.method === "HEAD") response.end();
  else createReadStream(filePath).pipe(response);
  return true;
}

export function createLastRouteServer({
  enetRoomTtlMs = 60_000,
  adminTicketTtlMs = DEFAULT_ADMIN_TICKET_TTL_MS,
  roomTtlMs = 300_000,
  playerTtlMs = 120_000,
  accountStorePath = null,
  guestNameOptions = {},
  moderationStorePath = null,
  moderationSalt = "",
  publicDirectory = null,
  customMapDirectory = null,
  cityMaps: cityMapOptions = {},
  skinDirectory = null,
  skinOptions = {},
  saveDirectory = null,
  saveOptions = {},
  updateDirectory = null,
  adminPassword = "",
  updates = {},
  updatesProvider = null,
  trustProxy = false,
  relayAddress = "",
  relayPort = 0,
  publicRouteStorePath = null,
  publicSettingsStorePath = null,
  publicRouteSeed = "PUBLIC-ROUTE",
  publicItemRespawnMs,
  modPackStorePath = null,
  crashReportDirectory = null,
  reservedRoomStorePath = null,
  reservedRoomCodes = [],
  agentTokens = new Map(),
  modDownloadBase = "",
  modPackLimits,
  rateLimitWindowMs = DEFAULT_RATE_LIMIT_WINDOW_MS,
  apiRateLimitMax = 600,
  modRateLimitMax = 120,
  accountRateLimitMax = 30,
  devTokenTtlMs = DEFAULT_DEV_TOKEN_TTL_MS,
  now = Date.now,
  logger = console,
} = {}) {
  const enetRooms = new Map();
  // Opaque proofs let a release guest demonstrate that the primary service
  // authenticated KYTTU without ever handing her account session token to a
  // player-hosted ENet room. The proof is bound to the room's secret host
  // token as well as its public code, so unregistering/reusing a code cannot
  // carry old authority into a new room.
  const adminTickets = new Map();
  const roomService = createRoomService({ now, roomTtlMs, playerTtlMs });
  // One public route per game (namespaced by game id); the Last Route one keeps its original code and store file.
  const cityStorePath = publicRouteStorePath ? publicRouteStorePath.replace(/(\.[^./\\]+)?$/, "_city$1") : null;
  // the owner's public route settings (join limit, player timeout, heartbeat, fallback); see public_settings.mjs
  const publicSettings = createPublicSettingsStore({ storePath: publicSettingsStorePath, logger });
  const publicRoutes = {
    last_route: createPublicRoute({
      now, logger, storePath: publicRouteStorePath, initialSeed: publicRouteSeed, code: PUBLIC_CODES.last_route,
      tuning: () => publicSettings.tuning(),
      ...(publicItemRespawnMs ? { itemRespawnMs: publicItemRespawnMs } : {}),
    }),
    city: createPublicRoute({
      now, logger, storePath: cityStorePath, initialSeed: `${publicRouteSeed}-CITY`, code: PUBLIC_CODES.city,
      ...(publicItemRespawnMs ? { itemRespawnMs: publicItemRespawnMs } : {}),
    }),
  };
  const publicCodes = new Set(Object.values(PUBLIC_CODES));
  const publicRouteReady = Promise.all(Object.values(publicRoutes).map((route) => route.load()));
  const accountService = createAccountService({ storePath: accountStorePath, now, logger });
  const guestNames = createGuestNames({ storePath: accountStorePath ? join(dirname(accountStorePath), "guest_names.json") : null,
    now, logger, ...guestNameOptions });
  const cityMaps = createCityMaps({ ...cityMapOptions, accountService, now, logger,
    roomExists: (code) => enetRooms.has(normalizeEnetCode(code)) });
  const moderation = createModeration({ storePath: moderationStorePath, salt: moderationSalt, now, logger,
    isOwner: (name) => accountService.isAdminOwner(name),
    accountExists: (name) => accountService.accountExists(name),
    updatesProvider: () => currentUpdates() });
  const crashReports = crashReportDirectory ? createCrashReports({ directory: crashReportDirectory, now }) : null;
  const reservedRooms = reservedRoomStorePath
    ? createReservedRooms({ storePath: reservedRoomStorePath, codes: reservedRoomCodes, now, logger }) : null;
  const regions = createRegions({ tokens: agentTokens, now });
  // code -> { ticket, expiresAt }: lets a dedicated host an agent spawned register that (reserved) code once
  const spawnTickets = new Map();
  // Rooms the directory spawned for a signed-in host (POST /api/rooms/hosted/create). The creator is the room owner: the
  // dedicated host registers the code later with no identity of its own, so the owner comes from here, not the request.
  const hostedRooms = new Map();
  // Owner tickets: issued to a hosted room's signed-in owner, checked by the room's dedicated host (host token) before it
  // runs an owner command. Bound to the room's code, host token and owner, and short-lived.
  const ownerTickets = new Map();
  const HOSTED_CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  const hostedCode = () => [...randomBytes(5)].map((byte) => HOSTED_CODE_ALPHABET[byte % HOSTED_CODE_ALPHABET.length]).join("");
  // A reserved room whose host stopped heartbeating this long ago may be taken over by another invited player.
  const RESERVED_TAKEOVER_MS = 30_000;
  const modPacks = createModPacks({ storePath: modPackStorePath, downloadBase: modDownloadBase, now, logger,
    ...(modPackLimits ? { limits: modPackLimits } : {}) });
  const skins = createSkins({ directory: skinDirectory, accountService, now, logger, ...skinOptions });
  const gameSaves = createGameSaves({ directory: saveDirectory, accountService, now, logger, ...saveOptions });
  const devTokens = new Map();   // token -> expiresAt; a leaked dev token must not grant developer proof forever
  const requestRates = new Map();
  let enetRelay = null;
  let reloadId = 0;
  const wsRelay = createWsRelay({
    roomByCode: (code) => {
      pruneEnetRooms();
      return enetRooms.get(code) || null;
    },
    now,
    logger,
  });

  function currentUpdates() {
    if (typeof updatesProvider !== "function") return updates;
    try {
      const current = updatesProvider();
      return current && typeof current === "object" ? current : updates;
    } catch (error) {
      logger.error(`Could not reload update manifest: ${error.message}`);
      return updates;
    }
  }

  function pruneEnetRooms() {
    const timestamp = now();
    for (const [game, route] of Object.entries(publicRoutes)) {
      if (route.authorityStale()) {
        enetRooms.delete(PUBLIC_CODES[game]);
        wsRelay.closeRoom(PUBLIC_CODES[game]);
        route.resetAuthority();
      }
    }
    for (const [code, room] of enetRooms) {
      // A web room whose host still holds its relay socket is alive even if the browser tab stopped sending heartbeats
      // (a hidden tab pauses the game loop); the WebSocket is answered by the browser itself.
      if (room.transport === "ws" && wsRelay.hasHost(code)) room.expiresAt = Math.max(room.expiresAt, timestamp + enetRoomTtlMs);
      if (room.expiresAt > timestamp) continue;
      enetRooms.delete(code);
      wsRelay.closeRoom(code);
    }
    for (const [ticket, proof] of adminTickets) {
      if (proof.expiresAt <= timestamp || !enetRooms.has(proof.code)) adminTickets.delete(ticket);
    }
    // spawn tickets are one-use and short-lived: an unused one must not sit in the map for ever
    for (const [spawnCode, spawn] of spawnTickets) {
      if (spawn.expiresAt <= timestamp) spawnTickets.delete(spawnCode);
    }
    for (const [ticket, proof] of ownerTickets) {
      if (proof.expiresAt <= timestamp || !enetRooms.has(proof.code)) ownerTickets.delete(ticket);
    }
  }

  function forgetEnetRoom(code) {
    const room = enetRooms.get(code);
    if (!room) return;
    enetRooms.delete(code);
    wsRelay.closeRoom(code);
    for (const [ticket, proof] of ownerTickets) {
      if (proof.code === code) ownerTickets.delete(ticket);
    }
    for (const [ticket, proof] of adminTickets) {
      if (proof.code === code && safeEqual(proof.hostToken, room.token)) adminTickets.delete(ticket);
    }
  }

  function promoteEnetRoomHost(code) {
    const room = enetRooms.get(code);
    if (!room) return null;
    const oldToken = room.token;
    room.token = randomBytes(24).toString("base64url");
    room.players = Math.max(1, room.players - 1);
    room.expiresAt = now() + enetRoomTtlMs;
    for (const [ticket, proof] of adminTickets) {
      if (proof.code === code && safeEqual(proof.hostToken, oldToken)) {
        adminTickets.delete(ticket);
      }
    }
    return { token: room.token };
  }

  function developerRequest(body) {
    if (body?.devToken) {
      const key = String(body.devToken);
      const expiresAt = devTokens.get(key);
      if (expiresAt && expiresAt > now()) return true;
      if (expiresAt) devTokens.delete(key);
    }
    if (adminPassword && safeEqual(body?.adminProof, adminPassword)) return true;
    return Boolean(accountService.sessionAdminIdentity(body?.sessionToken)?.adminOwner);
  }

  function issueDevToken() {
    const token = randomBytes(24).toString("hex");
    const timestamp = now();
    for (const [key, expiresAt] of devTokens) if (expiresAt <= timestamp) devTokens.delete(key);
    devTokens.set(token, timestamp + Math.max(1000, Number(devTokenTtlMs) || DEFAULT_DEV_TOKEN_TTL_MS));
    return token;
  }

  function rateLimitRequest(request, pathname) {
    const sensitive = ACCOUNT_RATE_LIMIT_PATHS.has(pathname);
    // mod index reads get their own, tighter bucket so a client polling packs cannot starve gameplay calls
    const mods = !sensitive && pathname.startsWith("/api/mods/");
    const maximum = Math.max(1, Number(sensitive ? accountRateLimitMax : mods ? modRateLimitMax : apiRateLimitMax) || 1);
    const windowMs = Math.max(1000, Number(rateLimitWindowMs) || DEFAULT_RATE_LIMIT_WINDOW_MS);
    const timestamp = now();
    const address = requesterAddress(request, trustProxy) || "unknown";
    const key = `${sensitive ? "account" : mods ? "mods" : "api"}|${address}`;
    let rate = requestRates.get(key);
    if (!rate || timestamp >= rate.resetAt) {
      rate = { count: 0, resetAt: timestamp + windowMs };
    }
    rate.count += 1;
    requestRates.set(key, rate);
    if (rate.count <= maximum) return 0;
    return Math.max(1, Math.ceil((rate.resetAt - timestamp) / 1000));
  }

  const server = createServer(async (request, response) => {
    if (request.method === "OPTIONS") return json(response, 204, {});
    pruneEnetRooms();
    const url = new URL(request.url || "/", "http://last-route.invalid");
    // Trace for room joins: one line per API call (status and time), skipping the frequent probes.
    if (url.pathname.startsWith("/api/") && url.pathname !== "/api/health" && url.pathname !== "/api/diag/display") {
      const started = Date.now();
      response.on("finish", () => console.log(`[api] ${request.method} ${url.pathname} ${response.statusCode} ${Date.now() - started}ms`));
    }
    const retryAfter = rateLimitRequest(request, url.pathname);
    if (retryAfter > 0) {
      return json(response, 429, {
        ok: false,
        code: "RATE_LIMITED",
        error: "Too many requests; try again shortly",
      }, { "retry-after": String(retryAfter) });
    }

    if (request.method === "GET" && url.pathname === "/api/health") {
      const updateStatus = publicUpdateStatus(currentUpdates());
      return json(response, 200, {
        ok: true,
        runtime: "node",
        service: "last-route-backend",
        instance: instanceName,
        port: request.socket.localPort,
        rooms: enetRooms.size,
        relay: enetRelay?.relayStatus() || { enabled: false },
        wsRelay: wsRelay.status(),
        webRooms: roomService.rooms.size,
        reloadId,
        updates: updateStatus,
        now: new Date(now()).toISOString(),
      });
    }

    if (new Set(["GET", "HEAD"]).has(request.method) && url.pathname.startsWith("/api/city-maps/files/")) {
      const result = cityMaps.file(url.pathname, request.headers);
      if (result && result[0] === 200) {
        response.writeHead(200, { ...responseHeaders(), "cache-control": "private, max-age=600",
          "content-type": "application/octet-stream", "content-length": result[1].length, etag: `"${result[2]}"` });
        return response.end(request.method === "HEAD" ? undefined : result[1]);
      }
      if (result) return json(response, result[0], result[1]);
    }

    if (new Set(["GET", "HEAD"]).has(request.method) && url.pathname.startsWith("/api/skins/files/")) {
      const result = skins.file(url.pathname);
      if (result[0] === 200) {
        response.writeHead(200, { ...responseHeaders(), "cache-control": "public, max-age=31536000, immutable",
          "content-type": "image/png", "content-length": result[1].length, etag: `"${result[2]}"` });
        return response.end(request.method === "HEAD" ? undefined : result[1]);
      }
      return json(response, result[0], result[1]);
    }

    // mod pack index (mod_packs.mjs): metadata only; the files themselves are served from VPS 2
    if (request.method === "GET" && url.pathname === "/api/mods/packs") {
      const result = modPacks.list();
      return json(response, result[0], result[1], { "cache-control": "public, max-age=60" });
    }
    const modPackPath = MOD_PACK_PATH.exec(url.pathname);
    if (request.method === "GET" && modPackPath) {
      const result = modPacks.get(modPackPath[1], url.searchParams.get("version"));
      return json(response, result[0], result[1], { "cache-control": "public, max-age=60" });
    }

    const skinAccount = SKIN_ACCOUNT_PATH.exec(url.pathname);
    if (skinAccount && new Set(["GET", "PUT"]).has(request.method)) {
      // account skins (skins.mjs): GET is public metadata, PUT is the owner's raw PNG (session + arms style in headers)
      if (request.method === "GET") {
        const found = skins.get(skinAccount[1]);
        return json(response, found[0], found[1], { "cache-control": "public, max-age=15" });
      }
      const chunks = [];
      let received = 0;
      for await (const chunk of request) {
        received += chunk.length;
        if (received > 1024 * 1024) { request.destroy(); return; }
        if (received <= SKIN_MAX_BYTES) chunks.push(chunk);
      }
      if (received > SKIN_MAX_BYTES) {
        return json(response, 413, { ok: false, code: "BAD_SKIN", error: "Skin refused: the file is too large" });
      }
      const putToken = String(request.headers[SKIN_TOKEN_HEADER] || "");
      const putRefused = moderation.checkSkinUpload(accountService.sessionUsername(putToken) || "");
      if (putRefused) return json(response, putRefused[0], putRefused[1]);
      const stored = skins.put(putToken, skinAccount[1], Buffer.concat(chunks), String(request.headers[SKIN_ARMS_HEADER] || "auto").toLowerCase());
      return json(response, stored[0], stored[1]);
    }

    const savePath = SAVE_PATH.exec(url.pathname);
    if (savePath && new Set(["GET", "PUT"]).has(request.method)) {
      // cloud saves (game_saves.mjs): the session rides in a header, the body is capped before it is parsed
      const saveToken = String(request.headers["x-session-token"] || "");
      const [gameId, slot] = [savePath[1], savePath[2]];   // allow-listed plain ids, never decoded
      if (request.method === "GET") {
        const loaded = gameSaves.get(saveToken, gameId, slot);
        return json(response, loaded[0], loaded[1]);
      }
      const chunks = [];
      let received = 0;
      for await (const chunk of request) {
        received += chunk.length;
        if (received > 1024 * 1024) { request.destroy(); return; }   // never read an unbounded body
        if (received <= SAVE_MAX_BYTES) chunks.push(chunk);
      }
      if (received > SAVE_MAX_BYTES) {
        return json(response, 413, { ok: false, code: "SAVE_TOO_LARGE", error: `A save is at most ${SAVE_MAX_BYTES / 1024} KB` });
      }
      let saveBody = null;
      try {
        saveBody = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      } catch {
        return json(response, 400, { ok: false, code: "BAD_SAVE", error: "INVALID JSON BODY" });
      }
      const stored = gameSaves.put(saveToken, gameId, slot, saveBody);
      return json(response, stored[0], stored[1]);
    }

    const customMapFile = url.pathname.match(/^\/api\/custom-maps\/([A-Za-z0-9][A-Za-z0-9._-]{0,180}\.png)$/i);
    if (new Set(["GET", "HEAD"]).has(request.method) && customMapFile) {
      if (tryStaticFile(request, response, customMapDirectory, `/${customMapFile[1]}`)) return;
      return json(response, 404, { ok: false, error: "CUSTOM MAP NOT FOUND" });
    }

    const updateLookup = url.pathname.match(/^\/api\/updates\/(debug|release)\/(linux|windows|windows_legacy|macos|web|steamvr|quest|phone)$/);
    if (request.method === "GET" && updateLookup) {
      const update = publicUpdateManifest(currentUpdates(), updateLookup[1], updateLookup[2]);
      if (!update) return json(response, 404, { ok: false, error: "NO UPDATE PUBLISHED" });
      return json(response, 200, { ok: true, channel: updateLookup[1], platform: updateLookup[2], update });
    }

    const updateFile = url.pathname.match(/^\/api\/updates\/files\/([^/]+)$/);
    if (new Set(["GET", "HEAD"]).has(request.method) && updateFile) {
      const assetPath = publishedUpdatePath(currentUpdates(), updateFile[1]);
      if (assetPath && tryStaticFile(request, response, updateDirectory, assetPath)) return;
      return json(response, 404, { ok: false, error: "UPDATE FILE NOT FOUND" });
    }

    const enetResolve = url.pathname.match(/^\/api\/enet\/rooms\/([^/]+)$/);
    if (request.method === "GET" && enetResolve) {
      const code = normalizeEnetCode(decodeURIComponent(enetResolve[1]));
      if (reservedRooms?.isReserved(code)) {
        return json(response, 403, { ok: false, code: "ROOM_RESERVED", reserved: true,
          error: "PRIVATE ROOM // SIGN IN WITH AN INVITE OR ENTER THE ROOM PASSWORD" });
      }
      const room = code ? enetRooms.get(code) : null;
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      return json(response, 200, {
        ok: true,
        room: publicEnetRoom(room, now(), relayAddress, relayPort),
      });
    }

    if (request.method === "GET" && url.pathname === "/api/regions") {
      return json(response, 200, { ok: true, regions: regions.publicList() });
    }

    if (request.method === "GET" && url.pathname === "/api/public/status") {
      await publicRouteReady;
      return json(response, 200, publicRoutes[publicGameId(url.searchParams.get("game"))].status());
    }

    if (request.method === "GET" && url.pathname === "/api/public/settings") {
      await publicSettings.load();
      return json(response, 200, { ok: true, settings: publicSettings.get(), specs: PUBLIC_SETTING_SPECS });
    }

    if (request.method !== "POST") {
      if (tryStaticFile(request, response, publicDirectory, url.pathname)) return;
      if (request.method === "GET" && url.pathname === "/") {
        return json(response, 200, { ok: true, service: "Last Route backend", health: "/api/health", staticFiles: Boolean(publicDirectory) });
      }
      return json(response, 404, { ok: false, code: "NOT_FOUND", error: "Not found" });
    }

    if (url.pathname === "/api/skins/upload") {
      // the one raw-body route: a PNG of at most SKIN_MAX_BYTES, the session in a header (a JSON wrapper would not fit the body limit)
      const chunks = [];
      let received = 0;
      for await (const chunk of request) {
        received += chunk.length;
        if (received > 1024 * 1024) { request.destroy(); return; }   // never read an unbounded body
        if (received <= SKIN_MAX_BYTES) chunks.push(chunk);
      }
      if (received > SKIN_MAX_BYTES) {
        return json(response, 413, { ok: false, code: "BAD_SKIN", error: "Skin refused: the file is too large" });
      }
      const skinToken = String(request.headers[SKIN_TOKEN_HEADER] || "");
      const refused = moderation.checkSkinUpload(accountService.sessionUsername(skinToken) || "");
      if (refused) return json(response, refused[0], refused[1]);
      const uploaded = skins.upload(String(request.headers[SKIN_TOKEN_HEADER] || ""), Buffer.concat(chunks));
      return json(response, uploaded[0], uploaded[1]);
    }

    if (url.pathname === "/api/regions/report") {
      // signed by the region agent (regions.mjs); the raw body is what was signed
      const chunks = [];
      let received = 0;
      for await (const chunk of request) {
        received += chunk.length;
        if (received > 64 * 1024) { request.destroy(); return; }
        chunks.push(chunk);
      }
      const raw = Buffer.concat(chunks).toString("utf8");
      const region = regions.verify(request.headers, raw);
      if (!region) return json(response, 403, { ok: false, code: "BAD_AGENT_SIGNATURE", error: "Unknown region or bad signature" });
      let report;
      try { report = JSON.parse(raw); } catch { return json(response, 400, { ok: false, error: "INVALID JSON" }); }
      return json(response, 200, { ok: true, actions: regions.report(region, report) });
    }

    let body;
    try {
      body = await readJson(request,
        url.pathname === "/api/admin/mods/register" ? MOD_REGISTER_MAX_BODY_BYTES
          : url.pathname === "/api/v1/crash-reports" ? CRASH_MAX_BODY_BYTES : MAX_BODY_BYTES);
    } catch (error) {
      return json(response, error.status || 400, { ok: false, error: error.message });
    }

    if (url.pathname === "/api/diag/display") {
      const now = Date.now();
      if (now - diagProbe.last >= 1000 && body && typeof body === "object") {
        diagProbe.last = now;
        const fields = {};
        for (const key of ["dpr", "inner", "vv", "backing", "css", "rect", "ua"]) {
          if (body[key] !== undefined) fields[key] = JSON.stringify(body[key]).slice(0, 160);
        }
        console.log(`[diag-display] ${JSON.stringify(fields)}`);
      }
      return json(response, 200, { ok: true });
    }

    // Crash / freeze reports (CrashWatch): store and echo only; per-client rate limit and field caps in crash_reports.mjs.
    if (url.pathname === "/api/v1/crash-reports") {
      if (!crashReports) return json(response, 404, { ok: false, code: "UNKNOWN_ENDPOINT", error: "Unknown endpoint" });
      const client = `${requesterAddress(request, trustProxy) || "unknown"}|${String(body.device_id || "").slice(0, 64)}`;
      const result = crashReports.submit(body, client);
      return json(response, result[0], result[1]);
    }
    if (url.pathname === "/api/admin/crash-reports") {
      const owner = accountService.sessionUsername(body.sessionToken);
      if (!owner || !accountService.isAdminOwner(owner)) {
        return json(response, 403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" });
      }
      if (!crashReports) return json(response, 200, { ok: true, reports: [], total: 0 });
      const result = crashReports.list({ since: body.since, limit: body.limit });
      return json(response, result[0], result[1]);
    }

    if (url.pathname === "/api/public/join" || url.pathname === "/api/public/state"
        || url.pathname === "/api/public/leave") {
      await publicRouteReady;
      const publicGame = publicGameId(body.game);
      const publicRoute = publicRoutes[publicGame];
      const publicEnet = enetRooms.get(PUBLIC_CODES[publicGame]);
      const hostTokenValid = Boolean(publicEnet && body.hostToken && safeEqual(body.hostToken, publicEnet.token));
      if (hostTokenValid) publicEnet.expiresAt = now() + PUBLIC_ENET_TTL_MS;
      const address = requesterAddress(request, trustProxy);
      if (url.pathname === "/api/public/join") {
        const refused = moderation.checkPublicJoin({ address, deviceId: body.deviceId,
          username: accountService.sessionUsername(body.sessionToken) || "" });
        if (refused) return json(response, refused[0], refused[1]);
      }
      const result = url.pathname === "/api/public/join"
        ? publicRoute.join(body, { address, hostTokenValid, enetListed: Boolean(publicEnet) })
        : url.pathname === "/api/public/state"
          ? publicRoute.post(body, { hostTokenValid })
          : publicRoute.leave(body);
      if (result.error) return json(response, result.status || 400, { ok: false, error: result.error });
      return json(response, result.status || 200, result.body);
    }

    // Reserved private rooms (reserved_rooms.mjs): access-checked resolve, and the owner's reservations.
    if (url.pathname === "/api/rooms/reserved/resolve") {
      const code = normalizeEnetCode(body.code);
      if (!code || !reservedRooms?.isReserved(code)) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      const username = body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : "";
      if (!reservedRooms.hasAccess(code, { username, password: body.password,
        owner: Boolean(username) && accountService.isAdminOwner(username) })) {
        return json(response, 403, { ok: false, code: "ROOM_ACCESS_DENIED", reserved: true,
          error: "NOT ON THIS ROOM'S INVITE LIST // WRONG ROOM PASSWORD" });
      }
      const live = enetRooms.get(code);
      const stale = !live || now() - (live.lastSeen || 0) > RESERVED_TAKEOVER_MS;
      return json(response, 200, { ok: true, reserved: true, live: !stale,
        room: live && !stale ? publicEnetRoom(live, now(), relayAddress, relayPort) : null,
        joinTicket: live && !stale ? reservedRooms.issueTicket(code) : "",
        canHost: stale, settings: reservedRooms.hostView(code).settings });
    }
    if (url.pathname === "/api/admin/regions") {
      const owner = accountService.sessionUsername(body.sessionToken);
      if (!owner || !accountService.isAdminOwner(owner)) {
        return json(response, 403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" });
      }
      const action = String(body.action || "list");
      const region = String(body.region || "");
      const code = normalizeEnetCode(body.code);
      if (action === "list") return json(response, 200, { ok: true, regions: regions.ownerList() });
      if (action === "stop") {
        const result = regions.requestStop(region, code);
        return json(response, result[0], result[1]);
      }
      if (action === "spawn") {
        if (!code) return json(response, 400, { ok: false, error: "INVALID ROOM CODE" });
        const live = enetRooms.get(code);
        if (live && now() - (live.lastSeen || 0) <= RESERVED_TAKEOVER_MS) {
          return json(response, 409, { ok: false, code: "ROOM_ALREADY_HOSTED", error: "Someone is hosting that code" });
        }
        const ticket = randomBytes(18).toString("base64url");
        spawnTickets.set(code, { ticket, expiresAt: now() + 5 * 60_000 });
        const result = regions.requestSpawn(region, code, { seed: body.seed, difficulty: body.difficulty,
          idleMinutes: body.idleMinutes, spawnTicket: ticket });
        if (result[0] !== 202) spawnTickets.delete(code);
        return json(response, result[0], result[1]);
      }
      return json(response, 400, { ok: false, code: "INVALID_ACTION", error: "Actions: list, spawn, stop" });
    }
    if (url.pathname === "/api/admin/reserved-rooms") {
      const owner = accountService.sessionUsername(body.sessionToken);
      if (!owner || !accountService.isAdminOwner(owner)) {
        return json(response, 403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" });
      }
      if (!reservedRooms) return json(response, 404, { ok: false, code: "UNKNOWN_ENDPOINT", error: "Unknown endpoint" });
      if (body.action === "command") {
        // a command waits for the current host only: queued for an empty room, it would hit whoever hosts next
        const live = enetRooms.get(normalizeEnetCode(body.code));
        if (!live || now() - (live.lastSeen || 0) > RESERVED_TAKEOVER_MS) {
          return json(response, 409, { ok: false, code: "ROOM_NOT_LIVE", error: "Nobody is hosting that room right now" });
        }
      }
      const result = reservedRooms.admin(body, owner);
      if (Array.isArray(result[1].rooms)) {
        // the owner's status view: is someone hosting it right now, and how many are in
        for (const room of result[1].rooms) {
          const live = enetRooms.get(room.code);
          room.live = Boolean(live && now() - (live.lastSeen || 0) <= RESERVED_TAKEOVER_MS);
          room.players = room.live ? live.players : 0;
          room.sessionActive = room.live ? Boolean(live.sessionActive) : false;
        }
      }
      return json(response, result[0], result[1]);
    }

    if (url.pathname === "/api/rooms/hosted/create") {
      const owner = body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : "";
      if (!owner) return json(response, 401, { ok: false, code: "SIGN_IN_REQUIRED", error: "Sign in to host a room" });
      // a hosted entry outlives its spawn window while its room is listed: the owner is kept for the life of the room
      for (const [code, entry] of hostedRooms) {
        if (now() - entry.createdAt > 30 * 60_000 && !enetRooms.has(code)) hostedRooms.delete(code);
      }
      const live = regions.publicList().filter((status) => status.free > 0);
      const picked = body.region ? live.find((status) => status.region === String(body.region)) : live[0];
      if (!picked) return json(response, 503, { ok: false, code: "NO_FREE_REGION", error: "No server has a free room slot right now" });
      let code = "";
      for (let attempt = 0; attempt < 20 && !code; attempt += 1) {
        const candidate = hostedCode();
        if (!hostedRooms.has(candidate) && !reservedRooms?.isReserved(candidate) && !regions.regionOf(candidate)) code = candidate;
      }
      if (!code) return json(response, 503, { ok: false, code: "NO_FREE_CODE", error: "Try again in a moment" });
      // the spawned host registers the code with this ticket; nothing else may take the code
      const ticket = randomBytes(18).toString("base64url");
      spawnTickets.set(code, { ticket, expiresAt: now() + 5 * 60_000 });
      // the Rust client asks for its own build ({build: "rust"}): a Godot host speaks another protocol, so its players are refused
      const build = body.build === "rust" ? "rust" : "";
      const [status, payload] = regions.requestSpawn(picked.region, code, { difficulty: body.difficulty, spawnTicket: ticket, build });
      if (status >= 300) {
        spawnTickets.delete(code);
        return json(response, status, payload);
      }
      hostedRooms.set(code, { owner, region: picked.region, createdAt: now() });
      return json(response, 202, { ok: true, code, region: picked.region, relay: picked.relay, owner });
    }

    if (url.pathname === "/api/enet/rooms/register") {
      const code = normalizeEnetCode(body.code);
      const webRoom = body.transport === "ws";
      const port = webRoom ? 0 : Number(body.port);
      if (!code) return json(response, 400, { ok: false, error: "INVALID ROOM CODE" });
      if (!webRoom && (!Number.isInteger(port) || port < 1 || port > 65_535)) return json(response, 400, { ok: false, error: "INVALID ENET PORT" });
      const hosted = hostedRooms.get(code);
      const hostUser = hosted && !reservedRooms?.isReserved(code) ? hosted.owner
        : body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : "";
      const reserved = Boolean(reservedRooms?.isReserved(code));
      const spawn = spawnTickets.get(code);
      const spawned = Boolean(spawn && spawn.expiresAt > now() && body.spawnTicket && safeEqual(body.spawnTicket, spawn.ticket));
      // a hosted code is registered only by the dedicated host the directory spawned for it (the owner is not a host)
      const hostedOwned = Boolean(hosted) && !reserved;
      if (hostedOwned && !spawned) {
        return json(response, 403, { ok: false, code: "HOSTED_ROOM_SPAWN_ONLY", error: "This room is hosted by the server" });
      }
      // the spawn ticket is spent only when this register actually lists the room (see below), so a 409 or 403 keeps it
      if (!reserved && !GENERATED_CODE_PATTERN.test(code)) return json(response, 400, { ok: false, error: "INVALID ROOM CODE" });
      if (reserved && !spawned && !reservedRooms.hasAccess(code, { username: hostUser, password: body.roomPassword,
        owner: Boolean(hostUser) && accountService.isAdminOwner(hostUser) })) {
        return json(response, 403, { ok: false, code: "ROOM_RESERVED", reserved: true,
          error: "THAT ROOM CODE IS RESERVED // INVITE OR ROOM PASSWORD REQUIRED" });
      }
      const live = enetRooms.get(code);
      // a reserved room's crashed host is replaced by the next invited player once its heartbeat has gone quiet
      if (live && !(reserved && now() - (live.lastSeen || 0) > RESERVED_TAKEOVER_MS)) {
        return json(response, 409, { ok: false, error: "ROOM CODE ALREADY REGISTERED" });
      }
      if (publicCodes.has(code)) {
        const refused = moderation.checkHostPublic({ username: hostUser, deviceId: body.deviceId,
          address: requesterAddress(request, trustProxy) });
        if (refused) return json(response, refused[0], refused[1]);
      }
      const token = randomBytes(24).toString("base64url");
      const room = {
        code,
        token,
        address: requesterAddress(request, trustProxy),
        port,
        transport: webRoom ? "ws" : "enet",
        hostUser,                                   // reporting identity for moderation (may be empty)
        owner: hostedOwned ? hosted.owner : "",     // the signed-in creator of a hosted room (shown to guests)
        instance: randomBytes(9).toString("base64url"),
        players: Math.trunc(finitePlayers(body.players, 1)),
        sessionActive: Boolean(body.sessionActive),
        daytime: Boolean(body.daytime),
        game: publicCodes.has(code) ? Object.keys(PUBLIC_CODES).find((g) => PUBLIC_CODES[g] === code) : cleanGame(body.game),
        expiresAt: now() + (publicCodes.has(code) ? PUBLIC_ENET_TTL_MS : enetRoomTtlMs),
        lastSeen: now(),
        protocol: /^[a-f0-9]{8}$/.test(String(body.protocol || "")) ? String(body.protocol) : "",
        dedicated: spawned,
        region: spawned ? regions.regionOf(code) || String(body.region || "").slice(0, 24) : "",
      };
      if (reserved) {
        room.reserved = true;
        room.admitGuest = (nonce) => reservedRooms.ticketValid(code, nonce);   // the relay's GUEST_JOIN gate
      }
      if (spawned) spawnTickets.delete(code);   // one dedicated host per ticket, spent now that the room is listed
      enetRooms.set(code, room);
      // hostedRooms keeps the owner for the life of the room (room.owner above); it is not deleted here, so a later
      // register of this code still needs a spawn ticket and still records the owner
      return json(response, 201, {
        ok: true,
        token,
        room: publicEnetRoom(room, now(), relayAddress, relayPort),
        // a new host starts clean: commands meant for the previous one are dropped
        ...(reserved ? { reserved: reservedRooms.hostView(code, { ack: Number.MAX_SAFE_INTEGER }) } : {}),
      });
    }

    if (url.pathname === "/api/enet/rooms/heartbeat") {
      const room = enetRooms.get(normalizeEnetCode(body.code));
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      if (!safeEqual(body.token, room.token)) return json(response, 403, { ok: false, error: "INVALID HOST TOKEN" });
      room.players = Math.trunc(finitePlayers(body.players, room.players));
      room.sessionActive = Boolean(body.sessionActive);
      room.daytime = Boolean(body.daytime);
      room.expiresAt = now() + (publicCodes.has(room.code) ? PUBLIC_ENET_TTL_MS : enetRoomTtlMs);
      room.lastSeen = now();
      return json(response, 200, {
        ok: true,
        room: publicEnetRoom(room, now(), relayAddress, relayPort),
        ...(room.reserved ? { reserved: reservedRooms.hostView(room.code,
          { since: body.commandSince, ack: body.commandAck }) } : {}),
      });
    }

    if (url.pathname === "/api/enet/rooms/unregister") {
      const code = normalizeEnetCode(body.code);
      const room = enetRooms.get(code);
      if (!room) return json(response, 200, { ok: true, removed: false });
      if (!safeEqual(body.token, room.token)) return json(response, 403, { ok: false, error: "INVALID HOST TOKEN" });
      // an explicit quit: guests are told now and the code stops resolving now
      enetRelay?.closeRoom(code);
      wsRelay.closeRoom(code);
      forgetEnetRoom(code);
      return json(response, 200, { ok: true, removed: true });
    }

    if (url.pathname === "/api/enet/rooms/release-host") {
      const code = normalizeEnetCode(body.code);
      const room = enetRooms.get(code);
      if (!room) return json(response, 200, { ok: true, migrating: false });
      if (!safeEqual(body.token, room.token)) return json(response, 403, { ok: false, error: "INVALID HOST TOKEN" });
      if (body.handoff && typeof body.handoff === "object") {
        // Deliberate handoff (task 010): a refusal leaves the room listed and its host in charge.
        if (room.transport === "ws" || !enetRelay?.requestHostHandoff) {
          return json(response, 200, { ok: true, migrating: false, unsupported: true,
            error: "HOST TRANSFER IS NOT SUPPORTED FOR THIS ROOM" });
        }
        const result = enetRelay.requestHostHandoff(code, body.handoff.candidateSession, body.handoff.epoch);
        return json(response, 200, { ok: true, migrating: Boolean(result.migrating),
          ...(result.error ? { error: result.error } : {}) });
      }
      const migrating = room.transport === "ws" ? false : Boolean(enetRelay?.requestHostMigration(code));
      if (!migrating) {
        enetRooms.delete(code);
        wsRelay.closeRoom(code);
      }
      return json(response, 200, { ok: true, migrating });
    }

    // Owner controls for a dedicated host: the owner's account gets a ticket for its own hosted room; the host checks it
    // with verify (its host token), so the authority never trusts a name the client sends.
    if (url.pathname === "/api/enet/owner-tickets/issue") {
      const code = normalizeEnetCode(body.code);
      const room = code ? enetRooms.get(code) : null;
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      const username = String(accountService.sessionUsername(body.sessionToken) || "").toUpperCase();
      const roomOwner = String(room.owner || "").toUpperCase();
      if (!roomOwner || username !== roomOwner) {
        return json(response, 403, { ok: false, code: "ROOM_OWNER_ONLY", error: "Only the room's owner can control it" });
      }
      const ticket = randomBytes(32).toString("base64url");
      // used: the owner actions this ticket already authorized; each (ticket, action) pair runs once
      ownerTickets.set(ticket, { code, hostToken: room.token, owner: roomOwner, used: new Set(), expiresAt: now() + OWNER_TICKET_TTL_MS });
      return json(response, 201, { ok: true, ticket, room: code, expiresIn: Math.round(OWNER_TICKET_TTL_MS / 1000) });
    }

    if (url.pathname === "/api/enet/owner-tickets/verify") {
      const code = normalizeEnetCode(body.code);
      const room = code ? enetRooms.get(code) : null;
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      if (!safeEqual(body.hostToken, room.token)) {
        return json(response, 403, { ok: false, error: "INVALID HOST TOKEN" });
      }
      const ticket = String(body.ticket || "");
      const action = String(body.action || "").slice(0, 32);
      const proof = ownerTickets.get(ticket);
      const roomOwner = String(room.owner || "").toUpperCase();
      const authorized = Boolean(proof && proof.expiresAt > now() && proof.code === code
        && safeEqual(proof.hostToken, room.token) && roomOwner && proof.owner === roomOwner);
      if (!authorized) {
        ownerTickets.delete(ticket);
        return json(response, 403, { ok: false, authorized: false, error: "INVALID OR EXPIRED OWNER TICKET" });
      }
      // one use per (ticket, action): a repeat of the same action on the same ticket is refused; other actions still pass
      if (!action || proof.used.has(action)) {
        return json(response, 403, { ok: false, authorized: false, error: "OWNER TICKET ALREADY USED FOR THIS ACTION" });
      }
      proof.used.add(action);
      return json(response, 200, { ok: true, authorized: true, room: code, owner: roomOwner, action });
    }

    if (url.pathname === "/api/enet/admin-tickets/issue") {
      const code = normalizeEnetCode(body.code);
      const room = code ? enetRooms.get(code) : null;
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      const identity = accountService.sessionAdminIdentity(body.sessionToken);
      if (!identity?.admin || !identity.adminPermissions.length) {
        return json(response, 403, { ok: false, error: "ADMIN ACCOUNT REQUIRED" });
      }
      const ticket = randomBytes(32).toString("base64url");
      const ttl = Math.max(5_000, Math.min(Number(adminTicketTtlMs) || DEFAULT_ADMIN_TICKET_TTL_MS,
        DEFAULT_ADMIN_TICKET_TTL_MS));
      const expiresAt = now() + ttl;
      adminTickets.set(ticket, { code, hostToken: room.token, expiresAt,
        username: identity.username, permissions: [...identity.adminPermissions], owner: identity.adminOwner === true });
      return json(response, 201, {
        ok: true,
        ticket,
        room: code,
        permissions: [...identity.adminPermissions],
        expiresIn: Math.max(1, Math.ceil(ttl / 1000)),
      });
    }

    if (url.pathname === "/api/enet/admin-tickets/verify") {
      const code = normalizeEnetCode(body.code);
      const room = code ? enetRooms.get(code) : null;
      if (!room) return json(response, 404, { ok: false, error: "ROOM NOT FOUND" });
      if (!safeEqual(body.hostToken, room.token)) {
        return json(response, 403, { ok: false, error: "INVALID HOST TOKEN" });
      }
      const ticket = String(body.ticket || "");
      const proof = adminTickets.get(ticket);
      const authorized = Boolean(proof && proof.expiresAt > now() && proof.code === code
        && safeEqual(proof.hostToken, room.token));
      if (!authorized) {
        adminTickets.delete(ticket);
        return json(response, 403, { ok: false, authorized: false, error: "INVALID OR EXPIRED ADMIN TICKET" });
      }
      return json(response, 200, {
        ok: true,
        authorized: true,
        room: code,
        permissions: [...proof.permissions],
        // the account-level owner flag: owner-only room routes (restart, end, room chat) need it, delegated admins never have it
        owner: proof.owner === true,
        expiresIn: Math.max(1, Math.ceil((proof.expiresAt - now()) / 1000)),
      });
    }

    if (url.pathname === "/api/moderation/report" || url.pathname === "/api/moderation/peers") {
      const code = normalizeEnetCode(body.code);
      const room = code ? enetRooms.get(code) : null;
      if (!room || !safeEqual(body.hostToken, room.token)) {
        return json(response, 403, { ok: false, code: "INVALID_HOST_TOKEN", error: "INVALID HOST TOKEN" });
      }
      const result = url.pathname === "/api/moderation/report"
        ? moderation.reportKick(body, { room: code, host: room.hostUser ? `user:${room.hostUser}` : `room:${room.instance}` })
        : moderation.peers(body);
      return json(response, result[0], result[1]);
    }
    if (url.pathname === "/api/moderation/integrity") {
      const result = moderation.integrity(body, { address: requesterAddress(request, trustProxy),
        username: body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : "" });
      return json(response, result[0], result[1]);
    }
    if (url.pathname === "/api/moderation/status") {
      return json(response, 200, { ok: true, status: moderation.status({ deviceId: body.deviceId,
        address: requesterAddress(request, trustProxy),
        username: body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : "" }) });
    }
    if (url.pathname.startsWith("/api/admin/mods/")) {
      const owner = accountService.sessionUsername(body.sessionToken);
      if (!owner || !accountService.isAdminOwner(owner)) {
        return json(response, 403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" });
      }
      let result;
      if (url.pathname === "/api/admin/mods/register") result = modPacks.register(body, owner);
      else if (url.pathname === "/api/admin/mods/block") result = modPacks.block(body, owner);
      else if (url.pathname === "/api/admin/mods/unblock") result = modPacks.unblock(body, owner);
      else result = [404, { ok: false, code: "UNKNOWN_ENDPOINT", error: "Unknown endpoint" }];
      return json(response, result[0], result[1]);
    }
    if (url.pathname.startsWith("/api/admin/flags") || url.pathname.startsWith("/api/admin/builds/")) {
      const owner = accountService.sessionUsername(body.sessionToken);
      if (!owner || !accountService.isAdminOwner(owner)) {
        return json(response, 403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" });
      }
      let result;
      if (url.pathname === "/api/admin/flags") result = moderation.adminList();
      else if (url.pathname === "/api/admin/flags/set") result = moderation.adminSet(body, owner);
      else if (url.pathname === "/api/admin/builds/add") result = moderation.addBuild(body, owner);
      else if (url.pathname === "/api/admin/builds/remove") result = moderation.removeBuild(body);
      else result = [404, { ok: false, code: "UNKNOWN_ENDPOINT", error: "Unknown endpoint" }];
      return json(response, result[0], result[1]);
    }

    if (url.pathname === "/api/dev/login") {
      if (!adminPassword) return json(response, 503, { ok: false, code: "DEV_LOGIN_DISABLED", error: "Developer login is not configured" });
      if (!safeEqual(body.password, adminPassword)) return json(response, 401, { ok: false, code: "INVALID_DEV_PASSWORD", error: "Invalid developer password" });
      return json(response, 200, { ok: true, developer: true, admin: true, devToken: issueDevToken() });
    }
    if (url.pathname === "/api/dev/reload") {
      if (!developerRequest(body)) return json(response, 401, { ok: false, code: "NOT_AUTHORIZED", error: "Developer proof required" });
      reloadId += 1;
      return json(response, 200, { ok: true, reloadId });
    }

    let result;
    if (url.pathname === "/api/rooms/join") result = roomService.joinRoom(body);
    else if (url.pathname === "/api/rooms/state") result = roomService.updateRoom(body);
    else if (url.pathname === "/api/rooms/settings") result = roomService.updateRoomSettings(body);
    else if (url.pathname === "/api/rooms/start") result = roomService.startRoomRound(body);
    else if (url.pathname === "/api/rooms/default-start") result = roomService.startDefaultRoute(body);
    else if (url.pathname === "/api/rooms/abandon") result = roomService.abandonRoute(body);
    else if (url.pathname === "/api/rooms/custom-map") result = roomService.setCustomMap(body);
    else if (url.pathname === "/api/rooms/dialog-start") result = roomService.startDialog(body);
    else if (url.pathname === "/api/rooms/dialog-advance") result = roomService.advanceDialog(body);
    else if (url.pathname === "/api/skins/mine") result = skins.mine(body);
    else if (url.pathname === "/api/skins/clear") result = skins.clear(body);
    else if (url.pathname === "/api/skins/remove") result = skins.remove(body);
    else if (url.pathname === "/api/city-maps/list") result = cityMaps.list(body);
    else if (url.pathname === "/api/city-maps/ticket") result = cityMaps.ticket(body);
    else if (url.pathname === "/api/rooms/leave") result = roomService.leaveRoom(body);
    else if (url.pathname === "/api/guest/name") result = guestNames.issue(body);
    else if (url.pathname === "/api/account/register") {
      result = await accountService.register(body, { allowDeveloperNames: developerRequest(body) });
      if (result[1]?.ok && result[1].adminOwner) result[1].devToken = issueDevToken();
    } else if (url.pathname === "/api/account/login") {
      result = await accountService.login(body);
      if (result[1]?.ok && result[1].adminOwner) result[1].devToken = issueDevToken();
    } else if (url.pathname === "/api/account/session") result = accountService.currentSession(body);
    else if (url.pathname === "/api/account/password") result = await accountService.changePassword(body);
    else if (url.pathname === "/api/account/rename") {
      const previous = accountService.sessionUsername(body.sessionToken);
      result = accountService.rename(body, { allowDeveloperNames: developerRequest(body) });
      if (result[1]?.ok && previous) moderation.renameAccount(previous, result[1].username);
    }
    else if (url.pathname === "/api/account/save") result = accountService.save(body);
    else if (url.pathname === "/api/account/load") result = accountService.loadProgress(body);
    else if (url.pathname === "/api/account/logout") result = accountService.logout(body);
    else if (url.pathname === "/api/admin/public/settings") {
      // the owner's change to the PUBLIC route settings: all of it or none (validated before anything is written)
      if (!developerRequest(body)) result = [403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Only an owner may change the public route" }];
      else {
        const saved = await publicSettings.update(body.changes);
        result = saved.ok
          ? [200, { ok: true, settings: saved.settings, specs: PUBLIC_SETTING_SPECS }]
          : [400, { ok: false, code: "INVALID_SETTING", error: saved.error, key: saved.key }];
      }
    }
    else if (url.pathname === "/api/admin/roster") result = accountService.listAdminRoster(body);
    else if (url.pathname === "/api/admin/roster/set") {
      result = accountService.setAdminRoster(body);
      if (result[1]?.ok) {
        for (const [ticket, proof] of adminTickets) {
          if (proof.username === result[1].entry?.username) adminTickets.delete(ticket);
        }
      }
    } else if (url.pathname === "/api/admin/roster/remove") {
      result = accountService.removeAdminRoster(body);
      if (result[1]?.ok) {
        for (const [ticket, proof] of adminTickets) {
          if (proof.username === result[1].username) adminTickets.delete(ticket);
        }
      }
    }
    else result = [404, { ok: false, code: "UNKNOWN_ENDPOINT", error: "Unknown endpoint" }];

    if (result[1]?.room && !Object.hasOwn(result[1], "ok")) result[1].reloadId = reloadId;
    return json(response, result[0], result[1]);
  });

  const timer = setInterval(() => {
    pruneEnetRooms();
    roomService.cleanup();
    const timestamp = now();
    for (const [key, rate] of requestRates) {
      if (timestamp >= rate.resetAt) requestRates.delete(key);
    }
  }, Math.max(1000, Math.min(enetRoomTtlMs, playerTtlMs, 15_000)));
  timer.unref();
  server.on("close", () => clearInterval(timer));
  server.startEnetRelay = ({ host = "0.0.0.0", port = relayPort } = {}) => {
    if (enetRelay) return enetRelay;
    enetRelay = createEnetRelay({
      roomByCode: (code) => {
        pruneEnetRooms();
        return enetRooms.get(code) || null;
      },
      promoteRoomHost: promoteEnetRoomHost,
      // the host quit, crashed or was killed: the code is invalid at once (a reserved room keeps its reservation)
      onRoomClosed: (code) => forgetEnetRoom(code),
      now,
      logger,
    });
    enetRelay.bind(port, host);
    return enetRelay;
  };
  server.on("upgrade", (request, socket, head) => {
    if (!wsRelay.handleUpgrade(request, socket, head)) socket.destroy();
  });
  server.on("close", () => {
    if (enetRelay) enetRelay.close();
    wsRelay.close();
  });
  return server;
}

function finitePlayers(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) ? Math.max(1, Math.min(16, number)) : fallback;
}

// Retain the old export name so existing Godot/backend smoke tests keep working.
export function createDirectoryServer({ roomTtlMs, ...options } = {}) {
  return createLastRouteServer({ ...options, enetRoomTtlMs: roomTtlMs ?? options.enetRoomTtlMs });
}

function environmentPath(name) {
  const value = String(process.env[name] || "").trim();
  return value ? resolve(process.cwd(), value) : null;
}

function validPort(value) {
  const number = Number(value);
  return Number.isInteger(number) && number >= 1 && number <= 65_535 ? number : 0;
}

export function deploymentPorts(environment = process.env) {
  const injectedPort = validPort(environment.PORT) || validPort(environment.SERVER_PORT);
  // The original deployment used TCP and UDP 1105 together. The client now
  // has a dedicated HTTP allocation, so automatically move a legacy injected
  // TCP 1105 to 8105 while leaving explicit non-relay ports alone.
  const httpPort = validPort(environment.LASTROUTE_HTTP_PORT)
    || (injectedPort === 1105 ? 8105 : injectedPort)
    || 8105;
  const relayPort = validPort(environment.LASTROUTE_RELAY_PORT) || 1105;
  return { httpPort, relayPort };
}

export function startLastRouteServerFromEnvironment() {
  const { httpPort: port, relayPort } = deploymentPorts();
  const host = process.env.HOST || "0.0.0.0";
  const relayEnabled = !/^(0|false|no|off)$/i.test(String(
    process.env.LASTROUTE_RELAY_ENABLED || "true"));
  const relayAddress = String(process.env.LASTROUTE_RELAY_HOST
    || "lastroute.kyttuslab.org").trim();
  const dataDirectory = environmentPath("LASTROUTE_DATA_DIR") || environmentPath("KYTTU_DATA_DIR") || resolve(process.cwd(), ".data");
  const accountStorePath = environmentPath("LASTROUTE_ACCOUNT_STORE") || environmentPath("KYTTU_ACCOUNT_STORE_PATH") || join(dataDirectory, "accounts.json");
  const publicDirectory = environmentPath("LASTROUTE_PUBLIC_DIR");
  const bundledCustomMapDirectory = resolve(process.cwd(), "assets/custom_maps");
  const requestedCustomMapDirectory = environmentPath("LASTROUTE_CUSTOM_MAP_DIR");
  const customMapDirectory = requestedCustomMapDirectory
    || (existsSync(bundledCustomMapDirectory) ? bundledCustomMapDirectory : null);
  const updateManifestPath = environmentPath("LASTROUTE_UPDATE_MANIFEST");
  const updateDirectory = environmentPath("LASTROUTE_UPDATE_DIR");
  let updates = {};
  if (updateManifestPath && existsSync(updateManifestPath)) {
    try {
      updates = JSON.parse(readFileSync(updateManifestPath, "utf8"));
    } catch (error) {
      console.error(`Could not read update manifest ${updateManifestPath}: ${error.message}`);
    }
  }
  const enetRoomTtlMs = Math.max(15, Number(process.env.ENET_ROOM_TTL_SECONDS) || 60) * 1000;
  const rateLimitWindowMs = Math.max(1,
    Number(process.env.LASTROUTE_RATE_LIMIT_WINDOW_SECONDS) || 60) * 1000;
  const apiRateLimitMax = Math.max(1,
    Number(process.env.LASTROUTE_API_RATE_LIMIT) || 600);
  const accountRateLimitMax = Math.max(1,
    Number(process.env.LASTROUTE_ACCOUNT_RATE_LIMIT) || 30);
  const cityMapDirectory = environmentPath("LASTROUTE_CITY_MAPS_DIR");
  const cityMapKeyPath = cityMapDirectory ? join(cityMapDirectory, "maps_pub.pem") : "";
  const server = createLastRouteServer({
    enetRoomTtlMs,
    cityMaps: {
      directory: cityMapDirectory,
      ticketSecret: String(process.env.LASTROUTE_CITY_MAP_TICKET_SECRET || ""),
      publicKeyPem: cityMapKeyPath && existsSync(cityMapKeyPath) ? readFileSync(cityMapKeyPath, "utf8") : "",
    },
    accountStorePath,
    moderationStorePath: environmentPath("LASTROUTE_MODERATION_STORE") || join(dirname(accountStorePath), "moderation.json"),
    moderationSalt: String(process.env.LASTROUTE_MODERATION_SALT || ""),
    skinDirectory: environmentPath("LASTROUTE_SKINS_DIR") || join(dataDirectory, "skins"),
    saveDirectory: environmentPath("LASTROUTE_SAVES_DIR") || join(dataDirectory, "saves"),
    modPackStorePath: environmentPath("LASTROUTE_MOD_INDEX_STORE") || join(dataDirectory, "mod_packs.json"),
    crashReportDirectory: environmentPath("LASTROUTE_CRASH_REPORTS_DIR") || join(dataDirectory, "crash-reports"),
    reservedRoomStorePath: environmentPath("LASTROUTE_RESERVED_ROOMS_STORE") || join(dataDirectory, "reserved_rooms.json"),
    agentTokens: parseAgentTokens(process.env.LASTROUTE_AGENT_TOKENS),
    reservedRoomCodes: String(process.env.LASTROUTE_RESERVED_ROOMS ?? "KATHY,AKIRI").split(",").map((code) => code.trim()).filter(Boolean),
    // public URL of the VPS 2 mod folder, e.g. https://<vps2>/lr3d/mods (the index never serves pack files itself)
    modDownloadBase: String(process.env.LASTROUTE_MOD_DOWNLOAD_BASE || "").trim(),
    publicDirectory: publicDirectory && existsSync(publicDirectory) ? publicDirectory : null,
    customMapDirectory: customMapDirectory && existsSync(customMapDirectory) ? customMapDirectory : null,
    updateDirectory: updateDirectory && existsSync(updateDirectory) ? updateDirectory : null,
    updates,
    updatesProvider: updateManifestPath ? () => {
      if (!existsSync(updateManifestPath)) return updates;
      return JSON.parse(readFileSync(updateManifestPath, "utf8"));
    } : null,
    adminPassword: String(process.env.LASTROUTE_ADMIN_PASSWORD || process.env.KYTTU_DEV_PASSWORD || ""),
    trustProxy: /^(1|true|yes|on)$/i.test(String(process.env.TRUST_PROXY || "")),
    relayAddress: relayEnabled ? relayAddress : "",
    relayPort: relayEnabled ? relayPort : 0,
    publicRouteStorePath: environmentPath("LASTROUTE_PUBLIC_ROUTE_STORE") || join(dataDirectory, "lastroute", "godot_publicroom.json"),
    publicSettingsStorePath: environmentPath("LASTROUTE_PUBLIC_SETTINGS_STORE") || join(dataDirectory, "lastroute", "public_settings.json"),
    publicRouteSeed: String(process.env.LASTROUTE_PUBLIC_ROUTE_SEED || "PUBLIC-ROUTE").slice(0, 64),
    rateLimitWindowMs,
    apiRateLimitMax,
    accountRateLimitMax,
    modRateLimitMax: Math.max(1, Number(process.env.LASTROUTE_MOD_RATE_LIMIT) || 120),
  });
  if (relayEnabled) server.startEnetRelay({ host, port: relayPort });
  server.listen(port, host, () => loggerStartup(host, port, accountStorePath,
    publicDirectory, customMapDirectory, updateManifestPath, updateDirectory, updates));
  const shutdown = (signal) => {
    console.log(`${signal}: closing Last Route backend`);
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(1), 10_000).unref();
  };
  process.once("SIGINT", () => shutdown("SIGINT"));
  process.once("SIGTERM", () => shutdown("SIGTERM"));
  return server;
}

function loggerStartup(host, port, accountStorePath, publicDirectory, customMapDirectory,
  updateManifestPath, updateDirectory, updates) {
  console.log(`Last Route backend listening on http://${host}:${port}`);
  console.log(`Account data: ${accountStorePath}`);
  if (publicDirectory) console.log(`Static files: ${publicDirectory}`);
  if (customMapDirectory) console.log(`Custom maps: ${customMapDirectory}`);
  if (updateManifestPath) console.log(`Update manifest: ${updateManifestPath}`);
  if (updateDirectory) console.log(`Update files: ${updateDirectory}`);
  for (const [channel, status] of Object.entries(publicUpdateStatus(updates))) {
    console.log(`Published ${channel} update: ${status.version} @ ${status.revision} (${status.platforms.join(", ")})`);
  }
}

const entry = process.argv[1] ? resolve(process.argv[1]) : "";
if (entry && fileURLToPath(import.meta.url) === entry) {
  startLastRouteServerFromEnvironment();
}
