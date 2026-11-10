#!/usr/bin/env node
// make-samples.mjs - (re)creates the four sample story packs in mods/:
//   mods/sample_greenhouse   mods/sample_lighthouse   mods/sample_archive   mods/sample_city
//
// They are samples of the lr_story pack format for modders, not lore stories. Each one shows the same features:
// a "story" block (player, start items/flags, multiplayer), two maps joined by exits (art at 8 px per tile, mask at
// 1 px per tile, legend with floor, wall, spawn, door, item, npc, trigger, exit and prop), items, scenes with
// speakers, choices with "if" conditions and "do" ops, a branch that rejoins, a once scene, NPC skins (64x64
// mannequin atlases recoloured from the game's own atlas) and a prop model of its own (a small .bbmodel).
//
// mods/ is gitignored, so this script is how the packs are kept in the repo. Run it from anywhere:
//     node packaging/story-samples/make-samples.mjs
// then check them:   lr_story check
// Zero dependencies: node only. PNGs are written here with zlib and a CRC32 table. The script owns the four pack
// folders and deletes and rewrites them on every run; it touches nothing else.
// Format reference: src/bin/lr_story/load.rs (module doc) and docs/LR_STORY.md.

import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');
const MODS = path.join(ROOT, 'mods');
const ATLAS = path.join(ROOT, 'src', 'bin', 'lr_story', 'assets', 'player', 'player_mannequin_atlas.png');
const TILE = 8; // art pixels per map tile

// ---------------------------------------------------------------- PNG (write and read)
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'latin1');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}
/** An RGBA image: `px` is a Uint8Array of w*h*4. */
function encodePng(w, h, px) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0; // filter: none
    Buffer.from(px.buffer, px.byteOffset + y * w * 4, w * 4).copy(raw, y * (w * 4 + 1) + 1);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}
/** Reads an 8-bit, non-interlaced RGB or RGBA PNG into {w, h, px}. */
function decodePng(buf) {
  let p = 8, w = 0, h = 0, ct = 0;
  const idat = [];
  while (p < buf.length) {
    const len = buf.readUInt32BE(p);
    const type = buf.toString('latin1', p + 4, p + 8);
    if (type === 'IHDR') {
      w = buf.readUInt32BE(p + 8);
      h = buf.readUInt32BE(p + 12);
      ct = buf[p + 17];
      if (buf[p + 16] !== 8 || buf[p + 20] !== 0 || (ct !== 6 && ct !== 2)) throw new Error('unsupported PNG layout');
    } else if (type === 'IDAT') idat.push(buf.subarray(p + 8, p + 8 + len));
    p += 12 + len;
  }
  const bpp = ct === 6 ? 4 : 3;
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const px = new Uint8Array(w * h * 4);
  let prev = Buffer.alloc(w * bpp), o = 0;
  for (let y = 0; y < h; y++) {
    const f = raw[o++];
    const row = Buffer.from(raw.subarray(o, o + w * bpp));
    o += w * bpp;
    for (let i = 0; i < row.length; i++) {
      const a = i >= bpp ? row[i - bpp] : 0, up = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
      let v = row[i];
      if (f === 1) v += a;
      else if (f === 2) v += up;
      else if (f === 3) v += (a + up) >> 1;
      else if (f === 4) {
        const pp = a + up - c, pa = Math.abs(pp - a), pb = Math.abs(pp - up), pc = Math.abs(pp - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? up : c;
      }
      row[i] = v & 255;
    }
    for (let x = 0; x < w; x++) {
      const d = (y * w + x) * 4;
      px[d] = row[x * bpp]; px[d + 1] = row[x * bpp + 1]; px[d + 2] = row[x * bpp + 2];
      px[d + 3] = bpp === 4 ? row[x * bpp + 3] : 255;
    }
    prev = row;
  }
  return { w, h, px };
}

// ---------------------------------------------------------------- tiny canvas and colour helpers
const hex = (s) => [parseInt(s.slice(1, 3), 16), parseInt(s.slice(3, 5), 16), parseInt(s.slice(5, 7), 16)];
const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);
const shade = (c, f) => c.map((v) => Math.max(0, Math.min(255, v * f)));

class Canvas {
  constructor(w, h) { this.w = w; this.h = h; this.px = new Uint8Array(w * h * 4); }
  set(x, y, c, a = 255) {
    if (x < 0 || y < 0 || x >= this.w || y >= this.h) return;
    const d = (y * this.w + x) * 4;
    this.px[d] = Math.round(c[0]); this.px[d + 1] = Math.round(c[1]); this.px[d + 2] = Math.round(c[2]); this.px[d + 3] = a;
  }
  get(x, y) { const d = (y * this.w + x) * 4; return [this.px[d], this.px[d + 1], this.px[d + 2]]; }
  /** Blends colour c over the pixel with weight t. */
  blend(x, y, c, t) { if (x >= 0 && y >= 0 && x < this.w && y < this.h) this.set(x, y, mix(this.get(x, y), c, t)); }
  rect(x, y, w, h, c) { for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) this.set(x + i, y + j, c); }
  png() { return encodePng(this.w, this.h, this.px); }
}
function rng(seed) { // mulberry32
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const hash = (...n) => n.reduce((a, v) => (Math.imul(a ^ (v | 0), 0x9e3779b1) >>> 0), 0x811c9dc5);
const id32 = (s) => [...s].reduce((a, ch) => (Math.imul(a, 31) + ch.charCodeAt(0)) >>> 0, 7);

// ---------------------------------------------------------------- tile painters (8x8 art pixels per tile)
// A tile's `c` is {cv, ox, oy, tx, ty, pal, rnd, above}: the canvas, the tile's pixel origin, its tile coordinates,
// the pack palette, a seeded random and the character of the tile above (for edge shadows).
function floorTile(c) {
  const { cv, ox, oy, pal, rnd, tx, ty } = c;
  if (pal.floorPainter) { pal.floorPainter(c, c.under); return; }
  const [f0, f1, f2, f3] = pal.floor.map(hex);
  const style = pal.floorStyle;
  const joint = (hash(tx, ty, 7) % 3) * 2 + 1;
  for (let y = 0; y < TILE; y++) {
    for (let x = 0; x < TILE; x++) {
      let col = f0;
      if (style === 'flag') { // paving stones: grout on two edges, speckle and the odd mossy corner
        col = x === 0 || y === 0 ? f3 : f0;
        const r = rnd();
        if (col === f0 && r < 0.14) col = f1;
        else if (col === f0 && r < 0.26) col = f2;
        if (col === f0 && ((tx + ty) & 1) && x > 4 && y > 4) col = mix(f0, f2, 0.4);
      } else if (style === 'tile') { // indoor floor tiles: 4x4 checker with grout on two edges
        col = x === 0 || y === 0 ? f3 : (((x >> 2) + (y >> 2) + tx + ty) & 1 ? f1 : f0);
        if (col === f0 && rnd() < 0.06) col = f2;
      } else if (style === 'plank') { // two horizontal planks per tile, joints that stagger
        col = y === 3 || y === 7 ? f3 : (y < 4 ? f0 : f1);
        const jx = y < 4 ? joint : (joint + 4) & 7;
        if (y !== 3 && y !== 7 && x === jx) col = f3;
        if (col === f0 || col === f1) {
          const r = rnd();
          if (r < 0.12) col = f2; else if (r < 0.2) col = mix(col, f1, 0.6);
        }
        if ((x === 1 || x === 6) && (y === 1 || y === 5)) col = shade(f2, 0.8); // nail heads
      } else { // parquet: 4x4 blocks striped one way or the other
        const flip = ((x >> 2) + (y >> 2) + tx + ty) & 1;
        col = (flip ? x : y) & 1 ? f1 : f0;
        if ((x & 3) === 0 || (y & 3) === 0) col = f3;
        else if (rnd() < 0.08) col = f2;
      }
      cv.set(ox + x, oy + y, col);
    }
  }
  if (c.above === '#') for (let x = 0; x < TILE; x++) { cv.blend(ox + x, oy, [0, 0, 0], 0.4); cv.blend(ox + x, oy + 1, [0, 0, 0], 0.18); }
}
function wallTile(c) { // brick: four courses of two pixels, the last pixel of each a mortar line
  const { cv, ox, oy, pal, rnd, tx, ty } = c;
  const [w0, w1, w2, mortar] = pal.wall.map(hex);
  for (let course = 0; course < 4; course++) {
    const off = course & 1 ? 2 : 0;
    for (let x = 0; x < TILE; x++) {
      const brick = Math.floor((x + off) / 4);
      const roll = hash(tx, ty, course, brick) % 6;
      const tone = roll === 0 ? w1 : roll === 1 ? w2 : w0;
      for (let j = 0; j < 2; j++) {
        let col = tone;
        if (j === 1 || (x + off) % 4 === 0) col = mortar;
        else if (rnd() < 0.15) col = shade(tone, 0.88);
        cv.set(ox + x, oy + course * 2 + j, col);
      }
    }
  }
  for (let x = 0; x < TILE; x++) cv.blend(ox + x, oy, hex(pal.glow), 0.12); // a faint top edge light
}
function lowWallTile(c) { // crate, bench or shelf top seen from above
  const { cv, ox, oy, pal } = c;
  const [l0, l1] = pal.low.map(hex);
  cv.rect(ox, oy, TILE, TILE, shade(l0, 0.7));
  cv.rect(ox + 1, oy + 1, 6, 6, l0);
  for (let y = 1; y < 7; y += 2) cv.rect(ox + 1, oy + y, 6, 1, l1);
  cv.set(ox + 1, oy + 1, shade(l1, 1.15));
}
function doorTile(c) {
  const { cv, ox, oy, pal } = c;
  const d0 = hex(pal.door[0]), d1 = hex(pal.door[1]), metal = hex(pal.door[2]);
  cv.rect(ox, oy, TILE, TILE, shade(d0, 0.55));
  for (let x = 1; x < 7; x++) for (let y = 0; y < TILE; y++) cv.set(ox + x, oy + y, x % 2 ? d0 : d1); // vertical planks
  cv.rect(ox + 1, oy + 1, 6, 1, metal); // iron bands
  cv.rect(ox + 1, oy + 6, 6, 1, metal);
  cv.set(ox + 5, oy + 3, hex(pal.glow));
  cv.set(ox + 5, oy + 4, hex(pal.glow)); // the handle
}
function itemTile(c, colour) { // a glowing diamond in the item's own colour
  floorTile(c);
  const { cv, ox, oy } = c;
  const col = hex(colour);
  for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) {
    const d = Math.abs(x - 3.5) + Math.abs(y - 3.5);
    if (d <= 2) cv.set(ox + x, oy + y, d <= 1 ? mix(col, [255, 255, 255], 0.55) : col);
    else if (d <= 3.5) cv.blend(ox + x, oy + y, col, 0.35);
  }
}
function npcTile(c) { // a small round rug under the figure
  floorTile(c);
  const { cv, ox, oy, pal } = c;
  const a = hex(pal.accent);
  for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) {
    const d = Math.hypot(x - 3.5, y - 3.5);
    if (d <= 3.7) cv.set(ox + x, oy + y, d >= 2.6 ? shade(a, 0.7) : shade(a, 0.95));
  }
}
function spawnTile(c) { // corner brackets
  floorTile(c);
  const { cv, ox, oy, pal } = c;
  const g = hex(pal.glow);
  for (const [x, y] of [[0, 0], [1, 0], [0, 1], [6, 0], [7, 0], [7, 1], [0, 6], [0, 7], [1, 7], [7, 6], [7, 7], [6, 7]]) cv.blend(ox + x, oy + y, g, 0.7);
}
function triggerTile(c) { // a faint cross
  floorTile(c);
  const { cv, ox, oy, pal } = c;
  const a = shade(hex(pal.accent), 0.8);
  for (let i = 1; i < 7; i++) { cv.blend(ox + i, oy + 3, a, 0.55); cv.blend(ox + 3, oy + i, a, 0.55); }
}
function exitTile(c) { // a glowing pad
  floorTile(c);
  const { cv, ox, oy, pal } = c;
  const g = hex(pal.glow);
  for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) {
    const ring = Math.max(Math.abs(x - 3.5), Math.abs(y - 3.5));
    const t = ring > 3 ? 0.9 : ring > 2 ? 0.25 : (x + y) & 1 ? 0.5 : 0.65;
    cv.blend(ox + x, oy + y, g, t);
  }
  cv.rect(ox + 3, oy + 3, 2, 2, [255, 255, 255]);
}
function propTile(c) { // floor with a soft shadow
  floorTile(c);
  const { cv, ox, oy } = c;
  for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) if (Math.hypot(x - 3.5, (y - 4) * 1.25) <= 3.4) cv.blend(ox + x, oy + y, [0, 0, 0], 0.32);
}

// ---------------------------------------------------------------- mask colours (one per tile character)
// The same letter means the same thing in every map of a pack; a map's `defs` may redefine a letter for itself.
const MASK = {
  '#': '000000', h: '202020', '.': 'ffffff',
  S: 'ff00ff', a: '00ffff', b: 'ff80ff',
  D: '804000', E: '804040',
  k: 'ffff00', l: 'ffcc00', m: 'ffaa00',
  N: '00ff00', M: '00c000',
  T: '0000ff', X: 'ff8000', Y: 'ff4000',
  P: '8000ff', Q: 'c000ff',
  // sample_city (the old town): more spawns, triggers and npcs, floor variants that all read as floor, and one wall colour per building
  e: 'ffffff', q: 'ffffff', g: '305020', K: '505080', L: '606090', j: '4080ff',
  s: 'a000ff', o: 'd000ff', f: 'e000ff', t: 'f000ff', x: '9000ff',
  c: 'ff80c0', d: '80c0ff', W: '0080ff', R: '008000',
  r: 'ffffff', n: 'ffffff', u: 'ffffff', v: 'ffffff', w: 'ffffff', Z: 'ffffff', z: 'ffffff', p: 'ffffff', y: 'ffffff',
  // life pass: bench, townsfolk, route spawns, shelter and flee spots, notice board, stool, brazier, cliff exits
  1: 'a0ff00', 2: '00ff80', 3: '00a000', 4: 'ff8040', 5: 'ff4080', 6: '80ff40', 7: 'c08040', 8: 'a0a0ff', 9: 'ffc080', 0: 'ff9040', '$': '40ff80', '%': '40c0ff',
  A: '101010', B: '202040', C: '203020', F: '402020', G: '404020', H: '204040', I: '402040', J: '606060',
};

// ---------------------------------------------------------------- skin recolouring
let atlasCache = null;
function atlas() {
  if (!atlasCache) atlasCache = decodePng(fs.readFileSync(ATLAS));
  return atlasCache;
}
/** The default mannequin atlas, recoloured: lightness picks a point on the `body` ramp (dark to light colour); the head
 *  block at the top left (the first 32x16 pixels of the usual layout) uses the `head` ramp instead. */
function makeSkin({ body, head }) {
  const a = atlas();
  const cv = new Canvas(a.w, a.h);
  for (let y = 0; y < a.h; y++) for (let x = 0; x < a.w; x++) {
    const d = (y * a.w + x) * 4;
    const al = a.px[d + 3];
    if (!al) continue;
    const lum = (0.3 * a.px[d] + 0.59 * a.px[d + 1] + 0.11 * a.px[d + 2]) / 255;
    const t = Math.max(0, Math.min(1, (lum - 0.3) / 0.35));
    const ramp = y < 16 && x < 32 ? head : body;
    const keep = al < 255 && lum > 0.9; // the pale see-through pixels (eyes and the like) stay as they are
    cv.set(x, y, keep ? [a.px[d], a.px[d + 1], a.px[d + 2]] : mix(hex(ramp[0]), hex(ramp[1]), t), al);
  }
  return cv.png();
}

// ---------------------------------------------------------------- prop models (.bbmodel, cubes with a built-in texture)
const uuidOf = (s) => {
  const h = crypto.createHash('sha1').update(s).digest('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
};
/** A small Blockbench "free" model: `cubes` are {from, to, c} in model units (Y up) and c indexes `colours`. The
 *  texture is generated here: one 4x4 pixel swatch per colour, and every face of a cube points at its swatch. */
function makeBbmodel(name, colours, cubes) {
  const side = 64;
  const cv = new Canvas(side, side);
  const r = rng(hash(name.length, colours.length));
  colours.forEach((cs, i) => {
    const base = hex(cs), cx = (i % 16) * 4, cy = Math.floor(i / 16) * 4;
    for (let y = 0; y < 4; y++) for (let x = 0; x < 4; x++) cv.set(cx + x, cy + y, shade(base, 0.94 + r() * 0.12));
  });
  const elements = cubes.map((cu, n) => {
    const cx = (cu.c % 16) * 4, cy = Math.floor(cu.c / 16) * 4;
    const face = () => ({ uv: [cx + 1, cy + 1, cx + 3, cy + 3], texture: 0 });
    return {
      name: `${name}_cube${n}`, type: 'cube', uuid: uuidOf(`${name}${n}`), box_uv: false, rescale: false, locked: false,
      from: cu.from, to: cu.to, autouv: 0, color: 0, origin: [0, 0, 0], rotation: [0, 0, 0],
      faces: { north: face(), south: face(), east: face(), west: face(), up: face(), down: face() },
    };
  });
  return JSON.stringify({
    meta: { format_version: '4.10', model_format: 'free', box_uv: false },
    name, model_identifier: '', visible_box: [1, 1, 0], variable_placeholders: '', variable_placeholder_buttons: [],
    timeline_setups: [], unhandled_root_fields: {}, resolution: { width: side, height: side },
    elements, outliner: elements.map((e) => e.uuid),
    textures: [{
      path: '', name: `${name}.png`, folder: '', namespace: '', id: 0, group: '', width: side, height: side,
      uv_width: side, uv_height: side, particle: false, use_as_default: false, layers_enabled: false,
      sync_to_project: '', render_mode: 'default', render_sides: 'auto', frame_time: 1, frame_order_type: 'loop',
      frame_order: '', frame_interpolate: false, visible: true, internal: true, saved: true,
      uuid: uuidOf(`${name}-tex`), relative_path: `${name}.png`,
      source: 'data:image/png;base64,' + cv.png().toString('base64'),
    }],
  });
}

// ---------------------------------------------------------------- pack building
// props that several packs share (usable by the player and by NPC routines)
const BENCH = {
  colours: ['#7a5a34', '#4a3420'],
  cubes: [
    { from: [-10, 5, -3], to: [10, 7, 3], c: 0 }, { from: [-10, 7, -3.5], to: [10, 13, -2.5], c: 0 },
    { from: [-9, 0, -2], to: [-7, 5, 2], c: 1 }, { from: [7, 0, -2], to: [9, 5, 2], c: 1 },
  ],
};

// ---------------------------------------------------------------- 16x16 item icons
function iconKind(id, item) {
  const t = (id + ' ' + item.name).toLowerCase();
  if (/key/.test(t)) return 'key';
  if (/tea|broth|tonic|mead|flask|oil|potion/.test(t)) return 'bottle';
  if (/letter|ledger|card|note|scroll|bookmark|parcel/.test(t)) return 'scroll';
  if (/seed/.test(t)) return 'seed';
  if (/can|crank/.test(t)) return 'tool';
  if (/stamp/.test(t)) return 'stamp';
  if (/flare/.test(t)) return 'flare';
  if (/pouch|pepper/.test(t)) return 'pouch';
  return 'coin';
}
function makeIcon(kind, base) {
  const cv = new Canvas(16, 16);
  const dark = shade(base, 0.45), light = mix(base, [255, 255, 255], 0.5), out = [20, 18, 24];
  const R = (x, y, w, h, c) => cv.rect(x, y, w, h, c);
  const disc = (cx, cy, r, c) => { for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) if ((x - cx) ** 2 + (y - cy) ** 2 <= r * r) cv.set(x, y, c); };
  if (kind === 'key') {
    disc(5, 5, 4, out); disc(5, 5, 3, base); disc(5, 5, 1, out);
    R(7, 7, 7, 3, out); R(8, 8, 6, 1, base); R(11, 10, 2, 3, out); R(11, 10, 1, 2, base); R(13, 10, 1, 2, base);
  } else if (kind === 'bottle') {
    R(6, 1, 4, 3, out); R(7, 2, 2, 1, [150, 110, 70]); R(5, 4, 6, 1, out); R(3, 5, 10, 10, out);
    R(4, 5, 8, 9, dark); R(4, 8, 8, 6, base); R(5, 9, 2, 3, light); R(4, 14, 8, 1, out);
  } else if (kind === 'scroll') {
    R(2, 3, 12, 10, out); R(3, 4, 10, 8, [232, 220, 188]); R(2, 2, 12, 2, out); R(2, 12, 12, 2, out);
    R(3, 3, 10, 1, base); R(3, 12, 10, 1, base); for (let y = 6; y < 11; y += 2) R(4, y, 7, 1, dark);
  } else if (kind === 'seed') {
    disc(8, 8, 6, out); disc(8, 8, 5, base); disc(6, 6, 2, light); R(7, 2, 2, 3, [90, 160, 80]);
  } else if (kind === 'tool') {
    R(3, 5, 9, 9, out); R(4, 6, 7, 7, base); R(4, 6, 7, 2, light); R(11, 7, 4, 2, out); R(12, 7, 3, 1, base); R(1, 7, 3, 4, out); R(2, 8, 1, 2, dark);
  } else if (kind === 'stamp') {
    R(5, 1, 6, 6, out); R(6, 2, 4, 4, light); R(2, 7, 12, 3, out); R(3, 8, 10, 1, base); R(4, 10, 8, 4, out); R(5, 10, 6, 3, dark); R(4, 14, 8, 1, base);
  } else if (kind === 'flare') {
    R(6, 5, 4, 10, out); R(7, 6, 2, 8, base); R(6, 8, 4, 1, light); disc(8, 3, 3, [255, 190, 80]); disc(8, 3, 1, [255, 255, 220]);
  } else if (kind === 'pouch') {
    R(5, 2, 6, 3, out); R(6, 3, 4, 1, dark); disc(8, 10, 5, out); disc(8, 10, 4, base); R(5, 5, 6, 1, light); disc(6, 9, 1, light);
  } else {
    disc(8, 8, 7, out); disc(8, 8, 6, base); disc(8, 8, 4, dark); disc(8, 8, 3, base); R(6, 6, 2, 2, light);
  }
  return cv.png();
}
function colourOf(id) { let h = 0; for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0; return hsl2rgb((h % 360) / 360, 0.55, 0.58); }
function hsl2rgb(h, s, l) {
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s, p2 = 2 * l - q;
  const f = (t) => { t = (t + 1) % 1; return Math.round(255 * (t < 1 / 6 ? p2 + (q - p2) * 6 * t : t < 0.5 ? q : t < 2 / 3 ? p2 + (q - p2) * (2 / 3 - t) * 6 : p2)); };
  return [f(h + 1 / 3), f(h), f(h - 1 / 3)];
}

function buildPack(spec) {
  const id = spec.id;
  const dir = path.join(MODS, id);
  fs.rmSync(dir, { recursive: true, force: true });
  for (const d of ['maps', 'items', 'scenes', 'skins', 'props', 'battles', 'quests', 'routines']) fs.mkdirSync(path.join(dir, d), { recursive: true });
  fs.mkdirSync(path.join(dir, 'items', 'icons'), { recursive: true });
  const w = (rel, data) => fs.writeFileSync(path.join(dir, rel), data);
  const json = (rel, v) => w(rel, JSON.stringify(v, null, 2) + '\n');

  json('pack.json', {
    schema: 'lr.pack/1', id, name: spec.name, version: '1.0.0', author: 'KyttusLab samples', description: spec.description,
    start: { map: spec.maps[0].id, spawn: spec.maps[0].startSpawn },
    story: {
      title: spec.title, era: 'sample', order: spec.order,
      player: { name: spec.player.name, role: spec.player.role, speed: spec.player.speed, start_items: spec.player.items || [], start_flags: spec.player.flags || [], stats: spec.player.stats },
      multiplayer: true,
    },
  });
  for (const [iid, it] of Object.entries(spec.items)) {
    const name = `${iid.replace(/^sample_[a-z]+_/, '')}.png`;
    w(`items/icons/${name}`, makeIcon(iconKind(iid, it), colourOf(iid)));
    it.icon = name;
    it.stack = it.use ? (it.use.heal ? 9 : 5) : 1;
  }
  json('items/items.json', { schema: 'lr.story.items/1', items: spec.items });
  json('battles/battles.json', { schema: 'lr.story.battles/1', battles: spec.battles });
  json('quests/quests.json', { schema: 'lr.story.quests/1', quests: spec.quests });
  if (spec.routines) json('routines/routines.json', { schema: 'lr.story.routines/1', routines: spec.routines });
  json('scenes/scenes.json', { schema: 'lr.story.scenes/1', scenes: spec.scenes });
  for (const [name, ramps] of Object.entries(spec.skins)) w(`skins/${id}_${name}.png`, makeSkin(ramps));
  for (const [name, p] of Object.entries(spec.props)) w(`props/${id}_${name}.bbmodel`, makeBbmodel(`${id}_${name}`, p.colours, p.cubes));
  for (const map of spec.maps) writeMap(spec, map, w, json);
  return dir;
}

function writeMap(spec, map, w, json) {
  const rows = map.rows;
  const mw = rows[0].length, mh = rows.length;
  rows.forEach((r, i) => { if (r.length !== mw) throw new Error(`${map.id}: row ${i} is ${r.length} wide, expected ${mw}`); });
  const defs = { ...spec.legend, ...map.defs };
  const pal = map.palette || spec.palette;
  const baseAt = (tx, ty) => (map.base ? map.base[ty]?.[tx] : rows[ty]?.[tx]) ?? '#';
  const art = new Canvas(mw * TILE, mh * TILE);
  const mask = new Canvas(mw, mh);
  const used = new Set();
  for (let ty = 0; ty < mh; ty++) {
    for (let tx = 0; tx < mw; tx++) {
      const ch = rows[ty][tx];
      const def = defs[ch];
      if (!def) throw new Error(`${map.id}: no legend entry for '${ch}' at ${tx},${ty}`);
      used.add(ch);
      mask.set(tx, ty, hex('#' + MASK[ch]));
      const c = { cv: art, ox: tx * TILE, oy: ty * TILE, tx, ty, pal, def, ch, rnd: rng(hash(tx, ty, id32(map.id))), above: ty > 0 ? rows[ty - 1][tx] : '#',
        under: baseAt(tx, ty), baseAt: (dx, dy) => baseAt(tx + dx, ty + dy) };
      const k = def.kind;
      if (k === 'wall') (pal.wallPainter ? pal.wallPainter : ch === '#' ? wallTile : lowWallTile)(c);
      else if (k === 'floor') floorTile(c);
      else if (k === 'door') doorTile(c);
      else if (k === 'item') itemTile(c, def.color || spec.palette.glow);
      else if (k === 'npc') npcTile(c);
      else if (k === 'spawn') spawnTile(c);
      else if (k === 'trigger') triggerTile(c);
      else if (k === 'exit') exitTile(c);
      else if (k === 'prop') propTile(c);
      const dm = map.dim;
      if (dm && tx >= dm.x0 && tx <= dm.x1 && ty >= dm.y0 && ty <= dm.y1) {
        for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) art.blend(c.ox + x, c.oy + y, [4, 4, 12], dm.a);
      }
    }
  }
  w(`maps/${map.id}.png`, art.png());
  w(`maps/${map.id}-mask.png`, mask.png());
  const legend = {};
  for (const ch of used) legend['#' + MASK[ch]] = defs[ch];
  json(`maps/${map.id}.json`, { schema: 'lr.story.map/1', id: map.id, name: map.name, art: `${map.id}.png`, mask: `${map.id}-mask.png`, ...(map.env || {}), legend });
}

// ================================================================ pack 1: the greenhouse (stone, soil, leaf green)
function greenhouse() {
  const id = 'sample_greenhouse';
  const p = (n) => `${id}_${n}`;
  const can = p('watering_can'), key = p('brass_key'), seed = p('moon_seed'), tea = p('mint_tea');
  return {
    id, name: 'Sample: The Greenhouse', title: 'Sample: The Greenhouse', order: 50,
    description: 'A sample story pack for modders: a potting shed, a locked dome, a key, a seed and a watering can. Shows player stats, a healing item, a quest, a boss fight, a small pest fight, items, a key door, NPC skins, a prop, a once scene, choices with conditions, a rejoining branch and an exit op.',
    player: { name: 'Wren', role: 'Gardener', speed: 1.1, items: [tea], stats: { max_hp: 40, attack: 6, defense: 2, speed: 5 } },
    palette: {
      floorStyle: 'flag', floor: ['#8d9a82', '#a3b096', '#76846d', '#5d6a55'],
      wall: ['#c9bfa3', '#d9d0b6', '#aaa088', '#7c735f'], low: ['#8a5a32', '#a87240'],
      door: ['#6b4a2a', '#805a35', '#2f3a3d'], accent: '#5fa35a', glow: '#9be08c',
    },
    legend: {
      '#': { kind: 'wall', height: 3, color: '#cfc6aa' }, h: { kind: 'wall', height: 1.1, color: '#8a5a32' }, '.': { kind: 'floor' },
      S: { kind: 'spawn', name: 'start' }, a: { kind: 'spawn', name: 'from_dome' }, b: { kind: 'spawn', name: 'pest' },
      D: { kind: 'door', if: { has: key }, locked: 'The dome door is locked. There is a brass key somewhere in the shed.', height: 3, color: '#4f6b5c' },
      k: { kind: 'item', item: key, color: '#e8c24a' }, l: { kind: 'item', item: seed, color: '#9ad8ff' },
      N: { kind: 'npc', name: 'Moss', scene: p('moss'), skin: p('moss'), color: '#6fbf8a' },
      T: { kind: 'trigger', scene: p('intro'), once: true },
      X: { kind: 'exit', map: p('dome'), spawn: 'from_potting' }, P: { kind: 'prop', prop: p('planter'), yaw: 0 },
    },
    maps: [
      {
        id: p('potting_shed'), name: 'Potting shed', startSpawn: 'start',
        env: { walls: 'plaster', floor: 'paving', ceiling: { color: '#b9b095' } }, // low plaster roof over the shed
        rows: [
          '################',
          '#.S..hhh...N.###',
          '#.T......k...###',
          '#...P........###',
          '#..........a.DX#',
          '#............###',
          '#.h......b...###',
          '#............###',
          '#............###',
          '################',
        ],
        defs: {},
      },
      {
        id: p('dome'), name: 'The dome', startSpawn: 'from_potting',
        env: { walls: 'plaster', floor: 'paving', ceiling: { color: '#d4ebe6' } }, // a glassy pale roof
        rows: [
          '##############',
          '#.....hh.....#',
          '#............#',
          '#.........N..#',
          '#Xa.P....l...#',
          '#............#',
          '#..hh.....b..#',
          '#............#',
          '##############',
        ],
        defs: {
          a: { kind: 'spawn', name: 'from_potting' },
          b: { kind: 'spawn', name: 'boss' },
          N: { kind: 'npc', name: 'Fern', scene: p('fern'), skin: p('fern'), color: '#e08aa8', body: 'blocky_slim' },
          X: { kind: 'exit', map: p('potting_shed'), spawn: 'from_dome' },
        },
      },
    ],
    items: {
      [key]: { name: 'Brass key', description: 'Heavy and green at the edges. It fits the dome door.' },
      [seed]: { name: 'Moon seed', description: 'A pale seed that is faintly cool to hold. Fern has been waiting for it.' },
      [can]: { name: 'Watering can', description: 'Dented tin, with a rose that sprays sideways.' },
      [tea]: { name: 'Mint tea', description: 'A warm flask. Drink it to heal 15 HP.', use: { heal: 15 } },
    },
    battles: {
      [p('aphids')]: {
        name: 'Aphid swarm', boss: false, max_hp: 14, defense: 0, move_speed: 2.6, skin: p('moss'), spawn: 'pest',
        attacks: [{ name: 'Nip', kind: 'melee', damage: 3, range: 1.6, windup: 0.6, cooldown: 1.6, weight: 1, text: 'The swarm lunges.' }],
        intro: 'A cloud of aphids lifts off the seedlings and comes straight at you.',
        win: p('aphids_won'),
      },
      [p('blight')]: {
        name: 'The Blight', boss: true, max_hp: 90, defense: 2, move_speed: 2.4, skin: p('fern'), spawn: 'boss',
        attacks: [
          { name: 'Thorn lash', kind: 'melee', damage: 7, range: 2.2, windup: 0.7, cooldown: 1.4, weight: 3, text: 'Thorns whip out.' },
          { name: 'Root burst', kind: 'slam', damage: 10, range: 9, windup: 1.1, cooldown: 3.2, weight: 2, radius: 2.6, text: 'The soil heaves.' },
          { name: 'Spore bolt', kind: 'shot', damage: 8, range: 12, windup: 0.9, cooldown: 3, weight: 2, min_phase: 1, radius: 0.35, speed: 9, text: 'A spore bolt spits out.' },
        ],
        phases: [66, 33],
        intro: 'The pale shoot splits and darkens. Something old climbs out of the bed.',
        win: p('finale'), lose: p('blight_lost'),
      },
    },
    quests: {
      [p('garden')]: {
        title: 'Wake the dome', description: 'Fern wants the pale seed in the soil.',
        steps: [
          { text: 'Get the watering can from Moss', done_if: { flag: p('got_can') } },
          { text: 'Plant the moon seed', done_if: { flag: p('planted') } },
          { text: 'Water the new bed', done_if: { flag: p('watered') } },
          { text: 'Face what grows', done_if: { flag: 'won_' + p('blight') } },
        ],
      },
    },
    skins: {
      moss: { body: ['#2d4a2a', '#9bc27a'], head: ['#8a5f43', '#e0b48e'] },
      fern: { body: ['#5a2a45', '#e9a0c0'], head: ['#6b4a38', '#d9a581'] },
    },
    props: {
      planter: {
        colours: ['#8a5a32', '#a87240', '#3b2a1e', '#3f8f3f', '#63b85a', '#e87ab0'],
        cubes: [
          { from: [-7, 0, -3], to: [7, 6, 3], c: 0 }, { from: [-7, 5, -3], to: [7, 6, 3], c: 1 },
          { from: [-6, 6, -2], to: [6, 7, 2], c: 2 },
          { from: [-5, 7, -1], to: [-3, 12, 1], c: 3 }, { from: [-1, 7, -1], to: [1, 15, 1], c: 4 }, { from: [3, 7, -1], to: [5, 11, 1], c: 3 },
          { from: [-1.5, 15, -1.5], to: [1.5, 17, 1.5], c: 5 },
        ],
      },
    },
    scenes: {
      [p('intro')]: {
        once: true,
        lines: [
          'Rain taps the roof of the potting shed. It smells of wet clay and mint.',
          { speaker: 'Wren', text: 'The dome has been shut since spring. Moss said a key might be about.' },
        ],
        do: [{ op: 'toast', text: 'Press E facing someone to talk. Step on a glowing marker to pick it up.' }],
      },
      [p('moss')]: {
        lines: [{ speaker: 'Moss', text: 'Wren! Mind the benches. What can I do for you?' }],
        choices: [
          { text: 'What is under the dome?', next: p('moss_dome') },
          { text: 'Where would a key be?', next: p('moss_key') },
          {
            text: 'Could I borrow the watering can?', if: { not_flag: p('got_can') },
            do: [{ op: 'give', item: can }, { op: 'set_flag', flag: p('got_can') }, { op: 'toast', text: 'You take the watering can.' }],
            next: p('moss_bye'),
          },
          { text: 'Deal with the aphids in the corner', if: { not_flag: 'won_' + p('aphids') }, do: [{ op: 'battle', battle: p('aphids') }] },
          { text: 'Never mind.' },
        ],
      },
      [p('aphids_won')]: {
        lines: [{ speaker: 'Moss', text: 'Clean as a whistle. Have some tea for the road.' }],
        do: [{ op: 'give', item: tea }],
      },
      [p('moss_dome')]: {
        lines: [{ speaker: 'Moss', text: 'Fern lives in there. She grows what nobody else can keep alive.' }],
        next: p('moss_bye'),
      },
      [p('moss_key')]: {
        lines: [{ speaker: 'Moss', text: 'On the bench, near the pots. I never could keep it in my pocket.' }],
        next: p('moss_bye'),
      },
      // the branches above all end here: a rejoin
      [p('moss_bye')]: { lines: [{ speaker: 'Moss', text: 'Off you go, then. And wipe your boots.' }] },
      [p('fern')]: {
        lines: [{ speaker: 'Fern', text: 'Welcome to the dome. Do not touch the blue ones. What brings you here?' }],
        choices: [
          {
            text: 'Plant the moon seed', if: { has: seed },
            do: [{ op: 'take', item: seed }, { op: 'set_flag', flag: p('planted') }, { op: 'toast', text: 'The seed is in the soil.' }],
            next: p('bloom'),
          },
          {
            text: 'Water the new bed', if: { all: [{ flag: p('planted') }, { has: can }, { not_flag: p('watered') }] },
            do: [{ op: 'set_flag', flag: p('watered') }], next: p('blight_wake'),
          },
          { text: 'Ask about the seed', if: { all: [{ not: { has: seed } }, { not_flag: p('planted') }] }, next: p('fern_seed') },
          { text: 'Take the shortcut back to the shed', do: [{ op: 'exit', map: p('potting_shed'), spawn: 'from_dome' }] },
          { text: 'Not now.' },
        ],
      },
      [p('bloom')]: {
        lines: [{ speaker: 'Fern', text: 'There. Now it wants water. The can is in the shed if you have not got it.' }],
        next: p('fern'),
      },
      [p('fern_seed')]: {
        lines: [{ speaker: 'Fern', text: 'A moon seed. I left it by the far bench. It glows a little when nobody looks.' }],
        next: p('fern'),
      },
      [p('blight_wake')]: {
        lines: ['The water sinks in. The bed shudders.'],
        do: [{ op: 'battle', battle: p('blight') }],
      },
      [p('blight_lost')]: {
        lines: [{ speaker: 'Fern', text: 'Back on your feet, Wren. It is still there. Water it again.' }],
        do: [{ op: 'set_flag', flag: p('watered'), value: false }],
      },
      [p('finale')]: {
        once: true,
        lines: [
          'The Blight folds back into the soil. Then a single pale shoot unfolds, and the dome gets very quiet.',
          { speaker: 'Fern', text: 'Thank you, Wren.' },
        ],
        do: [{ op: 'set_flag', flag: p('finished') }, { op: 'toast', text: 'The end of the sample.' }],
      },
    },
  };
}

// ================================================================ pack 2: the lighthouse (night blue, driftwood, lamp gold)
function lighthouse() {
  const id = 'sample_lighthouse';
  const p = (n) => `${id}_${n}`;
  const oil = p('oil_flask'), crank = p('lamp_crank'), broth = p('fish_broth'), flare = p('flare');
  return {
    id, name: 'Sample: The Lighthouse', title: 'Sample: The Lighthouse', order: 51,
    routines: {
      // Bram tends the lamp, then looks out from the stair door
      [p('bram_watch')]: { loop: true, steps: [{ go: 'prop:5,5', do: 'work', secs: 10 }, { go: 'from_lamp', do: 'look', secs: 6 }, { go: 'prop:11,5', do: 'work', secs: 8 }] },
    },
    description: 'A sample story pack for modders: a lighthouse keeper, a flag-locked stair door and a lamp to light, plus a stormy outdoor cliff path. Shows storm weather on an outdoor map, a keeper routine, a usable lamp and bench,  start flags, a door that opens on a flag, player stats, a heal item, a damage item, a quest, a boss fight, a lamp prop, two NPC skins, take ops and a scene chain that rejoins.',
    player: { name: 'Tamsin', role: "Keeper's apprentice", speed: 0.95, flags: [p('started')], items: [broth, flare], stats: { max_hp: 36, attack: 5, defense: 3, speed: 5 } },
    palette: {
      floorStyle: 'plank', floor: ['#6b5a47', '#7d6a54', '#4f4132', '#33291f'],
      wall: ['#5f6b86', '#7482a0', '#485169', '#2c3347'], low: ['#3c4a66', '#566785'],
      door: ['#4a3a2c', '#5e4a38', '#9aa6b8'], accent: '#2f5f8a', glow: '#ffd36a',
    },
    legend: {
      '#': { kind: 'wall', height: 3.2, color: '#6a7894' }, h: { kind: 'wall', height: 1.0, color: '#3c4a66' }, '.': { kind: 'floor' },
      S: { kind: 'spawn', name: 'start' }, a: { kind: 'spawn', name: 'from_lamp' },
      D: { kind: 'door', if: { flag: p('bram_ok') }, locked: 'The stair door is bolted. Bram has the say on who goes up.', height: 3.2, color: '#4a3a2c' },
      k: { kind: 'item', item: crank, color: '#c0c8d4' }, l: { kind: 'item', item: oil, color: '#e0a030' },
      N: { kind: 'npc', name: 'Bram', scene: p('bram'), skin: p('bram'), color: '#7a98c8', routine: p('bram_watch') },
      T: { kind: 'trigger', scene: p('intro'), once: true },
      X: { kind: 'exit', map: p('lamp_room'), spawn: 'from_base' }, P: { kind: 'prop', prop: p('lamp'), yaw: 45, use: { action: 'work', text: 'Tend the lamp' } },
    },
    maps: [
      {
        id: p('base'), name: 'Lighthouse base', startSpawn: 'start',
        env: { walls: 'concrete', ceiling: { color: '#4a5266' } },
        rows: [
          '######X#########',
          '######D#########',
          '#S..N.a.....k..#',
          '#.T....hhh.....#',
          '#.............5$',
          '#....P.....P...#',
          '#..............#',
          '#..........l...#',
          '#.h............#',
          '#..............#',
          '################',
        ],
        defs: {
          5: { kind: 'spawn', name: 'from_cliff' },
          $: { kind: 'exit', map: p('cliff'), spawn: 'from_base' },
        },
      },
      {
        id: p('lamp_room'), name: 'The lamp room', startSpawn: 'from_base',
        env: { walls: 'concrete', ceiling: { color: '#2c3347' } },
        rows: [
          '############',
          '#..........#',
          '#..hh..hh..#',
          '#....NP....#',
          '#..........#',
          '#.......b..#',
          '#.....T....#',
          '#..........#',
          '#.....a....#',
          '######X#####',
        ],
        defs: {
          a: { kind: 'spawn', name: 'from_base' },
          b: { kind: 'spawn', name: 'boss' },
          N: { kind: 'npc', name: 'Pell', scene: p('pell'), skin: p('pell'), color: '#e8b04a', body: 'blocky_slim' },
          T: { kind: 'trigger', scene: p('lamp_view'), once: true },
          X: { kind: 'exit', map: p('base'), spawn: 'from_lamp' },
        },
      },
      {
        id: p('cliff'), name: 'The cliff path', startSpawn: 'from_base',
        env: {
          outdoor: true, sky: { top: '#3c4658', bottom: '#8a96a6' }, walls: 'concrete', floor: 'paving',
          weather: { kind: 'storm', intensity: 0.7, changes: [{ kind: 'storm', weight: 3 }, { kind: 'rain', weight: 2 }, { kind: 'fog', weight: 1 }], every_secs: 60 },
        },
        rows: [
          '##############',
          '#hhhhhhhhhhh.#',
          '#............#',
          '#..1.........#',
          '#............#',
          '#....a.......#',
          '######%#######',
        ],
        defs: {
          a: { kind: 'spawn', name: 'from_base' },
          1: { kind: 'prop', prop: p('bench'), yaw: 0, use: { action: 'sit', text: 'Sit and watch the sea', seat_height: 0.45 } },
          '%': { kind: 'exit', map: p('base'), spawn: 'from_cliff' },
        },
      },
    ],
    items: {
      [oil]: { name: 'Oil flask', description: 'Lamp oil, still warm from the stove.' },
      [crank]: { name: 'Lamp crank', description: 'A brass crank for the lens clockwork. It is stamped with a gull.' },
      [broth]: { name: 'Fish broth', description: 'Salty and hot. Drink it to heal 20 HP.', use: { heal: 20 } },
      [flare]: { name: 'Signal flare', description: 'Hurts the enemy within 6 m of you for 18 damage.', use: { damage: 18 } },
    },
    battles: {
      [p('wraith')]: {
        name: 'Storm Wraith', boss: true, max_hp: 100, defense: 3, move_speed: 2.8, skin: p('bram'), spawn: 'boss',
        attacks: [
          { name: 'Gale rush', kind: 'charge', damage: 9, range: 10, windup: 1.0, cooldown: 3, weight: 3, speed: 11, radius: 1, text: 'The wraith crouches into the wind.' },
          { name: 'Lightning', kind: 'shot', damage: 8, range: 14, windup: 0.8, cooldown: 2.6, weight: 3, radius: 0.3, speed: 12, text: 'A bolt snaps toward you.' },
          { name: 'Whirlpool', kind: 'slam', damage: 12, range: 10, windup: 1.2, cooldown: 3.6, weight: 2, min_phase: 2, radius: 3, text: 'The air spirals.' },
        ],
        phases: [66, 33],
        intro: 'The beam finds something in the fog, and it turns to look back.',
        win: p('dawn'), lose: p('wraith_lost'),
      },
    },
    quests: {
      [p('light')]: {
        title: 'Light the lamp', description: 'Get the lamp lit before the fog.',
        steps: [
          { text: 'Ask Bram to draw the bolt', done_if: { flag: p('bram_ok') } },
          { text: 'Fill the lamp and wind the crank', done_if: { all: [{ flag: p('filled') }, { flag: p('wound') }] } },
          { text: 'Light the lamp', done_if: { flag: p('lit') } },
          { text: 'Drive off the Storm Wraith', done_if: { flag: 'won_' + p('wraith') } },
        ],
      },
    },
    skins: {
      bram: { body: ['#1e2a40', '#7f95bd'], head: ['#7b5240', '#d8a98a'] },
      pell: { body: ['#5a3a12', '#f0c060'], head: ['#5e4636', '#c99a78'] },
    },
    props: {
      bench: BENCH,
      lamp: {
        colours: ['#9a7a2a', '#d8b040', '#ffe08a', '#3a4258'],
        cubes: [
          { from: [-4, 0, -4], to: [4, 3, 4], c: 3 }, { from: [-1, 3, -1], to: [1, 11, 1], c: 0 },
          { from: [-3.5, 11, -3.5], to: [3.5, 18, 3.5], c: 2 }, { from: [-2.5, 12, -2.5], to: [2.5, 17, 2.5], c: 1 },
          { from: [-4.5, 18, -4.5], to: [4.5, 20, 4.5], c: 0 },
        ],
      },
    },
    scenes: {
      [p('intro')]: {
        once: true,
        lines: [
          'The wind has a voice tonight. Even down here, the tower hums.',
          { speaker: 'Tamsin', text: 'Bram will want the lamp lit before the fog sets in.' },
        ],
        do: [{ op: 'toast', text: 'Talk to Bram. The stair door opens on his say.' }],
      },
      [p('bram')]: {
        lines: [{ speaker: 'Bram', text: 'Back on your feet, apprentice. The glass will not light itself.' }],
        choices: [
          { text: 'How bad is the storm?', next: p('bram_storm') },
          { text: 'How does the lamp work?', next: p('bram_lamp') },
          { text: 'Any last advice?', if: { flag: p('bram_ok') }, next: p('bram_last') },
          { text: 'I will get to it.' },
        ],
      },
      [p('bram_storm')]: {
        lines: [{ speaker: 'Bram', text: 'Bad enough. Fog by dusk. Ships out there are counting on us.' }],
        next: p('bram_ok'),
      },
      [p('bram_lamp')]: {
        lines: [{ speaker: 'Bram', text: 'Oil in the well, crank wound, then a flame. In that order, mind.' }],
        next: p('bram_ok'),
      },
      // the two answers above rejoin here and open the stair door
      [p('bram_ok')]: {
        lines: [{ speaker: 'Bram', text: 'Right. I am drawing the bolt. Oil flask is in the corner, crank by the east wall.' }],
        do: [{ op: 'set_flag', flag: p('bram_ok') }, { op: 'toast', text: 'The stair door is unbolted.' }],
      },
      [p('bram_last')]: { lines: [{ speaker: 'Bram', text: 'Keep your hand off the lens. It bites.' }] },
      [p('lamp_view')]: {
        once: true,
        lines: ['The lamp room is all glass and dark water. The gallery rail hums in the wind.'],
      },
      [p('pell')]: {
        lines: [{ speaker: 'Pell', text: 'The lens is ready when you are.' }],
        choices: [
          {
            text: 'Fill the lamp with oil', if: { all: [{ has: oil }, { not_flag: p('filled') }] },
            do: [{ op: 'take', item: oil }, { op: 'set_flag', flag: p('filled') }, { op: 'toast', text: 'The well is full.' }],
            next: p('pell'),
          },
          {
            text: 'Wind the crank', if: { all: [{ has: crank }, { not_flag: p('wound') }] },
            do: [{ op: 'take', item: crank }, { op: 'set_flag', flag: p('wound') }, { op: 'toast', text: 'The clockwork ticks.' }],
            next: p('pell'),
          },
          {
            text: 'Light the lamp', if: { all: [{ flag: p('filled') }, { flag: p('wound') }, { not_flag: p('lit') }] },
            do: [{ op: 'set_flag', flag: p('lit') }], next: p('lit'),
          },
          { text: 'Go back down', do: [{ op: 'exit', map: p('base'), spawn: 'from_lamp' }] },
          { text: 'Not yet.' },
        ],
      },
      [p('lit')]: {
        once: true,
        lines: [
          'The flame catches. The lens turns, and a long white beam walks out over the water.',
          { speaker: 'Pell', text: 'There she is. Wait. What is that on the glass?' },
        ],
        do: [{ op: 'battle', battle: p('wraith') }],
      },
      [p('wraith_lost')]: {
        lines: [{ speaker: 'Pell', text: 'You are alive. The lamp still burns. Try again.' }],
        do: [{ op: 'set_flag', flag: p('lit'), value: false }],
      },
      [p('dawn')]: {
        once: true,
        lines: ['The wraith comes apart in the beam. By dawn the fog has gone.'],
        do: [{ op: 'toast', text: 'The end of the sample.' }],
      },
    },
  };
}

// ================================================================ pack 3: the archive (plum, parchment, ink)
function archive() {
  const id = 'sample_archive';
  const p = (n) => `${id}_${n}`;
  const book = p('notebook'), mark = p('bookmark'), card = p('reading_card'), stamp = p('ink_stamp'), ledger = p('sealed_ledger'), tonic = p('ink_tonic');
  return {
    id, name: 'Sample: The Archive', title: 'Sample: The Archive', order: 52,
    routines: {
      [p('odo_read')]: { loop: true, steps: [{ go: 'prop:9,3', do: 'read', secs: 10 }, { go: 'from_stacks', do: 'look', secs: 6 }] },
    },
    description: 'A sample story pack for modders: a reading hall, the stacks and a sealed nook. Shows player stats, a heal item, a quest, a boss fight, start items, two doors with all/any/not conditions, a trigger inside a locked nook, a bookshelf prop and give/take ops.',
    player: { name: 'Isa', role: 'Cataloguer', speed: 1, items: [book, tonic], stats: { max_hp: 32, attack: 5, defense: 2, speed: 6 } },
    palette: {
      floorStyle: 'parquet', floor: ['#7a4f3a', '#8d5f47', '#5c3a2b', '#3f261c'],
      wall: ['#5b4268', '#6e5380', '#46304f', '#2a1c33'], low: ['#4a3a2a', '#6a5238'],
      door: ['#3f2a1e', '#523724', '#c0a050'], accent: '#7a3a6a', glow: '#f0d890',
    },
    legend: {
      '#': { kind: 'wall', height: 3.4, color: '#6b5080' }, h: { kind: 'wall', height: 1.6, color: '#6a5238' }, '.': { kind: 'floor' },
      S: { kind: 'spawn', name: 'start' }, a: { kind: 'spawn', name: 'from_stacks' },
      D: { kind: 'door', if: { any: [{ has: card }, { flag: p('odo_waved') }] }, locked: 'Readers only past this door. Ask Odo for a reading card.', height: 3.4, color: '#523724' },
      E: { kind: 'door', if: { all: [{ flag: p('stamped') }, { not: { flag: p('sealed') } }] }, locked: 'The nook is sealed until the returns are stamped.', height: 3.4, color: '#2f2640' },
      k: { kind: 'item', item: mark, color: '#e86a7a' }, l: { kind: 'item', item: ledger, color: '#d8c070' }, m: { kind: 'item', item: stamp, color: '#7ad0c0' },
      N: { kind: 'npc', name: 'Odo', scene: p('odo'), skin: p('odo'), color: '#a080c0', routine: p('odo_read') },
      T: { kind: 'trigger', scene: p('intro'), once: true },
      X: { kind: 'exit', map: p('stacks'), spawn: 'from_hall' }, P: { kind: 'prop', prop: p('shelf'), yaw: 180, use: { action: 'read', text: 'Browse the shelf' } },
    },
    maps: [
      {
        id: p('hall'), name: 'Reading hall', startSpawn: 'start',
        env: { walls: 'brick', ceiling: { color: '#4b3858' } }, // a vault-dark ceiling
        rows: [
          '##############',
          '#S...N.....k.#',
          '#.T..........#',
          '#....hh..P...#',
          '#............#',
          '#..P.........#',
          '#.....a......#',
          '######D#######',
          '######X#######',
          '##############',
        ],
        defs: {},
      },
      {
        id: p('stacks'), name: 'The stacks', startSpawn: 'from_hall',
        env: { walls: 'brick', ceiling: { color: '#3a2a46' } },
        rows: [
          '######X#########',
          '#.....a........#',
          '#.hhhh..hhhh...#',
          '#..............#',
          '#.hhhh..hhhh.N.#',
          '#.......b......#',
          '#.hhhh..hhhh...#',
          '#.....m........#',
          '#..............#',
          '######E#########',
          '######.lT#######',
          '################',
        ],
        defs: {
          a: { kind: 'spawn', name: 'from_hall' },
          b: { kind: 'spawn', name: 'boss' },
          N: { kind: 'npc', name: 'Quill', scene: p('quill'), skin: p('quill'), color: '#e0c070', body: 'blocky_slim' },
          T: { kind: 'trigger', scene: p('nook'), once: true },
          X: { kind: 'exit', map: p('hall'), spawn: 'from_stacks' },
        },
      },
    ],
    items: {
      [book]: { name: 'Notebook', description: 'Your cataloguing notebook. Most pages are about ink.' },
      [mark]: { name: 'Bookmark', description: 'Embroidered with a small red bird. Someone lost it on the hall floor.' },
      [card]: { name: 'Reading card', description: 'Stamped READER. It is good for the stacks.' },
      [stamp]: { name: 'Ink stamp', description: 'RETURNED, in teal. The ink never quite dries.' },
      [ledger]: { name: 'Sealed ledger', description: 'A thin ledger with a wax seal. It is older than the building.' },
      [tonic]: { name: 'Ink tonic', description: 'Bitter and black. Drink it to heal 12 HP.', use: { heal: 12 } },
    },
    battles: {
      [p('inkbeast')]: {
        name: 'The Ink Beast', boss: true, max_hp: 95, defense: 2, move_speed: 2.6, skin: p('odo'), spawn: 'boss',
        attacks: [
          { name: 'Claw', kind: 'melee', damage: 8, range: 2.2, windup: 0.6, cooldown: 1.3, weight: 3, text: 'Wet claws sweep.' },
          { name: 'Blot rush', kind: 'charge', damage: 10, range: 9, windup: 0.9, cooldown: 3, weight: 2, speed: 10, radius: 1, text: 'It drops low and bolts.' },
          { name: 'Ink pool', kind: 'slam', damage: 11, range: 10, windup: 1.2, cooldown: 3.4, weight: 2, min_phase: 1, radius: 2.8, text: 'Ink spreads under your feet.' },
        ],
        phases: [66, 33],
        intro: 'The seal on the ledger cracks, and the ink runs off the page and stands up.',
        win: p('nook_won'), lose: p('nook_lost'),
      },
    },
    quests: {
      [p('returns')]: {
        title: 'Stamp the returns', description: 'Quill is waiting on the teal stamp.',
        steps: [
          { text: 'Get a reading card from Odo', done_if: { flag: p('got_card') } },
          { text: 'Stamp the returns for Quill', done_if: { flag: p('stamped') } },
          { text: 'Open the nook and face the ledger', done_if: { flag: 'won_' + p('inkbeast') } },
        ],
      },
    },
    skins: {
      odo: { body: ['#2b1d3a', '#a98bd0'], head: ['#7c5a45', '#e2b894'] },
      quill: { body: ['#4a3a0e', '#e8cf70'], head: ['#5a4234', '#c79a74'] },
    },
    props: {
      shelf: {
        colours: ['#5a3a22', '#7a5230', '#b03a4a', '#3a6aa8', '#d8b040', '#4a8a5a'],
        cubes: [
          { from: [-7, 0, -3], to: [7, 20, -2], c: 0 }, { from: [-7, 0, -3], to: [-6, 20, 3], c: 1 }, { from: [6, 0, -3], to: [7, 20, 3], c: 1 },
          { from: [-6, 6, -2], to: [6, 7, 3], c: 1 }, { from: [-6, 13, -2], to: [6, 14, 3], c: 1 }, { from: [-7, 19, -3], to: [7, 20, 3], c: 0 },
          { from: [-5, 7, -1.5], to: [-3, 12, 2], c: 2 }, { from: [-2.5, 7, -1.5], to: [-0.5, 12.5, 2], c: 3 }, { from: [1, 7, -1.5], to: [3, 11, 2], c: 4 },
          { from: [-5, 14, -1.5], to: [-2, 18, 2], c: 5 }, { from: [0, 14, -1.5], to: [2, 18.5, 2], c: 2 }, { from: [3, 14, -1.5], to: [5, 17, 2], c: 3 },
        ],
      },
    },
    scenes: {
      [p('intro')]: {
        once: true,
        lines: [
          'The hall smells of paper, glue and lamps that have been lit too long.',
          { speaker: 'Isa', text: 'Three crates of returns, and nobody to stamp them. Odo will know.' },
        ],
        do: [{ op: 'toast', text: 'A reading card opens the door to the stacks.' }],
      },
      [p('odo')]: {
        lines: [{ speaker: 'Odo', text: 'Quiet, please. How can the desk help?' }],
        choices: [
          { text: 'May I have a reading card?', if: { not_flag: p('got_card') }, do: [{ op: 'give', item: card }, { op: 'set_flag', flag: p('got_card') }], next: p('odo_card') },
          { text: 'What do I do in the stacks?', next: p('odo_stacks') },
          { text: 'Just passing through.' },
        ],
      },
      [p('odo_card')]: {
        lines: [{ speaker: 'Odo', text: 'There. Do not fold the corners.' }],
        do: [{ op: 'toast', text: 'You are given a reading card.' }],
        next: p('odo_bye'),
      },
      [p('odo_stacks')]: {
        lines: [{ speaker: 'Odo', text: 'Quill is down there with the returns. She wants the teal stamp.' }],
        next: p('odo_bye'),
      },
      // the card and stacks answers rejoin here
      [p('odo_bye')]: { lines: [{ speaker: 'Odo', text: 'Mind the step by the south door.' }] },
      [p('quill')]: {
        lines: [{ speaker: 'Quill', text: 'Shh. Oh, a reader. Have you brought the stamp?' }],
        choices: [
          {
            text: 'Hand over the ink stamp', if: { has: stamp },
            do: [{ op: 'take', item: stamp }, { op: 'set_flag', flag: p('stamped') }, { op: 'toast', text: 'The returns are stamped.' }],
            next: p('quill_thanks'),
          },
          { text: 'Where would I find a stamp?', if: { all: [{ not: { has: stamp } }, { not_flag: p('stamped') }] }, next: p('quill_task') },
          { text: 'What is behind the sealed door?', if: { flag: p('stamped') }, next: p('quill_nook') },
          { text: 'Back to the hall', do: [{ op: 'exit', map: p('hall'), spawn: 'from_stacks' }] },
          { text: 'Not now.' },
        ],
      },
      [p('quill_task')]: {
        lines: [{ speaker: 'Quill', text: 'On the floor by the south shelves, where I dropped it. Teal ink.' }],
        next: p('quill'),
      },
      [p('quill_thanks')]: {
        lines: [{ speaker: 'Quill', text: 'Finally. The nook is open to you. Do not read the ledger aloud.' }],
        next: p('quill'),
      },
      [p('quill_nook')]: {
        lines: [{ speaker: 'Quill', text: 'A ledger older than the building. Nobody knows who wrote it.' }],
        next: p('quill'),
      },
      [p('nook')]: {
        once: true,
        lines: ['The nook is small and silent. On the shelf, the ledger waits with its seal unbroken.'],
        do: [{ op: 'battle', battle: p('inkbeast') }],
      },
      [p('nook_won')]: {
        lines: ['The ink drains back into the ledger. The seal closes, quiet again.'],
        do: [{ op: 'toast', text: 'The end of the sample.' }],
      },
      [p('nook_lost')]: {
        lines: [{ speaker: 'Quill', text: 'Careful with that ledger. Try again when you are ready.' }],
      },
    },
  };
}

// ================================================================ pack 4: the old town (timber, stone, cobbles, late afternoon)
const lum = (c) => 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
/** Town floors seen from above: cobbles (p street, q market square, y alley) and a packed dirt path (e). `ch` is the floor character under the tile. */
function townFloor(c, ch) {
  const { cv, ox, oy, rnd, tx, ty } = c;
  if (ch === 'e') { // packed earth: ruts, pebbles and straw
    const earth = [124, 94, 62];
    for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) {
      const r = rnd();
      let col = r < 0.16 ? shade(earth, 0.84) : r < 0.28 ? shade(earth, 1.1) : earth;
      if (y === 2 || y === 5) col = shade(col, 0.82); // wheel ruts
      cv.set(ox + x, oy + y, col);
    }
    for (let i = 0; i < 3; i++) cv.set(ox + Math.floor(rnd() * 8), oy + Math.floor(rnd() * 8), [168, 160, 146]); // pebbles
    for (let i = 0; i < 2; i++) { const sx = Math.floor(rnd() * 6), sy = Math.floor(rnd() * 8); cv.set(ox + sx, oy + sy, [206, 178, 96]); cv.set(ox + sx + 1, oy + sy, [206, 178, 96]); } // straw
    return;
  }
  const tones = ch === 'q' ? [[150, 146, 138], [166, 160, 148], [138, 128, 116], [158, 140, 118]]
    : ch === 'y' ? [[96, 94, 92], [108, 102, 96], [86, 80, 76], [104, 92, 80]]
      : [[124, 122, 118], [140, 134, 124], [108, 102, 96], [132, 116, 98]];
  const gap = [52, 50, 50];
  cv.rect(ox, oy, TILE, TILE, gap);
  for (let row = 0; row < 2; row++) {
    const off = (row & 1) * 2 + (hash(tx, ty, row) & 1);
    for (let k = 0; k < 2; k++) {
      const sx = off + k * 4, sy = row * 4;
      const tone = tones[hash(tx, ty, row, k) % tones.length];
      for (let j = 0; j < 3; j++) for (let i = 0; i < 3; i++) {
        if ((i === 2 && j === 2) || (i === 0 && j === 2 && rnd() < 0.5)) continue; // rounded corners
        let col = tone;
        if (i === 0 && j === 0) col = shade(tone, 1.14);
        else if (i === 2 || j === 2) col = shade(tone, 0.88);
        cv.set(ox + ((sx + i) & 7), oy + sy + j, col);
      }
    }
  }
  if (ch === 'y' && hash(tx, ty, 5) % 3 === 0) for (let y = 2; y < 6; y++) for (let x = 2; x < 7; x++) if (Math.hypot(x - 4.5, (y - 4) * 1.4) < 2.2) cv.blend(ox + x, oy + y, [24, 30, 40], 0.45); // a puddle
  if (ch === 'q' && hash(tx, ty, 9) % 9 === 0) for (let i = 0; i < 4; i++) cv.set(ox + Math.floor(rnd() * 8), oy + Math.floor(rnd() * 8), [206, 178, 96]); // dropped straw
}
/** Town buildings seen from above: thatch or clay tile on the timber houses, slate on the stone ones, crenels on the town wall, a canopy for the forest. */
function townWall(c) {
  const { cv, ox, oy, rnd, ch, def, tx, ty } = c;
  const base = hex(def.color || '#777777');
  if (ch === 'g') { // tree canopy
    cv.rect(ox, oy, TILE, TILE, shade(base, 0.7));
    for (let i = 0; i < 6; i++) { const bx = Math.floor(rnd() * 8), by = Math.floor(rnd() * 8); cv.rect(ox + bx - 1, oy + by - 1, 3, 3, shade(base, 0.9 + rnd() * 0.5)); }
    for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) if (rnd() < 0.1) cv.set(ox + x, oy + y, shade(base, 0.5));
    return;
  }
  if (ch === '#') { // the town wall: a stone walk with blocks and crenels
    cv.rect(ox, oy, TILE, TILE, shade(base, 0.7));
    for (const [bx, by] of [[0, 0], [4, 0], [2, 4], [6, 4]]) cv.rect(ox + (bx & 7), oy + by, 3, 3, shade(base, 0.9 + rnd() * 0.3));
    if ((tx + ty) % 2 === 0) for (let i = 0; i < TILE; i++) cv.set(ox + i, oy, shade(base, 1.2));
    return;
  }
  const slate = lum(base) < 135;
  const thatch = ch === 'B' || ch === 'F' || ch === 'I';
  const roof = slate ? shade(mix(base, [60, 66, 80], 0.5), 0.9) : thatch ? shade(mix(base, [120, 84, 44], 0.7), 0.82) : mix(base, [168, 88, 60], 0.7);
  cv.rect(ox, oy, TILE, TILE, roof);
  if (!thatch) { // tile or slate courses
    for (let y = 1; y < TILE; y += 2) for (let x = 0; x < TILE; x++) cv.set(ox + x, oy + y, shade(roof, 0.82));
    for (let y = 0; y < TILE; y += 2) for (let x = (y >> 1) & 1; x < TILE; x += 2) cv.set(ox + x, oy + y, shade(roof, 1.1));
  } else { // thatch: loose straw
    for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) { const r = rnd(); if (r < 0.22) cv.set(ox + x, oy + y, shade(roof, 0.8)); else if (r < 0.34) cv.set(ox + x, oy + y, shade(roof, 1.2)); }
  }
  for (let i = 0; i < TILE; i++) { cv.set(ox + i, oy + 3, shade(roof, 1.25)); cv.set(ox + i, oy + 4, shade(roof, 0.65)); } // the ridge
  if ((def.height || 3) >= 12) cv.rect(ox + 2, oy + 2, 4, 4, shade(roof, 0.55)); // spire or tower top
  else if (hash(tx, ty, 11) % 9 === 0) { cv.rect(ox + 5, oy + 1, 2, 2, [86, 80, 78]); cv.set(ox + 5, oy + 1, [60, 56, 56]); } // a chimney
}
function town() {
  const id = 'sample_city';
  const p = (n) => `${id}_${n}`;
  const parcel = p('parcel'), key = p('shop_key'), mead = p('mead'), pepper = p('pepper_pouch');
  const W = 40, H = 30;
  const base = Array.from({ length: H }, () => Array(W).fill('#'));
  const top = Array.from({ length: H }, () => Array(W).fill('#'));
  const put = (g, x0, y0, x1, y1, ch) => { for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) g[y][x] = ch; };
  const both = (x0, y0, x1, y1, ch) => { put(base, x0, y0, x1, y1, ch); put(top, x0, y0, x1, y1, ch); };
  // the forest outside the wall, then the buildings (each block is one wall letter: its own colour and height)
  both(33, 1, 38, 28, 'g');
  both(1, 1, 10, 4, 'A'); both(1, 5, 7, 11, 'H'); both(8, 5, 10, 11, 'B');
  both(11, 1, 18, 7, 'K'); both(19, 1, 22, 4, 'L'); both(19, 5, 21, 7, 'C');
  both(23, 1, 31, 4, 'F'); both(24, 5, 31, 11, 'G');
  both(1, 16, 7, 28, 'I'); both(11, 22, 16, 28, 'J'); both(17, 22, 21, 28, 'C');
  both(22, 16, 26, 28, 'B'); both(29, 16, 31, 28, 'F');
  // streets: the main street (four wide), the market square, a lane up the east side, a winding lane in the south west
  both(1, 12, 31, 15, 'p'); both(32, 13, 32, 14, 'p');
  both(11, 8, 21, 20, 'q');
  both(22, 5, 23, 11, 'p');
  both(9, 16, 10, 22, 'p'); both(8, 23, 9, 28, 'p');
  // the dirt path out of the gate, winding north into the trees
  both(33, 13, 35, 14, 'e'); both(35, 11, 36, 13, 'e'); both(36, 8, 37, 10, 'e'); both(37, 5, 38, 7, 'e');
  // the back alley: two tiles wide, dead end
  both(27, 16, 28, 27, 'y');
  // doors: the smithy (barred) and the tavern
  put(base, 4, 10, 4, 11, 'p'); top[11][4] = 'D'; top[10][4] = 'X';
  put(base, 27, 11, 27, 11, 'p'); top[11][27] = 'Y';
  const mark = (x, y, ch) => { top[y][x] = ch; };
  mark(4, 12, 'a'); mark(27, 12, 'd'); mark(2, 14, 'S'); mark(5, 14, 'T');
  mark(15, 10, 'N'); mark(18, 18, 'M'); mark(9, 27, 'l'); mark(37, 6, 'j');
  // life: benches, the notice board, the guard's two ends, a shelter spot under the market awning, a flee spot at the west gate
  for (const [x, y] of [[17, 10], [14, 14], [18, 14]]) mark(x, y, '1');
  mark(10, 12, '8'); mark(14, 11, '6'); mark(1, 14, '7'); mark(2, 15, '4'); mark(30, 14, '5'); mark(14, 16, '2'); mark(6, 15, '3');
  mark(27, 17, 'W'); mark(28, 17, 'W'); mark(27, 20, 'b'); mark(27, 24, 'c');
  mark(28, 19, 'Q'); mark(27, 22, 'P'); mark(28, 25, 'Q'); mark(28, 27, 'R');
  // market props
  for (const [x, y] of [[13, 10], [19, 10], [13, 18]]) mark(x, y, 's');
  mark(16, 14, 'x'); mark(19, 17, 'o'); mark(20, 19, 't');
  for (const [x, y] of [[11, 11], [21, 11], [11, 17], [21, 17], [3, 12], [30, 12], [8, 13]]) mark(x, y, 'f');
  for (const [x, y] of [[12, 12], [14, 18], [20, 16], [31, 15], [24, 12]]) mark(x, y, 'Q');
  for (const [x, y] of [[12, 19], [20, 9], [25, 15]]) mark(x, y, 'P');
  const rows = top.map((r) => r.join(''));
  const baseRows = base.map((r) => r.join(''));
  const wall = (h, color) => ({ kind: 'wall', height: h, color });
  const smithy = {
    floorStyle: 'flag', floor: ['#7d7a74', '#8f8b82', '#686560', '#3c3a38'],
    wall: ['#8a8782', '#9a968e', '#74716c', '#4a4846'], low: ['#4a3a2c', '#6a5240'],
    door: ['#3a2a1c', '#5a4026', '#5a5e66'], accent: '#a85a30', glow: '#ffb060',
  };
  const tavern = {
    floorStyle: 'plank', floor: ['#6a4e30', '#7a5c38', '#54402a', '#2e2216'],
    wall: ['#e0d4b4', '#ece2c6', '#c8bc9c', '#5a3e26'], low: ['#6a4a2a', '#8a6638'],
    door: ['#3a2a1c', '#5a4026', '#5a5e66'], accent: '#8a2e2e', glow: '#ffc070',
  };
  return {
    id, name: 'Sample: The Old Town', title: 'Sample: The Old Town', order: 53,
    description: 'A sample story pack for modders: a medieval old town on a late afternoon, with timber houses and a stone church, keep and town wall 4 to 14 m tall, cobbled streets, a market square with a well, stalls and a hay cart, a dirt path out of the gate, a dead-end back alley, a barred smithy door and two indoor rooms (a stone smithy and a timber tavern with a dirt floor, both with beamed ceilings). Shows outdoor, sky, walls and floor map fields (timber, stone, cobble, dirt), changing weather, benches, a stool, a well, stalls, a brazier and a notice board that the player can use, NPC routines (work, sit, chat, wander, patrol, warm hands) with rain shelter and fight flee spots, a cutpurse fight, a bandit captain boss fight, townsfolk with choices and a quest.',
    player: { name: 'Jun', role: 'Messenger', speed: 1.05, items: [parcel, mead, mead, pepper], stats: { max_hp: 38, attack: 6, defense: 2, speed: 6 } },
    palette: {
      floorStyle: 'flag', floor: ['#7c7a76', '#8c867c', '#6c665e', '#343232'],
      wall: ['#b89c6e', '#cdb98c', '#8a7048', '#4a3a28'], low: ['#6a5238', '#8a6a48'],
      door: ['#3a2a1c', '#5a4026', '#5a5e66'], accent: '#a8452e', glow: '#ffd07a',
      floorPainter: townFloor, wallPainter: townWall,
    },
    legend: {
      '#': wall(5, '#4e5058'), A: wall(6, '#cdb98c'), B: wall(8, '#b89c6e'), C: wall(5, '#d8c8a0'), F: wall(9, '#a88858'),
      G: wall(7, '#d2bd90'), H: wall(5, '#6e6a66'), I: wall(8, '#c0a478'), J: wall(6, '#77797c'),
      K: wall(13, '#85857f'), L: wall(14, '#6a6c72'), g: wall(6, '#2f4a2c'),
      h: wall(1.1, '#6a5238'), '.': { kind: 'floor' },
      p: { kind: 'floor' }, q: { kind: 'floor' }, e: { kind: 'floor' }, y: { kind: 'floor' },
      S: { kind: 'spawn', name: 'start', facing: 'east' }, a: { kind: 'spawn', name: 'from_shop' }, d: { kind: 'spawn', name: 'from_cafe' },
      b: { kind: 'spawn', name: 'mugger' }, c: { kind: 'spawn', name: 'boss' },
      D: { kind: 'door', if: { has: key }, locked: 'The smithy door is barred. Maud at the bread stall keeps the spare key.', height: 3, color: '#5a4026' },
      l: { kind: 'item', item: mead, color: '#e8b84a' }, m: { kind: 'item', item: pepper, color: '#d8c8a0' },
      N: { kind: 'npc', name: 'Maud', scene: p('vera'), skin: p('vera'), color: '#d8b080', body: 'blocky_slim', routine: p('maud_day') },
      M: { kind: 'npc', name: 'Garrick', scene: p('marlo'), skin: p('marlo'), color: '#8a9a5a', routine: p('garrick_day') },
      2: { kind: 'npc', name: 'Tilda', scene: p('tilda'), skin: p('tilda'), color: '#7a9ad0', body: 'blocky_slim', routine: p('tilda_day') },
      3: { kind: 'npc', name: 'Warden Oss', scene: p('oss'), skin: p('oss'), color: '#6a7080', routine: p('oss_patrol') },
      4: { kind: 'spawn', name: 'g_west' }, 5: { kind: 'spawn', name: 'g_east' }, 6: { kind: 'spawn', name: 'shelter_market' }, 7: { kind: 'spawn', name: 'flee_west' },
      1: { kind: 'prop', prop: p('bench'), yaw: 0, use: { action: 'sit', text: 'Sit', seat_height: 0.45 } },
      8: { kind: 'prop', prop: p('notice_board'), yaw: 0, use: { action: 'read', text: 'Read the notices', scene: p('notice') } },
      R: { kind: 'npc', name: 'Black Sal', scene: p('sal'), skin: p('sal'), color: '#8a2e2e' },
      T: { kind: 'trigger', scene: p('intro'), once: true },
      W: { kind: 'trigger', scene: p('mugger_start'), once: false },
      j: { kind: 'trigger', scene: p('gate'), once: false },
      X: { kind: 'exit', map: p('shop'), spawn: 'from_city' }, Y: { kind: 'exit', map: p('cafe'), spawn: 'from_street' },
      P: { kind: 'prop', prop: p('crate'), yaw: 20 }, Q: { kind: 'prop', prop: p('barrel'), yaw: 90 },
      s: { kind: 'prop', prop: p('stall'), yaw: 0, use: { action: 'work', text: 'Tend the stall' } }, o: { kind: 'prop', prop: p('hay_cart'), yaw: 70 }, f: { kind: 'prop', prop: p('lantern'), yaw: 0 },
      t: { kind: 'prop', prop: p('trough'), yaw: 0 }, x: { kind: 'prop', prop: p('well'), yaw: 0, use: { action: 'draw_water', text: 'Draw water' } },
    },
    maps: [
      {
        id: p('streets'), name: 'Old Town', startSpawn: 'start', rows, base: baseRows, defs: {},
        env: {
          outdoor: true, sky: { top: '#6f9fd0', bottom: '#f0cf9a' }, walls: 'timber', floor: 'cobble',
          weather: { kind: 'clear', intensity: 0.6, changes: [{ kind: 'clear', weight: 3 }, { kind: 'rain', weight: 2 }, { kind: 'fog', weight: 1 }, { kind: 'leaves', weight: 1 }], every_secs: 90 },
        },
        dim: { x0: 27, y0: 16, x1: 28, y1: 27, a: 0.4 }, // the alley is shaded
      },
      {
        id: p('shop'), name: 'The smithy', startSpawn: 'from_city', palette: smithy,
        env: { walls: 'stone', floor: 'tiles', ceiling: { color: '#3a2c20' } },
        rows: [
          '##############',
          '#..hhhhhhhh..#',
          '#.......N....#',
          '#..........0.#',
          '#.P..........#',
          '#.....a......#',
          '#............#',
          '######X#######',
        ],
        defs: {
          '#': wall(3, '#8a8782'),
          a: { kind: 'spawn', name: 'from_city' },
          N: { kind: 'npc', name: 'Pru', scene: p('pru'), skin: p('pru'), color: '#c07040', body: 'blocky_slim', routine: p('pru_forge') },
          P: { kind: 'prop', prop: p('crate'), yaw: 20, use: { action: 'work', text: 'Hammer at the bench' } },
          0: { kind: 'prop', prop: p('brazier'), yaw: 0, use: { action: 'warm_hands', text: 'Warm your hands' } },
          X: { kind: 'exit', map: p('streets'), spawn: 'from_shop' },
        },
      },
      {
        id: p('cafe'), name: 'The Gilded Boar', startSpawn: 'from_street', palette: tavern,
        env: { walls: 'timber', floor: 'dirt', ceiling: { color: '#3e2c1c' } },
        rows: [
          '############',
          '#.hhhhhh...#',
          '#..........#',
          '#.....N..m.#',
          '#..P....9..#',
          '#.....a....#',
          '#..........#',
          '#####X######',
        ],
        defs: {
          '#': wall(3, '#d8ccac'),
          a: { kind: 'spawn', name: 'from_street' },
          N: { kind: 'npc', name: 'Ellis', scene: p('ellis'), skin: p('ellis'), color: '#a0584a', routine: p('ellis_bar') },
          9: { kind: 'prop', prop: p('stool'), yaw: 0, use: { action: 'sit', text: 'Sit', seat_height: 0.5 } },
          X: { kind: 'exit', map: p('streets'), spawn: 'from_cafe' },
        },
      },
    ],
    items: {
      [parcel]: { name: 'Sealed letter', description: 'A folded letter closed with a blob of red wax. It is addressed to Pru the smith.' },
      [key]: { name: 'Smithy key', description: 'A heavy iron key on a leather thong: SMITHY, BACK DOOR.' },
      [mead]: { name: 'Mead flask', description: 'A stoppered clay flask of honey mead. Drink it to heal 14 HP.', use: { heal: 14 } },
      [pepper]: { name: 'Pepper pouch', description: 'A pouch of ground pepper. Hurts the enemy within 6 m of you for 16 damage.', use: { damage: 16 } },
    },
    battles: {
      [p('mugger')]: {
        name: 'Alley cutpurse', boss: false, max_hp: 26, defense: 0, move_speed: 3, skin: p('mugger'), spawn: 'mugger',
        attacks: [
          { name: 'Shove', kind: 'melee', damage: 4, range: 1.6, windup: 0.6, cooldown: 1.5, weight: 2, text: 'He shoves you hard against the wall.' },
          { name: 'Lunge', kind: 'charge', damage: 6, range: 7, windup: 0.9, cooldown: 3, weight: 1, speed: 9, radius: 0.9, text: 'He ducks his head and lunges with a dagger.' },
        ],
        intro: 'A cutpurse steps out from behind the barrels. "Hand over the letter, messenger."',
        win: p('mugger_won'), lose: p('mugger_lost'),
      },
      [p('sal')]: {
        name: 'Black Sal', boss: true, max_hp: 110, defense: 2, move_speed: 2.6, skin: p('sal'), spawn: 'boss',
        attacks: [
          { name: 'Cudgel', kind: 'melee', damage: 9, range: 2.2, windup: 0.7, cooldown: 1.4, weight: 3, text: 'Sal swings an iron-banded cudgel.' },
          { name: 'Thrown axe', kind: 'shot', damage: 7, range: 12, windup: 0.8, cooldown: 2.8, weight: 2, min_phase: 1, radius: 0.3, speed: 10, text: 'A hand axe spins out of the dark.' },
          { name: 'Barrel slam', kind: 'slam', damage: 11, range: 9, windup: 1.1, cooldown: 3.4, weight: 2, min_phase: 1, radius: 2.4, text: 'Sal brings a barrel down on the cobbles.' },
        ],
        phases: [66, 33],
        intro: 'Black Sal, captain of the road bandits, cracks his knuckles. The alley is very narrow.',
        win: p('sal_won'), lose: p('sal_lost'),
      },
    },
    routines: {
      // Maud works her stall, then rests on the bench; in rain she runs under the awning, in a fight to the west gate
      [p('maud_day')]: { loop: true, shelter: 'shelter_market', flee: 'flee_west', steps: [{ go: 'prop:13,10', do: 'work', secs: 14 }, { go: 'prop:17,10', do: 'sit', secs: 10 }] },
      // Garrick and Tilda chat at the well, then wander
      [p('garrick_day')]: { loop: true, shelter: 'shelter_market', flee: 'flee_west', steps: [{ go: 'prop:16,14', do: 'chat', with: 'Tilda', secs: 12 }, { do: 'wander', radius: 3, secs: 14 }] },
      [p('tilda_day')]: { loop: true, shelter: 'shelter_market', flee: 'flee_west', steps: [{ go: 'prop:16,14', do: 'chat', with: 'Garrick', secs: 12 }, { do: 'wander', radius: 3, secs: 14 }, { go: 'prop:14,14', do: 'sit', secs: 8 }] },
      // the warden walks the main street from gate to gate
      [p('oss_patrol')]: { loop: true, steps: [{ go: 'g_east', do: 'look', secs: 3 }, { go: 'g_west', do: 'look', secs: 3 }] },
      [p('pru_forge')]: { loop: true, steps: [{ go: 'prop:2,4', do: 'work', secs: 12 }, { go: 'prop:11,3', do: 'warm_hands', secs: 8 }] },
      [p('ellis_bar')]: { loop: true, steps: [{ do: 'idle', secs: 6 }, { go: 'prop:8,4', do: 'sit', secs: 10 }, { go: 'from_street', do: 'look', secs: 5 }] },
    },
    quests: {
      [p('delivery')]: {
        title: 'The last letter', description: 'One sealed letter left, and the smithy door is barred.',
        steps: [
          { text: 'Get the smithy key from Maud at the bread stall', done_if: { flag: p('got_key') } },
          { text: 'Deliver the sealed letter to Pru in the smithy', done_if: { flag: p('delivered') } },
          { text: 'See off the cutpurse in the alley', done_if: { flag: 'won_' + p('mugger') } },
          { text: 'Face Black Sal at the end of the alley', done_if: { flag: 'won_' + p('sal') } },
        ],
      },
    },
    skins: {
      vera: { body: ['#4a3018', '#e0c08a'], head: ['#6b4a38', '#dcae8a'] },
      marlo: { body: ['#2a3a1a', '#9aaa6a'], head: ['#5a4030', '#c99a78'] },
      pru: { body: ['#2a2420', '#b07848'], head: ['#7b5240', '#e2b894'] },
      ellis: { body: ['#4a1a1a', '#c8806a'], head: ['#6a4a38', '#d9a581'] },
      mugger: { body: ['#1c1c14', '#6a6a4a'], head: ['#6b4a38', '#d0a080'] },
      sal: { body: ['#1a0c0c', '#a04040'], head: ['#5a4234', '#c79a74'] },
      tilda: { body: ['#1a2a4a', '#8aaae0'], head: ['#6a4a38', '#dcae8a'] },
      oss: { body: ['#20242c', '#7a8494'], head: ['#5a4234', '#c79a74'] },
    },
    props: {
      bench: BENCH,
      stool: {
        colours: ['#7a5a34', '#4a3420'],
        cubes: [
          { from: [-3, 7, -3], to: [3, 8.5, 3], c: 0 }, { from: [-2.5, 0, -2.5], to: [-1.5, 7, -1.5], c: 1 }, { from: [1.5, 0, -2.5], to: [2.5, 7, -1.5], c: 1 },
          { from: [-2.5, 0, 1.5], to: [-1.5, 7, 2.5], c: 1 }, { from: [1.5, 0, 1.5], to: [2.5, 7, 2.5], c: 1 },
        ],
      },
      brazier: {
        colours: ['#2c2a2a', '#e0602a', '#ffd070', '#5a4226'],
        cubes: [
          { from: [-5, 0, -5], to: [-4, 7, -4], c: 3 }, { from: [4, 0, -5], to: [5, 7, -4], c: 3 }, { from: [-5, 0, 4], to: [-4, 7, 5], c: 3 }, { from: [4, 0, 4], to: [5, 7, 5], c: 3 },
          { from: [-6, 7, -6], to: [6, 11, 6], c: 0 }, { from: [-4.5, 11, -4.5], to: [4.5, 14, 4.5], c: 1 }, { from: [-2, 11, -2], to: [2, 17, 2], c: 2 },
        ],
      },
      notice_board: {
        colours: ['#5a4226', '#d8c8a0', '#f0e8d0'],
        cubes: [
          { from: [-9, 0, -1], to: [-7, 26, 1], c: 0 }, { from: [7, 0, -1], to: [9, 26, 1], c: 0 },
          { from: [-8, 10, -1.5], to: [8, 25, -0.5], c: 1 }, { from: [-6, 18, -1.7], to: [-1, 23, -1.5], c: 2 }, { from: [1, 14, -1.7], to: [6, 22, -1.5], c: 2 },
        ],
      },
      crate: {
        colours: ['#8a6a3a', '#a8844a', '#5a4220'],
        cubes: [{ from: [-6, 0, -6], to: [6, 12, 6], c: 0 }, { from: [-6, 5, -6], to: [6, 7, 6], c: 2 }, { from: [-6, 11, -6], to: [6, 12, 6], c: 1 }],
      },
      barrel: {
        colours: ['#7a5430', '#5a3c20', '#3a3c40', '#9a7444'],
        cubes: [
          { from: [-4, 0, -4], to: [4, 12, 4], c: 0 }, { from: [-5, 4, -5], to: [5, 8, 5], c: 0 },
          { from: [-5.2, 2, -5.2], to: [5.2, 3, 5.2], c: 2 }, { from: [-5.2, 9, -5.2], to: [5.2, 10, 5.2], c: 2 },
          { from: [-3.5, 12, -3.5], to: [3.5, 12.5, 3.5], c: 3 },
        ],
      },
      stall: {
        colours: ['#6a4a2a', '#a8452e', '#e8dcb8', '#c8402a', '#d8a838', '#5a8a3a'],
        cubes: [
          { from: [-15, 0, -9], to: [-13, 18, -7], c: 0 }, { from: [13, 0, -9], to: [15, 18, -7], c: 0 },
          { from: [-15, 0, 7], to: [-13, 14, 9], c: 0 }, { from: [13, 0, 7], to: [15, 14, 9], c: 0 },
          { from: [-14, 7, -8], to: [14, 9, 8], c: 0 },
          { from: [-16, 18, -10], to: [16, 19, 0], c: 1 }, { from: [-16, 16, 0], to: [16, 18, 10], c: 2 },
          { from: [-16, 18, -10], to: [-8, 19.4, 0], c: 3 }, { from: [0, 18, -10], to: [8, 19.4, 0], c: 3 },
          { from: [-11, 9, -4], to: [-5, 12, 2], c: 4 }, { from: [-2, 9, -3], to: [4, 11, 3], c: 5 }, { from: [7, 9, -3], to: [11, 12, 2], c: 4 },
        ],
      },
      hay_cart: {
        colours: ['#7a5a34', '#d8b84a', '#3a2c1c', '#5a4226'],
        cubes: [
          { from: [-12, 6, -7], to: [12, 8, 7], c: 0 }, { from: [-12, 8, -7.5], to: [12, 11, -6.5], c: 3 }, { from: [-12, 8, 6.5], to: [12, 11, 7.5], c: 3 },
          { from: [-11, 8, -6], to: [11, 15, 6], c: 1 },
          { from: [-3, 0, -9], to: [3, 9, -7.5], c: 2 }, { from: [-3, 0, 7.5], to: [3, 9, 9], c: 2 },
          { from: [12, 5, -1], to: [26, 6.5, 1], c: 3 },
        ],
      },
      lantern: {
        colours: ['#2c2a2a', '#ffd070', '#5a4226'],
        cubes: [
          { from: [-1, 0, -1], to: [1, 24, 1], c: 0 }, { from: [-1, 24, -1], to: [5, 25, 1], c: 0 },
          { from: [2.5, 19, -1.5], to: [5.5, 24, 1.5], c: 1 }, { from: [2, 24, -2], to: [6, 25.5, 2], c: 0 },
        ],
      },
      trough: {
        colours: ['#6a4a2a', '#3a6a8a', '#4a3420'],
        cubes: [
          { from: [-10, 0, -3], to: [-8, 5, 3], c: 2 }, { from: [8, 0, -3], to: [10, 5, 3], c: 2 },
          { from: [-10, 5, -4], to: [10, 8, -3], c: 0 }, { from: [-10, 5, 3], to: [10, 8, 4], c: 0 },
          { from: [-10, 5, -3], to: [-9, 8, 3], c: 0 }, { from: [9, 5, -3], to: [10, 8, 3], c: 0 },
          { from: [-9, 5, -3], to: [9, 7, 3], c: 1 },
        ],
      },
      well: {
        colours: ['#8a8680', '#6a6660', '#2c5a7a', '#6a4a2a', '#a8452e'],
        cubes: [
          { from: [-8, 0, -8], to: [8, 9, -6], c: 0 }, { from: [-8, 0, 6], to: [8, 9, 8], c: 0 },
          { from: [-8, 0, -6], to: [-6, 9, 6], c: 1 }, { from: [6, 0, -6], to: [8, 9, 6], c: 1 },
          { from: [-6, 3, -6], to: [6, 4, 6], c: 2 },
          { from: [-7.5, 9, -1], to: [-6, 28, 1], c: 3 }, { from: [6, 9, -1], to: [7.5, 28, 1], c: 3 },
          { from: [-10, 28, -5], to: [10, 30, 5], c: 4 }, { from: [-6, 26, -0.6], to: [6, 27, 0.6], c: 3 },
        ],
      },
    },
    scenes: {
      [p('intro')]: {
        once: true,
        lines: [
          'Late afternoon in the old town. Smoke curls from the chimneys, and the bell in the stone church tower has just stopped ringing.',
          { speaker: 'Jun', text: 'One letter left. The smithy is just up the street, if the smith is at her forge.' },
        ],
        do: [{ op: 'toast', text: 'Step on a glowing marker to pick it up. Left mouse attacks in a fight.' }],
      },
      [p('vera')]: {
        lines: [{ speaker: 'Maud', text: 'Good day, messenger. Fresh loaves, honey cakes and gossip. What will it be?' }],
        choices: [
          {
            text: 'The smithy door is barred. Do you have a key?', if: { not_flag: p('got_key') },
            do: [{ op: 'give', item: key }, { op: 'set_flag', flag: p('got_key') }, { op: 'toast', text: 'Maud gives you the smithy key.' }],
            next: p('vera_key'),
          },
          { text: 'Anything I should know about?', next: p('vera_alley') },
          { text: 'Just passing.' },
        ],
      },
      [p('vera_key')]: {
        lines: [{ speaker: 'Maud', text: 'Pru leaves it with me when she goes to market. The door is on the west side of the street, under the dark stone gable.' }],
        next: p('vera_bye'),
      },
      [p('vera_alley')]: {
        lines: [{ speaker: 'Maud', text: 'Keep out of the alley past the tavern. Somebody has been lurking down there all week.' }],
        next: p('vera_bye'),
      },
      // both answers above rejoin here
      [p('vera_bye')]: { lines: [{ speaker: 'Maud', text: 'Mind the carts, and the puddles.' }] },
      [p('marlo')]: {
        lines: [{ speaker: 'Garrick', text: 'The hay can wait; I can talk. Where are you headed?' }],
        choices: [
          { text: 'How do I get about the town?', next: p('marlo_way') },
          { text: 'Heard of anyone in the alley?', next: p('marlo_sal') },
          { text: 'Never mind.' },
        ],
      },
      [p('marlo_way')]: { lines: [{ speaker: 'Garrick', text: 'The main street runs gate to gate. The smithy is on the west side, the Gilded Boar tavern on the east, and the alley is on the south side past the tavern.' }] },
      [p('marlo_sal')]: { lines: [{ speaker: 'Garrick', text: 'Black Sal. Captain of the road bandits, and he works the alley now. Do not turn your back on him, and do not run: it is a dead end.' }] },
      [p('pru')]: {
        lines: [{ speaker: 'Pru', text: 'Ah, a messenger. Is that my letter?' }],
        choices: [
          {
            text: 'Hand over the sealed letter', if: { all: [{ has: parcel }, { not_flag: p('delivered') }] },
            do: [{ op: 'take', item: parcel }, { op: 'set_flag', flag: p('delivered') }, { op: 'toast', text: 'Letter delivered.' }],
            next: p('pru_thanks'),
          },
          { text: 'Anything else I can do?', if: { flag: p('delivered') }, next: p('pru_thanks') },
          { text: 'Not yet.' },
        ],
      },
      [p('pru_thanks')]: {
        lines: [{ speaker: 'Pru', text: 'My thanks! There is a flask of mead hidden in the lane behind the houses, if you have not found it. And watch yourself in the alley.' }],
      },
      [p('ellis')]: {
        lines: [{ speaker: 'Ellis', text: 'We close the shutters at dusk. Take the pepper pouch from the shelf; I have never needed it.' }],
      },
      [p('notice')]: {
        lines: ['A board of curling notices: a lost goat, a toll raised again, and a hurried warning: "Rain is coming. Take shelter under the market awning."'],
        do: [{ op: 'weather', kind: 'rain', intensity: 0.7 }, { op: 'toast', text: 'The sky is darkening.' }],
      },
      [p('tilda')]: { lines: [{ speaker: 'Tilda', text: 'Garrick swears the well water tastes of iron. I say it tastes of Garrick.' }] },
      [p('oss')]: { lines: [{ speaker: 'Warden Oss', text: 'Gate to gate, every hour. Stay out of the alley if you can; I am told it has a tenant.' }] },
      [p('gate')]: {
        lines: ['The path winds into the trees. The road beyond the gate is dark and empty, and the messenger has one more errand in town first.'],
      },
      [p('mugger_start')]: {
        lines: ['The alley is shaded and narrow. Something moves behind the barrels.'],
        choices: [
          { text: 'Hold your ground', if: { not_flag: 'won_' + p('mugger') }, do: [{ op: 'battle', battle: p('mugger') }] },
          { text: 'Walk on. The alley is quiet now.', if: { flag: 'won_' + p('mugger') } },
          { text: 'Back out of the alley' },
        ],
      },
      [p('mugger_won')]: {
        lines: [{ speaker: 'Jun', text: 'He ran. Further down the alley, a big shape waits by a barrel.' }],
        do: [{ op: 'toast', text: 'The cutpurse is gone.' }],
      },
      [p('mugger_lost')]: {
        lines: ['You come to on the cobbles, bruised and a little embarrassed. The alley can wait.'],
      },
      [p('sal')]: {
        lines: [{ speaker: 'Black Sal', text: "You are far from the king's road, messenger." }],
        choices: [
          { text: 'Face Black Sal', if: { all: [{ flag: p('delivered') }, { not_flag: 'won_' + p('sal') }] }, do: [{ op: 'battle', battle: p('sal') }] },
          { text: 'What do you want?', if: { not_flag: p('delivered') }, next: p('sal_hint') },
          { text: 'Nothing. The alley is yours.', if: { flag: 'won_' + p('sal') } },
          { text: 'Back away.' },
        ],
      },
      [p('sal_hint')]: {
        lines: [{ speaker: 'Black Sal', text: 'Come back when your letters are delivered. I like a fair fight.' }],
      },
      [p('sal_won')]: {
        once: true,
        lines: [
          'Sal sags against the wall. The church bell rings out over the roofs, and the whole town seems to breathe out.',
          { speaker: 'Jun', text: 'Last letter. Done.' },
        ],
        do: [{ op: 'set_flag', flag: p('finished') }, { op: 'toast', text: 'The end of the sample.' }],
      },
      [p('sal_lost')]: {
        lines: ['Sal laughs. You wake on the cobbles with your satchel empty and your pride dented.'],
      },
    },
  };
}

// ---------------------------------------------------------------- self-check (runs first; throws on a broken writer)
function selfCheck() {
  const cv = new Canvas(3, 2);
  cv.set(0, 0, [1, 2, 3]); cv.set(2, 1, [250, 128, 7], 200);
  const back = decodePng(cv.png());
  if (back.w !== 3 || back.h !== 2 || Buffer.compare(Buffer.from(back.px), Buffer.from(cv.px)) !== 0) throw new Error('PNG round trip failed');
  const skin = decodePng(makeSkin({ body: ['#000000', '#ffffff'], head: ['#000000', '#ffffff'] }));
  if (skin.w !== 64 || skin.h !== 64) throw new Error('a skin must be 64x64');
}
selfCheck();

// ---------------------------------------------------------------- run
for (const make of [greenhouse, lighthouse, archive, town]) {
  const dir = buildPack(make());
  console.log('wrote', path.relative(ROOT, dir));
}
