// Writes index.json for a mod pack folder (the folder Caddy serves at mods.kyttuslab.org, e.g. /srv/lr3d/mods).
//
// Layout expected:  <root>/<pack-id>/<version>/pack.json  plus every other file of that version.
// Each version folder must contain pack.json. Every file under it is listed with its sha256 and size.
//
// Usage (dependency-free Node, from the Rust repo root or anywhere):
//   node tools/mods/publish-index.mjs <modsRoot>                 write <modsRoot>/index.json
//   node tools/mods/publish-index.mjs <modsRoot> --out <file>    write somewhere else
//   node tools/mods/publish-index.mjs <modsRoot> --stdout        print the index, write nothing
//
// It never uploads anything. Copy the result to VPS 2 by hand once it has been reviewed.

import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync, renameSync } from "node:fs";
import { join, posix, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const PACK_ID_PATTERN = /^[a-z0-9][a-z0-9_]{2,31}$/;
export const VERSION_PATTERN = /^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$/;
export const INDEX_VERSION = 1;

// Same skip rules as the game server's metadata cleanup: dotfiles and macOS junk never belong in a pack.
function skipped(name) {
  return name.startsWith(".") || name.startsWith("._") || name === "Thumbs.db" || name === "desktop.ini";
}

// JavaScript-style code unit order, so the index is identical on every platform.
function compare(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

function listFiles(dir, base = dir, out = []) {
  for (const name of readdirSync(dir).sort(compare)) {
    if (skipped(name)) continue;
    const full = join(dir, name);
    const info = statSync(full);
    if (info.isDirectory()) {
      listFiles(full, base, out);
    } else if (info.isFile()) {
      out.push({ path: relative(base, full).split(sep).join(posix.sep), full, size: info.size });
    }
  }
  return out;
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function folders(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && !skipped(entry.name))
    .map((entry) => entry.name)
    .sort(compare);
}

// Builds the index object for every pack and version under `root`. Throws on a layout it does not accept.
export function buildIndex(root) {
  const packs = [];
  for (const id of folders(root)) {
    if (!PACK_ID_PATTERN.test(id)) throw new Error(`Pack folder "${id}" is not a valid pack id`);
    const versions = [];
    for (const version of folders(join(root, id))) {
      if (!VERSION_PATTERN.test(version)) throw new Error(`Version folder "${id}/${version}" is not a valid version`);
      const versionDir = join(root, id, version);
      const files = listFiles(versionDir);
      if (!files.some((file) => file.path === "pack.json")) {
        throw new Error(`"${id}/${version}" has no pack.json`);
      }
      const entries = files.map((file) => ({ path: file.path, sha256: sha256File(file.full), size: file.size }));
      const totalBytes = entries.reduce((sum, entry) => sum + entry.size, 0);
      versions.push({ version, totalBytes, files: entries });
    }
    if (versions.length > 0) packs.push({ id, versions });
  }
  return { version: INDEX_VERSION, packs };
}

function parseArgs(argv) {
  const args = { root: null, out: null, stdout: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--stdout") args.stdout = true;
    else if (arg === "--out") args.out = argv[++i];
    else if (!args.root) args.root = arg;
    else throw new Error(`Unexpected argument: ${arg}`);
  }
  if (!args.root) throw new Error("Usage: node tools/mods/publish-index.mjs <modsRoot> [--out <file>] [--stdout]");
  return args;
}

function main(argv) {
  const args = parseArgs(argv);
  const index = buildIndex(args.root);
  const text = `${JSON.stringify(index, null, 2)}\n`;
  if (args.stdout) {
    process.stdout.write(text);
    return;
  }
  const out = args.out ?? join(args.root, "index.json");
  // Write to a sibling temp file, then rename, so a reader never sees a half-written index.
  const tmp = `${out}.tmp`;
  writeFileSync(tmp, text);
  renameSync(tmp, out);
  const count = index.packs.reduce((sum, pack) => sum + pack.versions.length, 0);
  process.stdout.write(`Wrote ${out}: ${index.packs.length} pack(s), ${count} version(s)\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
