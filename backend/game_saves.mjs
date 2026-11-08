// Account cloud saves for the Godot games (Last Route's one-run autosave, RunSaves in the client). One JSON record per
// account, stored as <directory>/<USERNAME>.json and replaced atomically (temp file + rename).
//
//   GET /api/v1/saves/:gameId/:slot   header x-session-token            -> {ok, data|null, updatedAt}
//   PUT /api/v1/saves/:gameId/:slot   header x-session-token, {data, updatedAt}
//                                                                       -> {ok, updatedAt} | 409 STALE {updatedAt}
//
// Only allow-listed game/slot pairs exist. `data: null` deletes the slot (a run that ended). Newest wins: a PUT whose
// updatedAt (unix seconds, the client's saved_at) is older than the stored one is refused with 409 and the stored time,
// so a stale offline copy can never overwrite a newer run. Bodies are capped (SAVE_MAX_BYTES) and writes rate-limited
// per account.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const SAVE_MAX_BYTES = 64 * 1024;
export const SAVE_TOKEN_HEADER = "x-session-token";
export const SAVE_PATH = /^\/api\/v1\/saves\/([^/]+)\/([^/]+)$/;
// Keep the pre-revision slot intact; beta world revisions use distinct slots.
export const ALLOWED_SAVES = Object.freeze({ "last-route-godot": Object.freeze(["run", "run-world-1"]) });
const USERNAME_FILE = /^[A-Z0-9_]{1,32}$/;

export function createGameSaves({ directory = null, accountService, now = Date.now, logger = console,
  allowed = ALLOWED_SAVES, writeLimit = 30, writeWindowMs = 60_000 } = {}) {
  const enabled = Boolean(directory);
  const writes = new Map();   // username -> recent write times
  if (enabled) mkdirSync(directory, { recursive: true });

  const off = () => [503, { ok: false, code: "SAVES_DISABLED", error: "Cloud saves are not configured" }];

  function slotAllowed(gameId, slot) {
    return Object.hasOwn(allowed, gameId) && allowed[gameId].includes(slot);
  }

  function who(token) {
    const name = accountService.sessionUsername(String(token || ""));
    return name && USERNAME_FILE.test(name) ? name : "";
  }

  function read(username) {
    const path = join(directory, `${username}.json`);
    if (!existsSync(path)) return { games: {} };
    try {
      const value = JSON.parse(readFileSync(path, "utf8"));
      return value && typeof value === "object" && value.games && typeof value.games === "object" ? value : { games: {} };
    } catch (error) {
      logger.error?.(`[saves] unreadable save file for ${username}: ${error.message}`);
      return { games: {} };
    }
  }

  function write(username, record) {
    const path = join(directory, `${username}.json`);
    const temp = `${path}.tmp`;
    writeFileSync(temp, JSON.stringify(record));
    renameSync(temp, path);
  }

  function guard(token, gameId, slot) {
    if (!enabled) return [null, off()];
    if (!slotAllowed(gameId, slot)) return [null, [404, { ok: false, code: "UNKNOWN_SAVE", error: "No such save" }]];
    const username = who(token);
    if (!username) return [null, [401, { ok: false, code: "NOT_SIGNED_IN", error: "Sign in to sync saves" }]];
    return [username, null];
  }

  function get(token, gameId, slot) {
    const [username, refused] = guard(token, gameId, slot);
    if (refused) return refused;
    const stored = read(username).games?.[gameId]?.[slot];
    return [200, { ok: true, gameId, slot, data: stored?.data ?? null, updatedAt: Number(stored?.updatedAt) || 0 }];
  }

  // body: the parsed JSON {data, updatedAt}
  function put(token, gameId, slot, body) {
    const [username, refused] = guard(token, gameId, slot);
    if (refused) return refused;
    if (!body || typeof body !== "object" || Array.isArray(body) || !Object.hasOwn(body, "data")
        || (body.data !== null && (typeof body.data !== "object" || Array.isArray(body.data)))) {
      return [400, { ok: false, code: "BAD_SAVE", error: "A save is {data: object|null, updatedAt}" }];
    }
    const recent = (writes.get(username) || []).filter((at) => now() - at < writeWindowMs);
    if (recent.length >= writeLimit) {
      return [429, { ok: false, code: "SAVE_RATE_LIMIT", error: "Too many saves; try again shortly" }];
    }
    const updatedAt = Number.isFinite(Number(body.updatedAt)) && Number(body.updatedAt) > 0
      ? Math.floor(Number(body.updatedAt)) : Math.floor(now() / 1000);
    const record = read(username);
    const stored = record.games?.[gameId]?.[slot];
    if (stored && Number(stored.updatedAt) > updatedAt) {
      return [409, { ok: false, code: "STALE", error: "A newer save is already stored", updatedAt: Number(stored.updatedAt) }];
    }
    recent.push(now());
    writes.set(username, recent);
    record.games[gameId] = record.games[gameId] || {};
    if (body.data === null) delete record.games[gameId][slot];
    else record.games[gameId][slot] = { data: body.data, updatedAt };
    write(username, record);
    return [200, { ok: true, gameId, slot, updatedAt }];
  }

  return { enabled, get, put };
}
