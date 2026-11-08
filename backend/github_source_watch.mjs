import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const SAFE_REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const SAFE_BRANCH = /^[A-Za-z0-9._\/-]{1,190}$/;
const GIT_SHA = /^[a-f0-9]{40}$/;

function enabled(value) {
  return /^(1|true|yes|on)$/i.test(String(value || ""));
}

function disabled(value) {
  return /^(0|false|no|off)$/i.test(String(value || ""));
}

function githubToken(env) {
  return String(env.LASTROUTE_GITHUB_TOKEN || env.GITHUB_TOKEN
    || env.GH_TOKEN || env.ACCESS_TOKEN || "").trim();
}

export function githubSourceWatchEnabled(env = process.env) {
  const setting = String(env.LASTROUTE_RESTART_ON_GITHUB_UPDATE || "").trim();
  return !disabled(setting) && (enabled(setting) || Boolean(githubToken(env)));
}

export async function checkGithubSourceRevision({
  repository = "KyttuXD/LastRouteGodot",
  branch = "main",
  token = "",
  workingDirectory = process.cwd(),
  deployedRevision = "",
  fetchImpl = globalThis.fetch,
  execFileImpl = execFileAsync,
} = {}) {
  if (!SAFE_REPOSITORY.test(String(repository))) throw new Error("invalid GitHub repository");
  if (!SAFE_BRANCH.test(String(branch)) || String(branch).includes("..")) {
    throw new Error("invalid GitHub branch");
  }
  if (typeof fetchImpl !== "function") throw new Error("fetch is unavailable; Node 18 or newer is required");

  let local = String(deployedRevision).trim().toLowerCase();
  if (!local) {
    const result = await execFileImpl("git", ["rev-parse", "HEAD"], {
      cwd: workingDirectory,
      timeout: 10_000,
    });
    local = String(result.stdout || "").trim().toLowerCase();
  }
  if (!GIT_SHA.test(local)) throw new Error("could not determine the deployed Git commit");

  const response = await fetchImpl(
    `https://api.github.com/repos/${repository}/commits/${encodeURIComponent(branch)}`,
    {
      headers: {
        accept: "application/vnd.github+json",
        "user-agent": "last-route-backend",
        "x-github-api-version": "2022-11-28",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
    },
  );
  if (!response.ok) throw new Error(`GitHub source check failed (HTTP ${response.status})`);
  const remote = String((await response.json())?.sha || "").trim().toLowerCase();
  if (!GIT_SHA.test(remote)) throw new Error("GitHub returned an invalid commit revision");
  return { changed: local !== remote, local, remote };
}

export function checkGithubSourceFromEnvironment(env = process.env, options = {}) {
  return checkGithubSourceRevision({
    repository: String(env.LASTROUTE_GITHUB_REPO || "KyttuXD/LastRouteGodot").trim(),
    branch: String(env.LASTROUTE_GITHUB_BRANCH || "main").trim(),
    token: githubToken(env),
    workingDirectory: process.cwd(),
    deployedRevision: String(env.LASTROUTE_DEPLOYED_REVISION || "").trim(),
    ...options,
  });
}
