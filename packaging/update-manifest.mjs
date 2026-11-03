// Sets one platform entry in the VPS update manifest. Run on the VPS by packaging/publish-vps.sh:
//   node - CHANNEL PLATFORM FILE VERSION BUILD REVISION SHA256 < update-manifest.mjs
// Plain CommonJS, because the VPS runs Node 18 and reads stdin as a script (no import syntax).
const { readFileSync, writeFileSync } = require("node:fs");

const MANIFEST = "/var/lib/lastroute-rust/update-manifest.json";
const [channel, platform, file, version, build, revision, sha256] = process.argv.slice(2);
if (![channel, platform, file, version, build, revision, sha256].every(Boolean)) {
  console.error("usage: node - CHANNEL PLATFORM FILE VERSION BUILD REVISION SHA256");
  process.exit(2);
}
let manifest = {};
try {
  manifest = JSON.parse(readFileSync(MANIFEST, "utf8"));
} catch {
  manifest = {};
}
manifest[channel] = manifest[channel] || {};
manifest[channel][platform] = { version, build: Number(build), revision, file, sha256 };
writeFileSync(MANIFEST, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`manifest: ${channel}/${platform} -> ${file} (build ${build})`);
