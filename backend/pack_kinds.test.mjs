import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, rm, cp } from "node:fs/promises";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { FALLBACK_PACK_KINDS } from "./mod_packs.mjs";
import { PACK_KINDS_FILE, loadPackKinds, readPackKindsFromRepo } from "./pack_kinds.mjs";

const SCRIPT = fileURLToPath(new URL("./pack_kinds.mjs", import.meta.url));
const KIND = /^[a-z][a-z0-9_]{0,31}$/;

function writeRegistry(root, body) {
  mkdirSync(join(root, "src", "config"), { recursive: true });
  writeFileSync(join(root, "src", "config", "content_registry.gd"), body);
}

test("a checkout that is not a repo has no pack kinds", async () => {
  const root = await mkdtemp(join(tmpdir(), "pack-kinds-empty-"));
  try {
    assert.equal(readPackKindsFromRepo(root), null);
    writeRegistry(root, "const OTHER := 1\n");
    assert.equal(readPackKindsFromRepo(root), null);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("built-in kinds stay in source order and extension kinds are sorted after them", async () => {
  const root = await mkdtemp(join(tmpdir(), "pack-kinds-"));
  try {
    writeRegistry(root, [
      "const PACK_KINDS := [",
      '"props", "rooms", "' + "a".repeat(33) + '", "9no", "Bad-Kind", "ok_extra"',
      "]\n",
    ].join("\n"));
    assert.deepEqual(readPackKindsFromRepo(root), ["props", "rooms", "ok_extra"]);

    mkdirSync(join(root, "data", "kinds"), { recursive: true });
    writeFileSync(join(root, "data", "kinds", "zeta.json"), JSON.stringify({
      zeta: {}, alpha: {}, props: {}, BAD: {}, "1no": {}, ["b".repeat(33)]: {},
    }));
    writeFileSync(join(root, "data", "kinds", "broken.json"), "{ not json");
    writeFileSync(join(root, "data", "kinds", "notes.txt"), "{\"skip\":{}}");
    assert.deepEqual(readPackKindsFromRepo(root), ["props", "rooms", "ok_extra", "alpha", "zeta"]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("loadPackKinds uses pack_kinds.json, otherwise the repo, and never the fallback while either exists", () => {
  const loaded = loadPackKinds(["fallback_only"]);
  assert.ok(loaded.length > 0);
  assert.ok(loaded.every((kind) => KIND.test(kind)));
  assert.equal(loaded.includes("fallback_only"), false);
  if (existsSync(PACK_KINDS_FILE)) {
    assert.deepEqual(loaded, JSON.parse(readFileSync(PACK_KINDS_FILE, "utf8")).kinds);
  } else {
    assert.deepEqual(loaded, readPackKindsFromRepo());
  }
  const checkout = readPackKindsFromRepo();
  const extras = ["nw_buildings", "nw_calls", "nw_casts", "nw_modes", "nw_offices", "nw_rules", "nw_scares", "nw_sounds"];
  assert.deepEqual(checkout, [...FALLBACK_PACK_KINDS, ...extras]);
});

test("the CLI prints kinds, and --write stores them beside a copied script without touching this checkout", async () => {
  const before = existsSync(PACK_KINDS_FILE) ? readFileSync(PACK_KINDS_FILE, "utf8") : null;
  const printed = spawnSync(process.execPath, [SCRIPT], { encoding: "utf8" });
  assert.equal(printed.status, 0, printed.stderr);
  assert.deepEqual(JSON.parse(printed.stdout), readPackKindsFromRepo());
  const after = existsSync(PACK_KINDS_FILE) ? readFileSync(PACK_KINDS_FILE, "utf8") : null;
  assert.equal(after, before);

  const root = await mkdtemp(join(tmpdir(), "pack-kinds-cli-"));
  try {
    const backend = join(root, "backend");
    mkdirSync(backend, { recursive: true });
    await cp(SCRIPT, join(backend, "pack_kinds.mjs"));
    const bare = spawnSync(process.execPath, [join(backend, "pack_kinds.mjs")], { encoding: "utf8" });
    assert.equal(bare.status, 1);
    assert.match(bare.stderr, /content_registry\.gd not found/);

    writeRegistry(root, 'const PACK_KINDS := ["props", "rooms"]\n');
    mkdirSync(join(root, "data", "kinds"), { recursive: true });
    writeFileSync(join(root, "data", "kinds", "extra.json"), JSON.stringify({ zeta: {}, alpha: {} }));
    const wrote = spawnSync(process.execPath, [join(backend, "pack_kinds.mjs"), "--write"], { encoding: "utf8" });
    assert.equal(wrote.status, 0, wrote.stderr);
    assert.match(wrote.stdout, /wrote 4 kinds/);
    const stored = JSON.parse(readFileSync(join(backend, "pack_kinds.json"), "utf8"));
    assert.deepEqual(stored.kinds, ["props", "rooms", "alpha", "zeta"]);
    const again = spawnSync(process.execPath, [join(backend, "pack_kinds.mjs")], { encoding: "utf8" });
    assert.deepEqual(JSON.parse(again.stdout), stored.kinds);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
