// Run with: node --test tools/mods/
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { buildIndex } from "./publish-index.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "publish-index.mjs");
const root = mkdtempSync(join(tmpdir(), "lr-mods-index-"));
after(() => rmSync(root, { recursive: true, force: true }));

function put(rel, text) {
  const full = join(root, rel);
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, text);
}
const sha = (text) => createHash("sha256").update(text).digest("hex");

put("ember/1.0.0/pack.json", "{}");
put("ember/1.0.0/props/office.json", '{"id":"office"}');
put("ember/1.0.0/assets/printer.glb", "glb-bytes");
put("ember/1.0.0/.DS_Store", "junk");
put("ember/1.0.0/._printer.glb", "junk");
put("ember/0.9.0/pack.json", "{\"old\":true}");
put("ember/.probe", "x");
put(".probe", "x");
put("index.json", "stale");

test("builds sorted packs, versions and files with sha256 and size, skipping junk", () => {
  const index = buildIndex(root);
  assert.equal(index.version, 1);
  assert.equal(index.packs.length, 1);
  const pack = index.packs[0];
  assert.equal(pack.id, "ember");
  assert.deepEqual(
    pack.versions.map((v) => v.version),
    ["0.9.0", "1.0.0"],
  );
  const v = pack.versions[1];
  assert.deepEqual(
    v.files.map((f) => f.path),
    ["assets/printer.glb", "pack.json", "props/office.json"],
  );
  const printer = v.files[0];
  assert.equal(printer.sha256, sha("glb-bytes"));
  assert.equal(printer.size, "glb-bytes".length);
  assert.equal(v.totalBytes, "glb-bytes".length + 2 + '{"id":"office"}'.length);
});

test("the CLI writes index.json at the root and it matches buildIndex", () => {
  const out = execFileSync(process.execPath, [script, root], { encoding: "utf8" });
  assert.match(out, /Wrote .*index\.json: 1 pack\(s\), 2 version\(s\)/);
  const written = JSON.parse(readFileSync(join(root, "index.json"), "utf8"));
  assert.deepEqual(written, buildIndex(root));
  assert.equal(existsSync(join(root, "index.json.tmp")), false);
});

test("a version folder without pack.json is refused", () => {
  put("broken/1.0.0/props/a.json", "{}");
  assert.throws(() => buildIndex(root), /broken\/1\.0\.0" has no pack\.json/);
  rmSync(join(root, "broken"), { recursive: true, force: true });
});

test("an invalid pack id is refused", () => {
  put("Bad-Id/1.0.0/pack.json", "{}");
  assert.throws(() => buildIndex(root), /"Bad-Id" is not a valid pack id/);
  rmSync(join(root, "Bad-Id"), { recursive: true, force: true });
});
