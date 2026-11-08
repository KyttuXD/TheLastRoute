#!/usr/bin/env node
// The folders a mod pack may ship, as the game sees them: ContentRegistry.PACK_KINDS (game/config/content_registry.gd)
// plus every extension kind other lanes declare in data/kinds/<owner>.json (ContentRegistry.pack_kinds(), DATA
// c81e3df5). The server ships without src/ and data/ (deploy/deploy.sh copies backend/ only), so the deploy writes
// them to backend/pack_kinds.json with `node backend/pack_kinds.mjs --write`; mod_packs.mjs reads that file, or the
// repo when it runs from a checkout, or its own fallback list.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
export const PACK_KINDS_FILE = join(HERE, "pack_kinds.json");
const KIND = /^[a-z][a-z0-9_]{0,31}$/;

// Built-in kinds then extension kinds (sorted), from a repo checkout; null when `root` is not one.
export function readPackKindsFromRepo(root = resolve(HERE, "..")) {
  const registry = join(root, "src", "config", "content_registry.gd");
  if (!existsSync(registry)) return null;
  const match = /^const PACK_KINDS\s*:=\s*\[([\s\S]*?)\]/m.exec(readFileSync(registry, "utf8"));
  if (!match) return null;
  const builtin = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).filter((kind) => KIND.test(kind));
  const extra = new Set();
  const kindsDir = join(root, "data", "kinds");
  if (existsSync(kindsDir)) {
    for (const name of readdirSync(kindsDir).filter((file) => file.endsWith(".json")).sort()) {
      try {
        const declared = JSON.parse(readFileSync(join(kindsDir, name), "utf8"));
        for (const kind of Object.keys(declared || {})) if (KIND.test(kind) && !builtin.includes(kind)) extra.add(kind);
      } catch {
        // a broken kinds file is the game's error to report; the index just does not learn that kind
      }
    }
  }
  return [...builtin, ...[...extra].sort()];
}

export function loadPackKinds(fallback) {
  try {
    const written = JSON.parse(readFileSync(PACK_KINDS_FILE, "utf8"));
    if (Array.isArray(written.kinds) && written.kinds.every((kind) => KIND.test(kind))) return written.kinds;
  } catch {
    // not written: a checkout, or an older release
  }
  return readPackKindsFromRepo() || fallback;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const kinds = readPackKindsFromRepo();
  if (!kinds) {
    console.error("pack_kinds: run from a repo checkout (game/config/content_registry.gd not found)");
    process.exit(1);
  }
  if (process.argv.includes("--write")) {
    writeFileSync(PACK_KINDS_FILE, `${JSON.stringify({ kinds }, null, 1)}\n`);
    console.log(`pack_kinds: wrote ${kinds.length} kinds to ${PACK_KINDS_FILE}`);
  } else {
    console.log(JSON.stringify(kinds));
  }
}
