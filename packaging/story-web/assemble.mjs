// Assembles the browser build of KYTTU - LRS (lr_story): node packaging/story-web/assemble.mjs [WASM] [OUT]
//
// WASM defaults to target/wasm32-unknown-unknown/release/lr_story.wasm (build it first:
//   cargo build --release --target wasm32-unknown-unknown --bin lr_story  with CARGO_BUILD_JOBS=4).
// OUT defaults to dist/story-web. Its vfs.bin, vfs.bin.gz, manifest.json and the wasm are replaced, and the page files
// are copied over. Zero dependencies, like the rest of the repo's tooling.
//
// What it writes (see src/bin/lr_story/assets_vfs.rs for how the game reads it):
//   manifest.json   {"schema": "lr.story.manifest/2", "bundle": "vfs.bin", "files": [{"key", "offset", "length"}, ...]}
//   vfs.bin         every listed file, back to back, in key order (one request for the browser)
//   vfs.bin.gz      the same bundle gzipped: Caddy's `precompressed gzip` serves it, and the browser decodes it
//   lr_story.wasm, index.html, mq_js_bundle.js, audio.js, hw.js, keys.js
//
// The listed files are: every file under the core pack (src/bin/last_route/data/lr_story), every file under lr_story's
// assets (src/bin/lr_story/assets), and every file under each mods/<pack id>/ folder that holds a pack.json. Dot files
// (such as .DS_Store) are skipped.
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const crate = path.resolve(here, "..", "..");
const wasm = path.resolve(process.argv[2] ?? path.join(crate, "target", "wasm32-unknown-unknown", "release", "lr_story.wasm"));
const out = path.resolve(process.argv[3] ?? path.join(crate, "dist", "story-web"));
const PAGE_FILES = ["index.html", "mq_js_bundle.js", "audio.js", "hw.js", "keys.js"];
const MANIFEST_SCHEMA = "lr.story.manifest/2";
const BUNDLE = "vfs.bin";

function fail(message) {
  console.error(`assemble: ${message}`);
  process.exit(1);
}

// Every folder the game reads for its files, relative to the crate folder with "/" separators.
function sourceFolders() {
  const folders = ["src/bin/lr_story/assets", "src/bin/last_route/data/lr_story"];
  const mods = path.join(crate, "mods");
  if (fs.existsSync(mods)) {
    for (const name of fs.readdirSync(mods).sort()) {
      const dir = path.join(mods, name);
      if (fs.statSync(dir).isDirectory() && fs.existsSync(path.join(dir, "pack.json"))) {
        folders.push(`mods/${name}`);
      }
    }
  }
  return folders;
}

// Every file under a folder (relative to the crate folder), recursively, dot files skipped.
function filesUnder(rel) {
  const found = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) found.push(path.relative(crate, full).split(path.sep).join("/"));
    }
  };
  walk(path.join(crate, rel));
  return found;
}

if (!fs.existsSync(wasm)) fail(`${wasm} is missing (build lr_story for wasm32 first)`);
for (const file of PAGE_FILES) {
  if (!fs.existsSync(path.join(here, file))) fail(`packaging/story-web/${file} is missing`);
}

const keys = [...new Set(sourceFolders().flatMap(filesUnder))].sort();
if (keys.length === 0) fail("no game files found");

// The bundle: each file's bytes back to back, with its offset and length in the manifest.
const parts = [];
const files = [];
let offset = 0;
for (const key of keys) {
  const bytes = fs.readFileSync(path.join(crate, ...key.split("/")));
  files.push({ key, offset, length: bytes.length });
  parts.push(bytes);
  offset += bytes.length;
}
const bundle = Buffer.concat(parts);

fs.mkdirSync(out, { recursive: true });
fs.rmSync(path.join(out, "vfs"), { recursive: true, force: true }); // the per-file tree of the previous build
fs.writeFileSync(path.join(out, BUNDLE), bundle);
fs.writeFileSync(path.join(out, `${BUNDLE}.gz`), zlib.gzipSync(bundle, { level: 9 }));
fs.writeFileSync(path.join(out, "manifest.json"), JSON.stringify({ schema: MANIFEST_SCHEMA, bundle: BUNDLE, files }, null, 2) + "\n");
fs.copyFileSync(wasm, path.join(out, "lr_story.wasm"));
for (const file of PAGE_FILES) {
  fs.copyFileSync(path.join(here, file), path.join(out, file));
}

const gz = fs.statSync(path.join(out, `${BUNDLE}.gz`)).size;
console.log(`assemble: ${keys.length} game file(s), ${bundle.length} bytes (${gz} gzipped) in ${BUNDLE}, into ${out}`);
