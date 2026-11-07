// Native build manifests for the launcher (docs/LAUNCHER.md). Zero dependencies, Node 18+, run on KT's PC.
//
//   node packaging/native/manifest.mjs keygen KEYFILE
//       Makes an ed25519 signing key. KEYFILE (PEM, private) must be OUTSIDE the repository and must not exist yet.
//       Prints the public key (64 hex digits) for crates/lr_update/keys/trusted.txt.
//   node packaging/native/manifest.mjs build --platform P --build N --revision SHA --version V --src DIR --run REL --out DIR
//        [--key KEYFILE | --no-sign] [--min-os 11.0] [--launcher-min 1] [--web-build SHA] [--notes TEXT] [--built ISO]
//       Compresses every file of DIR (zlib, one .z per file) into OUT, and writes OUT/manifest.json (schema lr.native/1)
//       and OUT/manifest.json.sig (ed25519 over the exact manifest bytes, 128 hex digits). OUT is the builds/N folder.
//   node packaging/native/manifest.mjs latest --platform-dir DIR --build N
//       Writes DIR/latest.json {"build":N,"manifest":"builds/N/manifest.json","sha256":...}. Publish this LAST.
//   node packaging/native/manifest.mjs verify --platform-dir DIR (--pub HEX | --key KEYFILE) [--platform P]
//       Re-checks a whole published tree the way the launcher does: pointer, manifest hash, signature, every file.
//
// The rules for file names and sizes are the same as crates/lr_update/src/manifest.rs; that file is the reference.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const SCHEMA = "lr.native/1";
const MAX_FILES = 2000;
const MAX_FILE_BYTES = 512 * 1024 * 1024;
const MAX_TOTAL_BYTES = 1024 * 1024 * 1024;
const PLATFORMS = ["windows-x86_64", "macos-universal", "linux-x86_64", "linux-aarch64"];
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex"); // DER prefix of an ed25519 public key
const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");

function fail(message) {
  console.error(`manifest.mjs: ${message}`);
  process.exit(1);
}

function sha256(data) {
  return crypto.createHash("sha256").update(data).digest("hex");
}

/** Why a file name is not allowed, or null (the same rules as bad_path in manifest.rs). */
export function badPath(p) {
  if (!p) return "empty path";
  if (p.length > 240) return "path too long";
  if (p.startsWith("/") || p.endsWith("/")) return "absolute path or trailing slash";
  // eslint-disable-next-line no-control-regex
  if (/[\x00-\x1f\x7f\\:<>"|?*]/.test(p)) return "forbidden character";
  for (const part of p.split("/")) {
    if (part === "") return "empty path part";
    if (part === "." || part === "..") return "dot path part";
    if (part.startsWith(".")) return "hidden path part";
    if (part.endsWith(".") || part.endsWith(" ")) return "path part ends with a dot or space";
    const stem = part.split(".")[0].toUpperCase();
    if (["CON", "PRN", "AUX", "NUL"].includes(stem) || /^(COM|LPT)[0-9]$/.test(stem)) return "reserved Windows name";
  }
  return null;
}

/** 755 for the program and anything in a bin/ or MacOS/ folder or a shell script; 644 for the rest. */
export function modeFor(rel, run) {
  return rel === run || /(^|\/)(bin|MacOS)\/[^/]+$/.test(rel) || rel.endsWith(".sh") ? "755" : "644";
}

function walk(root, dir = root, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
    const full = path.join(dir, entry.name);
    if (entry.isSymbolicLink()) fail(`${full} is a symbolic link; the manifest has plain files only`);
    if (entry.isDirectory()) walk(root, full, out);
    else if (entry.isFile()) out.push(path.relative(root, full).split(path.sep).join("/"));
    else fail(`${full} is not a plain file`);
  }
  return out;
}

function parseArgs(argv) {
  const opts = {};
  const positional = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith("--")) {
      const name = argv[i].slice(2);
      if (["no-sign"].includes(name)) opts[name] = true;
      else {
        if (i + 1 >= argv.length) fail(`--${name} needs a value`);
        opts[name] = argv[++i];
      }
    } else positional.push(argv[i]);
  }
  return { opts, positional };
}

function need(opts, ...names) {
  for (const n of names) if (opts[n] === undefined || opts[n] === "") fail(`--${n} is required`);
}

function publicHex(publicKey) {
  return Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url").toString("hex");
}

function loadPublic(opts) {
  if (opts.pub) {
    const raw = Buffer.from(opts.pub, "hex");
    if (raw.length !== 32) fail("--pub must be 64 hex digits");
    return crypto.createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: "der", type: "spki" });
  }
  if (opts.key) return crypto.createPublicKey(crypto.createPrivateKey(fs.readFileSync(opts.key)));
  fail("--pub or --key is required");
}

function keygen(args) {
  const out = args.positional[0];
  if (!out) fail("usage: keygen KEYFILE");
  const abs = path.resolve(out);
  const rel = path.relative(repoRoot, abs);
  if (!rel.startsWith("..") && !path.isAbsolute(rel)) fail(`${abs} is inside the repository; the private key must live outside it`);
  if (fs.existsSync(abs)) fail(`${abs} already exists; refusing to overwrite a key`);
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  fs.writeFileSync(abs, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 });
  console.log(`private key written to ${abs} (back it up; losing it means shipping a new launcher with a new public key)`);
  console.log(`public key (add a line to crates/lr_update/keys/trusted.txt, then rebuild the launcher and the game):`);
  console.log(publicHex(publicKey));
}

function build(args) {
  const o = args.opts;
  need(o, "platform", "build", "revision", "version", "src", "run", "out");
  if (!PLATFORMS.includes(o.platform)) fail(`--platform must be one of ${PLATFORMS.join(", ")}`);
  const buildNumber = Number(o.build);
  if (!Number.isInteger(buildNumber) || buildNumber < 1) fail("--build must be a whole number of at least 1");
  if (!o["no-sign"] && !o.key) fail("--key KEYFILE is required (or --no-sign for a dry run)");
  const src = path.resolve(o.src);
  if (!fs.statSync(src, { throwIfNoEntry: false })?.isDirectory()) fail(`${src} is not a folder`);
  const files = walk(src);
  if (files.length === 0 || files.length > MAX_FILES) fail(`${files.length} files (1 to ${MAX_FILES} are allowed)`);
  if (!files.includes(o.run)) fail(`--run ${o.run} is not one of the files`);
  const out = path.resolve(o.out);
  if (fs.existsSync(out) && fs.readdirSync(out).length > 0) fail(`${out} is not empty`);
  fs.mkdirSync(out, { recursive: true });
  const seen = new Set();
  let total = 0;
  const entries = files.map((rel, i) => {
    const why = badPath(rel);
    if (why) fail(`file name ${JSON.stringify(rel)} is not allowed: ${why}`);
    if (seen.has(rel.toLowerCase())) fail(`${rel} appears twice when case is ignored`);
    seen.add(rel.toLowerCase());
    const data = fs.readFileSync(path.join(src, ...rel.split("/")));
    if (data.length > MAX_FILE_BYTES) fail(`${rel} is larger than ${MAX_FILE_BYTES} bytes`);
    total += data.length;
    if (total > MAX_TOTAL_BYTES) fail("the build is larger than the allowed total");
    const z = zlib.deflateSync(data, { level: 9 });
    const zName = `f${i}.z`;
    fs.writeFileSync(path.join(out, zName), z);
    return { path: rel, size: data.length, sha256: sha256(data), z: zName, z_size: z.length, z_sha256: sha256(z), mode: modeFor(rel, o.run) };
  });
  const manifest = {
    schema: SCHEMA,
    platform: o.platform,
    build: buildNumber,
    version: o.version,
    revision: o.revision,
    built: o.built ?? new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    min_os: o["min-os"] ?? null,
    launcher_min: Number(o["launcher-min"] ?? 1),
    web_build: o["web-build"] ?? null,
    notes: o.notes ?? "",
    run: o.run,
    files: entries,
  };
  const text = `${JSON.stringify(manifest, null, 2)}\n`;
  fs.writeFileSync(path.join(out, "manifest.json"), text);
  if (!o["no-sign"]) {
    const key = crypto.createPrivateKey(fs.readFileSync(o.key));
    if (key.asymmetricKeyType !== "ed25519") fail("the key is not an ed25519 key");
    fs.writeFileSync(path.join(out, "manifest.json.sig"), `${crypto.sign(null, Buffer.from(text), key).toString("hex")}\n`);
  }
  const zTotal = entries.reduce((n, e) => n + e.z_size, 0);
  console.log(`build ${buildNumber} (${o.platform}): ${entries.length} files, ${total} bytes -> ${zTotal} bytes to download, manifest sha256 ${sha256(Buffer.from(text))}${o["no-sign"] ? " (NOT signed)" : ""}`);
}

function latest(args) {
  const o = args.opts;
  need(o, "platform-dir", "build");
  const n = Number(o.build);
  const manifestPath = path.join(o["platform-dir"], "builds", String(n), "manifest.json");
  if (!fs.existsSync(manifestPath)) fail(`${manifestPath} does not exist`);
  const pointer = { build: n, manifest: `builds/${n}/manifest.json`, sha256: sha256(fs.readFileSync(manifestPath)) };
  fs.writeFileSync(path.join(o["platform-dir"], "latest.json"), `${JSON.stringify(pointer)}\n`);
  console.log(`latest.json -> build ${n}`);
}

function verify(args) {
  const o = args.opts;
  need(o, "platform-dir");
  const root = path.resolve(o["platform-dir"]);
  const pub = loadPublic(o);
  const pointer = JSON.parse(fs.readFileSync(path.join(root, "latest.json"), "utf8"));
  if (pointer.manifest !== `builds/${pointer.build}/manifest.json`) fail("latest.json names a manifest outside builds/<build>/");
  const dir = path.join(root, "builds", String(pointer.build));
  const manifestBytes = fs.readFileSync(path.join(dir, "manifest.json"));
  if (sha256(manifestBytes) !== pointer.sha256) fail("manifest.json does not match latest.json");
  const sig = Buffer.from(fs.readFileSync(path.join(dir, "manifest.json.sig"), "utf8").trim(), "hex");
  if (!crypto.verify(null, manifestBytes, pub, sig)) fail("the signature does not verify");
  const m = JSON.parse(manifestBytes.toString("utf8"));
  if (m.schema !== SCHEMA) fail("wrong schema");
  if (o.platform && m.platform !== o.platform) fail(`the manifest is for ${m.platform}, not ${o.platform}`);
  if (m.build !== pointer.build) fail("manifest build differs from latest.json");
  for (const f of m.files) {
    const why = badPath(f.path);
    if (why) fail(`${f.path}: ${why}`);
    const z = fs.readFileSync(path.join(dir, f.z));
    if (z.length !== f.z_size || sha256(z) !== f.z_sha256) fail(`${f.path}: compressed file does not match`);
    const data = zlib.inflateSync(z, { maxOutputLength: f.size + 1 });
    if (data.length !== f.size || sha256(data) !== f.sha256) fail(`${f.path}: unpacked file does not match`);
  }
  console.log(`ok: build ${m.build} (${m.platform}), ${m.files.length} files verified, signature valid`);
}

const commands = { keygen, build, latest, verify };
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...rest] = process.argv.slice(2);
  if (!commands[command]) fail("usage: manifest.mjs keygen|build|latest|verify (see the comment at the top of the file)");
  commands[command](parseArgs(rest));
}
