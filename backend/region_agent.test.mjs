import test, { mock } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { signAgentRequest } from "./regions.mjs";
import { REPORT_MS, STOP_GRACE_MS, canSpawn, capacity, createAgent, machine } from "./region_agent.mjs";

const TOKEN = "virginia-test-token-0123456789abcdef";
const SCRIPT = fileURLToPath(new URL("./region_agent.mjs", import.meta.url));

function fakeSpawn() {
  const spawned = [];
  const spawnProcess = (bin, args, options) => {
    const child = new EventEmitter();
    child.pid = process.pid;
    child.signals = [];
    child.kill = (signal) => { child.signals.push(signal || "SIGTERM"); };
    spawned.push({ bin, args, options, child });
    return child;
  };
  return { spawned, spawnProcess };
}

function resources(overrides = {}) {
  return () => ({ cores: 12, totalMb: 32094, freeMb: 30000, load1: 0.2, ...overrides });
}

function agentEnv(overrides = {}) {
  return {
    LASTROUTE_REGION: "virginia",
    LASTROUTE_REGION_LABEL: "US East",
    LASTROUTE_AGENT_TOKEN: TOKEN,
    LASTROUTE_DIRECTORY_URL: "http://127.0.0.1:9/",
    LASTROUTE_GODOT_BIN: "/opt/lastroute/game.x86_64",
    LASTROUTE_GAME_PATH: "/opt/lastroute/project",
    LASTROUTE_RELAY_ADDRESS: "127.0.0.1",
    LASTROUTE_RELAY_PORT: "1105",
    LASTROUTE_MAX_ROOMS: "4",
    ...overrides,
  };
}

test("capacity keeps one core and 1 GB, and a busy or tight machine takes no new room", () => {
  assert.equal(REPORT_MS, 1_000);
  assert.equal(STOP_GRACE_MS, 10_000);
  assert.equal(capacity({ cores: 1, totalMb: 8192 }), 0);
  assert.equal(capacity({ cores: 4, totalMb: 1024 }), 0);
  assert.equal(capacity({ cores: 4, totalMb: 7941, maxRooms: 0 }), 3);
  assert.equal(capacity({ cores: 8, totalMb: 16384, maxRooms: -1 }), 7);
  assert.equal(capacity({ cores: 8, totalMb: 16384, roomMb: 700, maxRooms: 2 }), 2);
  assert.equal(canSpawn({ running: 2, cap: 3, load1: 4 * 0.8, cores: 4, freeMb: 700, roomMb: 700 }), true);
  assert.equal(canSpawn({ running: 2, cap: 3, load1: 4 * 0.8 + 0.01, cores: 4, freeMb: 9000, roomMb: 700 }), false);
  assert.equal(canSpawn({ running: 3, cap: 3, load1: 0, cores: 4, freeMb: 9000, roomMb: 700 }), false);
  assert.equal(canSpawn({ running: 0, cap: 3, load1: 0, cores: 4, freeMb: 699, roomMb: 700 }), false);
  const live = machine();
  assert.ok(live.cores >= 1);
  assert.equal(typeof live.totalMb, "number");
  assert.equal(typeof live.freeMb, "number");
  assert.equal(typeof live.load1, "number");
});

test("spawn puts the ticket in the environment, and stop signals only that child", () => {
  const { spawned, spawnProcess } = fakeSpawn();
  const logs = [];
  let clock = 10_000;
  const agent = createAgent(agentEnv(), {
    spawnProcess, log: (line) => logs.push(line), resources: resources(), now: () => clock,
  });
  assert.equal(agent.region, "virginia");
  assert.equal(agent.cap(), 4);
  const busy = createAgent(agentEnv({ LASTROUTE_MAX_ROOMS: "1" }), {
    spawnProcess, log() {}, resources: resources({ freeMb: 100 }), now: () => clock,
  });
  assert.match(busy.spawnHost("ROOM1"), /no capacity/);
  assert.equal(spawned.length, 0);

  assert.equal(agent.spawnHost("ROOM1", { seed: "KT SEED", difficulty: 2, idleMinutes: 15, spawnTicket: "ticket-value" }), "");
  assert.match(agent.spawnHost("ROOM1"), /already hosting ROOM1/);
  assert.equal(spawned.length, 1);
  const { bin, args, options } = spawned[0];
  assert.equal(bin, "/opt/lastroute/game.x86_64");
  assert.deepEqual(args.slice(0, 3), ["--headless", "--path", "/opt/lastroute/project"]);
  assert.ok(args.includes("--dedicated-host") && args[args.indexOf("--dedicated-host") + 1] === "ROOM1");
  assert.equal(args[args.indexOf("--port") + 1], "18100");
  assert.equal(args[args.indexOf("--idle-minutes") + 1], "15");
  assert.equal(args[args.indexOf("--seed") + 1], "KT SEED");
  assert.equal(args[args.indexOf("--difficulty") + 1], "2");
  assert.ok(args.includes("--backend=http://127.0.0.1:9"));
  assert.equal(args.join(" ").includes("ticket-value"), false);
  assert.equal(options.env.LASTROUTE_SPAWN_TICKET, "ticket-value");
  assert.equal(options.env.LASTROUTE_AGENT_TOKEN, "");
  assert.equal(options.env.LASTROUTE_REGION, "virginia");

  clock += 2_000;
  const status = agent.status();
  assert.equal(status.label, "US East");
  assert.deepEqual(status.relay, { address: "127.0.0.1", port: 1105 });
  assert.equal(status.rooms[0].code, "ROOM1");
  assert.equal(status.rooms[0].uptime, 2);
  assert.ok(status.rooms[0].rssMb > 0);
  assert.equal(status.version, "region-agent/1");

  assert.match(agent.stopHost("OTHER"), /not hosting OTHER/);
  assert.equal(agent.stopHost("ROOM1"), "");
  assert.deepEqual(spawned[0].child.signals, ["SIGTERM"]);
  spawned[0].child.emit("exit", 0, null);
  assert.equal(agent.hosts.size, 0);
  assert.match(logs.at(-1), /ROOM1 \(pid .*\) exited 0/);

  const plain = createAgent(agentEnv({
    LASTROUTE_DIRECTORY_URL: "https://directory.example",
    LASTROUTE_GAME_PATH: "",
    LASTROUTE_RELAY_ADDRESS: "",
  }), { spawnProcess, log() {}, resources: resources(), now: () => clock });
  assert.equal(plain.spawnHost("ROOM2", { difficulty: 0 }), "");
  const plainArgs = spawned[1].args;
  assert.equal(plainArgs.includes("--path"), false);
  assert.equal(plainArgs.some((arg) => String(arg).startsWith("--backend=")), false);
  assert.equal(plainArgs.includes("--difficulty"), false);
  assert.equal(plainArgs[plainArgs.indexOf("--idle-minutes") + 1], "10");
  assert.equal(plain.status().relay, null);
  plain.stopAll();
  assert.deepEqual(spawned[1].child.signals, ["SIGTERM"]);
});

test("a child that ignores SIGTERM is killed after the grace period", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  try {
    const { spawned, spawnProcess } = fakeSpawn();
    const agent = createAgent(agentEnv(), { spawnProcess, log() {}, resources: resources(), now: () => 0 });
    agent.spawnHost("ROOM1");
    agent.stopHost("ROOM1");
    mock.timers.tick(STOP_GRACE_MS - 1);
    assert.deepEqual(spawned[0].child.signals, ["SIGTERM"]);
    mock.timers.tick(1);
    assert.deepEqual(spawned[0].child.signals, ["SIGTERM", "SIGKILL"]);
  } finally {
    mock.timers.reset();
  }
});

test("the local port window is 200 hosts", () => {
  const { spawned, spawnProcess } = fakeSpawn();
  const agent = createAgent(agentEnv({
    LASTROUTE_MAX_ROOMS: "0",
    LASTROUTE_ROOM_MB: "1",
    LASTROUTE_PORT_BASE: "20000",
  }), {
    spawnProcess, log() {},
    resources: () => ({ cores: 250, totalMb: 1024 + 400, freeMb: 100_000, load1: 0 }),
    now: () => 0,
  });
  for (let index = 0; index < 200; index += 1) {
    assert.equal(agent.spawnHost(`R${String(index).padStart(4, "0")}`), "");
  }
  assert.equal(spawned[0].args[spawned[0].args.indexOf("--port") + 1], "20000");
  assert.equal(spawned[199].args[spawned[199].args.indexOf("--port") + 1], "20199");
  assert.equal(agent.spawnHost("EXTRA"), "no free local port");
  assert.equal(agent.hosts.size, 200);
});

test("reportOnce signs the body, runs spawn and stop, and survives a bad directory", async () => {
  const { spawned, spawnProcess } = fakeSpawn();
  const logs = [];
  let clock = 50_000;
  const calls = [];
  const queue = [];
  const fetchImpl = (url, init) => {
    calls.push({ url, init });
    const next = queue.shift();
    if (next instanceof Error) return Promise.reject(next);
    return Promise.resolve(next);
  };
  const agent = createAgent(agentEnv(), {
    spawnProcess, fetchImpl, log: (line) => logs.push(line), resources: resources(), now: () => clock,
  });

  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  queue.push(gate.then(() => ({ ok: true, json: async () => ({ actions: [] }) })));
  const first = agent.reportOnce();
  assert.deepEqual(await agent.reportOnce(), []);
  release();
  assert.deepEqual(await first, []);
  assert.equal(calls.length, 1);
  const { url, init } = calls[0];
  assert.equal(url, "http://127.0.0.1:9/api/regions/report");
  assert.equal(init.method, "POST");
  assert.equal(init.headers["content-type"], "application/json");
  assert.equal(init.headers["x-lr-region"], "virginia");
  assert.equal(init.headers["x-lr-time"], String(clock));
  assert.equal(init.headers["x-lr-signature"], signAgentRequest(TOKEN, "virginia", clock, init.body));
  assert.equal(init.body.includes(TOKEN), false);

  clock += 1;
  queue.push({
    ok: true,
    json: async () => ({
      actions: [
        { action: "spawn", code: "ROOM9", options: { seed: "S", idleMinutes: 4, spawnTicket: "hidden-ticket" } },
        { action: "stop", code: "ROOM9" },
        { action: "dance", code: "ROOM9" },
      ],
    }),
  });
  const done = await agent.reportOnce();
  assert.equal(done[0].problem, "");
  assert.equal(done[0].options, undefined);
  assert.equal(done[1].problem, "");
  assert.match(done[2].problem, /unknown action dance/);
  assert.equal(spawned.at(-1).options.env.LASTROUTE_SPAWN_TICKET, "hidden-ticket");
  assert.equal(spawned.at(-1).args.join(" ").includes("hidden-ticket"), false);
  assert.deepEqual(spawned.at(-1).child.signals, ["SIGTERM"]);
  spawned.at(-1).child.emit("exit", null, "SIGTERM");
  assert.equal(agent.hosts.size, 0);

  clock += 1;
  queue.push({ ok: false, status: 403, json: async () => ({ error: "bad signature" }) });
  assert.deepEqual(await agent.reportOnce(), []);
  assert.match(logs.at(-1), /report refused: 403 bad signature/);

  clock += 1;
  queue.push({ ok: true, json: async () => { throw new Error("bad json"); } });
  assert.deepEqual(await agent.reportOnce(), []);

  clock += 1;
  queue.push(new Error("connect ECONNREFUSED"));
  assert.deepEqual(await agent.reportOnce(), []);
  assert.match(logs.at(-1), /directory unreachable: connect ECONNREFUSED/);

  const full = createAgent(agentEnv({ LASTROUTE_MAX_ROOMS: "0" }), {
    spawnProcess, fetchImpl, log: (line) => logs.push(line),
    resources: resources({ freeMb: 10 }), now: () => clock,
  });
  queue.push({ ok: true, json: async () => ({ actions: [{ action: "spawn", code: "FULL1" }] }) });
  const refused = await full.reportOnce();
  assert.match(refused[0].problem, /no capacity for FULL1/);
});

test("the CLI exits before reporting when its environment is incomplete", () => {
  const env = { ...process.env };
  delete env.LASTROUTE_REGION;
  delete env.LASTROUTE_AGENT_TOKEN;
  delete env.LASTROUTE_DIRECTORY_URL;
  const result = spawnSync(process.execPath, [SCRIPT], { encoding: "utf8", env });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /LASTROUTE_REGION, LASTROUTE_AGENT_TOKEN and LASTROUTE_DIRECTORY_URL/);
});

test("a room that asks for the Rust build runs the Rust binary when there is one, else the Godot host", () => {
  const { spawned, spawnProcess } = fakeSpawn();
  const make = (extra) => createAgent(agentEnv({ LASTROUTE_DIRECTORY_URL: "http://127.0.0.1:8205", ...extra }), {
    spawnProcess, log() {}, resources: resources(), now: () => 1_000,
  });
  // LASTROUTE_RUST_BIN set: build "rust" runs it, with the same command line the Rust headless host reads, and no --path
  const both = make({ LASTROUTE_RUST_BIN: "/opt/lastroute-rust/current/last_route" });
  assert.equal(both.spawnHost("RUST1", { build: "rust", difficulty: 0, idleMinutes: 10, spawnTicket: "ticket-r" }), "");
  assert.equal(spawned[0].bin, "/opt/lastroute-rust/current/last_route");
  const rustArgs = spawned[0].args;
  assert.deepEqual(rustArgs.slice(0, 3), ["--headless", "--", "--dedicated-host"]);
  assert.equal(rustArgs.includes("--path"), false, "the Rust binary has no project path");
  assert.equal(rustArgs[rustArgs.indexOf("--difficulty") + 1], "0", "RELAXED reaches the Rust host");
  assert.ok(rustArgs.includes("--ws-rooms") && rustArgs.includes("--backend=http://127.0.0.1:8205"));
  assert.equal(rustArgs.join(" ").includes("ticket-r"), false);
  assert.equal(spawned[0].options.env.LASTROUTE_SPAWN_TICKET, "ticket-r");
  // the same agent still starts the Godot host for a room that did not ask for the Rust build
  assert.equal(both.spawnHost("GODOT1", { difficulty: 2, spawnTicket: "ticket-g" }), "");
  assert.equal(spawned[1].bin, "/opt/lastroute/game.x86_64");
  assert.deepEqual(spawned[1].args.slice(0, 3), ["--headless", "--path", "/opt/lastroute/project"]);
  // no LASTROUTE_RUST_BIN: build "rust" falls back to the Godot host (today's behavior), it does not fail
  const godotOnly = make({});
  assert.equal(godotOnly.spawnHost("RUST2", { build: "rust", difficulty: 0 }), "");
  assert.equal(spawned[2].bin, "/opt/lastroute/game.x86_64");
  assert.equal(spawned[2].args.includes("--difficulty"), false);
  both.stopAll();
  godotOnly.stopAll();
});
