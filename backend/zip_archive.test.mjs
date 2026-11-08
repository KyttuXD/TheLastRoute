import test from "node:test";
import assert from "node:assert/strict";
import { deflateRawSync } from "node:zlib";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { publishZipDirectory } from "./zip_archive.mjs";

function buildZip(entries, options = {}) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const entry of entries) {
    const name = Buffer.from(entry.name, "utf8");
    const raw = Buffer.from(entry.data ?? "");
    const method = entry.method ?? 0;
    const compressed = entry.compressed
      ?? (method === 8 ? deflateRawSync(raw) : raw);
    const flags = entry.flags ?? 0;
    const uncompressedSize = entry.uncompressedSize ?? raw.length;
    const localSize = entry.localCompressedSize ?? compressed.length;
    const centralSize = entry.centralCompressedSize ?? localSize;
    const local = Buffer.alloc(30 + name.length + compressed.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(flags, 6);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(localSize, 18);
    local.writeUInt32LE(uncompressedSize, 22);
    local.writeUInt16LE(name.length, 26);
    name.copy(local, 30);
    compressed.copy(local, 30 + name.length);
    locals.push(local);

    const unixMode = entry.unixMode ?? (entry.name.endsWith("/") ? 0o040755 : 0o100644);
    const central = Buffer.alloc(46 + name.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(flags, 8);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(centralSize, 20);
    central.writeUInt32LE(uncompressedSize, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE((unixMode * 0x10000) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    name.copy(central, 46);
    centrals.push(central);
    offset += local.length;
  }
  const centralDir = Buffer.concat(centrals);
  const comment = Buffer.from(options.comment ?? "");
  const eocd = Buffer.alloc(22 + comment.length);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(options.diskNumber ?? 0, 4);
  eocd.writeUInt16LE(options.diskStart ?? 0, 6);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(options.totalEntries ?? entries.length, 10);
  eocd.writeUInt32LE(centralDir.length, 12);
  eocd.writeUInt32LE(options.centralOffset ?? offset, 16);
  eocd.writeUInt16LE(comment.length, 20);
  comment.copy(eocd, 22);
  return Buffer.concat([...locals, centralDir, eocd]);
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "zip-archive-"));
  return {
    root,
    archive: join(root, "update.zip"),
    destination: join(root, "public"),
    async close() { await rm(root, { recursive: true, force: true }); },
  };
}

async function publish(box, entries, options = {}, publishOptions = {}) {
  writeFileSync(box.archive, buildZip(entries, options));
  return publishZipDirectory(box.archive, box.destination, publishOptions);
}

test("a stored and a deflated archive publish their files, stripping one shared root", async () => {
  const box = await fixture();
  try {
    const published = await publish(box, [
      { name: "build/", data: "" },
      { name: "build/index.html", data: "<h1>route</h1>" },
      { name: "build/js/app.js", data: "console.log(1)", method: 8 },
    ], { comment: "trailing" });
    assert.equal(published.files, 2);
    assert.equal(published.directory, box.destination);
    assert.equal(await readFile(join(box.destination, "index.html"), "utf8"), "<h1>route</h1>");
    assert.equal(await readFile(join(box.destination, "js", "app.js"), "utf8"), "console.log(1)");
    assert.equal(existsSync(join(box.destination, "build")), false);

    writeFileSync(join(box.destination, "old.txt"), "gone");
    await publish(box, [{ name: "app.txt", data: "fresh" }], {}, { markerName: "VERSION", markerContents: "46" });
    assert.equal(await readFile(join(box.destination, "app.txt"), "utf8"), "fresh");
    assert.equal(await readFile(join(box.destination, "VERSION"), "utf8"), "46");
    assert.equal(existsSync(join(box.destination, "old.txt")), false);
    assert.equal(existsSync(join(box.destination, "index.html")), false);
    assert.equal(existsSync(`${box.destination}.previous-${process.pid}`), false);
    assert.equal(existsSync(`${box.destination}.part-${process.pid}`), false);
  } finally {
    await box.close();
  }
});

test("a mixed root is kept, and backslashes are treated as folders", async () => {
  const box = await fixture();
  try {
    await publish(box, [
      { name: "a.txt", data: "top" },
      { name: "pack/b.txt", data: "nested" },
      { name: "dir\\leaf.txt", data: "leaf" },
    ]);
    assert.equal(await readFile(join(box.destination, "a.txt"), "utf8"), "top");
    assert.equal(await readFile(join(box.destination, "pack", "b.txt"), "utf8"), "nested");
    assert.equal(await readFile(join(box.destination, "dir", "leaf.txt"), "utf8"), "leaf");
    await publish(box, [{ name: "root/sub/only.txt", data: "deep" }]);
    assert.equal(await readFile(join(box.destination, "sub", "only.txt"), "utf8"), "deep");
    assert.equal(existsSync(join(box.destination, "a.txt")), false);
  } finally {
    await box.close();
  }
});

test("unsafe, empty, truncated and unsupported archives publish nothing", async () => {
  const box = await fixture();
  const outside = join(box.root, "outside.txt");
  try {
    const refusals = [
      [[{ name: "../outside.txt", data: "no" }], {}, /unsafe path/],
      [[{ name: "/etc/passwd", data: "no" }], {}, /unsafe path/],
      [[{ name: "a\0b.txt", data: "no" }], {}, /unsafe path/],
      [[{ name: "link", data: "target", unixMode: 0o120000 }], {}, /unsafe path/],
      [[{ name: "secret.txt", data: "no", flags: 1 }], {}, /unsupported entry/],
      [[{ name: "secret.txt", data: "no", method: 12 }], {}, /unsupported entry/],
      [[{ name: "dir/" }], {}, /no files/],
      [[], {}, /no files/],
      [[{ name: "ok.txt", data: "hello", uncompressedSize: 3 }], {}, /size is invalid/],
      [[{ name: "ok.txt", data: "hello", centralCompressedSize: 80 }], {}, /truncated/],
      [[{ name: "ok.txt", data: "hello" }], { diskNumber: 1 }, /multi-disk/],
      [[{ name: "ok.txt", data: "hello" }], { totalEntries: 257 }, /too large/],
      [[{ name: "ok.txt", data: "hello" }], { centralOffset: 50_000_000 }, /too large/],
    ];
    for (const [entries, options, pattern] of refusals) {
      await assert.rejects(publish(box, entries, options), pattern);
      assert.equal(existsSync(box.destination), false, String(pattern));
      assert.equal(existsSync(outside), false);
    }
    writeFileSync(box.archive, Buffer.from("not a zip"));
    await assert.rejects(publishZipDirectory(box.archive, box.destination), /not a supported ZIP archive/);
    const bytes = buildZip([{ name: "ok.txt", data: "hello" }]);
    const central = bytes.lastIndexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]));
    bytes[central] = 0;
    writeFileSync(box.archive, bytes);
    await assert.rejects(publishZipDirectory(box.archive, box.destination), /central directory is corrupt/);

    mkdirSync(box.destination, { recursive: true });
    writeFileSync(join(box.destination, "keep.txt"), "stay");
    writeFileSync(box.archive, buildZip([{ name: "ok.txt", data: "hello" }]));
    await assert.rejects(publishZipDirectory(box.archive, box.destination, { markerName: "a/b" }), /marker name is unsafe/);
    assert.equal(await readFile(join(box.destination, "keep.txt"), "utf8"), "stay");
    await assert.rejects(publish(box, [
      { name: "one.txt", data: "ok" },
      { name: "two.txt", data: "bad", uncompressedSize: 1 },
    ]), /size is invalid/);
    assert.equal(await readFile(join(box.destination, "keep.txt"), "utf8"), "stay");
    assert.equal(existsSync(join(box.destination, "one.txt")), false);
  } finally {
    await box.close();
  }
});
