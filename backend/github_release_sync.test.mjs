import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { syncGithubRelease } from "./github_release_sync.mjs";

function response(value, init = {}) {
  return new Response(typeof value === "string" || Buffer.isBuffer(value)
    ? value : JSON.stringify(value), init);
}

function storedZip(fileEntries) {
  const localParts = [];
  const centralParts = [];
  let localOffset = 0;
  for (const [name, value] of Object.entries(fileEntries)) {
    const nameBytes = Buffer.from(name);
    const data = Buffer.from(value);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(0, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    localParts.push(local, nameBytes, data);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(0, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(localOffset, 42);
    centralParts.push(central, nameBytes);
    localOffset += local.length + nameBytes.length + data.length;
  }
  const centralDirectory = Buffer.concat(centralParts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(fileEntries).length, 8);
  end.writeUInt16LE(Object.keys(fileEntries).length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(localOffset, 16);
  return Buffer.concat([...localParts, centralDirectory, end]);
}

test("mirrors and verifies a stable GitHub debug release", async () => {
  const directory = await mkdtemp(join(tmpdir(), "last-route-release-"));
  const artifact = Buffer.from("test-debug-build");
  const sha256 = createHash("sha256").update(artifact).digest("hex");
  const manifest = {
    debug: {
      linux: {
        version: "0.0.1.0",
        revision: "a".repeat(40),
        file: "LastRoute-Linux-Debug.zip",
        sha256,
        required: true,
      },
    },
  };
  const release = {
    assets: [
      { name: "updates.debug.json", url: "https://api.github.test/assets/manifest" },
      { name: "LastRoute-Linux-Debug.zip", url: "https://api.github.test/assets/linux" },
    ],
  };
  let artifactRequests = 0;
  const fetchImpl = async (url) => {
    if (String(url).endsWith("/releases/tags/debug")) return response(release);
    if (url === "https://api.github.test/assets/manifest") return response(manifest);
    if (url === "https://api.github.test/assets/linux") {
      artifactRequests += 1;
      return response(artifact);
    }
    return response("missing", { status: 404 });
  };
  const manifestPath = join(directory, "updates.debug.json");
  const messages = [];
  try {
    let result = await syncGithubRelease({
      repository: "KyttuXD/LastRouteGodot",
      updateDirectory: directory,
      manifestPath,
      fetchImpl,
      logger: { log(message) { messages.push(message); } },
    });
    assert.deepEqual({ downloaded: result.downloaded, reused: result.reused }, { downloaded: 1, reused: 0 });
    assert.equal(result.revision, "a".repeat(40));
    assert.match(messages[0], /sync ready @ a{12}/);
    assert.deepEqual(await readFile(join(directory, "LastRoute-Linux-Debug.zip")), artifact);
    assert.deepEqual(JSON.parse(await readFile(manifestPath, "utf8")), manifest);

    result = await syncGithubRelease({
      repository: "KyttuXD/LastRouteGodot",
      updateDirectory: directory,
      manifestPath,
      fetchImpl,
      logger: { log(message) { messages.push(message); } },
    });
    assert.deepEqual({ downloaded: result.downloaded, reused: result.reused }, { downloaded: 0, reused: 1 });
    assert.equal(artifactRequests, 1);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("does not publish a manifest when an artifact fails verification", async () => {
  const directory = await mkdtemp(join(tmpdir(), "last-route-release-bad-"));
  const manifest = {
    debug: {
      linux: {
        version: "0.0.1.0",
        revision: "b".repeat(40),
        file: "linux.zip",
        sha256: "f".repeat(64),
      },
    },
  };
  const fetchImpl = async (url) => {
    if (String(url).endsWith("/releases/tags/debug")) return response({ assets: [
      { name: "updates.debug.json", url: "manifest" },
      { name: "linux.zip", url: "linux" },
    ] });
    if (url === "manifest") return response(manifest);
    if (url === "linux") return response("corrupt");
    return response("missing", { status: 404 });
  };
  const manifestPath = join(directory, "updates.debug.json");
  try {
    await assert.rejects(syncGithubRelease({
      repository: "KyttuXD/LastRouteGodot",
      updateDirectory: directory,
      manifestPath,
      fetchImpl,
      logger: { log() {} },
    }), /SHA-256 verification/);
    await assert.rejects(readFile(manifestPath), /ENOENT/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("publishes the verified Web archive once and reuses an unchanged deployment", async () => {
  const directory = await mkdtemp(join(tmpdir(), "last-route-web-release-"));
  const archive = storedZip({
    "LastRoute-Web-Debug/index.html": "<title>Last Route</title>",
    "LastRoute-Web-Debug/index.js": "console.log('ready');",
  });
  const sha256 = createHash("sha256").update(archive).digest("hex");
  const manifest = { debug: { web: {
    version: "0.0.1.0",
    revision: "c".repeat(40),
    file: "LastRoute-Web-Debug.zip",
    sha256,
  } } };
  const release = { assets: [
    { name: "updates.debug.json", url: "manifest" },
    { name: "LastRoute-Web-Debug.zip", url: "web" },
  ] };
  const fetchImpl = async (url) => {
    if (String(url).endsWith("/releases/tags/debug")) return response(release);
    if (url === "manifest") return response(manifest);
    if (url === "web") return response(archive);
    return response("missing", { status: 404 });
  };
  const options = {
    repository: "KyttuXD/LastRouteGodot",
    updateDirectory: join(directory, "updates"),
    manifestPath: join(directory, "updates", "updates.debug.json"),
    publicDirectory: join(directory, "web"),
    fetchImpl,
    logger: { log() {} },
  };
  try {
    let result = await syncGithubRelease(options);
    assert.equal(result.webPublished, true);
    assert.equal(await readFile(join(directory, "web", "index.html"), "utf8"),
      "<title>Last Route</title>");
    assert.match(await readFile(join(directory, "web", ".last-route-web-release"), "utf8"),
      /^debug:c{40}:/);
    result = await syncGithubRelease(options);
    assert.equal(result.webPublished, false);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
