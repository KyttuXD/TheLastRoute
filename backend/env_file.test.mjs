import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { loadEnvironmentFile, parseEnvironmentFile } from "./env_file.mjs";

test("environment files support common dotenv syntax", () => {
  const parsed = parseEnvironmentFile(`
# deployment settings
LASTROUTE_HTTP_PORT=8105
export LASTROUTE_ADMIN_PASSWORD="secret value"
SINGLE='literal # value'
PLAIN=value # comment
MULTILINE="first\\nsecond"
`);

  assert.equal(parsed.get("LASTROUTE_HTTP_PORT"), "8105");
  assert.equal(parsed.get("LASTROUTE_ADMIN_PASSWORD"), "secret value");
  assert.equal(parsed.get("SINGLE"), "literal # value");
  assert.equal(parsed.get("PLAIN"), "value");
  assert.equal(parsed.get("MULTILINE"), "first\nsecond");
});

test("existing panel variables take precedence over an env file", async () => {
  const directory = await mkdtemp(join(tmpdir(), "lastroute-env-"));
  const path = join(directory, ".env");
  await writeFile(path, "PANEL_VALUE=from-file\nFILE_ONLY=loaded\n", "utf8");
  const environment = { PANEL_VALUE: "from-panel" };

  assert.equal(await loadEnvironmentFile(path, environment), true);
  assert.equal(environment.PANEL_VALUE, "from-panel");
  assert.equal(environment.FILE_ONLY, "loaded");
  assert.equal(await loadEnvironmentFile(join(directory, "missing.env"), environment), false);
});
