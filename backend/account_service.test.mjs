import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { ADMIN_PERMISSION_NAMES, createAccountService } from "./account_service.mjs";

const PASSWORD = "correct horse battery";

async function openAccounts(options = {}) {
  const root = await mkdtemp(join(tmpdir(), "accounts-"));
  const warnings = [];
  const errors = [];
  let clock = 1_700_000_000_000;
  const storePath = options.storePath === undefined ? join(root, "nested", "accounts.json") : options.storePath;
  const accounts = createAccountService({
    storePath,
    logger: { warn(message) { warnings.push(String(message)); }, error(message) { errors.push(String(message)); } },
    now: () => clock,
    ...options.service,
  });
  return {
    root,
    storePath,
    accounts,
    warnings,
    errors,
    now: () => clock,
    advance(ms) { clock += ms; },
    async close() { await rm(root, { recursive: true, force: true }); },
  };
}

test("admin permission names are the five owner-assignable keys", () => {
  assert.deepEqual(ADMIN_PERMISSION_NAMES, ["inventory", "player", "world", "hunters", "visual"]);
});

test("register, login, session, progress, logout and password change stay on the session that asked", async () => {
  const app = await openAccounts();
  try {
    const [badName] = await app.accounts.register({ username: "no", password: PASSWORD });
    assert.equal(badName, 400);
    const [reserved] = await app.accounts.register({ username: "kyttu", password: PASSWORD });
    assert.equal(reserved, 403);
    assert.equal(app.accounts.accountExists("KYTTU"), false);
    const [weak] = await app.accounts.register({ username: "ada", password: "short" });
    assert.equal(weak, 400);
    const [huge] = await app.accounts.register({ username: "ada", password: "x".repeat(201) });
    assert.equal(huge, 400);
    const [notString] = await app.accounts.register({ username: "ada", password: 123456 });
    assert.equal(notString, 400);

    const [status, created] = await app.accounts.register({ username: " ada ", password: PASSWORD });
    assert.equal(status, 200);
    assert.equal(created.username, "ADA");
    assert.equal(created.admin, false);
    assert.equal(created.adminOwner, false);
    assert.deepEqual(created.adminPermissions, []);
    assert.match(created.sessionToken, /^[a-f0-9]{48}$/);

    const [taken] = await app.accounts.register({ username: "ada", password: PASSWORD });
    assert.equal(taken, 409);
    const [unknown] = await app.accounts.login({ username: "missing", password: PASSWORD });
    assert.equal(unknown, 401);
    const [wrong] = await app.accounts.login({ username: "ada", password: "not-the-password" });
    assert.equal(wrong, 401);

    const [loginStatus, loggedIn] = await app.accounts.login({ username: "Ada", password: PASSWORD });
    assert.equal(loginStatus, 200);
    assert.equal(loggedIn.username, "ADA");
    assert.notEqual(loggedIn.sessionToken, created.sessionToken);
    assert.equal(loggedIn.progress, null);

    const saved = app.accounts.save({ sessionToken: created.sessionToken, progress: { floor: 2, note: "ok" } });
    assert.equal(saved[0], 200);
    assert.equal(saved[1].savedAt, app.now());
    assert.deepEqual(app.accounts.loadProgress({ sessionToken: created.sessionToken })[1].progress, { floor: 2, note: "ok" });
    const loop = {};
    loop.self = loop;
    assert.equal(app.accounts.save({ sessionToken: created.sessionToken, progress: loop })[0], 400);
    assert.equal(app.accounts.save({ sessionToken: created.sessionToken, progress: { n: 1n } })[0], 400);
    const tooBig = "z".repeat(32 * 1024);
    assert.equal(app.accounts.save({ sessionToken: created.sessionToken, progress: tooBig })[0], 413);
    assert.deepEqual(app.accounts.loadProgress({ sessionToken: loggedIn.sessionToken })[1].progress, { floor: 2, note: "ok" });

    const current = app.accounts.currentSession({ sessionToken: created.sessionToken });
    assert.equal(current[0], 200);
    assert.equal(current[1].sessionToken, created.sessionToken);
    assert.equal(current[1].progress.floor, 2);

    assert.equal(app.accounts.logout({ sessionToken: "not-a-session" })[0], 200);
    assert.equal(app.accounts.logout({ sessionToken: loggedIn.sessionToken })[0], 200);
    assert.equal(app.accounts.currentSession({ sessionToken: loggedIn.sessionToken })[0], 401);
    assert.equal(app.accounts.currentSession({ sessionToken: created.sessionToken })[0], 200);

    const [againStatus, again] = await app.accounts.login({ username: "ADA", password: PASSWORD });
    assert.equal(againStatus, 200);
    assert.equal(again.progress.floor, 2);
    const [changed] = await app.accounts.changePassword({ sessionToken: created.sessionToken, password: "new-secret" });
    assert.equal(changed, 200);
    assert.equal(app.accounts.currentSession({ sessionToken: created.sessionToken })[0], 200, "the caller keeps its session");
    assert.equal(app.accounts.currentSession({ sessionToken: again.sessionToken })[0], 401, "other sessions are dropped");
    assert.equal((await app.accounts.login({ username: "ADA", password: PASSWORD }))[0], 401);
    assert.equal((await app.accounts.login({ username: "ADA", password: "new-secret" }))[0], 200);
    assert.equal((await app.accounts.changePassword({ sessionToken: created.sessionToken, password: "short" }))[0], 400);
  } finally {
    await app.close();
  }
});

test("long usernames are clipped to 24, so a different 25th character still collides", async () => {
  const app = await openAccounts();
  try {
    const base = "A".repeat(24);
    const [status, created] = await app.accounts.register({ username: base + "B", password: PASSWORD });
    assert.equal(status, 200);
    assert.equal(created.username, base);
    const [taken, body] = await app.accounts.register({ username: base + "C", password: PASSWORD });
    assert.equal(taken, 409);
    assert.equal(body.code, "USERNAME_TAKEN");
  } finally {
    await app.close();
  }
});

test("rename keeps the session and progress, and blocks reserved or taken names", async () => {
  const app = await openAccounts();
  try {
    const [, owner] = await app.accounts.register({ username: "KYTTU", password: PASSWORD }, { allowDeveloperNames: true });
    const [, player] = await app.accounts.register({ username: "PLAYER", password: PASSWORD });
    app.accounts.save({ sessionToken: player.sessionToken, progress: { floor: 4 } });

    const ownerRename = app.accounts.rename({ sessionToken: owner.sessionToken, username: "OTHER" });
    assert.equal(ownerRename[0], 403);
    assert.equal(ownerRename[1].code, "OWNER_IMMUTABLE");
    assert.equal(app.accounts.sessionUsername(owner.sessionToken), "KYTTU");

    assert.equal(app.accounts.rename({ sessionToken: player.sessionToken, username: "kyttu" })[1].code, "RESERVED_USERNAME");
    assert.equal(app.accounts.rename({ sessionToken: player.sessionToken, username: "no" })[0], 400);
    const [, second] = await app.accounts.register({ username: "OTHER", password: PASSWORD });
    assert.equal(app.accounts.rename({ sessionToken: player.sessionToken, username: "other" })[0], 409);

    const renamed = app.accounts.rename({ sessionToken: player.sessionToken, username: "player" });
    assert.equal(renamed[0], 200);
    assert.equal(renamed[1].username, "PLAYER");
    const next = app.accounts.rename({ sessionToken: player.sessionToken, username: "runner" });
    assert.equal(next[0], 200);
    assert.equal(next[1].username, "RUNNER");
    assert.equal(next[1].sessionToken, player.sessionToken);
    assert.equal(app.accounts.loadProgress({ sessionToken: player.sessionToken })[1].progress.floor, 4);
    assert.equal((await app.accounts.login({ username: "PLAYER", password: PASSWORD }))[0], 401);
    assert.equal((await app.accounts.login({ username: "runner", password: PASSWORD }))[0], 200);
    assert.equal(app.accounts.sessionUsername(second.sessionToken), "OTHER");
    assert.equal(app.accounts.rename({ sessionToken: "missing" })[0], 401);
  } finally {
    await app.close();
  }
});

test("only a built-in owner can grant, list and clear admin permissions", async () => {
  const app = await openAccounts();
  try {
    const [, owner] = await app.accounts.register({ username: "AYTTU", password: PASSWORD }, { allowDeveloperNames: true });
    await app.accounts.register({ username: "KYTTU", password: PASSWORD }, { allowDeveloperNames: true });
    const [, player] = await app.accounts.register({ username: "MOD", password: PASSWORD });
    assert.equal(app.accounts.isAdminOwner("ayttu"), true);
    assert.equal(app.accounts.isAdminOwner("MOD"), false);
    assert.equal(app.accounts.sessionAdminIdentity(player.sessionToken).admin, false);
    assert.equal(app.accounts.listAdminRoster({ sessionToken: player.sessionToken })[0], 403);
    assert.equal(app.accounts.listAdminRoster({})[0], 403);

    const listed = app.accounts.listAdminRoster({ sessionToken: owner.sessionToken });
    assert.equal(listed[0], 200);
    assert.deepEqual(listed[1].roster.map((entry) => entry.username), ["AYTTU", "KYTTU"]);
    assert.equal(listed[1].roster[0].adminOwner, true);
    assert.deepEqual(listed[1].availablePermissions, [...ADMIN_PERMISSION_NAMES]);

    const empty = app.accounts.setAdminRoster({ sessionToken: owner.sessionToken, username: "mod", permissions: ["nope"] });
    assert.equal(empty[0], 400);
    assert.equal(empty[1].code, "NO_PERMISSIONS");
    assert.equal(app.accounts.setAdminRoster({ sessionToken: owner.sessionToken, username: "missing", permissions: ["world"] })[0], 404);
    assert.equal(app.accounts.setAdminRoster({ sessionToken: owner.sessionToken, username: "KYTTU", permissions: ["world"] })[1].code, "OWNER_IMMUTABLE");
    assert.equal(app.accounts.setAdminRoster({ sessionToken: player.sessionToken, username: "MOD", permissions: ["world"] })[0], 403);

    const granted = app.accounts.setAdminRoster({
      sessionToken: owner.sessionToken,
      username: " mod ",
      permissions: [" Visual ", "player", "nope", "player"],
    });
    assert.equal(granted[0], 200);
    assert.deepEqual(granted[1].entry.adminPermissions, ["player", "visual"]);
    assert.equal(granted[1].entry.admin, true);
    assert.equal(granted[1].entry.adminOwner, false);
    const identity = app.accounts.sessionAdminIdentity(player.sessionToken);
    assert.deepEqual(identity.adminPermissions, ["player", "visual"]);
    const roster = app.accounts.listAdminRoster({ sessionToken: owner.sessionToken })[1].roster.map((entry) => entry.username);
    assert.deepEqual(roster, ["AYTTU", "KYTTU", "MOD"]);

    assert.equal(app.accounts.removeAdminRoster({ sessionToken: owner.sessionToken, username: "AYTTU" })[1].code, "OWNER_IMMUTABLE");
    assert.equal(app.accounts.removeAdminRoster({ sessionToken: owner.sessionToken, username: "nobody" })[0], 404);
    assert.equal(app.accounts.removeAdminRoster({ sessionToken: player.sessionToken, username: "MOD" })[0], 403);
    const removed = app.accounts.removeAdminRoster({ sessionToken: owner.sessionToken, username: "mod" });
    assert.equal(removed[0], 200);
    assert.equal(removed[1].removed, true);
    assert.deepEqual(app.accounts.sessionAdminIdentity(player.sessionToken).adminPermissions, []);
  } finally {
    await app.close();
  }
});

test("the store reloads accounts and live sessions, skips junk, and warns on a corrupt file", async () => {
  const app = await openAccounts();
  try {
    const [, created] = await app.accounts.register({ username: "ADA", password: PASSWORD });
    app.accounts.save({ sessionToken: created.sessionToken, progress: { floor: 1 } });
    const raw = JSON.parse(readFileSync(app.storePath, "utf8"));
    assert.equal(raw.version, 2);
    assert.equal(raw.accounts[0].username, "ADA");
    assert.match(raw.accounts[0].hash, /^[a-f0-9]+$/);
    assert.match(raw.sessions[0].tokenHash, /^[a-f0-9]{64}$/);
    assert.equal(JSON.stringify(raw).includes(PASSWORD), false);
    assert.equal(JSON.stringify(raw).includes(created.sessionToken), false);

    const reopened = createAccountService({ storePath: app.storePath, now: app.now, logger: { warn() {}, error() {} } });
    assert.equal(reopened.currentSession({ sessionToken: created.sessionToken })[0], 200);
    assert.equal(reopened.loadProgress({ sessionToken: created.sessionToken })[1].progress.floor, 1);

    app.advance(31 * 24 * 60 * 60 * 1000);
    assert.equal(app.accounts.currentSession({ sessionToken: created.sessionToken })[0], 401);
    const expired = createAccountService({ storePath: app.storePath, now: app.now, logger: { warn() {}, error() {} } });
    assert.equal(expired.currentSession({ sessionToken: created.sessionToken })[0], 401);
    assert.equal(expired.accountExists("ada"), true);

    writeFileSync(app.storePath, "{");
    const broken = createAccountService({
      storePath: app.storePath,
      logger: { warn(message) { app.warnings.push(String(message)); }, error() {} },
    });
    assert.equal(broken.accountExists("ADA"), false);
    assert.match(app.warnings[0], /not loaded/);

    const missingPath = join(app.root, "missing", "accounts.json");
    const missingWarnings = [];
    createAccountService({
      storePath: missingPath,
      logger: { warn(message) { missingWarnings.push(String(message)); } },
    });
    assert.deepEqual(missingWarnings, []);
    assert.equal(existsSync(missingPath), false);
  } finally {
    await app.close();
  }
});
