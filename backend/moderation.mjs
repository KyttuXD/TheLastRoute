// Server-side moderation: the escalating consequences behind the host-side anti-cheat.
//
// The host client detects and kicks cheaters; it reports each kick here (authenticated by its ENet host token).
// KT: "accounts shouldnt be auto banned, only manually". Those reports and build-integrity reports only open REVIEW
// entries; owners read them in the admin MODERATION list and decide. Only an owner's action (flag/extend, ban, troll)
// restricts an account, and only the shared spaces: joining the PUBLIC route, hosting a public room, uploading a skin.
// Nothing here applies a restriction or a device/IP cooldown by itself. Solo play and private rooms are never restricted. Raw IP addresses and device ids are never stored, only sha256(salt + value).

import { createHash, randomBytes } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;
export const KICK_WINDOW_MS = 7 * DAY_MS;
export const KICK_FLAG_THRESHOLD = 2;
const AUTOMATIC_COOLDOWNS = false;
export const MAX_EXTEND_HOURS = 24 * 90;
export const MAX_PEER_ENTRIES = 16;
const MAX_LOG_ENTRIES = 100;
const MAX_HASH_ENTRIES = 50_000;
const USERNAME_PATTERN = /^[A-Z0-9_]{3,24}$/;
const BUILD_HASH_PATTERN = /^[a-f0-9]{64}$/;
const COMMIT_PATTERN = /^[a-f0-9]{7,40}$/;
const CHANNELS = new Set(["debug", "release"]);

function usernameOf(value) {
  return String(value || "").trim().toUpperCase().slice(0, 24);
}

function text(value, limit) {
  return String(value ?? "").replace(/[\u0000-\u001f\u007f]/g, " ").trim().slice(0, limit);
}

function formatUtc(ms) {
  return `${new Date(ms).toISOString().slice(0, 16).replace("T", " ")} UTC`;
}

export function createModeration({
  storePath = null,
  salt = "",
  now = Date.now,
  logger = console,
  isOwner = () => false,
  accountExists = () => true,
  updatesProvider = () => ({}),
} = {}) {
  const accounts = new Map();   // USERNAME -> record
  const cooldowns = new Map();  // salted hash (device or IP) -> { kicks:[at], until }
  const deviceAddress = new Map(); // device hash -> { ip: ip hash, at } so a kick also cools the guest's last IP
  const builds = new Map();     // buildHash -> { buildHash, commit, note, by, at }
  let secret = String(salt || "");

  if (storePath) {
    try {
      const saved = JSON.parse(readFileSync(storePath, "utf8"));
      if (!secret && typeof saved.secret === "string" && saved.secret.length >= 32) secret = saved.secret;
      for (const value of Array.isArray(saved.accounts) ? saved.accounts : []) {
        const username = usernameOf(value?.username);
        if (USERNAME_PATTERN.test(username)) accounts.set(username, normalizeRecord({ ...value, username }));
      }
      for (const [hash, value] of Array.isArray(saved.cooldowns) ? saved.cooldowns : []) {
        if (BUILD_HASH_PATTERN.test(hash)) {
          cooldowns.set(hash, { kicks: (value?.kicks || []).map(Number).filter(Number.isFinite), until: Number(value?.until) || 0 });
        }
      }
      for (const [hash, value] of Array.isArray(saved.deviceAddress) ? saved.deviceAddress : []) {
        if (BUILD_HASH_PATTERN.test(hash) && BUILD_HASH_PATTERN.test(value?.ip)) deviceAddress.set(hash, { ip: value.ip, at: Number(value.at) || 0 });
      }
      for (const value of Array.isArray(saved.builds) ? saved.builds : []) {
        if (BUILD_HASH_PATTERN.test(value?.buildHash)) builds.set(value.buildHash, value);
      }
    } catch (error) {
      if (error?.code !== "ENOENT") logger.warn?.(`Moderation store was not loaded: ${error?.message || error}`);
    }
  }
  if (!secret) secret = randomBytes(32).toString("hex");   // persisted below so hashes stay stable across restarts

  function normalizeRecord(value = {}) {
    return {
      username: usernameOf(value.username),
      flags: Array.isArray(value.flags) ? value.flags.slice(-MAX_LOG_ENTRIES) : [],
      kicks: Array.isArray(value.kicks) ? value.kicks.slice(-MAX_LOG_ENTRIES) : [],
      flaggedUntil: Number(value.flaggedUntil) || 0,
      permanent: Boolean(value.permanent),
      troll: Boolean(value.troll),
      ads: value.ads === undefined ? true : Boolean(value.ads),   // troll-mode "ads" (in-universe jokes), owner toggle
      integrity: value.integrity && typeof value.integrity === "object" ? value.integrity : null,
      flaggedHashes: Array.isArray(value.flaggedHashes) ? value.flaggedHashes.slice(-MAX_LOG_ENTRIES) : [],
      reviews: Array.isArray(value.reviews) ? value.reviews.slice(-MAX_LOG_ENTRIES) : [],
    };
  }

  function persist() {
    if (!storePath) return;
    try {
      mkdirSync(dirname(storePath), { recursive: true });
      const temporaryPath = `${storePath}.tmp`;
      writeFileSync(temporaryPath, JSON.stringify({
        version: 1,
        secret: salt ? undefined : secret,
        accounts: [...accounts.values()],
        cooldowns: [...cooldowns],
        deviceAddress: [...deviceAddress],
        builds: [...builds.values()],
      }));
      renameSync(temporaryPath, storePath);
    } catch (error) {
      logger.error?.(`Moderation store write failed: ${error?.message || error}`);
    }
  }
  if (storePath) persist();

  function hashOf(value) {
    const raw = String(value ?? "").trim().slice(0, 256);
    return raw ? createHash("sha256").update(secret).update(raw).digest("hex") : "";
  }

  function prune() {
    const timestamp = now();
    for (const [hash, entry] of cooldowns) {
      entry.kicks = entry.kicks.filter((at) => timestamp - at < KICK_WINDOW_MS);
      if (!entry.kicks.length && entry.until <= timestamp) cooldowns.delete(hash);
    }
    for (const [hash, entry] of deviceAddress) if (timestamp - entry.at > KICK_WINDOW_MS) deviceAddress.delete(hash);
    while (deviceAddress.size > MAX_HASH_ENTRIES) deviceAddress.delete(deviceAddress.keys().next().value);
    while (cooldowns.size > MAX_HASH_ENTRIES) cooldowns.delete(cooldowns.keys().next().value);
  }

  function record(username, create = false) {
    const name = usernameOf(username);
    if (!USERNAME_PATTERN.test(name)) return null;
    let entry = accounts.get(name);
    if (!entry && create) {
      entry = normalizeRecord({ username: name });
      accounts.set(name, entry);
    }
    return entry || null;
  }

  function active(entry, timestamp = now()) {
    return Boolean(entry && (entry.permanent || entry.flaggedUntil > timestamp));
  }

  function lastReason(entry) {
    return entry?.flags?.length ? entry.flags[entry.flags.length - 1].reason : "";
  }

  // Opens a review entry for owners. Never restricts anything by itself; owner accounts are never queued.
  function addReview(entry, reason, detail) {
    if (!entry || isOwner(entry.username)) return false;
    entry.reviews.push({ at: now(), reason, detail: text(detail, 200), open: true });
    entry.reviews = entry.reviews.slice(-MAX_LOG_ENTRIES);
    logger.log?.(`moderation: review opened for ${entry.username} (${reason})`);
    return true;
  }

  function openReviews(entry) {
    return entry.reviews.filter((review) => review.open);
  }

  function linkDevice(deviceId, address) {
    const device = hashOf(deviceId);
    const ip = hashOf(address);
    if (!device || !ip) return;
    const previous = deviceAddress.get(device);
    deviceAddress.delete(device);
    deviceAddress.set(device, { ip, at: now() });
    if (previous?.ip !== ip) persist();
  }

  // Automatic device/IP cooldowns are off (manual moderation only); kept as a hook so refusals stay uniform.
  function cooldownUntil(deviceId, address) {
    if (!AUTOMATIC_COOLDOWNS) return 0;
    const timestamp = now();
    let until = 0;
    const device = hashOf(deviceId);
    for (const hash of [device, hashOf(address), device ? deviceAddress.get(device)?.ip : ""]) {
      const value = hash ? cooldowns.get(hash)?.until || 0 : 0;
      if (value > timestamp) until = Math.max(until, value);
    }
    return until;
  }

  function status({ username = "", deviceId = "", address = "" } = {}) {
    const timestamp = now();
    const entry = record(username);
    const flagged = active(entry, timestamp);
    const permanent = Boolean(entry?.permanent);
    const until = flagged && !permanent ? entry.flaggedUntil : null;
    const cooldown = cooldownUntil(deviceId, address);
    let message = "";
    if (permanent) {
      message = "Your account is banned from shared play: you cannot join the Public Route, host public rooms, or upload skins. Solo play and private rooms still work.";
    } else if (flagged) {
      message = `Your account is flagged until ${formatUtc(until)}: until then you cannot join the Public Route, host public rooms, or upload skins. Solo play and private rooms still work.`;
    } else if (cooldown) {
      message = `You were recently kicked from a game: you cannot join the Public Route until ${formatUtc(cooldown)}. Solo play and private rooms still work.`;
    }
    return {
      flagged,
      permanent,
      until,
      reason: flagged ? lastReason(entry) : "",
      troll: Boolean(entry?.troll),
      restrictions: { publicRoute: flagged || Boolean(cooldown), hostPublic: flagged, skinUpload: flagged },
      cooldownUntil: cooldown || null,
      message,
    };
  }

  function refusal(current, restriction) {
    if (!current.restrictions[restriction]) return null;
    return [403, {
      ok: false,
      code: "FLAGGED",
      error: current.message,
      until: current.permanent ? null : (current.flagged ? current.until : current.cooldownUntil),
      permanent: current.permanent,
      cooldown: !current.flagged,
    }];
  }

  function checkPublicJoin(context) {
    if (context.deviceId && context.address) linkDevice(context.deviceId, context.address);
    return refusal(status(context), "publicRoute");
  }

  function checkHostPublic(context) {
    return refusal(status(context), "hostPublic");
  }

  function checkSkinUpload(username) {
    return refusal(status({ username }), "skinUpload");
  }

  function knownBuild({ buildHash, platform, channel, username }) {
    if (channel === "debug" && username && isOwner(username)) return true;
    if (builds.has(buildHash)) return true;
    let updates = {};
    try {
      updates = updatesProvider() || {};
    } catch {
      updates = {};
    }
    const entry = updates?.[channel]?.[platform];
    return Boolean(entry && String(entry.sha256 || "").trim().toLowerCase() === buildHash);
  }

  function integrity(body = {}, { username = "", address = "" } = {}) {
    const buildHash = String(body.buildHash || "").trim().toLowerCase();
    if (!BUILD_HASH_PATTERN.test(buildHash)) {
      return [400, { ok: false, code: "INVALID_BUILD_HASH", error: "buildHash must be a lowercase hex sha256" }];
    }
    const channel = String(body.channel || "").trim().toLowerCase();
    if (!CHANNELS.has(channel)) return [400, { ok: false, code: "INVALID_CHANNEL", error: "channel must be debug or release" }];
    const platform = text(body.platform, 32).toLowerCase();
    const commit = String(body.commit || "").trim().toLowerCase();
    if (body.deviceId && address) linkDevice(body.deviceId, address);
    const known = knownBuild({ buildHash, platform, channel, username });
    if (username) {
      const entry = record(username, true);
      entry.integrity = { commit: COMMIT_PATTERN.test(commit) ? commit : "", hash: buildHash, known, at: now() };
      if (!known && !entry.flaggedHashes.includes(buildHash)) {
        if (addReview(entry, "integrity", `unknown ${channel}/${platform} build ${buildHash.slice(0, 12)} (${text(body.context, 8) || "signin"})`)) {
          entry.flaggedHashes.push(buildHash);
        }
      }
      persist();
    }
    return [200, { ok: true, known, status: status({ username, deviceId: body.deviceId, address }) }];
  }

  // `host` identifies the reporting host (its account, or the registered room instance); only kicks from distinct
  // hosts escalate to an account flag, so one malicious host cannot flag anyone alone. Every report is still recorded.
  function reportKick(body = {}, { room = "", host = "" } = {}) {
    if (String(body.kind || "kick") !== "kick") return [400, { ok: false, code: "INVALID_KIND", error: "kind must be kick" }];
    prune();
    const timestamp = now();
    const reason = text(body.reason, 80) || "kick";
    let flagged = false;
    const username = usernameOf(body.username);
    if (USERNAME_PATTERN.test(username) && accountExists(username)) {
      const entry = record(username, true);
      entry.kicks.push({ at: timestamp, room, host: hashOf(`host:${host || room}`).slice(0, 16), reason, strikes: Math.max(0, Math.min(99, Math.trunc(Number(body.strikes) || 0))) });
      entry.kicks = entry.kicks.slice(-MAX_LOG_ENTRIES);
      const recent = new Set(entry.kicks.filter((kick) => timestamp - kick.at < KICK_WINDOW_MS)
        .map((kick) => kick.host || `${kick.room}:${kick.at}`)).size;
      // every kick is reviewable; kicks by 2+ different hosts in 7 days are marked as the stronger signal
      addReview(entry, recent >= KICK_FLAG_THRESHOLD ? "kicks" : "kick",
        recent >= KICK_FLAG_THRESHOLD ? `kicked by ${recent} different hosts in 7 days (last: ${reason})` : `kicked: ${reason}`);
    }
    persist();
    return [200, { ok: true, flagged, status: status({ username, deviceId: body.deviceId }) }];
  }

  function peers(body = {}) {
    const players = {};
    for (const value of (Array.isArray(body.usernames) ? body.usernames : []).slice(0, MAX_PEER_ENTRIES)) {
      const username = usernameOf(value);
      if (!USERNAME_PATTERN.test(username)) continue;
      const current = status({ username });
      // flagged accounts are trolled too, so the host turns troll mode on for both
      players[username] = { flagged: current.flagged, permanent: current.permanent, troll: current.troll || current.flagged,
        ads: record(username)?.ads ?? true, reason: current.reason };
    }
    const devices = {};
    for (const value of (Array.isArray(body.deviceIds) ? body.deviceIds : []).slice(0, MAX_PEER_ENTRIES)) {
      const key = String(value ?? "").slice(0, 256);
      if (!key) continue;
      const kicks = cooldowns.get(hashOf(key))?.kicks || [];
      devices[key] = { cooldown: Boolean(cooldownUntil(key, "")), troll: false };   // server troll is owner-set only
    }
    return [200, { ok: true, players, devices }];
  }

  function summary(entry) {
    return {
      username: entry.username,
      flagged: active(entry),
      permanent: entry.permanent,
      flaggedUntil: entry.flaggedUntil || null,
      troll: entry.troll,
      ads: entry.ads,
      reason: lastReason(entry),
      flags: entry.flags,
      kicks: entry.kicks,
      integrity: entry.integrity,
      reviews: entry.reviews,
      reviewOpen: openReviews(entry).length,
    };
  }

  function latestAt(entry) {
    return Math.max(0, ...entry.flags.map((flag) => flag.at), ...entry.kicks.map((kick) => kick.at),
      ...entry.reviews.map((review) => review.at), entry.integrity?.at || 0);
  }

  function adminList() {
    const list = [...accounts.values()]
      .filter((entry) => active(entry) || entry.kicks.length || entry.troll || entry.flags.length || entry.reviews.length)
      .sort((a, b) => (openReviews(b).length > 0) - (openReviews(a).length > 0) || latestAt(b) - latestAt(a))
      .map(summary);
    const reviews = list.filter((entry) => entry.reviewOpen > 0).map((entry) => entry.username);
    return [200, { ok: true, accounts: list, reviews, builds: [...builds.values()] }];
  }

  function adminSet(body = {}, by) {
    const username = usernameOf(body.username);
    if (!USERNAME_PATTERN.test(username) || !accountExists(username)) {
      return [404, { ok: false, code: "ACCOUNT_NOT_FOUND", error: "That account was not found" }];
    }
    const action = String(body.action || "");
    const entry = record(username, true);
    const timestamp = now();
    if (action === "clear") {
      entry.flaggedUntil = 0;
    } else if (action === "extend") {
      const hours = body.hours === undefined ? 24 : Number(body.hours);
      if (!Number.isFinite(hours) || hours <= 0 || hours > MAX_EXTEND_HOURS) {
        return [400, { ok: false, code: "INVALID_HOURS", error: `hours must be between 1 and ${MAX_EXTEND_HOURS}` }];
      }
      entry.flaggedUntil = Math.max(timestamp, entry.flaggedUntil) + Math.round(hours * HOUR_MS);
    } else if (action === "ban") entry.permanent = true;
    else if (action === "unban") entry.permanent = false;
    else if (action === "troll_on") entry.troll = true;
    else if (action === "troll_off") entry.troll = false;
    else if (action === "ads_on") entry.ads = true;
    else if (action === "ads_off") entry.ads = false;
    else if (action !== "dismiss") return [400, { ok: false, code: "INVALID_ACTION", error: "Unknown moderation action" }];
    entry.flags.push({ at: timestamp, reason: `admin:${action}`, detail: text(body.note, 200),
      until: entry.permanent ? null : (entry.flaggedUntil || null), by });
    entry.flags = entry.flags.slice(-MAX_LOG_ENTRIES);
    for (const review of entry.reviews) if (review.open) Object.assign(review, { open: false, closedBy: by, closedAt: timestamp, action });
    persist();
    return [200, { ok: true, record: summary(entry) }];
  }

  function addBuild(body = {}, by) {
    const buildHash = String(body.buildHash || "").trim().toLowerCase();
    if (!BUILD_HASH_PATTERN.test(buildHash)) {
      return [400, { ok: false, code: "INVALID_BUILD_HASH", error: "buildHash must be a lowercase hex sha256" }];
    }
    const commit = String(body.commit || "").trim().toLowerCase();
    const entry = { buildHash, commit: COMMIT_PATTERN.test(commit) ? commit : "", note: text(body.note, 200), by, at: now() };
    builds.set(buildHash, entry);
    persist();
    return [200, { ok: true, build: entry, builds: [...builds.values()] }];
  }

  function removeBuild(body = {}) {
    const buildHash = String(body.buildHash || "").trim().toLowerCase();
    const removed = builds.delete(buildHash);
    if (removed) persist();
    return [200, { ok: true, removed, builds: [...builds.values()] }];
  }

  // Keep a record attached to its account when the player renames, so a rename cannot shed a flag.
  function renameAccount(from, to) {
    const source = usernameOf(from);
    const target = usernameOf(to);
    const entry = accounts.get(source);
    if (!entry || source === target) return;
    accounts.delete(source);
    entry.username = target;
    accounts.set(target, entry);
    persist();
  }

  return { status, integrity, reportKick, peers, checkPublicJoin, checkHostPublic, checkSkinUpload,
    adminList, adminSet, addBuild, removeBuild, renameAccount, prune, hashOf };
}
