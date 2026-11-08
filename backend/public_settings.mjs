// The PUBLIC route's settings that an owner may change while the server runs.
//
// The values are kept in one small JSON file (atomic writes, like the other stores). Anything not stored keeps its
// default, and every write is checked against the spec below: an unknown key, a wrong type or an out-of-range value
// refuses the whole change, so a bad request never half-applies.

import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

export const PUBLIC_SETTING_SPECS = Object.freeze({
  // the directory heartbeat of the public room (the client clamps it the same way)
  heartbeat_seconds: { type: "number", min: 2, max: 10, default: 5 },
  // whether a runner may open the public room as a stand-in while no server process holds it
  client_host_fallback: { type: "boolean", default: true },
  // how many join requests one address may make a minute before it is refused
  join_per_minute: { type: "integer", min: 1, max: 120, default: 20 },
  // seconds a player may stay silent before the public route forgets them (their resume record is kept)
  player_ttl_seconds: { type: "number", min: 5, max: 120, default: 20 },
});

export function defaultPublicSettings() {
  const values = {};
  for (const [key, spec] of Object.entries(PUBLIC_SETTING_SPECS)) values[key] = spec.default;
  return values;
}

/** Checks `changes` against the specs. Returns `{ ok: true, changes }` or `{ ok: false, error, key }`. */
export function validatePublicSettingChanges(changes) {
  if (!changes || typeof changes !== "object" || Array.isArray(changes)) {
    return { ok: false, error: "changes must be an object of setting names to values", key: "" };
  }
  const entries = Object.entries(changes);
  if (entries.length === 0) return { ok: false, error: "no settings to change", key: "" };
  const clean = {};
  for (const [key, value] of entries) {
    const spec = PUBLIC_SETTING_SPECS[key];
    if (!spec) return { ok: false, error: `${key} is not a public route setting`, key };
    if (spec.type === "boolean") {
      if (typeof value !== "boolean") return { ok: false, error: `${key} must be true or false`, key };
      clean[key] = value;
      continue;
    }
    const number = typeof value === "number" ? value : Number.NaN;
    if (!Number.isFinite(number)) return { ok: false, error: `${key} must be a number`, key };
    if (spec.type === "integer" && !Number.isInteger(number)) return { ok: false, error: `${key} must be a whole number`, key };
    if (number < spec.min || number > spec.max) {
      return { ok: false, error: `${key} must be from ${spec.min} to ${spec.max}`, key };
    }
    clean[key] = number;
  }
  return { ok: true, changes: clean };
}

/** Reads a stored settings object, keeping only known keys with valid values; anything else falls back to the default. */
export function sanitizeStoredSettings(raw) {
  const values = defaultPublicSettings();
  if (!raw || typeof raw !== "object") return values;
  for (const key of Object.keys(PUBLIC_SETTING_SPECS)) {
    const check = validatePublicSettingChanges({ [key]: raw[key] });
    if (check.ok) values[key] = check.changes[key];
  }
  return values;
}

/**
 * The settings store: `get()` returns a copy of the effective values, `update(changes)` checks and saves them.
 * `storePath` null keeps the values in memory only (tests and a store-less run).
 */
export function createPublicSettingsStore({ storePath = null, logger = console } = {}) {
  let values = defaultPublicSettings();
  let loaded = false;

  async function load() {
    if (loaded) return;
    loaded = true;
    if (!storePath) return;
    try {
      values = sanitizeStoredSettings(JSON.parse(await readFile(storePath, "utf8")));
    } catch (error) {
      if (error?.code !== "ENOENT") logger.warn?.(`public settings not read, defaults used: ${error?.message || error}`);
    }
  }

  async function save() {
    if (!storePath) return;
    await mkdir(dirname(storePath), { recursive: true });
    const temp = join(dirname(storePath), `.public_settings.${process.pid}.tmp`);
    await writeFile(temp, `${JSON.stringify(values, null, 2)}\n`, { mode: 0o600 });
    await rename(temp, storePath);
  }

  return {
    load,
    get() {
      return { ...values };
    },
    /** The values the public route reads: the join limit and the player timeout in the units it uses. */
    tuning() {
      return {
        joinPerMinute: values.join_per_minute,
        playerTtlMs: Math.round(values.player_ttl_seconds * 1000),
      };
    },
    async update(changes) {
      const check = validatePublicSettingChanges(changes);
      if (!check.ok) return check;
      const previous = values;
      values = { ...values, ...check.changes };
      try {
        await save();
      } catch (error) {
        values = previous;
        throw error;
      }
      return { ok: true, settings: { ...values } };
    },
  };
}
