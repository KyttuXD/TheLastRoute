import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { createSocket } from "node:dgram";
import { mkdtemp, rm } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { createReservedRooms, RESERVED_LIMITS } from "./reserved_rooms.mjs";
import { createEnetRelay, decodeRelayFrame, encodeRelayFrame, RelayFrame } from "./enet_relay.mjs";

const quiet = { log() {}, warn() {}, error() {} };

async function post(url, path, body) {
  const response = await fetch(url + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  return { status: response.status, body: await response.json() };
}

async function get(url, path) {
  const response = await fetch(url + path);
  return { status: response.status, body: await response.json() };
}

test("reservations persist, keep environment codes, hash the password and queue owner commands", async () => {
  const root = await mkdtemp(join(tmpdir(), "reserved-"));
  try {
    const storePath = join(root, "data", "reserved_rooms.json");
    const rooms = createReservedRooms({ storePath, codes: ["ktprv", "bad!"], logger: quiet });
    assert.ok(rooms.isReserved("KTPRV") && !rooms.isReserved("ABCDE"));
    assert.equal(rooms.admin({ action: "delete", code: "KTPRV" }, "KYTTU")[0], 409, "an environment code is not deleted here");
    const [status, created] = rooms.admin({ action: "create", code: "PRV22", invites: ["Alice", "bob_2", "no way!"],
      password: "open sesame", settings: { seed: "NEON 7", mode: 2, difficulty: 1, mods: ["neon_halls", "Bad-Id"], junk: 1 } }, "KYTTU");
    assert.equal(status, 201);
    assert.deepEqual(created.room.invites, ["ALICE", "BOB_2"], "canonical uppercase account names");
    assert.deepEqual(created.room.settings.mods, ["neon_halls"]);
    assert.equal(created.room.settings.junk, undefined);
    assert.equal(created.room.hasPassword, true);
    assert.ok(!JSON.stringify(created).includes("open sesame") && !JSON.stringify(created).includes("scrypt"),
      "the owner view never carries the password or its hash");
    assert.ok(!readFileSync(storePath, "utf8").includes("open sesame"), "only a hash is stored");
    assert.equal(rooms.admin({ action: "create", code: "PRV22" }, "KYTTU")[0], 409);
    assert.equal(rooms.admin({ action: "update", code: "PRV22", password: "123" }, "KYTTU")[0], 400);

    assert.ok(rooms.hasAccess("PRV22", { username: "ALICE" }), "invites are case-insensitive");
    assert.ok(rooms.hasAccess("PRV22", { password: "open sesame" }));
    assert.ok(!rooms.hasAccess("PRV22", { username: "mallory", password: "guess" }));
    assert.ok(rooms.hasAccess("PRV22", { owner: true }));

    assert.equal(rooms.admin({ action: "command", code: "PRV22", command: "explode" }, "KYTTU")[0], 400);
    rooms.admin({ action: "command", code: "PRV22", command: "restart", sameSeed: true }, "KYTTU");
    rooms.admin({ action: "command", code: "PRV22", command: "kick", target: "bob_2" }, "KYTTU");
    let host = rooms.hostView("PRV22");
    assert.deepEqual(host.commands.map((c) => c.command), ["restart", "kick"]);
    assert.equal(host.commands[0].args.sameSeed, true);
    host = rooms.hostView("PRV22", { ack: host.commands[0].seq });
    assert.deepEqual(host.commands.map((c) => c.command), ["kick"], "an acked command is gone");
    for (let i = 0; i < 30; i += 1) rooms.admin({ action: "command", code: "PRV22", command: "end" }, "KYTTU");
    assert.ok(rooms.hostView("PRV22").commands.length <= RESERVED_LIMITS.commands, "the command queue is bounded");

    // whitelist: canonical uppercase names, add / remove / list
    assert.deepEqual(rooms.admin({ action: "whitelist", code: "PRV22", op: "add", name: "carol" }, "KYTTU")[1].whitelist,
      ["ALICE", "BOB_2", "CAROL"]);
    assert.ok(rooms.hasAccess("PRV22", { username: "Carol" }));
    assert.deepEqual(rooms.admin({ action: "whitelist", code: "PRV22", op: "remove", name: "BOB_2" }, "KYTTU")[1].whitelist,
      ["ALICE", "CAROL"]);
    assert.ok(!rooms.hasAccess("PRV22", { username: "bob_2" }), "a removed name loses access");
    assert.equal(rooms.admin({ action: "whitelist", code: "PRV22", op: "add", name: "x" }, "KYTTU")[0], 400);
    assert.deepEqual(rooms.admin({ action: "whitelist", code: "PRV22" }, "KYTTU")[1].whitelist, ["ALICE", "CAROL"]);
    // reserved codes may use letters the generated alphabet skips (AKIRI has an I)
    assert.equal(rooms.admin({ action: "create", code: "akiri" }, "KYTTU")[0], 201);
    assert.ok(rooms.isReserved("AKIRI"));

    // a restart (new process, same file): everything is still there
    const again = createReservedRooms({ storePath, codes: ["KTPRV"], logger: quiet });
    assert.ok(again.isReserved("PRV22") && again.isReserved("KTPRV"));
    assert.ok(again.hasAccess("PRV22", { password: "open sesame" }), "the password survives a restart");
    assert.equal(again.hostView("PRV22").settings.seed, "NEON 7");

    const ticket = again.issueTicket("PRV22");
    assert.ok(again.ticketValid("PRV22", Buffer.from(ticket, "base64url")));
    assert.ok(!again.ticketValid("PRV22", Buffer.from("not a ticket at all")));
    again.admin({ action: "update", code: "PRV22", password: "a new secret" }, "KYTTU");
    assert.ok(!again.ticketValid("PRV22", Buffer.from(ticket, "base64url")), "a new password revokes join tickets");
  } finally {
    await rm(root, { recursive: true });
  }
});

test("reserved codes: hidden from resolve, hosted and joined only with access, taken over when the host dies", async () => {
  const root = await mkdtemp(join(tmpdir(), "reserved-server-"));
  let clock = Date.now();
  const server = createLastRouteServer({
    accountStorePath: join(root, "accounts.json"), moderationStorePath: join(root, "moderation.json"),
    reservedRoomStorePath: join(root, "reserved_rooms.json"), reservedRoomCodes: ["KTPRV", "AKIRI"],
    adminPassword: "server setup proof", accountRateLimitMax: 10_000, apiRateLimitMax: 10_000, logger: quiet,
    now: () => clock,
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}`;
  try {
    const signUp = async (username, extra = {}) => (await post(url, "/api/account/register",
      { username, password: "correct horse battery", ...extra })).body.sessionToken;
    const owner = await signUp("KYTTU", { adminProof: "server setup proof" });
    const alice = await signUp("ALICE");
    const mallory = await signUp("MALLORY");
    assert.equal((await post(url, "/api/admin/reserved-rooms", { sessionToken: alice, action: "list" })).status, 403);
    const listed = await post(url, "/api/admin/reserved-rooms", { sessionToken: owner, action: "list" });
    assert.deepEqual(listed.body.rooms.map((r) => r.code), ["KTPRV", "AKIRI"]);
    await post(url, "/api/admin/reserved-rooms", { sessionToken: owner, action: "update", code: "KTPRV",
      invites: ["alice"], password: "room pass 1", settings: { seed: "KT SEED" } });

    // nobody without access may register the code
    assert.equal((await post(url, "/api/enet/rooms/register", { code: "KTPRV", port: 7000, sessionToken: mallory })).body.code,
      "ROOM_RESERVED");
    assert.equal((await post(url, "/api/enet/rooms/register", { code: "KTPRV", port: 7000 })).status, 403);
    const hosted = await post(url, "/api/enet/rooms/register", { code: "KTPRV", port: 7000, sessionToken: alice });
    assert.equal(hosted.status, 201, JSON.stringify(hosted.body));
    assert.equal(hosted.body.reserved.settings.seed, "KT SEED", "the host gets the stored route settings");

    // hidden: the plain resolve refuses; the access-checked one answers with a join ticket
    assert.equal((await get(url, "/api/enet/rooms/KTPRV")).body.code, "ROOM_RESERVED");
    assert.equal((await post(url, "/api/rooms/reserved/resolve", { code: "KTPRV", sessionToken: mallory })).status, 403);
    const byPassword = await post(url, "/api/rooms/reserved/resolve", { code: "KTPRV", password: "room pass 1" });
    assert.equal(byPassword.status, 200);
    assert.ok(byPassword.body.live && byPassword.body.joinTicket && byPassword.body.room.port === 7000);
    assert.equal(byPassword.body.canHost, false);

    // owner commands ride the host's heartbeat until it acks them
    await post(url, "/api/admin/reserved-rooms", { sessionToken: owner, action: "command", code: "KTPRV", command: "restart" });
    assert.equal((await post(url, "/api/admin/reserved-rooms", { sessionToken: owner, action: "command", code: "AKIRI",
      command: "end" })).body.code, "ROOM_NOT_LIVE", "a command for an empty room is refused, not left for the next host");
    let beat = await post(url, "/api/enet/rooms/heartbeat", { code: "KTPRV", token: hosted.body.token, players: 2 });
    assert.deepEqual(beat.body.reserved.commands.map((c) => c.command), ["restart"]);
    beat = await post(url, "/api/enet/rooms/heartbeat", { code: "KTPRV", token: hosted.body.token,
      commandAck: beat.body.reserved.commands[0].seq });
    assert.equal(beat.body.reserved.commands.length, 0);

    // the host is alive: nobody takes the code; it goes quiet: an invited player (or the password) takes it over
    assert.equal((await post(url, "/api/enet/rooms/register", { code: "KTPRV", port: 7001, sessionToken: owner })).status, 409);
    clock += 31_000;
    const resolved = await post(url, "/api/rooms/reserved/resolve", { code: "KTPRV", sessionToken: alice });
    assert.equal(resolved.body.canHost, true, "a silent host's room offers hosting to the next invited player");
    await post(url, "/api/admin/reserved-rooms", { sessionToken: owner, action: "command", code: "KTPRV", command: "end" })
      .catch(() => null);
    const takeover = await post(url, "/api/enet/rooms/register", { code: "KTPRV", port: 7002, roomPassword: "room pass 1" });
    assert.equal(takeover.status, 201);
    assert.equal(takeover.body.reserved.commands.length, 0, "a new host does not inherit the old host's commands");
    assert.notEqual(takeover.body.token, hosted.body.token);

    // ordinary codes are untouched, and a non-reserved code outside the generated alphabet is still refused
    assert.equal((await post(url, "/api/enet/rooms/register", { code: "ABCDE", port: 7003 })).status, 201);
    assert.equal((await post(url, "/api/enet/rooms/register", { code: "OIOIO", port: 7004 })).status, 400);
    assert.equal((await get(url, "/api/enet/rooms/ABCDE")).status, 200);
  } finally {
    server.close();
    await once(server, "close");
    await rm(root, { recursive: true });
  }
});

test("the relay admits a reserved room's guest only with a join ticket as its nonce", async () => {
  const ticket = Buffer.from("0123456789abcdef01", "utf8");
  const room = { code: "KTPRV", token: "host-secret", admitGuest: (nonce) => Buffer.compare(nonce, ticket) === 0 };
  const relay = createEnetRelay({ roomByCode: (code) => (code === "KTPRV" ? room : null), logger: quiet });
  relay.bind(0, "127.0.0.1");
  await once(relay, "listening");
  const port = relay.address().port;
  const sockets = [];
  const udp = async () => {
    const socket = createSocket("udp4");
    socket.bind(0, "127.0.0.1");
    await once(socket, "listening");
    sockets.push(socket);
    return socket;
  };
  const exchange = async (socket, frame) => {
    socket.send(frame, port, "127.0.0.1");
    const [message] = await once(socket, "message");
    return decodeRelayFrame(message);
  };
  try {
    const host = await udp();
    assert.equal((await exchange(host, encodeRelayFrame(RelayFrame.HOST_REGISTER, 0, "KTPRV", Buffer.from("host-secret")))).type,
      RelayFrame.HOST_ACK);
    const stranger = await udp();
    const refused = await exchange(stranger, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, "KTPRV", Buffer.from("random-nonce-12")));
    assert.equal(refused.type, RelayFrame.ERROR);
    assert.match(refused.payload.toString(), /PRIVATE ROOM/);
    const invited = await udp();
    assert.equal((await exchange(invited, encodeRelayFrame(RelayFrame.GUEST_JOIN, 0, "KTPRV", ticket))).type, RelayFrame.GUEST_ACK);
  } finally {
    for (const socket of sockets) socket.close();
    relay.close();
    await once(relay, "close");
  }
});
