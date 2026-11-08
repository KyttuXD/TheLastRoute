// The persistent, server-owned PUBLIC route.
//
// Gameplay itself is still the Godot ENet host-authoritative session; this module
// is the part that must outlive any one host: the world seed, relays collected,
// claimed items (which respawn on a timer), the last sanitized hunter snapshot,
// per-player resume records, the UTC day and the player-scaled hunter target.
// Whichever client holds the ENet host token for room code PUBLIC is the
// authority and may post snapshots/claims; everyone may post their own position
// and inventory. All input is clamped and rate limited here, never trusted.

import { randomBytes } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

// Five characters from the room-code alphabet: the relay wire format carries exactly
// five. Players see it as "PUBLIC ROUTE", never as a code to type.
export const PUBLIC_CODE = "PUBLC";
// Each game has its own public room: its own code, seed, store file and state.
export const PUBLIC_CODES = { last_route: PUBLIC_CODE, city: "CTYPR" };
export const PUBLIC_RESUME_TTL_MS = 6 * 60 * 60 * 1000;
export const PUBLIC_ITEM_RESPAWN_MS = 3 * 60 * 1000;
export const PUBLIC_JOIN_GRACE_MS = 10_000;
export const PUBLIC_PLAYER_TTL_MS = 20_000;
export const PUBLIC_AUTHORITY_STALE_MS = 15_000;
export const PUBLIC_HUNTER_CAP = 8;
const SNAPSHOT_MAX_CHARS = 400_000;
const WORLD_LIMIT = 1_000_000;
const MAX_CLAIMS = 20_000;
const STATE_MIN_INTERVAL_MS = 250;
const JOIN_LIMIT_PER_MINUTE = 20;
const PLAYER_KEY_PATTERN = /^[a-f0-9]{16,48}$/;
const ITEM_ID_PATTERN = /^[A-Za-z0-9_:-]{1,64}$/;
const CLAIM_ID_PATTERN = /^-?\d+:-?\d+:-?\d+:PICKUP:[A-Z_]{1,16}:\d{1,6}$/;
const NAME_PATTERN = /[^A-Z0-9 _-]/g;

export function utcDayKey(ms = Date.now()) {
  const value = Number(ms);
  return new Date(Number.isFinite(value) ? value : Date.now()).toISOString().slice(0, 10);
}

/** 1 + floor((players - 1) / 2), clamped to 1..cap (same rule as the 2D build). */
export function publicHunterTargetCount(players = 1) {
  const count = Math.max(1, Math.floor(Number(players) || 1));
  return Math.max(1, Math.min(PUBLIC_HUNTER_CAP, 1 + Math.floor((count - 1) / 2)));
}

const finite = (value, fallback, low, high) => {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.max(low, Math.min(high, number));
};

export function sanitizePosition(value) {
  if (!value || typeof value !== "object") return null;
  const x = Number(value.x);
  const z = Number(value.z);
  if (!Number.isFinite(x) || !Number.isFinite(z) || Math.abs(x) > WORLD_LIMIT || Math.abs(z) > WORLD_LIMIT) return null;
  return { x: Math.round(x * 100) / 100, z: Math.round(z * 100) / 100 };
}

export function sanitizeInventory(value) {
  if (!Array.isArray(value)) return [];
  const result = [];
  for (const entry of value.slice(0, 24)) {
    if (!entry || typeof entry !== "object") continue;
    const id = String(entry.id || "");
    if (!/^[a-z_]{1,24}$/.test(id)) continue;
    result.push({ id, durability: finite(entry.durability, 0, 0, 1000), slot: Math.trunc(finite(entry.slot, 0, 0, 64)) });
  }
  return result;
}

function cleanName(value) {
  const name = String(value || "").toUpperCase().replace(NAME_PATTERN, "").trim().slice(0, 32);
  return name || "RUNNER";
}

export function createPublicRoute({
  now = Date.now,
  storePath = null,
  logger = console,
  initialSeed = "PUBLIC-ROUTE",
  code = PUBLIC_CODE,
  itemRespawnMs = PUBLIC_ITEM_RESPAWN_MS,
  resumeTtlMs = PUBLIC_RESUME_TTL_MS,
  // the owner's live settings (public_settings.mjs `tuning()`); without it the constants above apply
  tuning = null,
} = {}) {
  const limits = () => tuning?.() ?? { joinPerMinute: JOIN_LIMIT_PER_MINUTE, playerTtlMs: PUBLIC_PLAYER_TTL_MS };
  const state = {
    seed: initialSeed,
    epoch: 1,                       // bumps when the route is rebuilt from scratch
    claims: new Map(),              // pickup id -> { kind, claimedAt }
    snapshot: "",                   // opaque, authority-posted, size-limited
    snapshotAt: 0,
    hunterTarget: 1,
    resumes: new Map(),             // playerKey -> { position, inventory, savedAt }
  };
  const players = new Map();        // sessionId -> live player
  const joinRates = new Map();
  let authority = { sessionId: "", lastBeat: 0 };
  let saveTimer = null;
  let loaded = false;

  async function load() {
    if (!storePath || loaded) return;
    loaded = true;
    try {
      const saved = JSON.parse(await readFile(storePath, "utf8"));
      if (typeof saved.seed === "string" && saved.seed) state.seed = saved.seed.slice(0, 64);
      state.epoch = Math.max(1, Math.trunc(Number(saved.epoch) || 1));
      state.hunterTarget = Math.max(1, Math.min(PUBLIC_HUNTER_CAP, Math.trunc(Number(saved.hunterTarget) || 1)));
      if (typeof saved.snapshot === "string" && saved.snapshot.length <= SNAPSHOT_MAX_CHARS) {
        state.snapshot = saved.snapshot;
        state.snapshotAt = Number(saved.snapshotAt) || 0;
      }
      for (const [id, claim] of Array.isArray(saved.claims) ? saved.claims : []) {
        if (CLAIM_ID_PATTERN.test(id) && state.claims.size < MAX_CLAIMS) {
          state.claims.set(id, { kind: String(claim?.kind || "").slice(0, 16), claimedAt: Number(claim?.claimedAt) || now() });
        }
      }
      const timestamp = now();
      for (const [key, resume] of Array.isArray(saved.resumes) ? saved.resumes : []) {
        const position = sanitizePosition(resume?.position);
        if (PLAYER_KEY_PATTERN.test(key) && position && timestamp - Number(resume.savedAt) <= resumeTtlMs) {
          state.resumes.set(key, { position, inventory: sanitizeInventory(resume.inventory), savedAt: Number(resume.savedAt) });
        }
      }
      logger.log?.(`PUBLIC route restored: ${state.claims.size} claims, ${state.resumes.size} resumes`);
    } catch (error) {
      if (error?.code !== "ENOENT") logger.warn?.(`PUBLIC route state could not be loaded: ${error.message}`);
    }
  }

  function serialise() {
    return {
      version: 1,
      seed: state.seed,
      epoch: state.epoch,
      hunterTarget: state.hunterTarget,
      snapshot: state.snapshot,
      snapshotAt: state.snapshotAt,
      claims: [...state.claims],
      resumes: [...state.resumes],
    };
  }

  async function saveNow() {
    if (!storePath) return;
    try {
      await mkdir(dirname(storePath), { recursive: true });
      const temporary = `${storePath}.tmp`;
      await writeFile(temporary, JSON.stringify(serialise()));
      await rename(temporary, storePath);
    } catch (error) {
      logger.error?.(`PUBLIC route state could not be saved: ${error.message}`);
    }
  }

  function scheduleSave() {
    if (!storePath || saveTimer) return;
    saveTimer = setTimeout(() => { saveTimer = null; saveNow(); }, 1500);
    saveTimer.unref?.();
  }

  async function flush() {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null; }
    await saveNow();
  }

  function respawnDue() {
    const timestamp = now();
    const respawned = [];
    for (const [id, claim] of state.claims) {
      if (claim.kind !== "RELAY" && timestamp - claim.claimedAt >= itemRespawnMs) {
        state.claims.delete(id);
        respawned.push(id);
      }
    }
    return respawned;
  }

  function prune() {
    const timestamp = now();
    for (const [sessionId, player] of players) {
      if (timestamp - player.lastSeen > limits().playerTtlMs) {
        saveResume(player);
        players.delete(sessionId);
        if (authority.sessionId === sessionId) authority = { sessionId: "", lastBeat: 0 };
      }
    }
    for (const [key, resume] of state.resumes) if (timestamp - resume.savedAt > resumeTtlMs) state.resumes.delete(key);
    for (const [key, rate] of joinRates) if (timestamp >= rate.resetAt) joinRates.delete(key);
  }

  function saveResume(player) {
    if (!player.position || !PLAYER_KEY_PATTERN.test(player.playerKey)) return;
    state.resumes.set(player.playerKey, {
      position: player.position, inventory: player.inventory, savedAt: player.lastSeen,
    });
    scheduleSave();
  }

  function relaysCollected() {
    let count = 0;
    for (const claim of state.claims.values()) if (claim.kind === "RELAY") count += 1;
    return count;
  }

  function onlineCount() {
    prune();
    return Math.max(0, players.size);
  }

  function publicStatus(extra = {}) {
    const online = onlineCount();
    state.hunterTarget = Math.max(state.hunterTarget,
      publicHunterTargetCount(online));
    return {
      ok: true,
      code,
      seed: state.seed,
      epoch: state.epoch,
      day: utcDayKey(now()),
      playersOnline: online,
      relaysCollected: relaysCollected(),
      hunterTargetCount: state.hunterTarget,
      ...extra,
    };
  }

  function rateLimited(address) {
    const timestamp = now();
    let rate = joinRates.get(address);
    if (!rate || timestamp >= rate.resetAt) rate = { count: 0, resetAt: timestamp + 60_000 };
    rate.count += 1;
    joinRates.set(address, rate);
    return rate.count > limits().joinPerMinute;
  }

  /** hostTokenValid: does the caller hold the ENet host token for PUBLIC right now? */
  function join(body, { address = "", hostTokenValid = false, enetListed = false } = {}) {
    prune();
    if (rateLimited(address || "unknown")) return { status: 429, error: "TOO MANY JOIN REQUESTS" };
    const playerKey = String(body.playerKey || "");
    if (!PLAYER_KEY_PATTERN.test(playerKey)) return { status: 400, error: "INVALID PLAYER KEY" };
    const timestamp = now();
    // One live session per player key: a reload or crash replaces the old one.
    for (const [sessionId, existing] of players) {
      if (existing.playerKey === playerKey) {
        saveResume(existing);
        players.delete(sessionId);
        if (authority.sessionId === sessionId) authority = { sessionId: "", lastBeat: 0 };
      }
    }
    const sessionId = randomBytes(18).toString("base64url");
    const resume = state.resumes.get(playerKey);
    const player = {
      sessionId, playerKey, name: cleanName(body.name), joinedAt: timestamp, lastSeen: timestamp,
      lastStateAt: 0, position: resume?.position || null, inventory: resume?.inventory || [],
    };
    players.set(sessionId, player);
    respawnDue();
    const authorityLive = hostTokenValid || enetListed || (authority.sessionId && timestamp - authority.lastBeat < PUBLIC_AUTHORITY_STALE_MS);
    return {
      status: 200,
      body: publicStatus({
        sessionId,
        joinGraceMs: PUBLIC_JOIN_GRACE_MS,
        resume: resume ? { position: resume.position, inventory: resume.inventory, ageMs: timestamp - resume.savedAt } : null,
        claimed: [...state.claims.keys()],
        snapshot: state.snapshot,
        authorityLive: Boolean(authorityLive),
      }),
    };
  }

  function post(body, { hostTokenValid = false } = {}) {
    prune();
    const player = players.get(String(body.sessionId || ""));
    if (!player) return { status: 404, error: "UNKNOWN SESSION // JOIN AGAIN" };
    const timestamp = now();
    if (timestamp - player.lastStateAt < STATE_MIN_INTERVAL_MS) {
      return { status: 429, error: "STATE UPDATES TOO FAST" };
    }
    player.lastStateAt = timestamp;
    player.lastSeen = timestamp;
    const position = sanitizePosition(body.position);
    if (position) player.position = position;
    if (body.inventory !== undefined) player.inventory = sanitizeInventory(body.inventory);
    if (body.name) player.name = cleanName(body.name);
    let respawned = [];
    if (hostTokenValid) {
      authority = { sessionId: player.sessionId, lastBeat: timestamp };
      if (typeof body.snapshot === "string" && body.snapshot.length > 0 && body.snapshot.length <= SNAPSHOT_MAX_CHARS) {
        state.snapshot = body.snapshot;
        state.snapshotAt = timestamp;
      }
      for (const entry of Array.isArray(body.claims) ? body.claims.slice(0, 200) : []) {
        const id = String(entry?.id || "");
        if (!CLAIM_ID_PATTERN.test(id) || state.claims.size >= MAX_CLAIMS) continue;
        if (!state.claims.has(id)) state.claims.set(id, { kind: id.split(":")[4] || "", claimedAt: timestamp });
      }
      respawned = respawnDue();
    }
    scheduleSave();
    return {
      status: 200,
      body: publicStatus({
        authority: hostTokenValid,
        respawned,
        // everyone learns about claims so a guest's HUD and a future authority agree
        claimCount: state.claims.size,
      }),
    };
  }

  function leave(body) {
    const player = players.get(String(body.sessionId || ""));
    if (!player) return { status: 200, body: { ok: true } };
    saveResume(player);
    players.delete(player.sessionId);
    if (authority.sessionId === player.sessionId) authority = { sessionId: "", lastBeat: 0 };
    return { status: 200, body: { ok: true } };
  }

  /** True when no authority has beaten recently: the ENet entry should be dropped. */
  function authorityStale() {
    return Boolean(authority.sessionId) && now() - authority.lastBeat > PUBLIC_AUTHORITY_STALE_MS;
  }

  function resetAuthority() { authority = { sessionId: "", lastBeat: 0 }; }

  return {
    load, flush, join, post, leave, status: publicStatus, authorityStale, resetAuthority,
    players: () => [...players.values()].map((p) => ({ name: p.name, position: p.position, joinedAt: p.joinedAt })),
    _state: state,
  };
}
