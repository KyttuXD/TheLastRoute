import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createLastRouteServer } from "./server.mjs";
import { GUEST_NAME_PATTERN, createGuestNames, randomGuestName } from "./guest_names.mjs";

test("guest names are one letter, a dash and four digits, without I, O, 0 or 1", () => {
  for (let i = 0; i < 500; i += 1) {
    const name = randomGuestName();
    assert.match(name, GUEST_NAME_PATTERN);
    assert.doesNotMatch(name, /[IO01]/);
  }
  for (const bad of ["R-482", "I-2345", "O-2345", "R-1234", "R-0234", "r-2345", "RR-2345", "R_2345", "FAR-LIGHT-658"]) {
    assert.doesNotMatch(bad, GUEST_NAME_PATTERN, bad);
  }
});

test("a clash is retried: a held name is never issued to someone else", () => {
  // a random source that first lands on R-2222 twice, then on another name
  const sequence = [0.64, 0, 0, 0, 0, 0.64, 0, 0, 0, 0, 0.1, 0.5, 0.5, 0.5, 0.5];
  let index = 0;
  const names = createGuestNames({ random: () => sequence[index++ % sequence.length] });
  const [, first] = names.issue({});
  const [, second] = names.issue({});
  assert.equal(first.name, "R-2222");
  assert.notEqual(second.name, first.name, "the second guest got a clashing name");
  assert.match(second.name, GUEST_NAME_PATTERN);
});

test("a guest keeps its name with its token; others cannot take it; a free previous name is reused", () => {
  const names = createGuestNames({});
  const [, mine] = names.issue({});
  const [, again] = names.issue({ previous: mine.name, token: mine.token });
  assert.equal(again.name, mine.name, "the holder lost its name on renewal");
  const [, thief] = names.issue({ previous: mine.name, token: "guess" });
  assert.notEqual(thief.name, mine.name, "another client took a held name");
  const [, free] = names.issue({ previous: "z-9876" });
  assert.equal(free.name, "Z-9876", "a free previous name is kept, canonical uppercase");
  const [, legacy] = names.issue({ previous: "FAR-LIGHT-658" });
  assert.match(legacy.name, GUEST_NAME_PATTERN, "an old-style name gets a new-style one");
});

test("leases survive a restart and expire", async () => {
  const root = await mkdtemp(join(tmpdir(), "guests-"));
  try {
    let clock = 1_000_000;
    const path = join(root, "guest_names.json");
    const first = createGuestNames({ storePath: path, now: () => clock, leaseMs: 1000 });
    const [, issued] = first.issue({});
    const reopened = createGuestNames({ storePath: path, now: () => clock, leaseMs: 1000 });
    assert.ok(reopened.held(issued.name), "a lease was lost on restart");
    clock += 5000;
    assert.ok(!reopened.held(issued.name), "an expired lease still blocks the name");
  } finally {
    await rm(root, { recursive: true });
  }
});

test("POST /api/guest/name issues unique names over HTTP", async () => {
  const root = await mkdtemp(join(tmpdir(), "guests-http-"));
  const server = createLastRouteServer({ accountStorePath: join(root, "accounts.json"), logger: { error() {}, log() {} } });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const url = `http://127.0.0.1:${server.address().port}/api/guest/name`;
  try {
    const call = async (body) => (await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })).json();
    const a = await call({});
    const b = await call({ previous: a.name });
    assert.ok(a.ok && b.ok);
    assert.match(a.name, GUEST_NAME_PATTERN);
    assert.notEqual(b.name, a.name, "a second client asking for a held name got it");
    assert.equal((await call({ previous: a.name, token: a.token })).name, a.name);
  } finally {
    server.close();
    await once(server, "close");
    await rm(root, { recursive: true });
  }
});
