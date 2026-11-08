import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Crash and freeze reports from game clients (CrashWatch, lr.crash/1). Store and echo only: a report is clipped to
// fixed field limits, scrubbed of anything that looks like a credential, written as one JSON file under the data
// directory, and echoed back as stored. Nothing here acts on a report; the owner lists them (POST
// /api/admin/crash-reports) and routes them to whoever owns the code.

export const CRASH_SCHEMA = "lr.crash/1";
export const CRASH_MAX_BODY_BYTES = 96 * 1024;
export const CRASH_LIMITS = Object.freeze({
  logTailChars: 32 * 1024,
  breadcrumbs: 50,
  errors: 50,
  lineChars: 400,
  fieldChars: 120,
  stored: 5000,
  perClientPerHour: 6,
  listMax: 200,
});
const KINDS = new Set(["crash", "freeze", "script_errors"]);
const TEXT_FIELDS = ["build", "version", "os", "os_version", "device", "gpu", "renderer", "locale", "scene", "room_role",
  "device_id"];
const COUNT_FIELDS = ["errors", "warnings", "script_errors", "dropped"];
// Anything credential-shaped is replaced, wherever it appears (log lines, breadcrumbs, field values).
const SECRET_PATTERNS = [
  /\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}/gi,
  /\b(session(?:_?token)?|token|password|passwd|secret|api_?key|admin_?ticket|host_?token|authorization)\b(\s*["']?\s*[:=]\s*["']?)[^\s"',;&}]{4,}/gi,
  /\b[A-Fa-f0-9]{40,}\b/g,
];
const REPORT_ID_PATTERN = /^[0-9]{8}T[0-9]{6}-[a-f0-9]{8}$/;

export function scrubSecrets(text) {
  let out = String(text ?? "");
  out = out.replace(SECRET_PATTERNS[0], "$1 [redacted]");
  out = out.replace(SECRET_PATTERNS[1], "$1$2[redacted]");
  out = out.replace(SECRET_PATTERNS[2], "[redacted-hex]");
  return out;
}

function clip(value, max) {
  return scrubSecrets(value).replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "").slice(0, max);
}

// The stored shape of a report, or { error } when it is not one.
export function sanitizeCrashReport(body) {
  if (!body || typeof body !== "object" || Array.isArray(body)) return { error: "INVALID_REPORT" };
  if (body.schema !== CRASH_SCHEMA) return { error: "INVALID_REPORT_SCHEMA" };
  const kind = String(body.kind || "");
  if (!KINDS.has(kind)) return { error: "INVALID_REPORT_KIND" };
  const report = { schema: CRASH_SCHEMA, kind };
  for (const field of TEXT_FIELDS) {
    if (body[field] !== undefined && body[field] !== null) report[field] = clip(body[field], CRASH_LIMITS.fieldChars);
  }
  const counts = {};
  for (const field of COUNT_FIELDS) {
    const value = Number(body.counts?.[field]);
    if (Number.isFinite(value) && value >= 0) counts[field] = Math.min(Math.floor(value), 1e9);
  }
  report.counts = counts;
  for (const field of ["session_started", "session_ended"]) {
    const value = Number(body[field]);
    if (Number.isFinite(value) && value > 0) report[field] = Math.floor(value);
  }
  const listOf = (value, max) => (Array.isArray(value) ? value.slice(-max) : [])
    .map((entry) => clip(typeof entry === "string" ? entry : JSON.stringify(entry), CRASH_LIMITS.lineChars));
  report.breadcrumbs = listOf(body.breadcrumbs, CRASH_LIMITS.breadcrumbs);
  report.recent_errors = listOf(body.recent_errors, CRASH_LIMITS.errors);
  report.log_tail = clip(body.log_tail, CRASH_LIMITS.logTailChars);
  return { report };
}

export function createCrashReports({ directory, now = Date.now } = {}) {
  if (!directory) throw new TypeError("directory is required");
  mkdirSync(directory, { recursive: true });
  const rates = new Map();

  function files() {
    return readdirSync(directory).filter((name) => name.endsWith(".json")).sort();
  }

  function allow(client) {
    const hour = 3600 * 1000;
    const timestamp = now();
    const recent = (rates.get(client) || []).filter((at) => timestamp - at < hour);
    if (recent.length >= CRASH_LIMITS.perClientPerHour) {
      rates.set(client, recent);
      return false;
    }
    recent.push(timestamp);
    rates.set(client, recent);
    if (rates.size > 10000) {
      for (const [key, times] of rates) if (!times.some((at) => timestamp - at < hour)) rates.delete(key);
    }
    return true;
  }

  // [status, body]. `client` is the requester address plus the report's device id (rate limit only; never stored).
  function submit(body, client) {
    const checked = sanitizeCrashReport(body);
    if (checked.error) return [400, { ok: false, code: checked.error, error: "Not a crash report" }];
    if (!allow(String(client || "unknown"))) {
      return [429, { ok: false, code: "CRASH_RATE_LIMITED", error: "Too many crash reports from this client" }];
    }
    const stamp = new Date(now()).toISOString().replace(/[-:]/g, "").slice(0, 15);
    const id = `${stamp}-${randomBytes(4).toString("hex")}`;
    const report = { id, received_at: new Date(now()).toISOString(), ...checked.report };
    const path = join(directory, `${id}.json`);
    writeFileSync(`${path}.tmp`, JSON.stringify(report));
    renameSync(`${path}.tmp`, path);
    const stored = files();
    for (const name of stored.slice(0, Math.max(0, stored.length - CRASH_LIMITS.stored))) {
      rmSync(join(directory, name), { force: true });
    }
    return [201, { ok: true, id, report }];
  }

  // Owner view: newest first, reports received after `since` (an id), at most `limit`.
  function list({ since = "", limit = 50 } = {}) {
    const max = Math.min(Math.max(1, Number(limit) || 50), CRASH_LIMITS.listMax);
    const after = REPORT_ID_PATTERN.test(String(since)) ? String(since) : "";
    const reports = [];
    for (const name of files().reverse()) {
      const id = name.slice(0, -5);
      if (after && id <= after) break;
      try {
        reports.push(JSON.parse(readFileSync(join(directory, name), "utf8")));
      } catch {
        continue;
      }
      if (reports.length >= max) break;
    }
    return [200, { ok: true, reports, total: files().length }];
  }

  return { submit, list, directory, exists: () => existsSync(directory) };
}
