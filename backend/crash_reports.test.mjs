import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { CRASH_LIMITS, createCrashReports, sanitizeCrashReport, scrubSecrets } from "./crash_reports.mjs";

const quiet = { log() {}, warn() {}, error() {} };

function report(extra = {}) {
  return {
    schema: "lr.crash/1", kind: "crash", build: "abc1234", os: "macOS", os_version: "15.1", device: "Mac15,3",
    gpu: "Apple M3", scene: "res://game/infinite_route/infinite_route_test.tscn", room_role: "guest", device_id: "dev-1",
    session_started: 1_800_000_000, session_ended: 1_800_000_100,
    counts: { errors: 3, warnings: 1, script_errors: 2 },
    breadcrumbs: ["scene infinite_route_test", "interact stream_door d1"], recent_errors: ["E 12 x"],
    log_tail: "line 1\nline 2\n", ...extra,
  };
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "crash-reports-"));
  const server = createLastRouteServer({
    accountStorePath: join(root, "accounts.json"),
    moderationStorePath: join(root, "moderation.json"),
    crashReportDirectory: join(root, "crash-reports"),
    adminPassword: "server setup proof",
    accountRateLimitMax: 10_000, apiRateLimitMax: 10_000,
    logger: quiet,
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return {
    root,
    url: `http://127.0.0.1:${server.address().port}`,
    async close() {
      // An oversized body is refused mid-stream and can leave the socket open without
      // ref'ing the event loop, so close() would never emit and node:test cancels the run.
      await new Promise((resolve) => {
        server.close(resolve);
        server.closeAllConnections();
      });
      await rm(root, { recursive: true });
    },
  };
}

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" },
    body: typeof body === "string" ? body : JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

test("a report is clipped to its limits, scrubbed of credentials and refused when it is not lr.crash/1", () => {
  assert.equal(sanitizeCrashReport({ schema: "lr.crash/2", kind: "crash" }).error, "INVALID_REPORT_SCHEMA");
  assert.equal(sanitizeCrashReport({ schema: "lr.crash/1", kind: "boom" }).error, "INVALID_REPORT_KIND");
  assert.equal(sanitizeCrashReport([]).error, "INVALID_REPORT");
  const big = sanitizeCrashReport(report({
    log_tail: "x".repeat(CRASH_LIMITS.logTailChars * 2),
    breadcrumbs: Array.from({ length: 200 }, (_, i) => `crumb ${i} ${"y".repeat(1000)}`),
    build: "q".repeat(500),
    unknown_field: "dropped",
    counts: { errors: -1, warnings: "7", script_errors: 1e12 },
  })).report;
  assert.equal(big.log_tail.length, CRASH_LIMITS.logTailChars);
  assert.equal(big.breadcrumbs.length, CRASH_LIMITS.breadcrumbs);
  assert.ok(big.breadcrumbs[0].startsWith("crumb 150"), "the newest breadcrumbs are kept");
  assert.ok(big.breadcrumbs.every((line) => line.length <= CRASH_LIMITS.lineChars));
  assert.equal(big.build.length, CRASH_LIMITS.fieldChars);
  assert.equal(big.unknown_field, undefined);
  assert.deepEqual(big.counts, { warnings: 7, script_errors: 1e9 });

  const leaky = "auth: Bearer abcdefghijklmnop sessionToken=s3cr3tvalue password: hunter22 "
    + "admin_ticket=\"tkt-123456\" " + "f".repeat(64);
  const scrubbed = scrubSecrets(leaky);
  for (const secret of ["abcdefghijklmnop", "s3cr3tvalue", "hunter22", "tkt-123456", "f".repeat(64)]) {
    assert.ok(!scrubbed.includes(secret), `${secret} survived: ${scrubbed}`);
  }
  const stored = sanitizeCrashReport(report({ log_tail: leaky, breadcrumbs: [{ what: "join", token: "abcd1234zz" }] })).report;
  assert.ok(!stored.log_tail.includes("hunter22") && !stored.breadcrumbs[0].includes("abcd1234zz"));
});

test("store and echo: a client is rate limited, old reports are pruned, list is newest first after an id", async () => {
  const root = await mkdtemp(join(tmpdir(), "crash-store-"));
  let clock = Date.UTC(2026, 9, 28, 20, 0, 0);
  try {
    const store = createCrashReports({ directory: root, now: () => clock });
    const ids = [];
    for (let i = 0; i < CRASH_LIMITS.perClientPerHour; i += 1) {
      clock += 1000;
      const [status, body] = store.submit(report({ build: `b${i}` }), "1.2.3.4|dev-1");
      assert.equal(status, 201);
      assert.equal(body.report.build, `b${i}`, "the stored report is echoed");
      ids.push(body.id);
    }
    assert.equal(store.submit(report(), "1.2.3.4|dev-1")[0], 429, "a seventh report in an hour is refused");
    assert.equal(store.submit(report(), "5.6.7.8|dev-2")[0], 201, "another client is not limited by the first");
    clock += 3600 * 1000;
    assert.equal(store.submit(report(), "1.2.3.4|dev-1")[0], 201, "the limit resets after an hour");
    const [, listed] = store.list({ limit: 3 });
    assert.equal(listed.reports.length, 3);
    assert.ok(listed.reports[0].id > listed.reports[1].id, "newest first");
    const [, after] = store.list({ since: ids[3] });
    assert.ok(after.reports.every((entry) => entry.id > ids[3]) && after.reports.length === 4);
    const onDisk = JSON.parse(readFileSync(join(root, `${ids[0]}.json`), "utf8"));
    assert.equal(onDisk.device_id, "dev-1");
    assert.ok(!JSON.stringify(onDisk).includes("1.2.3.4"), "the client address is never stored");
    assert.ok(readdirSync(root).every((name) => name.endsWith(".json")), "no temp files are left");
  } finally {
    await rm(root, { recursive: true });
  }
});

test("POST /api/v1/crash-reports stores and echoes; only an owner lists them", async () => {
  const app = await fixture();
  try {
    const sent = await post(app.url, "/api/v1/crash-reports", report());
    assert.equal(sent.status, 201, JSON.stringify(sent.body));
    assert.equal(sent.body.report.kind, "crash");
    const bad = await post(app.url, "/api/v1/crash-reports", { schema: "nope" });
    assert.equal(bad.status, 400);

    const player = (await post(app.url, "/api/account/register", { username: "PLAYER", password: "correct horse battery" })).body.sessionToken;
    assert.equal((await post(app.url, "/api/admin/crash-reports", { sessionToken: player })).status, 403);
    assert.equal((await post(app.url, "/api/admin/crash-reports", {})).status, 403);
    const owner = (await post(app.url, "/api/account/register", { username: "KYTTU", password: "correct horse battery",
      adminProof: "server setup proof" })).body.sessionToken;
    const listed = await post(app.url, "/api/admin/crash-reports", { sessionToken: owner });
    assert.equal(listed.status, 200);
    assert.equal(listed.body.reports.length, 1);
    assert.equal(listed.body.reports[0].id, sent.body.id);
    // a body over the cap is refused before it is parsed (413, or the socket is cut while it is still sending)
    const huge = await post(app.url, "/api/v1/crash-reports", JSON.stringify(report({ log_tail: "z".repeat(200_000) })))
      .catch(() => ({ status: "reset" }));
    assert.ok(huge.status === 413 || huge.status === "reset", `oversized report got ${huge.status}`);
    assert.equal(readdirSync(join(app.root, "crash-reports")).length, 1, "the oversized report was not stored");
  } finally {
    await app.close();
  }
});
