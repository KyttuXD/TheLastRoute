// Custom player skins (docs/PLAYER_SKINS.md): one 64x64 PNG per account, stored by the sha256 of its bytes and served from a
// file route. The game re-validates every download, so this side only has to make sure nothing but a well-formed PNG of the
// right size is ever stored or served, and that uploads cannot be spammed.
//
//   POST /api/skins/upload   raw PNG body, header x-session-token   -> {ok, hash}      (one skin per account, rate limited)
//   POST /api/skins/mine     {sessionToken}                         -> {ok, hash|""}
//   POST /api/skins/clear    {sessionToken}                         -> {ok}            (an account removes its own skin)
//   POST /api/skins/remove   {sessionToken, hash|username}          -> {ok}            (owners only; the hash is blocked from re-upload)
//   GET  /api/skins/files/<sha256>.png                              -> the PNG, immutable
//
// Account skins (the skin lives on the account, not on the computer):
//   GET  /api/v1/skins/<ACCOUNT>   public (co-op peers)  -> {ok, account, hash|"", arms, updatedAt, versions:[hash]}
//   PUT  /api/v1/skins/<ACCOUNT>   raw PNG body, headers x-session-token (the owner's) and x-skin-arms (auto|classic|slim)
//                                                         -> {ok, hash, arms, updatedAt}   (same checks and rate limit as /upload)
// A PNG is 64x64 (or the old 64x32). The last SKIN_VERSIONS skins of an account are kept as versions.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { inflateSync } from "node:zlib";

export const SKIN_SIZE = 64;
export const SKIN_MAX_BYTES = 65536;
export const SKIN_TOKEN_HEADER = "x-session-token";
export const SKIN_ARMS_HEADER = "x-skin-arms";
export const SKIN_VERSIONS = 5;
export const SKIN_ACCOUNT_PATH = /^\/api\/v1\/skins\/([A-Za-z0-9_]{1,32})$/;
const ARMS_VALUES = new Set(["auto", "classic", "slim"]);
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const HASH_PATTERN = /^[0-9a-f]{64}$/;
const FILE_PATTERN = /^\/api\/skins\/files\/([0-9a-f]{64})\.png$/;
const COLOUR_CHANNELS = new Map([[0, 1], [2, 3], [3, 1], [4, 2], [6, 4]]);
const ALLOWED_DEPTHS = new Map([[0, [1, 2, 4, 8, 16]], [2, [8, 16]], [3, [1, 2, 4, 8]], [4, [8, 16]], [6, [8, 16]]]);

let crcTable = null;
function crc32(buffer) {
  if (!crcTable) {
    crcTable = new Uint32Array(256);
    for (let n = 0; n < 256; n += 1) {
      let c = n;
      for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[n] = c >>> 0;
    }
  }
  let crc = 0xffffffff;
  for (const byte of buffer) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

// Structural validation without trusting the file: signature, every chunk's length and CRC, IHDR first with exactly 64x64 and a
// legal colour type / depth, IDAT present, IEND last with nothing after it, and the zlib stream inflating to exactly the
// number of bytes the header promises. Returns "" when fine, else the reason.
export function validateSkinPng(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0) return "the file is empty";
  if (bytes.length > SKIN_MAX_BYTES) return `the file is over ${SKIN_MAX_BYTES / 1024} KB`;
  if (bytes.length < PNG_SIGNATURE.length + 12 || !bytes.subarray(0, 8).equals(PNG_SIGNATURE)) return "not a PNG";
  let offset = 8;
  let header = null;
  let sawEnd = false;
  const data = [];
  let first = true;
  while (offset < bytes.length) {
    if (sawEnd) return "data after the end of the PNG";
    if (offset + 12 > bytes.length) return "truncated PNG";
    const length = bytes.readUInt32BE(offset);
    if (length > SKIN_MAX_BYTES || offset + 12 + length > bytes.length) return "truncated PNG";
    const type = bytes.toString("latin1", offset + 4, offset + 8);
    const body = bytes.subarray(offset + 8, offset + 8 + length);
    if (bytes.readUInt32BE(offset + 8 + length) !== crc32(bytes.subarray(offset + 4, offset + 8 + length))) return "corrupt PNG";
    if (first) {
      if (type !== "IHDR" || length !== 13) return "not a PNG";
      header = { width: body.readUInt32BE(0), height: body.readUInt32BE(4), depth: body[8], colour: body[9],
        compression: body[10], filter: body[11], interlace: body[12] };
      first = false;
    } else if (type === "IDAT") {
      data.push(body);
    } else if (type === "IEND") {
      sawEnd = true;
    } else if (type === "IHDR") {
      return "corrupt PNG";
    }
    offset += 12 + length;
  }
  if (!header || !sawEnd || data.length === 0) return "truncated PNG";
  if (header.width !== SKIN_SIZE || (header.height !== SKIN_SIZE && header.height !== SKIN_SIZE / 2)) {
    return `a skin is ${SKIN_SIZE}x${SKIN_SIZE} (or the old ${SKIN_SIZE}x${SKIN_SIZE / 2})`;
  }
  const depths = ALLOWED_DEPTHS.get(header.colour);
  if (!depths || !depths.includes(header.depth) || header.compression !== 0 || header.filter !== 0 || header.interlace > 1) {
    return "unsupported PNG format";
  }
  if (header.interlace === 1) return "interlaced PNGs are not supported";
  const rowBytes = Math.ceil((SKIN_SIZE * COLOUR_CHANNELS.get(header.colour) * header.depth) / 8);
  const expected = (rowBytes + 1) * header.height;
  let raw;
  try {
    raw = inflateSync(Buffer.concat(data), { maxOutputLength: expected + 1 });
  } catch {
    return "corrupt PNG";
  }
  if (raw.length !== expected) return "corrupt PNG";
  return "";
}

export function createSkins({ directory = null, accountService, now = Date.now, logger = console,
  uploadLimit = 5, uploadWindowMs = 10 * 60 * 1000 } = {}) {
  const enabled = Boolean(directory);
  const filesDirectory = enabled ? join(directory, "files") : "";
  const indexPath = enabled ? join(directory, "index.json") : "";
  let owners = new Map();     // username -> hash
  let blocked = new Set();    // hashes an owner removed
  let meta = new Map();       // username -> {arms, updatedAt, versions: [{hash, updatedAt}]} (newest first)
  const uploads = new Map();  // username -> [timestamps]
  const cache = new Map();    // hash -> Buffer (bounded)

  if (enabled) {
    mkdirSync(filesDirectory, { recursive: true });
    if (existsSync(indexPath)) {
      try {
        const saved = JSON.parse(readFileSync(indexPath, "utf8"));
        owners = new Map(Object.entries(saved.owners || {}).filter(([, hash]) => HASH_PATTERN.test(hash)));
        blocked = new Set((saved.blocked || []).filter((hash) => HASH_PATTERN.test(hash)));
        meta = new Map(Object.entries(saved.meta || {}).map(([who, entry]) => [who, {
          arms: ARMS_VALUES.has(entry?.arms) ? entry.arms : "auto",
          updatedAt: Number(entry?.updatedAt) || 0,
          versions: (Array.isArray(entry?.versions) ? entry.versions : []).filter((v) => HASH_PATTERN.test(v?.hash)).slice(0, SKIN_VERSIONS),
        }]));
      } catch (error) {
        logger.error(`skins: unreadable index.json: ${error.message}`);
      }
    }
  }

  function persist() {
    const next = `${indexPath}.tmp`;
    writeFileSync(next, JSON.stringify({ owners: Object.fromEntries(owners), blocked: [...blocked], meta: Object.fromEntries(meta) }));
    renameSync(next, indexPath);
  }

  function off() {
    return [503, { ok: false, code: "SKINS_OFF", error: "Skins are not available on this server" }];
  }

  function username(token) {
    return accountService.sessionUsername(token) || "";
  }

  function stillUsed(hash) {
    for (const value of owners.values()) if (value === hash) return true;
    for (const entry of meta.values()) if (entry.versions.some((v) => v.hash === hash)) return true;
    return false;
  }

  function dropFile(hash) {
    cache.delete(hash);
    const path = join(filesDirectory, `${hash}.png`);
    if (existsSync(path)) unlinkSync(path);
  }

  // the checks every upload shares; returns [status, body] on refusal, else null
  function refuse(who, bytes) {
    if (!who) return [401, { ok: false, code: "NOT_SIGNED_IN", error: "Sign in to share a skin" }];
    const recent = (uploads.get(who) || []).filter((at) => now() - at < uploadWindowMs);
    if (recent.length >= uploadLimit) {
      return [429, { ok: false, code: "UPLOAD_RATE_LIMIT", error: "Too many skin uploads; try again later" }];
    }
    const reason = validateSkinPng(bytes);
    if (reason) return [400, { ok: false, code: "BAD_SKIN", error: `Skin refused: ${reason}` }];
    const hash = createHash("sha256").update(bytes).digest("hex");
    if (blocked.has(hash)) return [403, { ok: false, code: "SKIN_BLOCKED", error: "That skin was removed by an owner" }];
    return null;
  }

  function store(who, bytes, arms) {
    const hash = createHash("sha256").update(bytes).digest("hex");
    const recent = (uploads.get(who) || []).filter((at) => now() - at < uploadWindowMs);
    recent.push(now());
    uploads.set(who, recent);
    const path = join(filesDirectory, `${hash}.png`);
    if (!existsSync(path)) writeFileSync(path, bytes);
    const previous = owners.get(who);
    owners.set(who, hash);
    const entry = meta.get(who) || { arms: "auto", updatedAt: 0, versions: [] };
    const updatedAt = Math.floor(now() / 1000);
    entry.arms = ARMS_VALUES.has(arms) ? arms : "auto";
    entry.updatedAt = updatedAt;
    entry.versions = [{ hash, updatedAt }, ...entry.versions.filter((v) => v.hash !== hash)];
    const dropped = entry.versions.splice(SKIN_VERSIONS);
    meta.set(who, entry);
    for (const old of [previous, ...dropped.map((v) => v.hash)]) {
      if (old && old !== hash && !stillUsed(old)) dropFile(old);
    }
    persist();
    return { hash, arms: entry.arms, updatedAt };
  }

  function upload(token, bytes) {
    if (!enabled) return off();
    const who = username(token);
    const refused = refuse(who, bytes);
    if (refused) return refused;
    return [200, { ok: true, hash: store(who, bytes, meta.get(who)?.arms).hash }];
  }

  function account(name) {
    return String(name || "").toUpperCase();
  }

  // PUT /api/v1/skins/<ACCOUNT>: only the account's own session may write it
  function put(token, accountName, bytes, arms) {
    if (!enabled) return off();
    const who = username(token);
    if (!who) return [401, { ok: false, code: "NOT_SIGNED_IN", error: "Sign in to save your skin" }];
    if (who !== account(accountName)) return [403, { ok: false, code: "NOT_YOUR_SKIN", error: "That skin belongs to another account" }];
    const refused = refuse(who, bytes);
    if (refused) return refused;
    return [200, { ok: true, account: who, ...store(who, bytes, arms) }];
  }

  // GET /api/v1/skins/<ACCOUNT>: public; "" when the account has no skin
  function get(accountName) {
    if (!enabled) return off();
    const who = account(accountName);
    const hash = owners.get(who) || "";
    const entry = meta.get(who);
    return [200, { ok: true, account: who, hash: blocked.has(hash) ? "" : hash, arms: entry?.arms || "auto",
      updatedAt: entry?.updatedAt || 0, versions: (entry?.versions || []).map((v) => v.hash).filter((h) => !blocked.has(h)) }];
  }

  function mine(body) {
    if (!enabled) return off();
    const who = username(body?.sessionToken);
    if (!who) return [401, { ok: false, code: "NOT_SIGNED_IN", error: "Sign in first" }];
    return [200, { ok: true, hash: owners.get(who) || "" }];
  }

  function clear(body) {
    if (!enabled) return off();
    const who = username(body?.sessionToken);
    if (!who) return [401, { ok: false, code: "NOT_SIGNED_IN", error: "Sign in first" }];
    const previous = owners.get(who);
    const old = (meta.get(who)?.versions || []).map((v) => v.hash);
    owners.delete(who);
    meta.delete(who);
    for (const hash of [previous, ...old]) if (hash && !stillUsed(hash)) dropFile(hash);
    persist();
    return [200, { ok: true }];
  }

  function remove(body) {
    if (!enabled) return off();
    const requester = username(body?.sessionToken);
    if (!requester || !accountService.isAdminOwner(requester)) {
      return [403, { ok: false, code: "ADMIN_OWNER_REQUIRED", error: "Full admin account required" }];
    }
    let hash = String(body?.hash || "").toLowerCase();
    if (!hash && body?.username) hash = owners.get(String(body.username).toUpperCase()) || "";
    if (!HASH_PATTERN.test(hash)) return [404, { ok: false, code: "SKIN_NOT_FOUND", error: "No such skin" }];
    for (const [who, value] of [...owners]) if (value === hash) owners.delete(who);
    for (const entry of meta.values()) entry.versions = entry.versions.filter((v) => v.hash !== hash);
    blocked.add(hash);
    dropFile(hash);
    persist();
    return [200, { ok: true, hash, blocked: true }];
  }

  // pathname -> [status, bytes|body, hash]
  function file(pathname) {
    if (!enabled) return off();
    const match = FILE_PATTERN.exec(pathname);
    if (!match) return [404, { ok: false, code: "SKIN_NOT_FOUND", error: "No such skin" }];
    const hash = match[1];
    if (blocked.has(hash)) return [404, { ok: false, code: "SKIN_NOT_FOUND", error: "No such skin" }];
    let bytes = cache.get(hash);
    if (!bytes) {
      const path = join(filesDirectory, `${hash}.png`);
      if (!existsSync(path)) return [404, { ok: false, code: "SKIN_NOT_FOUND", error: "No such skin" }];
      bytes = readFileSync(path);
      // a stored file must still be its own hash and a valid skin; anything else is never served
      if (createHash("sha256").update(bytes).digest("hex") !== hash || validateSkinPng(bytes)) {
        return [404, { ok: false, code: "SKIN_NOT_FOUND", error: "No such skin" }];
      }
      if (cache.size >= 64) cache.delete(cache.keys().next().value);
      cache.set(hash, bytes);
    }
    return [200, bytes, hash];
  }

  return { enabled, upload, put, get, mine, clear, remove, file, owners: () => new Map(owners), blocked: () => new Set(blocked) };
}
