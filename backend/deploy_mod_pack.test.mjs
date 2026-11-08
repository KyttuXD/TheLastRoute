import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildPack, deployPack, localTransport, sshTransport } from "./deploy_mod_pack.mjs";

const SCRIPT = fileURLToPath(new URL("./deploy_mod_pack.mjs", import.meta.url));
const sha = (text) => createHash("sha256").update(text).digest("hex");

function writePack(directory, { version = "1.0.0", files = { "props/hall.json": "hall" }, manifestFiles = null, extra = {} } = {}) {
  mkdirSync(join(directory, "props"), { recursive: true });
  for (const [path, contents] of Object.entries(files)) {
    const full = join(directory, ...path.split("/"));
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, contents);
  }
  const manifest = {
    schema: "lr.pack/1", id: "neon_halls", name: "Neon Halls", version, author: "KT", ...extra,
  };
  if (manifestFiles) manifest.files = manifestFiles;
  writeFileSync(join(directory, "pack.json"), JSON.stringify(manifest));
  return manifest;
}

async function workspace() {
  const root = await mkdtemp(join(tmpdir(), "deploy-pack-"));
  return {
    root,
    pack: join(root, "neon_halls"),
    remote: join(root, "vps2"),
    async close() { await rm(root, { recursive: true, force: true }); },
  };
}

test("buildPack hashes the tree, skips dotfiles, and checks an optional files list", async () => {
  const box = await workspace();
  try {
    assert.throws(() => buildPack(box.pack), /pack\.json missing/);
    writePack(box.pack);
    writeFileSync(join(box.pack, ".DS_Store"), "junk");
    mkdirSync(join(box.pack, ".git"));
    writeFileSync(join(box.pack, ".git", "config"), "nope");
    writeFileSync(join(box.pack, "props", ".keep"), "hidden");
    const built = buildPack(box.pack);
    assert.deepEqual(built.files.map((file) => file.path), ["pack.json", "props/hall.json"]);
    assert.equal(built.files[1].sha256, sha("hall"));
    assert.equal(built.files[1].size, 4);
    assert.equal(built.id, "neon_halls");
    assert.equal(built.totalBytes, built.files.reduce((sum, file) => sum + file.size, 0));

    const hall = built.files.find((file) => file.path === "props/hall.json");
    writePack(box.pack, { manifestFiles: [{ path: "props/hall.json", sha256: sha("other"), size: hall.size }] });
    assert.throws(() => buildPack(box.pack), /sha256 that does not match/);
    writePack(box.pack, { manifestFiles: [{ path: "props/hall.json", sha256: hall.sha256, size: 99 }] });
    assert.throws(() => buildPack(box.pack), /wrong size/);
    writePack(box.pack, { manifestFiles: [] });
    assert.throws(() => buildPack(box.pack), /does not list props\/hall.json/);
    writePack(box.pack, { manifestFiles: [
      { path: "props/hall.json", sha256: hall.sha256, size: hall.size },
      { path: "props/missing.json", sha256: sha("x"), size: 1 },
    ] });
    assert.throws(() => buildPack(box.pack), /lists props\/missing.json/);
    writePack(box.pack, { extra: { id: "Bad Id" } });
    assert.throws(() => buildPack(box.pack), /INVALID_PACK_ID/);
    rmSync(join(box.pack, "props"), { recursive: true, force: true });
    writePack(box.pack, { files: {}, manifestFiles: [{ path: "pack.json", sha256: sha("{}"), size: 2 }] });
    assert.throws(() => buildPack(box.pack), /lists pack\.json, which is not in the pack/);
    writeFileSync(join(box.pack, "notes.txt"), "x");
    writePack(box.pack, { files: {} });
    assert.throws(() => buildPack(box.pack), /INVALID_FILE_PATH/);
  } finally {
    await box.close();
  }
});

test("dry run writes nothing; apply copies, checks hashes, and posts the owner token only in the body", async () => {
  const box = await workspace();
  try {
    writePack(box.pack, { files: { "props/hall.json": "hall" } });
    writeFileSync(join(box.pack, ".DS_Store"), "junk");
    const logs = [];
    const log = (line) => logs.push(line);
    const dry = await deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: `${box.remote}/`,
      indexUrl: "http://127.0.0.1:9/index/", sessionToken: "owner-session", log,
    });
    assert.equal(dry.applied, false);
    assert.equal(dry.destination, `${box.remote}/neon_halls/1.0.0`);
    assert.equal(existsSync(box.remote), false);
    assert.match(logs.join("\n"), /dry run/);
    assert.equal(logs.join("\n").includes("owner-session"), false);

    await assert.rejects(deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: box.remote,
      indexUrl: "http://127.0.0.1:9", sessionToken: "", apply: true, log() {},
    }), /LASTROUTE_OWNER_SESSION/);

    let posted = null;
    const done = await deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: box.remote,
      indexUrl: "http://127.0.0.1:9/index/", sessionToken: "owner-session", apply: true, log() {},
      fetchImpl: async (url, init) => {
        posted = { url, init };
        return { ok: true, status: 200, json: async () => ({ ok: true, pack: { id: "neon_halls", version: "1.0.0", downloadBase: "http://127.0.0.1:9/mods/neon_halls/1.0.0/" } }) };
      },
    });
    assert.equal(done.applied, true);
    assert.equal(done.index.downloadBase, "http://127.0.0.1:9/mods/neon_halls/1.0.0/");
    assert.equal(posted.url, "http://127.0.0.1:9/index/api/admin/mods/register");
    const body = JSON.parse(posted.init.body);
    assert.equal(body.sessionToken, "owner-session");
    assert.equal(body.replace, false);
    assert.equal(body.pack.files.find((file) => file.path === "props/hall.json").sha256, sha("hall"));
    assert.deepEqual(readdirSync(join(box.remote, "neon_halls", "1.0.0")).sort(), ["pack.json", "props"]);
    assert.equal(existsSync(join(box.remote, "neon_halls", "1.0.0", ".DS_Store")), false);
    assert.equal(readdirSync(join(box.remote, "neon_halls")).some((name) => name.includes("uploading")), false);

    let fetched = false;
    await assert.rejects(deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: box.remote,
      indexUrl: "http://127.0.0.1:9", sessionToken: "owner-session", apply: true, log() {},
      fetchImpl: async () => { fetched = true; return { ok: true, json: async () => ({ ok: true, pack: {} }) }; },
    }), /already exists/);
    assert.equal(fetched, false);
  } finally {
    await box.close();
  }
});

test("a bad upload is discarded, a refused index leaves the files, and replace overwrites that version", async () => {
  const box = await workspace();
  try {
    writePack(box.pack);
    const corrupt = {
      ...localTransport(),
      hashes: (_dir, paths) => new Map(paths.map((path) => [path, "0".repeat(64)])),
    };
    let fetched = false;
    await assert.rejects(deployPack({
      directory: box.pack, transport: corrupt, remoteRoot: box.remote, indexUrl: "http://127.0.0.1:9",
      sessionToken: "owner-session", apply: true, log() {},
      fetchImpl: async () => { fetched = true; throw new Error("should not register"); },
    }), /upload check failed/);
    assert.equal(fetched, false);
    assert.equal(existsSync(join(box.remote, "neon_halls", "1.0.0")), false);
    assert.equal(existsSync(`${box.remote}/neon_halls/1.0.0.uploading-${process.pid}`), false);

    await deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: box.remote, indexUrl: "http://127.0.0.1:9",
      sessionToken: "owner-session", apply: true, log() {},
      fetchImpl: async () => ({ ok: false, status: 422, json: async () => ({ ok: false, code: "INDEX_FULL" }) }),
    }).then(() => { throw new Error("index refusal should reject"); }, (error) => {
      assert.match(error.message, /index refused the pack \(422 INDEX_FULL\)/);
    });
    assert.equal(readFileSync(join(box.remote, "neon_halls", "1.0.0", "props", "hall.json"), "utf8"), "hall");

    writeFileSync(join(box.pack, "props", "hall.json"), "hall-v2");
    writeFileSync(join(box.pack, "props", "extra.json"), "{}");
    writePack(box.pack, { files: { "props/hall.json": "hall-v2", "props/extra.json": "{}" } });
    await deployPack({
      directory: box.pack, transport: localTransport(), remoteRoot: box.remote, indexUrl: "http://127.0.0.1:9/",
      sessionToken: "owner-session", apply: true, replace: true, log() {},
      fetchImpl: async () => ({ ok: true, status: 200, json: async () => ({ ok: true, pack: { id: "neon_halls", version: "1.0.0", downloadBase: "" } }) }),
    });
    assert.equal(readFileSync(join(box.remote, "neon_halls", "1.0.0", "props", "hall.json"), "utf8"), "hall-v2");
    assert.equal(readFileSync(join(box.remote, "neon_halls", "1.0.0", "props", "extra.json"), "utf8"), "{}");
  } finally {
    await box.close();
  }
});

test("local promote refuses an existing folder unless replace is set; ssh setup does not connect", async () => {
  const box = await workspace();
  try {
    const transport = localTransport();
    const from = join(box.root, "stage");
    const to = join(box.root, "mods", "neon_halls", "1.0.0");
    mkdirSync(from, { recursive: true });
    writeFileSync(join(from, "pack.json"), "{}");
    transport.promote(from, to, false);
    assert.equal(readFileSync(join(to, "pack.json"), "utf8"), "{}");
    assert.equal(existsSync(from), false);
    mkdirSync(from, { recursive: true });
    writeFileSync(join(from, "pack.json"), "next");
    assert.throws(() => transport.promote(from, to, false), /already on the file server/);
    assert.equal(readFileSync(join(to, "pack.json"), "utf8"), "{}");
    transport.promote(from, to, true);
    assert.equal(readFileSync(join(to, "pack.json"), "utf8"), "next");
    assert.equal(transport.hashes(to, ["pack.json", "missing.txt"]).get("pack.json"), sha("next"));
    assert.equal(transport.hashes(to, ["missing.txt"]).get("missing.txt"), "");

    const ssh = sshTransport("not-a-real-host.example");
    assert.equal(ssh.describe, "ssh not-a-real-host.example");
    assert.equal(typeof ssh.upload, "function");
    assert.equal(typeof ssh.exists, "function");
  } finally {
    await box.close();
  }
});

test("the CLI prints usage without arguments and a dry run does not copy or register", async () => {
  const env = { ...process.env };
  delete env.LASTROUTE_OWNER_SESSION;
  const usage = spawnSync(process.execPath, [SCRIPT], { encoding: "utf8", env });
  assert.equal(usage.status, 2);
  assert.match(usage.stderr, /usage:/);
  assert.equal(usage.stderr.includes("LASTROUTE_OWNER_SESSION"), false);

  const box = await workspace();
  try {
    writePack(box.pack);
    const dry = spawnSync(process.execPath, [
      SCRIPT, box.pack, "--local-root", box.remote, "--index", "http://127.0.0.1:9", "--remote-root", "/ignored",
    ], { encoding: "utf8", env });
    assert.equal(dry.status, 0, dry.stderr);
    assert.match(dry.stdout, /dry run/);
    assert.equal(existsSync(box.remote), false);
  } finally {
    await box.close();
  }
});
