// Guest names (KT 2026-10-28, the 2D game's short style): one letter, a dash, four digits, e.g. "R-4821". Letters skip I and O and
// digits skip 0 and 1, so a name read aloud or typed from a screenshot is unambiguous. The server hands them out unique: a name is
// leased to whoever asked for it (renewed every time they ask again with it) and is never handed to anyone else while the lease runs.
//
//   POST /api/guest/name   {previous?, token?}   -> {ok, name, token}
// `previous` is kept when it is a valid guest name that nobody holds, or when `token` proves this client holds it (a renewal);
// otherwise a fresh name is issued. The token is a random secret only the holder has.
//
// Store-and-echo: <data>/guest_names.json (name -> lease expiry), atomic writes. Registered accounts can never clash: their names
// have no dash (account_service USERNAME_PATTERN).
import { randomBytes } from "node:crypto";
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";

export const GUEST_LETTERS = "ABCDEFGHJKLMNPQRSTUVWXYZ";
export const GUEST_DIGITS = "23456789";
export const GUEST_NAME_PATTERN = /^[A-HJ-NP-Z]-[2-9]{4}$/;
const LEASE_MS = 30 * 24 * 60 * 60 * 1000;
const MAX_TRIES = 64;

export function randomGuestName(random = Math.random) {
  let name = GUEST_LETTERS[Math.floor(random() * GUEST_LETTERS.length)] + "-";
  for (let i = 0; i < 4; i += 1) name += GUEST_DIGITS[Math.floor(random() * GUEST_DIGITS.length)];
  return name;
}

export function createGuestNames({ storePath = null, now = Date.now, random = Math.random, leaseMs = LEASE_MS, logger = console } = {}) {
  const leases = new Map();   // name -> {expiry (ms), token}
  if (storePath && existsSync(storePath)) {
    try {
      for (const [name, lease] of Object.entries(JSON.parse(readFileSync(storePath, "utf8")).leases || {})) {
        if (GUEST_NAME_PATTERN.test(name) && Number(lease?.expiry) > now() && typeof lease?.token === "string") {
          leases.set(name, { expiry: Number(lease.expiry), token: lease.token });
        }
      }
    } catch (error) {
      logger.error?.(`[guest-names] unreadable ${storePath}: ${error.message}`);
    }
  }

  function persist() {
    if (!storePath) return;
    for (const [name, lease] of leases) if (lease.expiry <= now()) leases.delete(name);
    const next = `${storePath}.tmp`;
    writeFileSync(next, JSON.stringify({ leases: Object.fromEntries(leases) }));
    renameSync(next, storePath);
  }

  function held(name) {
    const lease = leases.get(name);
    return lease !== undefined && lease.expiry > now();
  }

  function grant(name, token = randomBytes(18).toString("base64url")) {
    leases.set(name, { expiry: now() + leaseMs, token });
    persist();
    return token;
  }

  function issue(body = {}) {
    const previous = String(body?.previous || "").toUpperCase();
    const token = typeof body?.token === "string" ? body.token : "";
    if (GUEST_NAME_PATTERN.test(previous)) {
      if (held(previous) && token && leases.get(previous).token === token) {
        return [200, { ok: true, name: previous, token: grant(previous, token), renewed: true }];
      }
      if (!held(previous)) return [200, { ok: true, name: previous, token: grant(previous) }];
    }
    for (let attempt = 0; attempt < MAX_TRIES; attempt += 1) {
      const name = randomGuestName(random);
      if (held(name)) continue;   // a clash: try again
      return [200, { ok: true, name, token: grant(name) }];
    }
    return [503, { ok: false, code: "GUEST_NAMES_FULL", error: "No free guest name right now; try again" }];
  }

  return { issue, held, size: () => leases.size };
}
