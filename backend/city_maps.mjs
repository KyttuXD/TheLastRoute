// Online-only real-world City maps (docs/CITY_OSM_MAPS.md). Maps are built and signed on VPS 2, promoted into
// `<dir>/live/<id>/<version>/` on this server and listed in `<dir>/live/index.json`. Clients must hold a live
// account session to list maps or get a ticket, and every file request needs a short-lived ticket bound to the
// account, the map id and the version. Nothing here is ever bundled with the game.
//
//   POST /api/city-maps/list    {sessionToken}                              -> curated list
//   POST /api/city-maps/ticket  {sessionToken, id, version, roomCode?}      -> {ticket, expiresAt}
//   GET  /api/city-maps/files/<id>/<version>/<file>  header x-city-map-ticket -> file bytes
import { createHash, createHmac, createPublicKey, verify as verifySignature } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

export const CITY_MAP_TICKET_HEADER = "x-city-map-ticket";
const ID_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;
const FILE_PATTERN = /^(manifest\.json|manifest\.sig|manifest\.rsa\.sig|features\.json\.gz|c_\d{1,3}_\d{1,3}\.bin)$/;
const DEFAULT_TICKET_TTL_MS = 10 * 60 * 1000;
const INDEX_RECHECK_MS = 10_000;

function base64url(value) {
  return Buffer.from(value).toString("base64url");
}

export function createCityMaps({ directory = null, ticketSecret = "", publicKeyPem = "", accountService,
  roomExists = () => true, now = Date.now, ticketTtlMs = DEFAULT_TICKET_TTL_MS, logger = console } = {}) {
  const enabled = Boolean(directory && ticketSecret && publicKeyPem && existsSync(join(directory, "live")));
  const publicKey = enabled ? createPublicKey(publicKeyPem) : null;
  // "<id>/<version>" -> {manifest, files: Map(name -> Buffer)}; loaded once per version (versions are immutable)
  const maps = new Map();
  let listing = [];
  let indexStamp = "";
  let checkedAt = -Infinity;

  function loadVersion(id, version) {
    const key = `${id}/${version}`;
    if (maps.has(key)) return maps.get(key);
    const folder = join(directory, "live", id, String(version));
    const manifestBytes = readFileSync(join(folder, "manifest.json"));
    const signature = readFileSync(join(folder, "manifest.sig"));
    if (!verifySignature(null, manifestBytes, publicKey, signature)) throw new Error("bad manifest signature");
    const manifest = JSON.parse(manifestBytes.toString("utf8"));
    if (manifest.format !== "LRCM1" || manifest.id !== id || Number(manifest.version) !== Number(version)) {
      throw new Error("manifest does not match its folder");
    }
    const files = new Map([["manifest.json", manifestBytes], ["manifest.sig", signature]]);
    // RSA-SHA256 signature of the same manifest for the Godot client (its Crypto API has no Ed25519)
    const rsaPath = join(folder, "manifest.rsa.sig");
    if (existsSync(rsaPath)) files.set("manifest.rsa.sig", readFileSync(rsaPath));
    for (const [name, sha] of Object.entries(manifest.files || {})) {
      if (!FILE_PATTERN.test(name)) throw new Error(`unexpected file ${name}`);
      const bytes = readFileSync(join(folder, name));
      if (createHash("sha256").update(bytes).digest("hex") !== sha) throw new Error(`${name} does not match its sha256`);
      files.set(name, bytes);
    }
    const entry = { manifest, files };
    maps.set(key, entry);
    return entry;
  }

  function refresh() {
    if (!enabled || now() - checkedAt < INDEX_RECHECK_MS) return;
    checkedAt = now();
    const indexPath = join(directory, "live", "index.json");
    if (!existsSync(indexPath)) { listing = []; return; }
    const stamp = String(statSync(indexPath).mtimeMs);
    if (stamp === indexStamp) return;
    indexStamp = stamp;
    const next = [];
    let index;
    try {
      index = JSON.parse(readFileSync(indexPath, "utf8"));
    } catch (error) {
      logger.error(`city maps: unreadable index.json: ${error.message}`);
      return;
    }
    for (const item of Array.isArray(index.maps) ? index.maps : []) {
      const id = String(item.id || "");
      const version = Number(item.version);
      if (!ID_PATTERN.test(id) || !Number.isInteger(version) || version < 1) continue;
      try {
        const { manifest, files } = loadVersion(id, version);
        let bytes = 0;
        for (const data of files.values()) bytes += data.length;
        next.push({ id, version, name: manifest.name, bbox: manifest.bbox, chunksXZ: manifest.chunks_xz,
          stats: manifest.stats, attribution: manifest.attribution, licence: manifest.licence,
          files: files.size, bytes, meta: manifest.meta || {}, shape: manifest.shape || "bbox",
          sizeCells: manifest.size_cells, preview: manifest.preview_png || "" });
      } catch (error) {
        logger.error(`city maps: skipping ${id}/${version}: ${error.message}`);
      }
    }
    next.sort((a, b) => String(a.meta.region || "").localeCompare(String(b.meta.region || ""))
      || String(a.meta.state || "").localeCompare(String(b.meta.state || "")) || a.name.localeCompare(b.name));
    listing = next;
  }

  function sign(payload) {
    const body = base64url(JSON.stringify(payload));
    return `${body}.${createHmac("sha256", ticketSecret).update(body).digest("base64url")}`;
  }

  function readTicket(ticket) {
    const [body, mac] = String(ticket || "").split(".");
    if (!body || !mac) return null;
    const expected = createHmac("sha256", ticketSecret).update(body).digest("base64url");
    if (mac.length !== expected.length || !Buffer.from(mac).equals(Buffer.from(expected))) return null;
    try {
      const payload = JSON.parse(Buffer.from(body, "base64url").toString("utf8"));
      return payload.exp > now() ? payload : null;
    } catch {
      return null;
    }
  }

  function session(body) {
    return accountService.sessionUsername(body?.sessionToken) || "";
  }

  function list(body) {
    if (!enabled) return [503, { ok: false, code: "CITY_MAPS_OFF", error: "Real places are not available on this server" }];
    if (!session(body)) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to play real places" }];
    refresh();
    return [200, { ok: true, maps: listing }];
  }

  function ticket(body) {
    if (!enabled) return [503, { ok: false, code: "CITY_MAPS_OFF", error: "Real places are not available on this server" }];
    const username = session(body);
    if (!username) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to play real places" }];
    refresh();
    const id = String(body.id || "");
    const version = Number(body.version);
    if (!listing.some((map) => map.id === id && map.version === version)) {
      return [404, { ok: false, code: "UNKNOWN_MAP", error: "That map is not available" }];
    }
    const roomCode = String(body.roomCode || "");
    if (roomCode && !roomExists(roomCode)) return [409, { ok: false, code: "NO_ROOM", error: "The room is no longer open" }];
    const expiresAt = now() + ticketTtlMs;
    return [200, { ok: true, ticket: sign({ u: username, id, v: version, room: roomCode, exp: expiresAt }), expiresAt }];
  }

  // Returns null when the path is not a city map file route; otherwise [status, bytes|json].
  function file(pathname, headers = {}) {
    const match = pathname.match(/^\/api\/city-maps\/files\/([^/]+)\/(\d{1,6})\/([^/]+)$/);
    if (!match) return null;
    if (!enabled) return [503, { ok: false, code: "CITY_MAPS_OFF", error: "Real places are not available on this server" }];
    const [, id, versionText, name] = match;
    const payload = readTicket(headers[CITY_MAP_TICKET_HEADER]);
    if (!payload) return [401, { ok: false, code: "BAD_TICKET", error: "Map ticket missing or expired" }];
    if (payload.id !== id || payload.v !== Number(versionText)) {
      return [403, { ok: false, code: "WRONG_MAP", error: "Ticket is for another map" }];
    }
    if (payload.room && !roomExists(payload.room)) {
      return [409, { ok: false, code: "NO_ROOM", error: "The room is no longer open" }];
    }
    refresh();
    const entry = maps.get(`${id}/${Number(versionText)}`);
    const bytes = entry?.files.get(name);
    if (!bytes) return [404, { ok: false, code: "NOT_FOUND", error: "Not found" }];
    return [200, bytes, createHash("sha256").update(bytes).digest("hex")];
  }

  return { enabled, list, ticket, file };
}
