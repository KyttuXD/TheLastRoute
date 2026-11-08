// Deploy one mod pack: copy its files to VPS 2 and register it in the VPS 3 pack index.
//
//   node backend/deploy_mod_pack.mjs <pack-dir> --vps2 <ssh-host> --index <https://vps3-host> [--remote-root /srv/lr3d/mods]
//                                    [--replace] [--apply]
//   node backend/deploy_mod_pack.mjs <pack-dir> --local-root <dir> --index <url> --apply      (tests / a local mirror)
//
// Without --apply it only prints the plan (files, sizes, hashes, destination, index call). With --apply:
//   1. every file in <pack-dir> is hashed; pack.json gives id/name/version/author; caps from mod_packs.mjs are enforced
//   2. the files are copied to <remote-root>/<id>/<version>.uploading-<pid> on VPS 2, re-hashed THERE (sha256sum) and
//      compared, then moved into <remote-root>/<id>/<version> in one rename; an existing version is refused unless --replace
//   3. the pack (with its file list) is POSTed to <index>/api/admin/mods/register
// The owner session token comes only from the environment (LASTROUTE_OWNER_SESSION), never from arguments or files.
// Layout on VPS 2: <remote-root>/<pack_id>/<version>/<files>, served as <LASTROUTE_MOD_DOWNLOAD_BASE>/<pack_id>/<version>/<file>.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { MOD_LIMITS, validatePackManifest } from "./mod_packs.mjs";

export const DEFAULT_REMOTE_ROOT = "/srv/lr3d/mods";

function walk(directory, root = directory, out = []) {
  for (const name of readdirSync(directory).sort()) {
    if (name.startsWith(".")) continue;   // no dotfiles (.DS_Store, .git, editor droppings)
    const full = join(directory, name);
    const stat = statSync(full);
    if (stat.isDirectory()) walk(full, root, out);
    else if (stat.isFile()) out.push(relative(root, full).split(sep).join("/"));
  }
  return out;
}

export function buildPack(directory, limits = MOD_LIMITS) {
  const manifestPath = join(directory, "pack.json");
  if (!existsSync(manifestPath)) throw new Error("pack.json missing in " + directory);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const files = walk(directory).map((path) => {
    const data = readFileSync(join(directory, ...path.split("/")));
    return { path, size: data.length, sha256: createHash("sha256").update(data).digest("hex") };
  });
  // a pack.json "files" list (optional) must list every file except pack.json itself, with exact sha256 and size --
  // the client loader refuses the pack otherwise
  if (Array.isArray(manifest.files)) {
    const actual = new Map(files.filter((f) => f.path !== "pack.json").map((f) => [f.path, f]));
    const listed = new Map(manifest.files.map((f) => [String(f?.path || ""), f]));
    for (const [path, file] of actual) {
      const entry = listed.get(path);
      if (!entry) throw new Error(`pack.json "files" does not list ${path}`);
      if (String(entry.sha256 || "").toLowerCase() !== file.sha256) throw new Error(`pack.json lists ${path} with a sha256 that does not match the file`);
      if (Number(entry.size) !== file.size) throw new Error(`pack.json lists ${path} with the wrong size`);
    }
    for (const path of listed.keys()) if (!actual.has(path)) throw new Error(`pack.json "files" lists ${path}, which is not in the pack`);
  }
  const checked = validatePackManifest({ ...manifest, files }, limits);
  if (checked.error) throw new Error(`pack refused: ${checked.error}${checked.path ? ` (${checked.path})` : ""}`);
  return checked.pack;
}

function sh(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

// VPS 2 over ssh/scp (BatchMode: keys only, never a password prompt)
export function sshTransport(host) {
  const ssh = (command) => execFileSync("ssh", ["-o", "BatchMode=yes", host, command], { encoding: "utf8" });
  return {
    describe: `ssh ${host}`,
    exists: (path) => ssh(`test -e ${sh(path)} && echo yes || echo no`).trim() === "yes",
    upload: (from, to) => {
      ssh(`mkdir -p ${sh(to)}`);
      execFileSync("scp", ["-o", "BatchMode=yes", "-q", "-r", `${from}/.`, `${host}:${to}/`]);
    },
    hashes: (dir, paths) => {
      const out = ssh(`cd ${sh(dir)} && sha256sum -- ${paths.map(sh).join(" ")}`);
      return new Map(out.trim().split("\n").map((line) => {
        const [hash, ...rest] = line.trim().split(/\s+/);
        return [rest.join(" ").replace(/^\*/, ""), hash];
      }));
    },
    promote: (from, to, replace) => ssh(`mkdir -p ${sh(to.replace(/\/[^/]+$/, ""))} && `
      + (replace ? `rm -rf ${sh(to)} && ` : `test ! -e ${sh(to)} && `) + `mv ${sh(from)} ${sh(to)}`),
    discard: (path) => ssh(`rm -rf ${sh(path)}`),
  };
}

// a local folder standing in for VPS 2 (tests, or a mirror served by any static web server)
export function localTransport() {
  const hashFile = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
  return {
    describe: "local folder",
    exists: (path) => existsSync(path),
    upload: (from, to) => { mkdirSync(to, { recursive: true }); cpSync(from, to, { recursive: true, filter: (p) => !p.split(sep).pop().startsWith(".") || p === from }); },
    hashes: (dir, paths) => new Map(paths.map((p) => [p, existsSync(join(dir, p)) ? hashFile(join(dir, p)) : ""])),
    promote: (from, to, replace) => {
      mkdirSync(join(to, ".."), { recursive: true });
      if (existsSync(to)) {
        if (!replace) throw new Error("version already on the file server: " + to);
        rmSync(to, { recursive: true, force: true });
      }
      renameSync(from, to);
    },
    discard: (path) => rmSync(path, { recursive: true, force: true }),
  };
}

export async function deployPack({ directory, transport, remoteRoot = DEFAULT_REMOTE_ROOT, indexUrl, sessionToken,
  replace = false, apply = false, log = console.log, fetchImpl = fetch }) {
  const pack = buildPack(directory);
  const destination = `${remoteRoot.replace(/\/+$/, "")}/${pack.id}/${pack.version}`;
  log(`pack ${pack.id} ${pack.version} "${pack.name}" by ${pack.author}: ${pack.files.length} file(s), ${pack.totalBytes} bytes`);
  for (const file of pack.files) log(`  ${file.sha256}  ${String(file.size).padStart(10)}  ${file.path}`);
  log(`files   -> ${transport.describe}: ${destination}/`);
  log(`index   -> POST ${indexUrl}/api/admin/mods/register${replace ? " (replace)" : ""}`);
  if (!apply) {
    log("dry run: nothing copied or registered (add --apply)");
    return { pack, destination, applied: false };
  }
  if (!sessionToken) throw new Error("LASTROUTE_OWNER_SESSION is not set (owner session token, environment only)");
  if (transport.exists(destination) && !replace) throw new Error(`${destination} already exists; bump the version or pass --replace`);
  const staging = `${destination}.uploading-${process.pid}`;
  try {
    transport.upload(directory, staging);
    const remote = transport.hashes(staging, pack.files.map((f) => f.path));
    for (const file of pack.files) {
      if (remote.get(file.path) !== file.sha256) throw new Error(`upload check failed for ${file.path}: hash differs on the file server`);
    }
    transport.promote(staging, destination, replace);
  } catch (error) {
    try { transport.discard(staging); } catch { /* keep the original error */ }
    throw error;
  }
  log("files copied and verified on the file server");
  const manifest = JSON.parse(readFileSync(join(directory, "pack.json"), "utf8"));
  const response = await fetchImpl(`${indexUrl.replace(/\/+$/, "")}/api/admin/mods/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ sessionToken, replace, pack: { ...manifest, files: pack.files } }),
  });
  const result = await response.json().catch(() => ({}));
  if (!response.ok || !result.ok) {
    throw new Error(`index refused the pack (${response.status} ${result.code || ""}); files stay at ${destination}`);
  }
  log(`registered: ${result.pack.id} ${result.pack.version} -> ${result.pack.downloadBase || "(download base not configured on the index)"}`);
  return { pack, destination, applied: true, index: result.pack };
}

function parseArgs(argv) {
  const options = { replace: false, apply: false, remoteRoot: DEFAULT_REMOTE_ROOT };
  const rest = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--replace") options.replace = true;
    else if (arg === "--apply") options.apply = true;
    else if (arg === "--vps2") options.vps2 = argv[++i];
    else if (arg === "--local-root") options.localRoot = argv[++i];
    else if (arg === "--remote-root") options.remoteRoot = argv[++i];
    else if (arg === "--index") options.indexUrl = argv[++i];
    else rest.push(arg);
  }
  options.directory = rest[0];
  return options;
}

const entry = process.argv[1] ? fileURLToPath(import.meta.url) === process.argv[1] : false;
if (entry) {
  const options = parseArgs(process.argv.slice(2));
  if (!options.directory || !options.indexUrl || (!options.vps2 && !options.localRoot)) {
    console.error("usage: node deploy_mod_pack.mjs <pack-dir> (--vps2 <ssh-host> | --local-root <dir>) --index <url> [--remote-root DIR] [--replace] [--apply]");
    process.exit(2);
  }
  deployPack({
    directory: options.directory,
    transport: options.localRoot ? localTransport() : sshTransport(options.vps2),
    remoteRoot: options.localRoot || options.remoteRoot,
    indexUrl: options.indexUrl,
    sessionToken: process.env.LASTROUTE_OWNER_SESSION || "",
    replace: options.replace,
    apply: options.apply,
  }).catch((error) => { console.error(`deploy failed: ${error.message}`); process.exit(1); });
}
