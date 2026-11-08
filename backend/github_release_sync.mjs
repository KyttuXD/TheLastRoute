import { createHash } from "node:crypto";
import { basename, dirname, join, resolve } from "node:path";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { publishZipDirectory } from "./zip_archive.mjs";

const SAFE_REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const SAFE_TAG = /^(debug|release)$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const WEB_REVISION_MARKER = ".last-route-web-release";

function enabled(value) {
  return /^(1|true|yes|on)$/i.test(String(value || ""));
}

function disabled(value) {
  return /^(0|false|no|off)$/i.test(String(value || ""));
}

function headers(token, accept = "application/vnd.github+json") {
  return {
    accept,
    "user-agent": "last-route-backend",
    "x-github-api-version": "2022-11-28",
    ...(token ? { authorization: `Bearer ${token}` } : {}),
  };
}

async function responseBytes(response, label) {
  if (!response.ok) throw new Error(`${label} failed (HTTP ${response.status})`);
  return Buffer.from(await response.arrayBuffer());
}

async function fileSha256(path) {
  try {
    return createHash("sha256").update(await readFile(path)).digest("hex");
  } catch (error) {
    if (error.code === "ENOENT") return "";
    throw error;
  }
}

async function atomicWrite(path, bytes) {
  const temporary = `${path}.part-${process.pid}`;
  try {
    await writeFile(temporary, bytes);
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}

function channelFiles(manifest, channel) {
  const files = new Map();
  for (const entry of Object.values(manifest?.[channel] || {})) {
    const file = String(entry?.file || "");
    const sha256 = String(entry?.sha256 || "").toLowerCase();
    if (!SAFE_FILE.test(file) || basename(file) !== file || !SHA256.test(sha256)) {
      throw new Error(`${channel} update manifest contains an invalid file or SHA-256`);
    }
    files.set(file, sha256);
  }
  if (!files.size) throw new Error(`${channel} update manifest contains no artifacts`);
  return files;
}

function channelRevision(manifest, channel) {
  const revisions = new Set(Object.values(manifest?.[channel] || {})
    .map((entry) => String(entry?.revision || "").trim().toLowerCase())
    .filter((revision) => /^[a-f0-9]{7,40}$/.test(revision)));
  if (revisions.size > 1) throw new Error(`${channel} update manifest mixes build revisions`);
  return revisions.values().next().value || "";
}

export async function syncGithubRelease({
  repository,
  tag = "debug",
  token = "",
  updateDirectory,
  manifestPath,
  publicDirectory = "",
  fetchImpl = globalThis.fetch,
  logger = console,
} = {}) {
  if (!SAFE_REPOSITORY.test(String(repository || ""))) throw new Error("invalid GitHub repository");
  if (!SAFE_TAG.test(String(tag || ""))) throw new Error("invalid GitHub release tag");
  if (typeof fetchImpl !== "function") throw new Error("fetch is unavailable; Node 18 or newer is required");

  const directory = resolve(updateDirectory);
  const destinationManifest = resolve(manifestPath);
  await mkdir(directory, { recursive: true });
  await mkdir(dirname(destinationManifest), { recursive: true });

  const releaseResponse = await fetchImpl(
    `https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(tag)}`,
    { headers: headers(token) },
  );
  const releaseBytes = await responseBytes(releaseResponse, `GitHub release ${tag}`);
  const release = JSON.parse(releaseBytes.toString("utf8"));
  const assets = new Map((release.assets || []).map((asset) => [asset.name, asset]));
  const manifestName = `updates.${tag}.json`;
  const manifestAsset = assets.get(manifestName);
  if (!manifestAsset?.url) throw new Error(`${manifestName} is missing from GitHub release ${tag}`);

  const manifestResponse = await fetchImpl(manifestAsset.url, {
    headers: headers(token, "application/octet-stream"),
  });
  const manifestBytes = await responseBytes(manifestResponse, manifestName);
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  const wanted = channelFiles(manifest, tag);
  const revision = channelRevision(manifest, tag);
  let downloaded = 0;
  let reused = 0;

  for (const [file, expectedSha256] of wanted) {
    const asset = assets.get(file);
    if (!asset?.url) throw new Error(`${file} is missing from GitHub release ${tag}`);
    const destination = join(directory, file);
    if (await fileSha256(destination) === expectedSha256) {
      reused += 1;
      continue;
    }
    const response = await fetchImpl(asset.url, {
      headers: headers(token, "application/octet-stream"),
    });
    const bytes = await responseBytes(response, file);
    const actualSha256 = createHash("sha256").update(bytes).digest("hex");
    if (actualSha256 !== expectedSha256) {
      throw new Error(`${file} failed SHA-256 verification`);
    }
    await atomicWrite(destination, bytes);
    downloaded += 1;
  }

  let webPublished = false;
  const webFile = String(manifest?.[tag]?.web?.file || "");
  if (publicDirectory && SAFE_FILE.test(webFile) && wanted.has(webFile)) {
    const webIdentity = `${tag}:${revision}:${wanted.get(webFile)}`;
    let publishedIdentity = "";
    try {
      publishedIdentity = (await readFile(
        join(resolve(publicDirectory), WEB_REVISION_MARKER), "utf8",
      )).trim();
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    if (publishedIdentity !== webIdentity) {
      const published = await publishZipDirectory(
        join(directory, webFile), publicDirectory,
        { markerName: WEB_REVISION_MARKER, markerContents: `${webIdentity}\n` },
      );
      webPublished = true;
      logger.log(`Web build ready: ${published.files} files in ${published.directory}`);
    }
  }

  // Publish the manifest only after every referenced artifact is verified.
  await atomicWrite(destinationManifest, manifestBytes);
  const revisionLabel = revision ? revision.slice(0, 12) : "revision not reported";
  logger.log(`GitHub ${tag} sync ready @ ${revisionLabel}: ${downloaded} downloaded, ${reused} unchanged`);
  return { downloaded, reused, revision, webPublished,
    manifestPath: destinationManifest, updateDirectory: directory };
}

export async function syncGithubReleaseFromEnvironment(env = process.env, options = {}) {
  if (!githubReleaseSyncEnabled(env)) return null;
  const token = String(env.LASTROUTE_GITHUB_TOKEN || env.GITHUB_TOKEN
    || env.GH_TOKEN || env.ACCESS_TOKEN || "").trim();

  const updateDirectory = resolve(process.cwd(), env.LASTROUTE_UPDATE_DIR || "updates");
  const tag = String(env.LASTROUTE_GITHUB_TAG || "debug").trim();
  const manifestPath = resolve(process.cwd(), env.LASTROUTE_UPDATE_MANIFEST
    || join(updateDirectory, `updates.${tag}.json`));
  const publicDirectory = String(env.LASTROUTE_PUBLIC_DIR || "").trim();
  const result = await syncGithubRelease({
    repository: String(env.LASTROUTE_GITHUB_REPO || "KyttuXD/LastRouteGodot").trim(),
    tag,
    token,
    updateDirectory,
    manifestPath,
    publicDirectory: publicDirectory ? resolve(process.cwd(), publicDirectory) : "",
    ...options,
  });
  env.LASTROUTE_UPDATE_DIR ||= updateDirectory;
  env.LASTROUTE_UPDATE_MANIFEST ||= manifestPath;
  return result;
}

export function githubReleaseSyncEnabled(env = process.env) {
  const token = String(env.LASTROUTE_GITHUB_TOKEN || env.GITHUB_TOKEN
    || env.GH_TOKEN || env.ACCESS_TOKEN || "").trim();
  const setting = String(env.LASTROUTE_AUTO_UPDATE || "").trim();
  return !disabled(setting) && (enabled(setting) || Boolean(token));
}
