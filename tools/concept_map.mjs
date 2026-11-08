// Converts src/bin/last_route/data/regions/map_concept.png into src/bin/last_route/data/concept/map_concept.json.
// Run once after the PNG changes:  node tools/concept_map.mjs
// No dependencies: the PNG (8-bit, non-interlaced) is inflated with node's zlib.
import { readFileSync, writeFileSync } from 'node:fs';
import { inflateSync } from 'node:zlib';

const SRC = 'src/bin/last_route/data/regions/map_concept.png';
const OUT = 'src/bin/last_route/data/concept/map_concept.json';
// colour -> [key char, id]. '#' is solid wall and padding. A0A0A0 (props overlay) is dropped: it takes the
// colour of the nearest region pixel.
const LEGEND = {
  '000000': ['#', 'wall'],
  'FFFFFF': ['M', 'maze'],
  'A17FFF': ['A', 'apartments'],
  '7F3300': ['P', 'prison'],
  '007F7F': ['I', 'ice'],
  '3F7F62': ['S', 'stalls'],
  '503F7F': ['F', 'inferno_lite'],
  '7F006E': ['T', 'tunnel'],
  'FFB27F': ['L', 'lab'],
  '7F3F5B': ['O', 'offices'],
  '7F3F3F': ['U', 'utility'],
  '007F0E': ['G', 'greenhouse'],
};
const DROPPED = 'A0A0A0';

const buf = readFileSync(SRC);
let pos = 8, w = 0, h = 0, depth = 0, ctype = 0;
const idat = [];
let palette = null;
while (pos < buf.length) {
  const len = buf.readUInt32BE(pos), type = buf.toString('latin1', pos + 4, pos + 8);
  const data = buf.subarray(pos + 8, pos + 8 + len);
  if (type === 'IHDR') { w = data.readUInt32BE(0); h = data.readUInt32BE(4); depth = data[8]; ctype = data[9]; if (data[12] !== 0) throw new Error('interlaced PNG'); }
  if (type === 'PLTE') palette = data;
  if (type === 'IDAT') idat.push(data);
  pos += 12 + len;
}
if (depth !== 8) throw new Error('only 8-bit PNG');
const bpp = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[ctype];
const raw = inflateSync(Buffer.concat(idat));
const stride = w * bpp;
const px = Buffer.alloc(h * stride);
for (let y = 0; y < h; y++) {
  const f = raw[y * (stride + 1)];
  for (let x = 0; x < stride; x++) {
    const v = raw[y * (stride + 1) + 1 + x];
    const a = x >= bpp ? px[y * stride + x - bpp] : 0;
    const b = y > 0 ? px[(y - 1) * stride + x] : 0;
    const c = x >= bpp && y > 0 ? px[(y - 1) * stride + x - bpp] : 0;
    let r;
    if (f === 0) r = v; else if (f === 1) r = v + a; else if (f === 2) r = v + b; else if (f === 3) r = v + ((a + b) >> 1);
    else { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); r = v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c); }
    px[y * stride + x] = r & 255;
  }
}
const rgba = (x, y) => {
  const i = y * stride + x * bpp;
  if (ctype === 6) return [px[i], px[i + 1], px[i + 2], px[i + 3]];
  if (ctype === 2) return [px[i], px[i + 1], px[i + 2], 255];
  if (ctype === 3) return [palette[px[i] * 3], palette[px[i] * 3 + 1], palette[px[i] * 3 + 2], 255];
  return [px[i], px[i], px[i], 255];
};
const hex = (c) => c.slice(0, 3).map((n) => n.toString(16).padStart(2, '0')).join('').toUpperCase();
const keys = Object.keys(LEGEND).map((k) => [k, [0, 2, 4].map((o) => parseInt(k.slice(o, o + 2), 16))]);
const nearest = (c) => {
  let best = null, bd = Infinity;
  for (const [k, rgb] of keys) { const d = rgb.reduce((s, v, i) => s + (v - c[i]) ** 2, 0); if (d < bd) { bd = d; best = k; } }
  return best;
};
const grid = [];
for (let y = 0; y < h; y++) {
  const row = [];
  for (let x = 0; x < w; x++) {
    const c = rgba(x, y);
    const k = c[3] < 128 ? '000000' : hex(c);
    row.push(k === DROPPED ? null : (LEGEND[k] ? k : nearest(c)));
  }
  grid.push(row);
}
// fill the dropped overlay pixels from the nearest kept pixel (repeat until none are left)
for (let guard = 0; guard < w + h; guard++) {
  let left = 0;
  const next = grid.map((r) => r.slice());
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    if (grid[y][x] !== null) continue;
    const n = [[1, 0], [-1, 0], [0, 1], [0, -1]].map(([dx, dy]) => grid[y + dy]?.[x + dx]).find((v) => v);
    if (n) next[y][x] = n; else left++;
  }
  for (let y = 0; y < h; y++) grid[y] = next[y];
  if (!left) break;
}
const rows = grid.map((r) => r.map((k) => LEGEND[k][0]).join(''));
const legend = Object.fromEntries(Object.entries(LEGEND).map(([k, [ch, id]]) => [ch, { colour: k, id }]));
writeFileSync(OUT, JSON.stringify({ schema: 'lr.concept_map/1', width: w, height: h, legend, rows }, null, 1) + '\n');
const counts = {};
for (const r of rows) for (const ch of r) counts[ch] = (counts[ch] || 0) + 1;
console.log(w, h, counts);
