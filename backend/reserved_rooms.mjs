import { randomBytes, scryptSync, timingSafeEqual } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

// Reserved private rooms (KT, 2026-10-28: "a private room on the server side, like a public room but restartable").
// Store and echo only. A reservation is a room code the owner keeps: it never expires, nobody else can register it,
// and only invited accounts (or holders of the room password, kept as a scrypt hash) may host or join it. The live
// ENet session on that code still comes and goes with whoever hosts it; the reservation, its stored route settings,
// invite list and mods live in one JSON file under the data directory and survive restarts and deploys.
//
// Owner controls (restart, end, kick, settings, transfer host) are not game logic here: each is queued as a
// command with a sequence number and echoed to the current host on its heartbeat, which carries it out and acks it.

export const RESERVED_LIMITS = Object.freeze({ rooms: 50, invites: 64, commands: 20, mods: 32, passwordMin: 6,
  passwordMax: 128, ticketTtlMs: 10 * 60 * 1000, ticketsPerRoom: 64 });
export const RESERVED_ACTIONS = Object.freeze(["restart", "end", "kick", "settings", "transfer"]);
// Reserved codes may use any five letters or digits (KATHY, AKIRI): the generated alphabet's exclusions do not apply.
const CODE_PATTERN = /^[A-Z0-9]{5}$/;
const USERNAME_PATTERN = /^[A-Za-z0-9_]{3,20}$/;
const SEED_PATTERN = /^[A-Za-z0-9 _-]{0,40}$/;
const MOD_ID_PATTERN = /^[a-z0-9][a-z0-9_]{2,31}$/;

export function normalizeReservedCode(value) {
  const code = String(value ?? "").trim().toUpperCase();
  return CODE_PATTERN.test(code) ? code : "";
}

function hashPassword(password) {
  const salt = randomBytes(16);
  return `scrypt$${salt.toString("base64url")}$${scryptSync(String(password), salt, 32).toString("base64url")}`;
}

function passwordMatches(stored, password) {
  const [kind, salt, hash] = String(stored || "").split("$");
  if (kind !== "scrypt" || !salt || !hash || !password) return false;
  const expected = Buffer.from(hash, "base64url");
  const actual = scryptSync(String(password), Buffer.from(salt, "base64url"), expected.length);
  return expected.length === actual.length && timingSafeEqual(expected, actual);
}

// Route settings a reservation remembers; anything else is dropped. Echoed to the host, never interpreted here.
export function cleanRoomSettings(value = {}, previous = {}) {
  const settings = { ...previous };
  if (value.seed !== undefined) settings.seed = SEED_PATTERN.test(String(value.seed)) ? String(value.seed) : previous.seed || "";
  for (const key of ["mode", "scale", "difficulty"]) {
    if (value[key] !== undefined && Number.isInteger(Number(value[key])) && Number(value[key]) >= 0 && Number(value[key]) < 32) {
      settings[key] = Number(value[key]);
    }
  }
  if (value.daytime !== undefined) settings.daytime = Boolean(value.daytime);
  if (Array.isArray(value.mods)) {
    settings.mods = [...new Set(value.mods.map(String).filter((id) => MOD_ID_PATTERN.test(id)))].slice(0, RESERVED_LIMITS.mods);
  }
  return settings;
}

function cleanInvites(list) {
  if (!Array.isArray(list)) return null;
  return [...new Set(list.map((name) => String(name).trim()).filter((name) => USERNAME_PATTERN.test(name))
    .map((name) => name.toUpperCase()))].slice(0, RESERVED_LIMITS.invites);
}

// What a reservation looks like to its owner (never the password hash).
export function ownerView(room) {
  return {
    code: room.code, settings: room.settings, invites: room.invites, hasPassword: Boolean(room.passwordHash),
    commands: room.commands, commandSeq: room.commandSeq, createdBy: room.createdBy, updatedAt: room.updatedAt,
    fromEnvironment: Boolean(room.fromEnvironment),
  };
}

export function createReservedRooms({ storePath, codes = [], now = Date.now, logger = console } = {}) {
  if (!storePath) throw new TypeError("storePath is required");
  let rooms = {};
  if (existsSync(storePath)) {
    try {
      const parsed = JSON.parse(readFileSync(storePath, "utf8"));
      if (parsed && typeof parsed.rooms === "object") rooms = parsed.rooms;
    } catch (error) {
      logger.error(`Reserved rooms: could not read ${storePath}: ${error.message}`);
    }
  }
  // join tickets: code -> Map(ticketHex -> expiresAt); memory only (a restart asks joiners to resolve again)
  const tickets = new Map();

  function save() {
    mkdirSync(dirname(storePath), { recursive: true });
    writeFileSync(`${storePath}.tmp`, JSON.stringify({ rooms }, null, 1));
    renameSync(`${storePath}.tmp`, storePath);
  }

  function fresh(code, createdBy, fromEnvironment = false) {
    return { code, settings: cleanRoomSettings({ seed: "", mods: [] }), invites: [], passwordHash: "", commands: [],
      commandSeq: 0, createdBy, updatedAt: new Date(now()).toISOString(), fromEnvironment };
  }

  for (const room of Object.values(rooms)) room.invites = cleanInvites(room.invites) || [];
  // Codes from the environment (LASTROUTE_RESERVED_ROOMS) always exist; their stored settings are kept.
  let changed = false;
  for (const value of codes) {
    const code = normalizeReservedCode(value);
    if (code && !rooms[code]) {
      rooms[code] = fresh(code, "environment", true);
      changed = true;
    } else if (code) {
      rooms[code].fromEnvironment = true;
    }
  }
  if (changed) save();

  const isReserved = (code) => Boolean(rooms[normalizeReservedCode(code)]);

  // May this person host or join? The owner always; an invited account; or anyone with the room password.
  function hasAccess(code, { username = "", password = "", owner = false } = {}) {
    const room = rooms[normalizeReservedCode(code)];
    if (!room) return false;
    if (owner) return true;
    if (username && room.invites.includes(String(username).toUpperCase())) return true;
    return Boolean(password) && passwordMatches(room.passwordHash, password);
  }

  // A short-lived ticket the relay accepts as the guest's join nonce for this code (base64url, 18 bytes).
  function issueTicket(code) {
    const clean = normalizeReservedCode(code);
    const ticket = randomBytes(18);
    const map = tickets.get(clean) || new Map();
    const timestamp = now();
    for (const [key, expiresAt] of map) if (expiresAt <= timestamp) map.delete(key);
    while (map.size >= RESERVED_LIMITS.ticketsPerRoom) map.delete(map.keys().next().value);
    map.set(ticket.toString("hex"), timestamp + RESERVED_LIMITS.ticketTtlMs);
    tickets.set(clean, map);
    return ticket.toString("base64url");
  }

  function ticketValid(code, payload) {
    const map = tickets.get(normalizeReservedCode(code));
    const expiresAt = map?.get(Buffer.from(payload).toString("hex"));
    return Boolean(expiresAt && expiresAt > now());
  }

  // Owner admin: [status, body]
  function admin(body, owner) {
    const action = String(body.action || "list");
    const code = normalizeReservedCode(body.code);
    if (action === "list") {
      return [200, { ok: true, rooms: Object.values(rooms).map(ownerView) }];
    }
    if (!code) return [400, { ok: false, code: "INVALID_ROOM_CODE", error: "Reserved codes are 5 letters/digits" }];
    if (action === "create") {
      if (rooms[code]) return [409, { ok: false, code: "ROOM_ALREADY_RESERVED", error: "That code is already reserved" }];
      if (Object.keys(rooms).length >= RESERVED_LIMITS.rooms) return [409, { ok: false, code: "TOO_MANY_RESERVED_ROOMS", error: "Reservation limit reached" }];
      rooms[code] = fresh(code, owner);
    }
    const room = rooms[code];
    if (!room) return [404, { ok: false, code: "ROOM_NOT_RESERVED", error: "That code is not reserved" }];
    if (action === "delete") {
      if (room.fromEnvironment) return [409, { ok: false, code: "ROOM_FROM_ENVIRONMENT", error: "Remove it from LASTROUTE_RESERVED_ROOMS instead" }];
      delete rooms[code];
      tickets.delete(code);
      save();
      return [200, { ok: true, deleted: code }];
    }
    if (action === "create" || action === "update") {
      if (body.settings) room.settings = cleanRoomSettings(body.settings, room.settings);
      const invites = cleanInvites(body.invites);
      if (invites) room.invites = invites;
      if (body.password !== undefined) {
        const password = String(body.password ?? "");
        if (password && (password.length < RESERVED_LIMITS.passwordMin || password.length > RESERVED_LIMITS.passwordMax)) {
          return [400, { ok: false, code: "INVALID_ROOM_PASSWORD", error: `Room passwords are ${RESERVED_LIMITS.passwordMin}-${RESERVED_LIMITS.passwordMax} characters` }];
        }
        room.passwordHash = password ? hashPassword(password) : "";
        tickets.delete(code);   // a new password revokes outstanding join tickets
      }
      room.updatedAt = new Date(now()).toISOString();
      save();
      return [action === "create" ? 201 : 200, { ok: true, room: ownerView(room) }];
    }
    if (action === "whitelist") {
      // add / remove / list one account on the room's whitelist (canonical uppercase account names)
      const op = String(body.op || "list");
      const name = String(body.name || "").trim().toUpperCase();
      if (op !== "list") {
        if (!USERNAME_PATTERN.test(name)) return [400, { ok: false, code: "INVALID_ACCOUNT_NAME", error: "Account names are 3-20 letters, digits or _" }];
        if (op === "add" && !room.invites.includes(name)) {
          if (room.invites.length >= RESERVED_LIMITS.invites) return [409, { ok: false, code: "WHITELIST_FULL", error: "Whitelist is full" }];
          room.invites.push(name);
        } else if (op === "remove") {
          room.invites = room.invites.filter((entry) => entry !== name);
        } else if (op !== "add") {
          return [400, { ok: false, code: "INVALID_WHITELIST_OP", error: "op: add, remove or list" }];
        }
        room.updatedAt = new Date(now()).toISOString();
        save();
      }
      return [200, { ok: true, code, whitelist: room.invites }];
    }
    if (action === "command") {
      const command = String(body.command || "");
      if (!RESERVED_ACTIONS.includes(command)) {
        return [400, { ok: false, code: "INVALID_ROOM_COMMAND", error: `Commands: ${RESERVED_ACTIONS.join(", ")}` }];
      }
      const args = {};
      if (command === "restart") args.sameSeed = Boolean(body.sameSeed);
      if (command === "kick" || command === "transfer") {
        const target = String(body.target || "").trim();
        if (!USERNAME_PATTERN.test(target) && !/^[A-Za-z0-9 _-]{1,24}$/.test(target)) {
          return [400, { ok: false, code: "INVALID_COMMAND_TARGET", error: "Name the player (account or display name)" }];
        }
        args.target = target;
      }
      if (command === "settings" || command === "restart") {
        if (body.settings) room.settings = cleanRoomSettings(body.settings, room.settings);
        args.settings = room.settings;
      }
      room.commandSeq += 1;
      room.commands.push({ seq: room.commandSeq, command, args, by: owner, at: new Date(now()).toISOString() });
      room.commands = room.commands.slice(-RESERVED_LIMITS.commands);
      room.updatedAt = new Date(now()).toISOString();
      save();
      return [202, { ok: true, queued: room.commands.at(-1), room: ownerView(room) }];
    }
    return [400, { ok: false, code: "INVALID_ACTION", error: "Actions: list, create, update, delete, whitelist, command" }];
  }

  // The host's view on register/heartbeat: stored settings and the commands after `since`, then acks drop up to `ack`.
  function hostView(code, { since = 0, ack = 0 } = {}) {
    const room = rooms[normalizeReservedCode(code)];
    if (!room) return null;
    const acked = Number(ack) || 0;
    if (acked > 0 && room.commands.some((entry) => entry.seq <= acked)) {
      room.commands = room.commands.filter((entry) => entry.seq > acked);
      save();
    }
    const after = Math.max(Number(since) || 0, acked);
    return { reserved: true, settings: room.settings, commands: room.commands.filter((entry) => entry.seq > after),
      commandSeq: room.commandSeq };
  }

  return { isReserved, hasAccess, issueTicket, ticketValid, admin, hostView, codes: () => Object.keys(rooms) };
}
