import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  createPublicSettingsStore,
  defaultPublicSettings,
  PUBLIC_SETTING_SPECS,
  sanitizeStoredSettings,
  validatePublicSettingChanges,
} from "./public_settings.mjs";

test("the defaults match the values the public route shipped with", () => {
  assert.deepEqual(defaultPublicSettings(), {
    heartbeat_seconds: 5,
    client_host_fallback: true,
    join_per_minute: 20,
    player_ttl_seconds: 20,
  });
});

test("a valid change is accepted and an unknown key refuses the whole change", () => {
  assert.equal(validatePublicSettingChanges({ join_per_minute: 30 }).ok, true);
  const bad = validatePublicSettingChanges({ join_per_minute: 30, not_a_setting: 1 });
  assert.equal(bad.ok, false);
  assert.equal(bad.key, "not_a_setting");
});

test("numbers are range checked and integers must be whole", () => {
  assert.equal(validatePublicSettingChanges({ heartbeat_seconds: 1 }).ok, false, "below the minimum");
  assert.equal(validatePublicSettingChanges({ heartbeat_seconds: 11 }).ok, false, "above the cap");
  assert.equal(validatePublicSettingChanges({ heartbeat_seconds: 2.5 }).ok, true, "heartbeats may be fractional");
  assert.equal(validatePublicSettingChanges({ join_per_minute: 2.5 }).ok, false, "join limits are whole numbers");
  assert.equal(validatePublicSettingChanges({ join_per_minute: "20" }).ok, false, "a string is not a number");
  assert.equal(validatePublicSettingChanges({ join_per_minute: Number.NaN }).ok, false);
});

test("booleans must be real booleans", () => {
  assert.equal(validatePublicSettingChanges({ client_host_fallback: false }).ok, true);
  assert.equal(validatePublicSettingChanges({ client_host_fallback: "no" }).ok, false);
});

test("empty and non-object changes are refused", () => {
  assert.equal(validatePublicSettingChanges({}).ok, false);
  assert.equal(validatePublicSettingChanges(null).ok, false);
  assert.equal(validatePublicSettingChanges(["join_per_minute"]).ok, false);
});

test("stored values that are out of range or unknown fall back to the defaults", () => {
  const values = sanitizeStoredSettings({ join_per_minute: 9999, player_ttl_seconds: 30, extra: true });
  assert.equal(values.join_per_minute, PUBLIC_SETTING_SPECS.join_per_minute.default);
  assert.equal(values.player_ttl_seconds, 30);
  assert.equal("extra" in values, false);
});

test("the store saves atomically and survives a restart", async () => {
  const dir = await mkdtemp(join(tmpdir(), "public-settings-"));
  try {
    const path = join(dir, "public_settings.json");
    const store = createPublicSettingsStore({ storePath: path });
    await store.load();
    const result = await store.update({ join_per_minute: 40, client_host_fallback: false });
    assert.equal(result.ok, true);
    assert.equal(store.tuning().joinPerMinute, 40);
    assert.equal(JSON.parse(await readFile(path, "utf8")).join_per_minute, 40);

    const reopened = createPublicSettingsStore({ storePath: path });
    await reopened.load();
    assert.equal(reopened.get().client_host_fallback, false);
    assert.equal(reopened.get().join_per_minute, 40);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a refused change changes nothing", async () => {
  const store = createPublicSettingsStore({ storePath: null });
  await store.load();
  const result = await store.update({ join_per_minute: 0 });
  assert.equal(result.ok, false);
  assert.equal(store.get().join_per_minute, 20);
});

test("the public route tuning converts the player timeout to milliseconds", async () => {
  const store = createPublicSettingsStore({ storePath: null });
  await store.load();
  await store.update({ player_ttl_seconds: 45 });
  assert.equal(store.tuning().playerTtlMs, 45_000);
});
