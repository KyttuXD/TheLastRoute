import { randomBytes, timingSafeEqual } from "node:crypto";
import { canonicalHunterId } from "./hunter_id_aliases.mjs";

export const DEFAULT_ROOM_SETTINGS = Object.freeze({
  mapScale: "standard",
  difficulty: "standard",
  mode: "lastRoute",
  fakeRunners: false,
});

const SETTING_VALUES = Object.freeze({
  mapScale: new Set(["small", "standard", "large", "huge", "colossal", "endless"]),
  difficulty: new Set(["relaxed", "standard", "nightmare"]),
  mode: new Set(["lastRoute", "hideAndSeek"]),
  hiderNpcs: new Set([...Array(9).keys()]),
  seekerNpcs: new Set([...Array(9).keys()]),
  fakeRunners: new Set([true, false]),
});

const PERSONAS = new Set([
  "none", "aki", "briselle", "ceryth", "dovelle", "elise", "ferryn", "glyra",
  "haelith", "ilyra", "javelle", "kathy", "loxelle", "mavrine", "nyxie", "opalie",
  "porcelle", "quenelle", "ravelle", "solyth", "thessine", "umbriel", "verrine",
  "wynelle", "xivvy", "ysolde", "zellia",
]);

const MAX_PLAYERS = 16;
const MAX_ROOMS = 512;
const HIDE_PHASE_MS = 30_000;
const ITEM_LIMIT = 96;
const DISCOVERY_LIMIT = 512;
const DISCOVERY_TOTAL_LIMIT = 30_000;

function finite(value, fallback, minimum, maximum) {
  const number = Number(value);
  return Number.isFinite(number) ? Math.max(minimum, Math.min(maximum, number)) : fallback;
}

function cleanRoom(value) {
  return String(value || "").toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8);
}

function cleanPlayerId(value) {
  return typeof value === "string" ? value.replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 72) : "";
}

function cleanName(value) {
  return String(value || "").replace(/[^a-zA-Z0-9 _-]/g, "").replace(/\s+/g, " ").trim().slice(0, 16).toUpperCase();
}

function cleanAppearance(value, fallback = "player") {
  if (typeof value !== "string") return fallback;
  return value.trim().replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 32) || fallback;
}

function safeEqual(leftValue, rightValue) {
  const left = Buffer.from(String(leftValue || ""));
  const right = Buffer.from(String(rightValue || ""));
  const length = Math.max(left.length, right.length, 1);
  const paddedLeft = Buffer.alloc(length);
  const paddedRight = Buffer.alloc(length);
  left.copy(paddedLeft);
  right.copy(paddedRight);
  return timingSafeEqual(paddedLeft, paddedRight) && left.length === right.length;
}

function emptyRound() {
  return { roundId: 0, phase: "", seekerId: "", seekerSpawnIndex: -1, startedAt: 0, hideEndsAt: 0 };
}

function emptyDialog() {
  return { dialogId: 0, file: "", nodeId: "", stepId: 0, ended: false };
}

function emptyWorld() {
  return { depth: 0, endlessExpansion: 0, hostId: "", at: 0, hunters: [], mannequins: [], rooms: [] };
}

function createRoom(code, timestamp) {
  return {
    code,
    worldSeed: `ROOM-${code}-${randomBytes(10).toString("hex").toUpperCase()}`,
    hostPlayerId: "",
    settings: { ...DEFAULT_ROOM_SETTINGS },
    round: emptyRound(),
    roundCounter: 0,
    launchId: 0,
    abandonId: 0,
    customMapFile: "",
    customMapId: 0,
    dialog: emptyDialog(),
    activeDepth: 0,
    world: emptyWorld(),
    items: {},
    discovered: new Set(),
    discoveredList: [],
    discoveredEpoch: 0,
    createdAt: timestamp,
    lastActiveAt: timestamp,
    players: [],
  };
}

function publicPlayer(player, room, timestamp) {
  const roundId = Math.max(0, Number(room.round?.roundId) || 0);
  const role = roundId > 0 && player.spawnIndex === room.round.seekerSpawnIndex ? "seeker" : roundId > 0 ? "hider" : "runner";
  return {
    name: player.name,
    isHost: player.id === room.hostPlayerId,
    role,
    released: roundId <= 0 || role === "hider" || timestamp >= room.round.hideEndsAt,
    tagged: roundId > 0 && Boolean(player.tagged),
    roundId,
    spawnIndex: player.spawnIndex,
    depth: player.depth,
    x: player.x,
    y: player.y,
    mode: player.mode,
    facingX: player.facingX,
    facingY: player.facingY,
    hidden: player.hidden,
    flashlightOn: player.flashlightOn,
    radialActive: player.radialActive,
    fanOn: player.fanOn,
    heaterOn: player.heaterOn,
    frame: player.frame,
    persona: player.persona,
    appearanceId: player.appearanceId,
    noise: player.noise,
    inSafeZone: player.inSafeZone,
    lamp: player.lamp,
    travelSpeed: player.travelSpeed,
    crouching: player.crouching,
    vx: player.vx,
    vy: player.vy,
  };
}

function cleanSettings(patch, current) {
  if (!patch || typeof patch !== "object" || Array.isArray(patch)) return null;
  if (Object.keys(patch).some((key) => !Object.hasOwn(SETTING_VALUES, key))) return null;
  const next = { ...DEFAULT_ROOM_SETTINGS, ...current };
  for (const [key, value] of Object.entries(patch)) {
    if (!SETTING_VALUES[key].has(value)) {
      // An unknown mode (for example the removed "officeSurvivor" from an old client) falls back to
      // Last Route instead of rejecting the whole settings update.
      if (key === "mode") { next.mode = DEFAULT_ROOM_SETTINGS.mode; continue; }
      return null;
    }
    next[key] = value;
  }
  return next;
}

function sanitizeWorld(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const entities = (source, includeName = false) => (Array.isArray(source) ? source : []).slice(0, 64).flatMap((entry) => {
    if (!entry || typeof entry !== "object" || typeof entry.id !== "string" || !entry.id) return [];
    const output = {
      id: entry.id.slice(0, 48),
      x: finite(entry.x, 0, -1_000, 1_000),
      y: finite(entry.y, 0, -1_000, 1_000),
      facingX: finite(entry.facingX, 1, -1, 1),
      facingY: finite(entry.facingY, 0, -1, 1),
      mode: String(entry.mode || "").slice(0, 24),
      frame: Math.round(finite(entry.frame, 0, 0, 3)),
      active: entry.active === undefined ? true : Boolean(entry.active),
    };
    if (includeName) output.name = String(entry.name || "").slice(0, 48);
    else {
      output.state = String(entry.state || "").slice(0, 24);
      output.alerted = Boolean(entry.alerted);
      output.adminSummoned = Boolean(entry.adminSummoned);
    }
    return [output];
  });
  const rooms = (Array.isArray(value.rooms) ? value.rooms : []).slice(0, 64).flatMap((entry) => {
    if (!entry || typeof entry.id !== "string" || !entry.id) return [];
    return [{ id: entry.id.slice(0, 48), temperature: finite(entry.temperature, 60, 0, 100) }];
  });
  return {
    depth: Math.round(finite(value.depth, 0, 0, 9_999)),
    endlessExpansion: Math.round(finite(value.endlessExpansion, 0, 0, 999)),
    hunters: entities(value.hunters),
    mannequins: entities(value.mannequins, true),
    rooms,
  };
}

function applyItemClaim(items, claim) {
  if (!claim || typeof claim !== "object" || !["relay", "battery", "prop"].includes(claim.kind)) return;
  const id = typeof claim.id === "string" ? claim.id.slice(0, 48) : "";
  if (!id) return;
  const changes = claim.changes && typeof claim.changes === "object" ? claim.changes : {};
  const key = `${claim.kind}:${id}`;
  const record = items[key] || { kind: claim.kind, id };
  if (changes.collected === true) record.collected = true;
  if (claim.kind === "relay" && Object.hasOwn(changes, "sabotaged")) record.sabotaged = Boolean(changes.sabotaged);
  if (claim.kind === "prop") {
    if (typeof changes.interactionState === "string") record.interactionState = changes.interactionState.slice(0, 24);
    if (changes.fired === true) record.stimulusSeq = (Number(record.stimulusSeq) || 0) + 1;
    if (Object.hasOwn(changes, "cooldownUntil")) record.cooldownUntil = Math.max(record.cooldownUntil || 0, finite(changes.cooldownUntil, 0, 0, 1e9));
    if (Object.hasOwn(changes, "x")) record.x = finite(changes.x, record.x || 0, -1_000, 1_000);
    if (Object.hasOwn(changes, "y")) record.y = finite(changes.y, record.y || 0, -1_000, 1_000);
  }
  items[key] = record;
}

export function createRoomService({
  now = Date.now,
  playerTtlMs = 120_000,
  roomTtlMs = 300_000,
} = {}) {
  const rooms = new Map();

  function reconcile(room) {
    if (!room.players.some((player) => player.id === room.hostPlayerId)) {
      room.hostPlayerId = [...room.players].sort((a, b) => a.spawnIndex - b.spawnIndex)[0]?.id || "";
    }
    const npcSeeker = String(room.round?.seekerId || "").startsWith("npc:seeker:");
    const seekerPresent = npcSeeker || room.players.some((player) => player.spawnIndex === room.round?.seekerSpawnIndex);
    if ((room.round?.roundId || 0) > 0 && !seekerPresent) room.round = emptyRound();
  }

  function pruneRoom(code, room, timestamp = now()) {
    room.players = room.players.filter((player) => timestamp - player.lastSeen <= playerTtlMs);
    reconcile(room);
    if (!room.players.length || timestamp - room.lastActiveAt > roomTtlMs) {
      rooms.delete(code);
      return null;
    }
    return room;
  }

  function cleanup() {
    const timestamp = now();
    for (const [code, room] of rooms) pruneRoom(code, room, timestamp);
  }

  function load(code) {
    const room = rooms.get(code);
    return room ? pruneRoom(code, room) : null;
  }

  function payload(room, requesterId, timestamp, discovery = null) {
    reconcile(room);
    const current = room.players.find((player) => player.id === requesterId) || null;
    const cursorValid = discovery && discovery.epoch === room.discoveredEpoch;
    const discovered = cursorValid ? room.discoveredList.slice(Math.min(discovery.since, room.discoveredList.length)) : room.discoveredList;
    return {
      room: room.code,
      worldSeed: room.worldSeed,
      settings: { ...DEFAULT_ROOM_SETTINGS, ...room.settings },
      round: { ...emptyRound(), ...room.round },
      launchId: room.launchId,
      abandonId: room.abandonId,
      customMapFile: room.customMapFile,
      customMapId: room.customMapId,
      dialog: { ...emptyDialog(), ...room.dialog },
      reloadId: 0,
      serverNow: timestamp,
      isHost: Boolean(current && current.id === room.hostPlayerId),
      spawnIndex: current?.spawnIndex ?? -1,
      roomSize: room.players.length,
      maxPlayers: MAX_PLAYERS,
      sessionToken: current?.sessionToken || "",
      activeDepth: room.activeDepth,
      world: room.world,
      items: room.items,
      discovered,
      discoveredEpoch: room.discoveredEpoch,
      discoveredTotal: room.discoveredList.length,
      discoveredReset: !cursorValid,
      player: current ? publicPlayer(current, room, timestamp) : null,
      players: room.players.filter((player) => player.id !== requesterId).map((player) => publicPlayer(player, room, timestamp)),
    };
  }

  function authenticate(body, { host = false } = {}) {
    const code = cleanRoom(body.room);
    const id = cleanPlayerId(body.playerId);
    const room = load(code);
    if (!room || !id) return [null, null, [404, { code: "ROOM_NOT_FOUND", error: "Room not found" }]];
    const player = room.players.find((entry) => entry.id === id);
    if (!player) return [null, null, [404, { code: "PLAYER_NOT_JOINED", error: "Player not joined" }]];
    if (!safeEqual(body.sessionToken, player.sessionToken)) return [null, null, [403, { code: "INVALID_SESSION", error: "Room session expired; leave and rejoin" }]];
    if (host && player.id !== room.hostPlayerId) return [null, null, [403, { code: "HOST_ONLY", error: "Only the room host can do that" }]];
    return [room, player, null];
  }

  function resetRuntime(room) {
    room.activeDepth = 0;
    room.world = emptyWorld();
    room.items = {};
    room.discovered = new Set();
    room.discoveredList = [];
    room.discoveredEpoch += 1;
    room.customMapFile = "";
    room.dialog = emptyDialog();
    room.round = emptyRound();
    for (const player of room.players) player.tagged = false;
  }

  function joinRoom(body) {
    const code = cleanRoom(body.room);
    const id = cleanPlayerId(body.playerId);
    const name = cleanName(body.name);
    if (code.length < 5 || !id) return [400, { code: "INVALID_ROOM", error: "Room codes must be 5-8 characters" }];
    if (!name) return [400, { code: "INVALID_NAME", error: "Enter a valid player name" }];
    const timestamp = now();
    let room = load(code);
    if (!room) {
      cleanup();
      if (rooms.size >= MAX_ROOMS) return [503, { code: "SERVER_BUSY", error: "Too many active rooms" }];
      room = createRoom(code, timestamp);
      rooms.set(code, room);
    }
    const existing = room.players.find((player) => player.id === id);
    if (existing && !safeEqual(body.sessionToken, existing.sessionToken)) return [403, { code: "INVALID_SESSION", error: "Room session expired; leave and rejoin" }];
    if (room.players.some((player) => player.id !== id && player.name.toLowerCase() === name.toLowerCase())) {
      return [409, { code: "NAME_TAKEN", error: "That name is already in use in this room" }];
    }
    if (!existing && room.players.length >= MAX_PLAYERS) return [409, { code: "ROOM_FULL", error: `Room capacity is ${MAX_PLAYERS} players` }];
    const occupied = new Set(room.players.map((player) => player.spawnIndex));
    const spawnIndex = existing?.spawnIndex ?? [...Array(MAX_PLAYERS).keys()].find((index) => !occupied.has(index));
    const player = {
      id,
      sessionToken: existing?.sessionToken || randomBytes(24).toString("hex"),
      name,
      spawnIndex,
      depth: existing?.depth || 0,
      x: existing?.x || 0,
      y: existing?.y || 0,
      facingX: existing?.facingX ?? 1,
      facingY: existing?.facingY || 0,
      mode: existing?.mode || "idle",
      hidden: existing?.hidden || false,
      flashlightOn: existing?.flashlightOn ?? true,
      radialActive: existing?.radialActive || false,
      fanOn: existing?.fanOn || false,
      heaterOn: existing?.heaterOn || false,
      frame: existing?.frame || 0,
      persona: existing?.persona || "none",
      appearanceId: existing?.appearanceId || "player",
      tagged: existing?.tagged || false,
      noise: existing?.noise || 0,
      inSafeZone: existing?.inSafeZone || false,
      lamp: existing?.lamp || 0,
      travelSpeed: existing?.travelSpeed || 0,
      crouching: existing?.crouching || false,
      vx: existing?.vx || 0,
      vy: existing?.vy || 0,
      lastSeen: timestamp,
    };
    if (existing) room.players[room.players.indexOf(existing)] = player;
    else room.players.push(player);
    reconcile(room);
    room.lastActiveAt = timestamp;
    return [200, payload(room, id, timestamp)];
  }

  function updateRoom(body) {
    const [room, current, error] = authenticate(body);
    if (error) return error;
    if (body.name !== undefined && cleanName(body.name).toLowerCase() !== current.name.toLowerCase()) {
      return [409, { code: "NAME_LOCKED", error: "Leave and rejoin to change your name" }];
    }
    const state = body.state && typeof body.state === "object" ? body.state : {};
    const modes = new Set(["idle", "walk", "sprint", "crouch", "lobby"]);
    current.depth = Math.round(finite(state.depth, current.depth, 0, 9_999));
    current.x = finite(state.x, current.x, -1_000, 1_000);
    current.y = finite(state.y, current.y, -1_000, 1_000);
    current.facingX = finite(state.facingX, current.facingX, -1, 1);
    current.facingY = finite(state.facingY, current.facingY, -1, 1);
    current.mode = modes.has(state.mode) ? state.mode : "idle";
    current.hidden = Boolean(state.hidden);
    if (typeof state.flashlightOn === "boolean") current.flashlightOn = state.flashlightOn;
    current.radialActive = Boolean(state.radialActive);
    current.fanOn = Boolean(state.fanOn);
    current.heaterOn = Boolean(state.heaterOn);
    current.frame = Math.round(finite(state.frame, current.frame, 0, 3));
    const persona = canonicalHunterId(state.persona);
    current.persona = PERSONAS.has(persona) ? persona : current.persona;
    current.appearanceId = cleanAppearance(state.appearanceId, current.appearanceId);
    current.tagged = Number(state.roundId) === room.round.roundId && room.round.roundId > 0 ? current.tagged || Boolean(state.tagged) : false;
    current.noise = finite(state.noise, 0, 0, 100);
    current.inSafeZone = Boolean(state.inSafeZone);
    current.lamp = finite(state.lamp, current.lamp, 0, 100);
    current.travelSpeed = finite(state.travelSpeed, 0, 0, 100);
    current.crouching = Boolean(state.crouching);
    current.vx = finite(state.vx, current.vx, -100, 100);
    current.vy = finite(state.vy, current.vy, -100, 100);
    const timestamp = now();
    current.lastSeen = timestamp;
    room.lastActiveAt = timestamp;
    const claimDepth = Math.round(finite(body.claimDepth, 0, 0, 9_999));
    if (claimDepth > room.activeDepth) {
      room.activeDepth = claimDepth;
      room.items = {};
      room.world = emptyWorld();
      room.discovered = new Set();
      room.discoveredList = [];
      room.discoveredEpoch += 1;
    }
    if (current.id === room.hostPlayerId && body.world) {
      const world = sanitizeWorld(body.world);
      if (world && world.depth === room.activeDepth) room.world = { ...world, hostId: current.id, at: timestamp };
    }
    for (const claim of (Array.isArray(body.itemClaims) ? body.itemClaims : []).slice(0, ITEM_LIMIT)) {
      if (Math.round(finite(claim?.depth, -1, 0, 9_999)) === room.activeDepth) applyItemClaim(room.items, claim);
    }
    const maxDiscovery = room.settings.mapScale === "endless" ? Infinity : DISCOVERY_TOTAL_LIMIT;
    for (const cell of (Array.isArray(body.discoveryClaims) ? body.discoveryClaims : []).slice(0, DISCOVERY_LIMIT)) {
      if (typeof cell === "string" && cell.length <= 12 && room.discovered.size < maxDiscovery && !room.discovered.has(cell)) {
        room.discovered.add(cell);
        room.discoveredList.push(cell);
      }
    }
    if (current.id === room.hostPlayerId && room.round.phase === "seeking") {
      const hiders = room.players.filter((player) => player.spawnIndex !== room.round.seekerSpawnIndex);
      if (hiders.length && hiders.every((player) => player.tagged)) room.round.phase = "complete";
    }
    return [200, payload(room, current.id, timestamp, {
      since: Math.round(finite(body.discoverySince, 0, 0, 9_999_999)),
      epoch: Math.round(finite(body.discoveryEpoch, -1, -1, 9_999_999)),
    })];
  }

  function updateRoomSettings(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    const settings = cleanSettings(body.settings, room.settings);
    if (!settings) return [400, { code: "INVALID_SETTINGS", error: "Invalid room settings" }];
    room.settings = settings;
    room.round = emptyRound();
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function startRoomRound(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    if (room.settings.mode !== "hideAndSeek") return [409, { code: "WRONG_MODE", error: "Select Hide & Seek before starting" }];
    const npcCount = Number(room.settings.hiderNpcs || 0) + Number(room.settings.seekerNpcs || 0);
    if (room.players.length + npcCount < 2) return [409, { code: "NOT_ENOUGH_PLAYERS", error: "Hide & Seek needs at least two players or NPCs" }];
    const timestamp = now();
    room.roundCounter += 1;
    const soloHider = room.players.length === 1 && String(body.role || "").toLowerCase() === "hider";
    const randomPlayer = room.players[randomBytes(4).readUInt32BE(0) % room.players.length];
    room.round = {
      roundId: room.roundCounter,
      phase: "hiding",
      seekerId: soloHider ? "npc:seeker:0" : randomPlayer.id,
      seekerSpawnIndex: soloHider ? -1 : randomPlayer.spawnIndex,
      startedAt: timestamp,
      hideEndsAt: timestamp + HIDE_PHASE_MS,
    };
    for (const player of room.players) player.tagged = false;
    current.lastSeen = room.lastActiveAt = timestamp;
    return [200, payload(room, current.id, timestamp)];
  }

  function launch(body, mode) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    if (room.settings.mode !== mode) return [409, { code: "WRONG_MODE", error: `Select ${mode} before starting` }];
    resetRuntime(room);
    if (mode === "lastRoute") room.launchId += 1;
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function abandonRoute(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    room.abandonId += 1;
    resetRuntime(room);
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function setCustomMap(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    const filename = String(body.filename || "").trim().slice(0, 128);
    if (filename && !/^[a-z0-9][a-z0-9._-]*\.png$/i.test(filename)) return [400, { code: "INVALID_FILENAME", error: "Custom map filename must be a plain .png name" }];
    room.customMapFile = filename;
    room.customMapId += 1;
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function startDialog(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    const file = String(body.file || "").trim().slice(0, 128);
    if (!/^[a-z0-9][a-z0-9._-]*\.yaml(#[a-z0-9][a-z0-9_-]*)?$/i.test(file)) return [400, { code: "INVALID_FILENAME", error: "Dialog filename must be a plain .yaml name" }];
    room.dialog = { dialogId: room.dialog.dialogId + 1, file, nodeId: "", stepId: 0, ended: false };
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function advanceDialog(body) {
    const [room, current, error] = authenticate(body, { host: true });
    if (error) return error;
    if (Number(body.dialogId) !== room.dialog.dialogId || room.dialog.dialogId < 1) return [409, { code: "STALE_DIALOG", error: "That dialog is no longer active" }];
    room.dialog.nodeId = String(body.nodeId || "").trim().slice(0, 96);
    room.dialog.ended = Boolean(body.ended);
    room.dialog.stepId += 1;
    current.lastSeen = room.lastActiveAt = now();
    return [200, payload(room, current.id, room.lastActiveAt)];
  }

  function leaveRoom(body) {
    const code = cleanRoom(body.room);
    const id = cleanPlayerId(body.playerId);
    const room = load(code);
    const current = room?.players.find((player) => player.id === id);
    if (!room || !current) return [200, { ok: true }];
    if (!safeEqual(body.sessionToken, current.sessionToken)) return [403, { code: "INVALID_SESSION", error: "Room session expired; leave and rejoin" }];
    room.players.splice(room.players.indexOf(current), 1);
    if (!room.players.length) rooms.delete(code);
    else {
      room.lastActiveAt = now();
      reconcile(room);
    }
    return [200, { ok: true }];
  }

  return {
    rooms,
    cleanup,
    joinRoom,
    updateRoom,
    updateRoomSettings,
    startRoomRound,
    startDefaultRoute: (body) => launch(body, "lastRoute"),
    abandonRoute,
    setCustomMap,
    startDialog,
    advanceDialog,
    leaveRoom,
  };
}
