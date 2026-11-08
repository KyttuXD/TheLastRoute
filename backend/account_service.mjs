import { createHash, randomBytes, scrypt, timingSafeEqual } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const USERNAME_PATTERN = /^[A-Z0-9_]{3,24}$/;
const ADMIN_OWNER_USERNAMES = new Set(["KYTTU", "AYTTU"]);
export const ADMIN_PERMISSION_NAMES = Object.freeze([
  "inventory", "player", "world", "hunters", "visual",
]);
const MIN_PASSWORD_LENGTH = 6;
const MAX_PASSWORD_LENGTH = 200;
const MAX_PROGRESS_BYTES = 32 * 1024;
const SCRYPT_KEY_LENGTH = 64;
const DEFAULT_SESSION_TTL_MS = 30 * 24 * 60 * 60 * 1000;

function derive(password, salt) {
  return new Promise((resolve, reject) => {
    scrypt(String(password), salt, SCRYPT_KEY_LENGTH, (error, result) => {
      if (error) reject(error);
      else resolve(result);
    });
  });
}

async function passwordRecord(password) {
  const salt = randomBytes(16).toString("hex");
  return { salt, hash: (await derive(password, salt)).toString("hex") };
}

async function passwordMatches(password, salt, hash) {
  const expected = Buffer.from(String(hash || ""), "hex");
  const actual = await derive(password, String(salt || ""));
  return expected.length === actual.length && timingSafeEqual(expected, actual);
}

function usernameOf(value) {
  return String(value || "").trim().toUpperCase().slice(0, 24);
}

function isAdminOwnerUsername(value) {
  return ADMIN_OWNER_USERNAMES.has(usernameOf(value));
}

function cleanAdminPermissions(value) {
  const supplied = Array.isArray(value) ? value : [];
  return [...new Set(supplied.map((entry) => String(entry || "").trim().toLowerCase())
    .filter((entry) => ADMIN_PERMISSION_NAMES.includes(entry)))].sort();
}

function sessionHash(token) {
  return createHash("sha256").update(String(token || "")).digest("hex");
}

export function createAccountService({ storePath = null, logger = console, now = Date.now,
  sessionTtlMs = DEFAULT_SESSION_TTL_MS } = {}) {
  const accounts = new Map();
  const sessions = new Map();

  if (storePath) {
    try {
      const parsed = JSON.parse(readFileSync(storePath, "utf8"));
      for (const value of Array.isArray(parsed?.accounts) ? parsed.accounts : []) {
        const username = usernameOf(value?.username);
        if (USERNAME_PATTERN.test(username) && value?.salt && value?.hash) {
          accounts.set(username, { ...value, username });
        }
      }
      for (const value of Array.isArray(parsed?.sessions) ? parsed.sessions : []) {
        const tokenHash = String(value?.tokenHash || "");
        const username = usernameOf(value?.username);
        const expiresAt = Number(value?.expiresAt) || 0;
        if (/^[a-f0-9]{64}$/.test(tokenHash) && accounts.has(username)
          && expiresAt > now()) {
          sessions.set(tokenHash, { username, expiresAt });
        }
      }
    } catch (error) {
      if (error?.code !== "ENOENT") logger.warn?.(`Account store was not loaded: ${error?.message || error}`);
    }
  }

  function persist() {
    if (!storePath) return;
    try {
      mkdirSync(dirname(storePath), { recursive: true });
      const temporaryPath = `${storePath}.tmp`;
      writeFileSync(temporaryPath, JSON.stringify({
        version: 2,
        accounts: [...accounts.values()],
        sessions: [...sessions].map(([tokenHash, session]) => ({ tokenHash, ...session })),
      }));
      renameSync(temporaryPath, storePath);
    } catch (error) {
      logger.error?.(`Account store write failed: ${error?.message || error}`);
    }
  }

  function issueSession(username) {
    const token = randomBytes(24).toString("hex");
    sessions.set(sessionHash(token), { username, expiresAt: now() + sessionTtlMs });
    persist();
    return token;
  }

  function validPassword(password) {
    return typeof password === "string" && password.length >= MIN_PASSWORD_LENGTH && password.length <= MAX_PASSWORD_LENGTH;
  }

  function adminIdentity(usernameValue) {
    const username = usernameOf(usernameValue);
    const owner = isAdminOwnerUsername(username);
    const permissions = owner
      ? cleanAdminPermissions(ADMIN_PERMISSION_NAMES)
      : cleanAdminPermissions(accounts.get(username)?.adminPermissions);
    return {
      admin: owner || permissions.length > 0,
      adminOwner: owner,
      adminPermissions: permissions,
    };
  }

  function accountResult(username, extra = {}) {
    return { ok: true, username, ...adminIdentity(username), ...extra };
  }

  async function register(body = {}, { allowDeveloperNames = false } = {}) {
    const username = usernameOf(body.username);
    if (!USERNAME_PATTERN.test(username)) {
      return [400, { ok: false, code: "INVALID_USERNAME", error: "Username must be 3-24 characters: letters, numbers, underscore" }];
    }
    if (isAdminOwnerUsername(username) && !allowDeveloperNames) {
      return [403, { ok: false, code: "RESERVED_USERNAME", error: "That username is reserved" }];
    }
    if (!validPassword(body.password)) {
      return [400, { ok: false, code: "WEAK_PASSWORD", error: "Password must be 6-200 characters" }];
    }
    if (accounts.has(username)) {
      return [409, { ok: false, code: "USERNAME_TAKEN", error: "That username is already taken" }];
    }
    const credentials = await passwordRecord(body.password);
    accounts.set(username, { username, ...credentials, progress: null, savedAt: 0,
      adminPermissions: [] });
    persist();
    return [200, accountResult(username, { sessionToken: issueSession(username) })];
  }

  async function login(body = {}) {
    const username = usernameOf(body.username);
    const account = accounts.get(username);
    const valid = account
      ? await passwordMatches(body.password, account.salt, account.hash)
      : (await derive(body.password, "last-route-decoy-salt"), false);
    if (!valid) return [401, { ok: false, code: "INVALID_CREDENTIALS", error: "Incorrect username or password" }];
    return [200, accountResult(username, {
      sessionToken: issueSession(username),
      progress: account.progress,
      savedAt: account.savedAt,
    })];
  }

  function sessionUsername(token) {
    const key = sessionHash(token);
    const session = sessions.get(key);
    if (!session) return null;
    if (session.expiresAt <= now()) {
      sessions.delete(key);
      persist();
      return null;
    }
    return session.username;
  }

  function currentSession(body = {}) {
    const username = sessionUsername(body.sessionToken);
    if (!username) return [401, { ok: false, code: "NO_SESSION", error: "Session expired; sign in again" }];
    const account = accounts.get(username);
    return [200, accountResult(username, {
      sessionToken: String(body.sessionToken),
      progress: account.progress,
      savedAt: account.savedAt,
    })];
  }

  function save(body = {}) {
    const username = sessionUsername(body.sessionToken);
    if (!username) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to save progress" }];
    let serialized;
    try {
      serialized = JSON.stringify(body.progress ?? null);
    } catch {
      return [400, { ok: false, code: "INVALID_PROGRESS", error: "Progress must be serializable" }];
    }
    if (Buffer.byteLength(serialized) > MAX_PROGRESS_BYTES) {
      return [413, { ok: false, code: "PROGRESS_TOO_LARGE", error: "Save data is too large" }];
    }
    const account = accounts.get(username);
    account.progress = body.progress ?? null;
    account.savedAt = now();
    persist();
    return [200, { ok: true, progress: account.progress, savedAt: account.savedAt }];
  }

  function loadProgress(body = {}) {
    const username = sessionUsername(body.sessionToken);
    if (!username) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to load progress" }];
    const account = accounts.get(username);
    return [200, { ok: true, progress: account.progress, savedAt: account.savedAt }];
  }

  function logout(body = {}) {
    if (sessions.delete(sessionHash(body.sessionToken))) persist();
    return [200, { ok: true }];
  }

  async function changePassword(body = {}) {
    const currentSessionHash = sessionHash(body.sessionToken);
    const username = sessionUsername(body.sessionToken);
    if (!username) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to change your password" }];
    if (!validPassword(body.password)) {
      return [400, { ok: false, code: "WEAK_PASSWORD", error: "Password must be 6-200 characters" }];
    }
    Object.assign(accounts.get(username), await passwordRecord(body.password));
    // Password replacement is also account recovery: any other device holding
    // a bearer token must lose access. Keep the session which authorized this
    // request so the player is not signed out of the client doing the change.
    for (const [tokenHash, session] of sessions) {
      if (session.username === username && tokenHash !== currentSessionHash) {
        sessions.delete(tokenHash);
      }
    }
    persist();
    return [200, { ok: true, username }];
  }

  function rename(body = {}, { allowDeveloperNames = false } = {}) {
    const currentUsername = sessionUsername(body.sessionToken);
    if (!currentUsername) return [401, { ok: false, code: "NO_SESSION", error: "Sign in to rename your account" }];
    const username = usernameOf(body.username);
    if (!USERNAME_PATTERN.test(username)) {
      return [400, { ok: false, code: "INVALID_USERNAME", error: "Username must be 3-24 characters: letters, numbers, underscore" }];
    }
    if (isAdminOwnerUsername(currentUsername) && username !== currentUsername) {
      return [403, { ok: false, code: "OWNER_IMMUTABLE",
        error: "Full admin account usernames cannot be changed" }];
    }
    if (isAdminOwnerUsername(username) && username !== currentUsername && !allowDeveloperNames) {
      return [403, { ok: false, code: "RESERVED_USERNAME", error: "That username is reserved" }];
    }
    if (username !== currentUsername && accounts.has(username)) {
      return [409, { ok: false, code: "USERNAME_TAKEN", error: "That username is already taken" }];
    }
    const account = accounts.get(currentUsername);
    if (username !== currentUsername) {
      accounts.delete(currentUsername);
      account.username = username;
      accounts.set(username, account);
      for (const session of sessions.values()) {
        if (session.username === currentUsername) session.username = username;
      }
      persist();
    }
    return [200, accountResult(username, { sessionToken: String(body.sessionToken) })];
  }

  function sessionAdminIdentity(token) {
    const username = sessionUsername(token);
    return username ? { username, ...adminIdentity(username) } : null;
  }

  function listAdminRoster(body = {}) {
    const requester = sessionUsername(body.sessionToken);
    if (!isAdminOwnerUsername(requester)) {
      return [403, { ok: false, code: "ADMIN_OWNER_REQUIRED",
        error: "Full admin account required" }];
    }
    const roster = [...ADMIN_OWNER_USERNAMES].sort().map((username) => ({
      username, ...adminIdentity(username),
    }));
    for (const [username, account] of accounts) {
      if (isAdminOwnerUsername(username)) continue;
      const permissions = cleanAdminPermissions(account.adminPermissions);
      if (permissions.length > 0) roster.push({ username, ...adminIdentity(username) });
    }
    roster.sort((a, b) => a.username.localeCompare(b.username));
    return [200, { ok: true, roster, availablePermissions: [...ADMIN_PERMISSION_NAMES] }];
  }

  function setAdminRoster(body = {}) {
    const requester = sessionUsername(body.sessionToken);
    if (!isAdminOwnerUsername(requester)) {
      return [403, { ok: false, code: "ADMIN_OWNER_REQUIRED",
        error: "Full admin account required" }];
    }
    const username = usernameOf(body.username);
    if (!USERNAME_PATTERN.test(username) || !accounts.has(username)) {
      return [404, { ok: false, code: "ACCOUNT_NOT_FOUND",
        error: "Admin target account was not found" }];
    }
    if (isAdminOwnerUsername(username)) {
      return [403, { ok: false, code: "OWNER_IMMUTABLE",
        error: "Built-in owner permissions cannot be changed" }];
    }
    const permissions = cleanAdminPermissions(body.permissions);
    if (permissions.length === 0) {
      return [400, { ok: false, code: "NO_PERMISSIONS",
        error: "Choose at least one valid admin permission" }];
    }
    accounts.get(username).adminPermissions = permissions;
    persist();
    return [200, { ok: true, entry: { username, ...adminIdentity(username) } }];
  }

  function removeAdminRoster(body = {}) {
    const requester = sessionUsername(body.sessionToken);
    if (!isAdminOwnerUsername(requester)) {
      return [403, { ok: false, code: "ADMIN_OWNER_REQUIRED",
        error: "Full admin account required" }];
    }
    const username = usernameOf(body.username);
    if (isAdminOwnerUsername(username)) {
      return [403, { ok: false, code: "OWNER_IMMUTABLE",
        error: "Built-in owners cannot be removed" }];
    }
    const account = accounts.get(username);
    if (!account) {
      return [404, { ok: false, code: "ACCOUNT_NOT_FOUND",
        error: "Admin target account was not found" }];
    }
    account.adminPermissions = [];
    persist();
    return [200, { ok: true, username, removed: true }];
  }

  return { register, login, currentSession, save, loadProgress, logout,
    changePassword, rename, sessionUsername, sessionAdminIdentity,
    listAdminRoster, setAdminRoster, removeAdminRoster,
    isAdminOwner: (name) => isAdminOwnerUsername(name),
    accountExists: (name) => accounts.has(usernameOf(name)) };
}
