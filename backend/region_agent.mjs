#!/usr/bin/env node
// Region agent (docs/SERVER_HOSTING.md): one per VPS. Reports this region's load and dedicated hosts to the
// directory every REPORT_MS, signed (regions.mjs), and carries out the spawn / stop requests the directory answers
// with. It never listens on a port, never holds a secret but its own token, and only ever signals its own children
// by exact PID.
//
// Environment (deploy/region-agent.env.example; the token lives only in /etc/lastroute-agent/agent.env):
//   LASTROUTE_REGION            virginia | texas | california ...
//   LASTROUTE_REGION_LABEL      shown to players ("US East")
//   LASTROUTE_AGENT_TOKEN       shared with the directory's LASTROUTE_AGENT_TOKENS entry for this region
//   LASTROUTE_DIRECTORY_URL     http://lastroute.kyttuslab.org:8105
//   LASTROUTE_GODOT_BIN         the headless game binary (a Linux export of the game)
//   LASTROUTE_GAME_PATH         optional --path for a project checkout instead of an exported binary
//   LASTROUTE_RUST_BIN          the Rust port's last_route binary; a room created with {build: "rust"} runs it
//                               (`last_route --headless -- --dedicated-host CODE ...`), every other room the Godot host
//   LASTROUTE_RELAY_ADDRESS / LASTROUTE_RELAY_PORT   this region's relay, advertised to clients (stage c)
//   LASTROUTE_ROOM_MB           RAM budget per dedicated host (default 700)
//   LASTROUTE_MAX_ROOMS         hard cap (default: from cores and RAM)
//   LASTROUTE_ROOMS_PER_CORE    rooms per spare core (default 1; headless rooms are light)
//   LASTROUTE_PORT_BASE         first local ENet port for dedicated hosts (default 18100)

import { spawn } from "node:child_process";
import { cpus, freemem, loadavg, totalmem } from "node:os";
import { readFileSync } from "node:fs";
import { signAgentRequest } from "./regions.mjs";

export const REPORT_MS = 1_000;
export const STOP_GRACE_MS = 10_000;

// How many dedicated hosts this machine may run: one core is kept for the system (and the relay), and each room gets
// LASTROUTE_ROOM_MB of RAM with 1 GB left over. A busy machine (load above 80% of the cores) takes no new rooms.
export function capacity({ cores, totalMb, roomMb = 700, maxRooms = 0, roomsPerCore = 1 }) {
  const byCpu = Math.max(0, cores - 1) * Math.max(1, roomsPerCore);
  const byRam = Math.max(0, Math.floor((totalMb - 1024) / roomMb));
  const cap = Math.min(byCpu, byRam);
  return maxRooms > 0 ? Math.min(cap, maxRooms) : cap;
}

export function canSpawn({ running, cap, load1, cores, freeMb, roomMb }) {
  return running < cap && load1 <= cores * 0.8 && freeMb >= roomMb;
}

// What the machine has right now (tests pass their own).
export function machine() {
  return { cores: cpus().length, totalMb: Math.round(totalmem() / 1048576), freeMb: Math.round(freemem() / 1048576),
    load1: loadavg()[0] };
}

export function createAgent(env = process.env, { spawnProcess = spawn, fetchImpl = fetch, now = Date.now,
  log = console.log, resources = machine } = {}) {
  const region = String(env.LASTROUTE_REGION || "");
  const token = String(env.LASTROUTE_AGENT_TOKEN || "");
  const directory = String(env.LASTROUTE_DIRECTORY_URL || "").replace(/\/$/, "");
  const roomMb = Number(env.LASTROUTE_ROOM_MB) || 700;
  const portBase = Number(env.LASTROUTE_PORT_BASE) || 18100;
  const hosts = new Map();   // code -> { child, pid, port, started }
  let reporting = false;

  function cap() {
    const m = resources();
    return capacity({ cores: m.cores, totalMb: m.totalMb, roomMb, maxRooms: Number(env.LASTROUTE_MAX_ROOMS) || 0,
      roomsPerCore: Number(env.LASTROUTE_ROOMS_PER_CORE) || 1 });
  }

  function freePort() {
    const used = new Set([...hosts.values()].map((host) => host.port));
    for (let port = portBase; port < portBase + 200; port += 1) if (!used.has(port)) return port;
    return 0;
  }

  function rssMb(pid) {
    try {
      const status = readFileSync(`/proc/${pid}/status`, "utf8");
      return Math.round(Number(/VmRSS:\s+(\d+)/.exec(status)?.[1] || 0) / 1024);
    } catch {
      return 0;
    }
  }

  function spawnHost(code, options = {}) {
    if (hosts.has(code)) return `already hosting ${code}`;
    const running = hosts.size;
    const m = resources();
    if (!canSpawn({ running, cap: cap(), load1: m.load1, cores: m.cores, freeMb: m.freeMb, roomMb })) {
      return `no capacity for ${code} (${running}/${cap()} rooms, load ${m.load1.toFixed(2)}, ${m.freeMb} MB free)`;
    }
    const port = freePort();
    if (!port) return "no free local port";
    // a room that asks for the Rust build runs the Rust port's headless host when this machine has one
    // (LASTROUTE_RUST_BIN); otherwise, and for every other room, the Godot host
    const rust = options.build === "rust" && Boolean(env.LASTROUTE_RUST_BIN);
    const args = ["--headless"];
    if (env.LASTROUTE_GAME_PATH && !rust) args.push("--path", env.LASTROUTE_GAME_PATH);
    args.push("--", "--dedicated-host", code, "--port", String(port),
      "--idle-minutes", String(Number(options.idleMinutes) || 10));
    if (options.seed) args.push("--seed", String(options.seed));
    // every hosted room is a WebSocket room, so browsers can join it as well as desktop builds
    args.push("--ws-rooms");
    // the Rust host takes 0 (RELAXED) too: leaving it out would start the room on its default, STANDARD
    if (options.difficulty || (rust && Number.isInteger(options.difficulty))) args.push("--difficulty", String(options.difficulty));
    if (directory.startsWith("http://127.0.0.1:") || directory.startsWith("http://localhost:")) {
      args.push(`--backend=${directory}`);   // a local directory (tests); the game's built-in one otherwise
    }
    // the ticket and region go through the environment, never the command line (ps shows arguments)
    const child = spawnProcess(rust ? env.LASTROUTE_RUST_BIN : env.LASTROUTE_GODOT_BIN || "godot", args, {
      env: { ...process.env, LASTROUTE_SPAWN_TICKET: String(options.spawnTicket || ""), LASTROUTE_REGION: region,
        LASTROUTE_AGENT_TOKEN: "" },
      stdio: ["ignore", "inherit", "inherit"],
    });
    hosts.set(code, { child, pid: child.pid, port, started: now() });
    child.on("exit", (exitCode, signal) => {
      if (hosts.get(code)?.child === child) hosts.delete(code);
      log(`[agent] ${code} (pid ${child.pid}) exited ${signal || exitCode}`);
    });
    log(`[agent] ${code} spawned (pid ${child.pid}, port ${port})`);
    return "";
  }

  function stopHost(code) {
    const host = hosts.get(code);
    if (!host) return `not hosting ${code}`;
    host.child.kill("SIGTERM");   // exact child; the game unregisters on its way out
    setTimeout(() => {
      if (hosts.get(code)?.child === host.child) host.child.kill("SIGKILL");
    }, STOP_GRACE_MS).unref();
    return "";
  }

  function status() {
    return {
      label: env.LASTROUTE_REGION_LABEL || region,
      relay: env.LASTROUTE_RELAY_ADDRESS
        ? { address: env.LASTROUTE_RELAY_ADDRESS, port: Number(env.LASTROUTE_RELAY_PORT) || 0 } : null,
      capacity: cap(),
      load: resources().load1,
      cores: resources().cores,
      memFreeMb: resources().freeMb,
      rooms: [...hosts.entries()].map(([code, host]) => ({ code, pid: host.pid, uptime: Math.round((now() - host.started) / 1000),
        rssMb: rssMb(host.pid) })),
      version: "region-agent/1",
    };
  }

  async function reportOnce() {
    if (reporting) return [];
    reporting = true;
    try {
      const body = JSON.stringify(status());
      const time = now();
      const response = await fetchImpl(`${directory}/api/regions/report`, {
        method: "POST", body,
        headers: { "content-type": "application/json", "x-lr-region": region, "x-lr-time": String(time),
          "x-lr-signature": signAgentRequest(token, region, time, body) },
        signal: AbortSignal.timeout(8_000),
      });
      const answer = await response.json().catch(() => ({}));
      if (!response.ok) {
        log(`[agent] report refused: ${response.status} ${answer.error || ""}`);
        return [];
      }
      const done = [];
      for (const action of answer.actions || []) {
        const problem = action.action === "spawn" ? spawnHost(String(action.code), action.options || {})
          : action.action === "stop" ? stopHost(String(action.code)) : `unknown action ${action.action}`;
        if (problem) log(`[agent] ${action.action} ${action.code}: ${problem}`);
        done.push({ ...action, problem, options: undefined });
      }
      return done;
    } catch (error) {
      log(`[agent] directory unreachable: ${error.message}`);
      return [];
    } finally {
      reporting = false;
    }
  }

  function stopAll() {
    for (const code of [...hosts.keys()]) stopHost(code);
  }

  return { region, reportOnce, spawnHost, stopHost, stopAll, status, hosts, cap };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const agent = createAgent();
  if (!agent.region || !process.env.LASTROUTE_AGENT_TOKEN || !process.env.LASTROUTE_DIRECTORY_URL) {
    console.error("region agent: set LASTROUTE_REGION, LASTROUTE_AGENT_TOKEN and LASTROUTE_DIRECTORY_URL");
    process.exit(2);
  }
  console.log(`[agent] region ${agent.region}: capacity ${agent.cap()} dedicated hosts`);
  agent.reportOnce();
  const timer = setInterval(() => agent.reportOnce(), REPORT_MS);
  for (const signal of ["SIGTERM", "SIGINT"]) {
    process.on(signal, () => {
      clearInterval(timer);
      agent.stopAll();
      setTimeout(() => process.exit(0), STOP_GRACE_MS + 500).unref();
    });
  }
}
