#!/usr/bin/env node
// glb_to_bbmodel: Godot-side blocky GLB -> Blockbench .bbmodel the Rust loader (src/bin/last_route/render/bbmodel.rs) accepts.
//
//   node tools/props/glb_to_bbmodel.mjs <in.glb> <out.bbmodel> [--name NAME] [--atlas-name FILE.png] [--quiet]
//   node tools/props/glb_to_bbmodel.mjs --dir <glb_dir> --out <out_dir> [--recursive] [--quiet]
//   node tools/props/glb_to_bbmodel.mjs --all      (every GLB in tools/props/source_glb -> the game's assets folder)
//   node tools/props/glb_to_bbmodel.mjs --check    (same, writes nothing, fails if a bbmodel on disk is not what the converter makes)
//   add --no-simplify to any of these to keep chamfered boxes as bevelled mesh elements (the pre-simplification output)
//
// Chamfered boxes (a mesh element that is a plain box with bevelled edges, see chamferedBox) become cube elements
// unless --no-simplify is given. Element names, uuids, group membership, export and visibility are unchanged.
//
// Dependency-free (Node built-ins only). Conventions copied from tools/prop_convert (the Rust converter that
// produced assets/props/*.bbmodel): glTF metres * 16 = bbmodel units, Y up, no axis flip, node transforms baked
// in, element origin [0,0,0], UVs in atlas pixels (glTF u*width, v*height), embedded PNG kept as the texture.
// Axis-aligned boxes whose faces each cover one full side of the box and whose UVs are rectangles become
// bbmodel "cube" elements (from/to + per-face uv/rotation). Everything else becomes a "mesh" element
// (vertices + faces, triangle pairs merged back into quads).
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';

export const UNITS_PER_METRE = 16;
// node names the runtime looks for (kept in step with render/door.rs `closed_hinge_deg` and render/prop_model.rs `locker_door_part`)
const HINGES = ['DoorHinge', 'DoorHingeRight'];
const FRAME_NODES = ['DoorFrameTop', 'DoorFrameL', 'DoorFrameR'];
const LOCKER_GROUP = 'LockerDoor';
const LOCKER_DOOR_NODES = [
  'BlockLatch', 'DoorLowerMass', 'DoorRail_L', 'DoorRail_R', 'DoorTopMass', 'Handle', 'HandlePlate', 'LowerInset', 'LowerPanel',
  'NumberPlate', 'SightCrackLeftRail', 'SightCrackRightRail', 'SightSlat_1', 'SightSlat_2', 'SightSlat_3',
];
const r4 = (x) => Math.round(x * 1e4) / 1e4;

// ---------- GLB / glTF parsing ----------
export function parseGlb(buf) {
  if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error('not a GLB (bad magic)');
  const total = buf.readUInt32LE(8);
  let off = 12, json = null, bin = null;
  while (off + 8 <= Math.min(total, buf.length)) {
    const len = buf.readUInt32LE(off), type = buf.readUInt32LE(off + 4);
    const data = buf.subarray(off + 8, off + 8 + len);
    if (type === 0x4e4f534a) json = JSON.parse(data.toString('utf8'));
    else if (type === 0x004e4942 && !bin) bin = data;
    off += 8 + ((len + 3) & ~3);
  }
  if (!json) throw new Error('GLB has no JSON chunk');
  return { gltf: json, bin };
}

const COMP = { 5120: [1, true], 5121: [1, false], 5122: [2, true], 5123: [2, false], 5125: [4, false], 5126: [4, true] };
const NCOMP = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT4: 16 };

function readAccessor(g, bin, idx) {
  const a = g.accessors[idx];
  if (a.sparse) throw new Error('sparse accessors not supported');
  const n = NCOMP[a.type], [size, signed] = COMP[a.componentType];
  const out = new Float64Array(a.count * n);
  if (a.bufferView === undefined) return { data: out, n };
  const bv = g.bufferViews[a.bufferView];
  const stride = bv.byteStride || size * n;
  const base = (bv.byteOffset || 0) + (a.byteOffset || 0);
  const dv = new DataView(bin.buffer, bin.byteOffset, bin.byteLength);
  for (let i = 0; i < a.count; i++) for (let c = 0; c < n; c++) {
    const p = base + i * stride + c * size;
    let v;
    switch (a.componentType) {
      case 5120: v = dv.getInt8(p); break;
      case 5121: v = dv.getUint8(p); break;
      case 5122: v = dv.getInt16(p, true); break;
      case 5123: v = dv.getUint16(p, true); break;
      case 5125: v = dv.getUint32(p, true); break;
      default: v = dv.getFloat32(p, true);
    }
    if (a.normalized) v = signed ? Math.max(v / (2 ** (size * 8 - 1) - 1), -1) : v / (2 ** (size * 8) - 1);
    out[i * n + c] = v;
  }
  return { data: out, n };
}

// column-major 4x4
const I4 = () => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
const mmul = (a, b) => {
  const o = new Array(16).fill(0);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) for (let k = 0; k < 4; k++) o[c * 4 + r] += a[k * 4 + r] * b[c * 4 + k];
  return o;
};
function nodeMatrix(n) {
  if (n.matrix) return n.matrix.slice();
  const [x, y, z, w] = n.rotation || [0, 0, 0, 1], [sx, sy, sz] = n.scale || [1, 1, 1], [tx, ty, tz] = n.translation || [0, 0, 0];
  return [
    (1 - 2 * (y * y + z * z)) * sx, 2 * (x * y + z * w) * sx, 2 * (x * z - y * w) * sx, 0,
    2 * (x * y - z * w) * sy, (1 - 2 * (x * x + z * z)) * sy, 2 * (y * z + x * w) * sy, 0,
    2 * (x * z + y * w) * sz, 2 * (y * z - x * w) * sz, (1 - 2 * (x * x + y * y)) * sz, 0,
    tx, ty, tz, 1];
}
const xp = (m, p) => [
  m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
  m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
  m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]];
const det3 = (m) => m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2]) + m[8] * (m[1] * m[6] - m[5] * m[2]);

// ---------- minimal PNG codec (8-bit, non-interlaced, colour types 0/2/3/4/6) for stacking several images into one atlas ----------
function decodePng(png) {
  const w = png.readUInt32BE(16), h = png.readUInt32BE(20), depth = png[24], ct = png[25];
  if (depth !== 8 || png[28] !== 0) throw new Error('PNG must be 8-bit and not interlaced to be merged');
  const chans = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[ct];
  if (!chans) throw new Error('PNG colour type ' + ct + ' not supported');
  let off = 8, plte = null, trns = null; const idat = [];
  while (off + 8 <= png.length) {
    const len = png.readUInt32BE(off), type = png.toString('latin1', off + 4, off + 8), data = png.subarray(off + 8, off + 8 + len);
    if (type === 'IDAT') idat.push(data); else if (type === 'PLTE') plte = data; else if (type === 'tRNS') trns = data;
    off += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(idat)), stride = w * chans, cur = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const ft = raw[y * (stride + 1)], row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= chans ? cur[y * stride + x - chans] : 0, b = y ? cur[(y - 1) * stride + x] : 0, c = x >= chans && y ? cur[(y - 1) * stride + x - chans] : 0;
      let p = 0;
      if (ft === 2) p = b; else if (ft === 1) p = a; else if (ft === 3) p = (a + b) >> 1;
      else if (ft === 4) { const q = a + b - c, pa = Math.abs(q - a), pb = Math.abs(q - b), pc = Math.abs(q - c); p = pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      cur[y * stride + x] = (row[x] + p) & 255;
    }
  }
  const rgba = Buffer.alloc(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    let r, g, b, a = 255;
    if (ct === 6) [r, g, b, a] = cur.subarray(i * 4, i * 4 + 4);
    else if (ct === 2) [r, g, b] = cur.subarray(i * 3, i * 3 + 3);
    else if (ct === 0) r = g = b = cur[i];
    else if (ct === 4) { r = g = b = cur[i * 2]; a = cur[i * 2 + 1]; }
    else { const k = cur[i]; r = plte[k * 3]; g = plte[k * 3 + 1]; b = plte[k * 3 + 2]; if (trns && k < trns.length) a = trns[k]; }
    rgba.set([r, g, b, a], i * 4);
  }
  return { w, h, rgba };
}
const crcTable = (() => { const t = new Uint32Array(256); for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; } return t; })();
function crc32(b) { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; }
function encodePng(w, h, rgba) {
  const chunk = (type, data) => {
    const out = Buffer.alloc(12 + data.length), t = Buffer.from(type, 'latin1');
    out.writeUInt32BE(data.length, 0); t.copy(out, 4); data.copy(out, 8); out.writeUInt32BE(crc32(Buffer.concat([t, data])), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc(h * (w * 4 + 1));
  for (let y = 0; y < h; y++) rgba.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

/**
 * Primitives with world-space positions (metres), uvs in atlas pixels, triangles. A model whose materials use more than
 * one image gets those images stacked top to bottom into one atlas (the loader draws one texture per model), with each
 * primitive's UVs moved down by its image's offset.
 */
export function readGlb(buf) {
  const { gltf: g, bin } = parseGlb(buf);
  if (!g.images || !g.images.length) throw new Error('no image in GLB');
  const pngOf = (i) => {
    const im = g.images[i];
    if (im.bufferView === undefined) throw new Error('image is not embedded');
    const bv = g.bufferViews[im.bufferView];
    const png = Buffer.from(bin.subarray(bv.byteOffset || 0, (bv.byteOffset || 0) + bv.byteLength));
    if (png.readUInt32BE(0) !== 0x89504e47) throw new Error('embedded image is not a PNG');
    return png;
  };
  // the image a primitive's base colour reads (material -> texture -> source), image 0 when it names none
  const imageOfPrim = (p) => {
    const t = g.materials && g.materials[p.material] && g.materials[p.material].pbrMetallicRoughness && g.materials[p.material].pbrMetallicRoughness.baseColorTexture;
    const src = t && g.textures && g.textures[t.index] ? g.textures[t.index].source : 0;
    return src === undefined ? 0 : src;
  };
  const used = [...new Set(g.meshes.flatMap((m) => m.primitives.map(imageOfPrim)))].sort((a, b) => a - b);
  let png, width, height;
  const offset = new Map(), size = new Map(); // image index -> v offset (px), [w, h]
  if (used.length <= 1) {
    const only = used.length ? used[0] : 0;
    png = pngOf(only); width = png.readUInt32BE(16); height = png.readUInt32BE(20);
    offset.set(only, 0); size.set(only, [width, height]);
  } else {
    const dec = used.map((i) => ({ i, ...decodePng(pngOf(i)) }));
    width = Math.max(...dec.map((d) => d.w)); height = dec.reduce((a, d) => a + d.h, 0);
    const rgba = Buffer.alloc(width * height * 4);
    let y0 = 0;
    for (const d of dec) {
      for (let y = 0; y < d.h; y++) d.rgba.copy(rgba, ((y0 + y) * width) * 4, y * d.w * 4, (y + 1) * d.w * 4);
      offset.set(d.i, y0); size.set(d.i, [d.w, d.h]); y0 += d.h;
    }
    png = encodePng(width, height, rgba);
  }
  const prims = [];
  const scene = g.scenes && g.scenes[g.scene || 0];
  const roots = scene ? scene.nodes : g.nodes.map((_, i) => i);
  // Structure the runtime reads by name (render/prop_model.rs, render/door.rs): a room door's hinge nodes (the leaves hang under
  // them and swing about the hinge), its frame pieces (measured for the raise and widen) and a hiding spot's door leaf.
  const groups = new Map(); // group name -> pivot, in metres
  const lockerNode = g.nodes.find((n) => n.name === 'BlockHinge_1');
  const lockerPivot = lockerNode ? (lockerNode.translation || [0, 0, 0]) : null;
  if (lockerPivot) groups.set(LOCKER_GROUP, lockerPivot);
  const visit = (ni, parent, hinge) => {
    const n = g.nodes[ni], world = mmul(parent, nodeMatrix(n));
    if (!hinge && HINGES.includes(n.name)) {
      hinge = n.name;
      groups.set(hinge, [world[12], world[13], world[14]]);
    }
    if (n.mesh !== undefined) {
      const group = hinge || (lockerPivot && LOCKER_DOOR_NODES.includes(n.name) ? LOCKER_GROUP : FRAME_NODES.includes(n.name) ? n.name : null);
      if (group && !groups.has(group)) groups.set(group, [0, 0, 0]);
      const mesh = g.meshes[n.mesh];
      mesh.primitives.forEach((p, pi) => {
        if ((p.mode === undefined ? 4 : p.mode) !== 4) throw new Error('primitive mode ' + p.mode + ' not supported');
        const P = readAccessor(g, bin, p.attributes.POSITION);
        const pos = [];
        for (let i = 0; i < P.data.length; i += 3) pos.push(xp(world, [P.data[i], P.data[i + 1], P.data[i + 2]]));
        const img = imageOfPrim(p), [iw, ih] = size.get(img), io = offset.get(img);
        let uv;
        if (p.attributes.TEXCOORD_0 !== undefined) {
          const T = readAccessor(g, bin, p.attributes.TEXCOORD_0);
          uv = [];
          for (let i = 0; i < T.data.length; i += 2) uv.push([T.data[i] * iw, T.data[i + 1] * ih + io]);
        } else uv = pos.map(() => [0, 0]);
        const idx = p.indices !== undefined ? Array.from(readAccessor(g, bin, p.indices).data) : pos.map((_, i) => i);
        const flip = det3(world) < 0;
        const tris = [];
        for (let i = 0; i + 2 < idx.length; i += 3) tris.push(flip ? [idx[i], idx[i + 2], idx[i + 1]] : [idx[i], idx[i + 1], idx[i + 2]]);
        prims.push({ name: n.name || mesh.name || 'mesh', multi: mesh.primitives.length > 1 ? pi : -1, pos, uv, tris, group });
      });
    }
    (n.children || []).forEach((c) => visit(c, world, hinge));
  };
  roots.forEach((r) => visit(r, I4(), null));
  if (!prims.length) throw new Error('no triangle geometry');
  return { prims, png, width, height, imageCount: g.images.length, groups };
}

// ---------- geometry helpers ----------
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]); return l < 1e-12 ? [0, 0, 0] : [a[0] / l, a[1] / l, a[2] / l]; };
const triN = (t, pos) => norm(cross(sub(pos[t[1]], pos[t[0]]), sub(pos[t[2]], pos[t[0]])));
const key = (p) => p.map((c) => Math.round(c * 1e4)).join(',');

/** Triangle list -> faces (index loops), merging coplanar neighbouring pairs into quads. */
function buildFaces(tris, pos) {
  const used = new Array(tris.length).fill(false), faces = [];
  const tryMerge = (a, b) => {
    const A = tris[a], B = tris[b];
    const kB = B.map((v) => key(pos[v]));
    const shared = A.filter((v) => kB.includes(key(pos[v])));
    if (shared.length !== 2) return null;
    if (dot(triN(A, pos), triN(B, pos)) < 0.9999) return null;
    const sk = shared.map((v) => key(pos[v]));
    const k = A.findIndex((v) => !sk.includes(key(pos[v])));
    const w = B.find((v) => !sk.includes(key(pos[v])));
    if (k < 0 || w === undefined) return null;
    return [A[k], A[(k + 1) % 3], w, A[(k + 2) % 3]];
  };
  for (let i = 0; i < tris.length; i++) {
    if (used[i]) continue;
    used[i] = true;
    let merged = null;
    for (let j = i + 1; j < Math.min(tris.length, i + 4) && !merged; j++) {
      if (used[j]) continue;
      const q = tryMerge(i, j);
      if (q) { merged = q; used[j] = true; }
    }
    faces.push(merged || tris[i].slice());
  }
  return faces;
}

const SIDES = ['north', 'south', 'east', 'west', 'up', 'down'];
// Same corner order as the loader (TL, TR, BR, BL) for a box from f to t.
function sideCorners(side, f, t) {
  switch (side) {
    case 'north': return [[t[0], t[1], f[2]], [f[0], t[1], f[2]], [f[0], f[1], f[2]], [t[0], f[1], f[2]]];
    case 'south': return [[f[0], t[1], t[2]], [t[0], t[1], t[2]], [t[0], f[1], t[2]], [f[0], f[1], t[2]]];
    case 'east': return [[t[0], t[1], t[2]], [t[0], t[1], f[2]], [t[0], f[1], f[2]], [t[0], f[1], t[2]]];
    case 'west': return [[f[0], t[1], f[2]], [f[0], t[1], t[2]], [f[0], f[1], t[2]], [f[0], f[1], f[2]]];
    case 'up': return [[f[0], t[1], f[2]], [t[0], t[1], f[2]], [t[0], t[1], t[2]], [f[0], t[1], t[2]]];
    default: return [[f[0], f[1], t[2]], [t[0], f[1], t[2]], [t[0], f[1], f[2]], [f[0], f[1], f[2]]];
  }
}
const SIDE_N = { north: [0, 0, -1], south: [0, 0, 1], east: [1, 0, 0], west: [-1, 0, 0], up: [0, 1, 0], down: [0, -1, 0] };

/** The loader's face UV order: [(u1,v1),(u2,v1),(u2,v2),(u1,v2)] rotated right by `rotation`/90. */
export function cubeUvCorners(uv, rotation) {
  const [u1, v1, u2, v2] = uv, b = [[u1, v1], [u2, v1], [u2, v2], [u1, v2]];
  const turns = ((Math.round((rotation || 0) / 90) % 4) + 4) % 4;
  return b.map((_, i) => b[(i - turns + 4) % 4]);
}

function fitCubeFace(cornerUvs) {
  const us = cornerUvs.map((c) => c[0]), vs = cornerUvs.map((c) => c[1]);
  const lo = [Math.min(...us), Math.min(...vs)], hi = [Math.max(...us), Math.max(...vs)];
  for (const sw of [[0, 0], [1, 0], [0, 1], [1, 1]]) for (let r = 0; r < 4; r++) {
    const uv = [sw[0] ? hi[0] : lo[0], sw[1] ? hi[1] : lo[1], sw[0] ? lo[0] : hi[0], sw[1] ? lo[1] : hi[1]];
    const got = cubeUvCorners(uv, r * 90);
    if (got.every((c, i) => Math.abs(c[0] - cornerUvs[i][0]) < 1e-3 && Math.abs(c[1] - cornerUvs[i][1]) < 1e-3)) return { uv, rotation: r * 90 };
  }
  return null;
}

/** A connected component (list of {p,uv} faces) -> cube spec or null. */
function asCube(comp) {
  if (comp.some((f) => f.p.length !== 4)) return null;
  const all = comp.flatMap((f) => f.p);
  const f = [0, 1, 2].map((a) => Math.min(...all.map((p) => p[a])));
  const t = [0, 1, 2].map((a) => Math.max(...all.map((p) => p[a])));
  if ([0, 1, 2].some((a) => t[a] - f[a] < 1e-4)) return null;
  const faces = {};
  for (const face of comp) {
    const n = norm(cross(sub(face.p[1], face.p[0]), sub(face.p[2], face.p[0])));
    const side = SIDES.find((s) => dot(n, SIDE_N[s]) > 0.9999);
    if (!side || faces[side]) return null;
    const cuv = [];
    for (const w of sideCorners(side, f, t)) {
      const i = face.p.findIndex((p) => key(p) === key(w));
      if (i < 0) return null;
      cuv.push(face.uv[i]);
    }
    const fit = fitCubeFace(cuv);
    if (!fit) return null;
    faces[side] = fit;
  }
  return { from: f, to: t, faces };
}

// ---------- chamfered-box simplification ----------
// A mesh element that is really a plain box with bevelled edges and corners (every vertex on the outer hull of its
// bounding box, every box corner within CHAMFER_MAX_UNITS of a vertex, every side covered by a planar quad that
// reaches the box edges to within CHAMFER_MAX_UNITS) is emitted as one bbmodel cube: 12 triangles instead of ~44.
export const CHAMFER_MAX_UNITS = 0.25;
const CHAMFER_MAX = CHAMFER_MAX_UNITS / UNITS_PER_METRE;
const CHAMFER_UV_TOL = 0.5; // uv pixels a chamfer face may drift from the rectangle it is reduced to
const polyArea = (p) => { let s = 0; for (let i = 1; i + 1 < p.length; i++) s += triArea(p[0], p[i], p[i + 1]); return s; };

/** 3x3 linear solve (Gaussian elimination with partial pivoting); null when singular. */
function solve3(M, r) {
  const A = M.map((row, i) => [...row, r[i]]);
  for (let i = 0; i < 3; i++) {
    let p = i;
    for (let k = i + 1; k < 3; k++) if (Math.abs(A[k][i]) > Math.abs(A[p][i])) p = k;
    [A[i], A[p]] = [A[p], A[i]];
    if (Math.abs(A[i][i]) < 1e-12) return null;
    for (let k = 0; k < 3; k++) if (k !== i) { const m = A[k][i] / A[i][i]; for (let j = i; j < 4; j++) A[k][j] -= m * A[i][j]; }
  }
  return [A[0][3] / A[0][0], A[1][3] / A[1][1], A[2][3] / A[2][2]];
}
/** Like fitCubeFace, but accepts the rectangle whose corners are closest and within tol (uv pixels). */
function fitCubeFaceApprox(cornerUvs, tol) {
  const us = cornerUvs.map((c) => c[0]), vs = cornerUvs.map((c) => c[1]);
  const lo = [Math.min(...us), Math.min(...vs)], hi = [Math.max(...us), Math.max(...vs)];
  let best = null, bestErr = Infinity;
  for (const sw of [[0, 0], [1, 0], [0, 1], [1, 1]]) for (let r = 0; r < 4; r++) {
    const uv = [sw[0] ? hi[0] : lo[0], sw[1] ? hi[1] : lo[1], sw[0] ? lo[0] : hi[0], sw[1] ? lo[1] : hi[1]];
    const got = cubeUvCorners(uv, r * 90);
    const err = Math.max(...got.map((c, i) => Math.max(Math.abs(c[0] - cornerUvs[i][0]), Math.abs(c[1] - cornerUvs[i][1]))));
    if (err < bestErr) { bestErr = err; best = { uv, rotation: r * 90 }; }
  }
  return bestErr <= tol ? best : null;
}

/** Leftover faces of one primitive ({p,uv}) -> {from, to, faces} cube spec in metres, or null when they are not a chamfered box. */
export function chamferedBox(faces) {
  const all = faces.flatMap((f) => f.p);
  const f = [0, 1, 2].map((a) => Math.min(...all.map((p) => p[a])));
  const t = [0, 1, 2].map((a) => Math.max(...all.map((p) => p[a])));
  if ([0, 1, 2].some((a) => t[a] - f[a] < 1e-4)) return null;
  const verts = [...new Map(all.map((p) => [key(p), p])).values()];
  if (!verts.every((p) => [0, 1, 2].some((a) => Math.abs(p[a] - f[a]) < 1e-4 || Math.abs(p[a] - t[a]) < 1e-4))) return null;
  for (const cx of [f[0], t[0]]) for (const cy of [f[1], t[1]]) for (const cz of [f[2], t[2]]) {
    if (Math.min(...verts.map((p) => Math.hypot(p[0] - cx, p[1] - cy, p[2] - cz))) > CHAMFER_MAX) return null;
  }
  const out = {};
  for (const side of SIDES) {
    const n = SIDE_N[side];
    const ax = [0, 1, 2].find((a) => n[a] !== 0);
    const [a1, a2] = [0, 1, 2].filter((a) => a !== ax);
    const onSide = faces.filter((face) => dot(norm(cross(sub(face.p[1], face.p[0]), sub(face.p[2], face.p[0]))), n) > 0.9999);
    if (onSide.length !== 1) return null; // one flat face per side; several pieces on a side is detail, not a plain box
    const big = onSide.reduce((best, face) => (polyArea(face.p) > polyArea(best.p) ? face : best));
    if (big.p.length !== 4) return null;
    const lo1 = Math.min(...big.p.map((p) => p[a1])), hi1 = Math.max(...big.p.map((p) => p[a1]));
    const lo2 = Math.min(...big.p.map((p) => p[a2])), hi2 = Math.max(...big.p.map((p) => p[a2]));
    if (lo1 - f[a1] > CHAMFER_MAX || t[a1] - hi1 > CHAMFER_MAX || lo2 - f[a2] > CHAMFER_MAX || t[a2] - hi2 > CHAMFER_MAX) return null;
    // least-squares affine uv map of the largest face (its uv is a skewed quad, not an exact rectangle), extended to
    // the box edges so the texel density stays what the mesh had
    const P = big.p.map((p) => [p[a1], p[a2]]), U = big.uv;
    const phi = P.map(([x, y]) => [x, y, 1]);
    const N = [0, 1, 2].map((i) => [0, 1, 2].map((j) => phi.reduce((acc, ph) => acc + ph[i] * ph[j], 0)));
    const au = solve3(N, [0, 1, 2].map((i) => phi.reduce((acc, ph, k) => acc + ph[i] * U[k][0], 0)));
    const av = solve3(N, [0, 1, 2].map((i) => phi.reduce((acc, ph, k) => acc + ph[i] * U[k][1], 0)));
    if (!au || !av) return null;
    const toUv = (x, y) => [au[0] * x + au[1] * y + au[2], av[0] * x + av[1] * y + av[2]];
    if (P.some(([x, y], k) => Math.abs(toUv(x, y)[0] - U[k][0]) > CHAMFER_UV_TOL || Math.abs(toUv(x, y)[1] - U[k][1]) > CHAMFER_UV_TOL)) return null;
    const cuv = sideCorners(side, f, t).map((w) => toUv(w[a1], w[a2]));
    const fit = fitCubeFaceApprox(cuv, CHAMFER_UV_TOL);
    if (!fit) return null;
    out[side] = fit;
  }
  return { from: f, to: t, faces: out };
}

// ---------- conversion ----------
export function stableUuid(s) {
  const h = crypto.createHash('sha1').update(s).digest('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
}

export function convert(buf, name, opts = {}) {
  const src = readGlb(buf);
  src.simplifiedUuids = new Set();
  const simplify = opts.simplify !== false;
  const atlasName = opts.atlasName || `${name}_atlas.png`;
  const elements = [], outliner = [], members = {};
  const stats = {
    cubes: 0, simplified: 0, meshElements: 0, meshFaces: 0, meshTriangles: 0, srcTriangles: 0,
    bboxMin: [1e9, 1e9, 1e9], bboxMax: [-1e9, -1e9, -1e9], imageCount: src.imageCount, width: src.width, height: src.height,
  };
  const cubesSpec = [], meshSpec = [];
  for (const prim of src.prims) {
    stats.srcTriangles += prim.tris.length;
    const faces = buildFaces(prim.tris, prim.pos)
      .map((loop) => ({ p: loop.map((v) => prim.pos[v]), uv: loop.map((v) => prim.uv[v]) }))
      .filter((f) => new Set(f.p.map(key)).size === f.p.length); // drop degenerate faces
    // connected components by shared (rounded) positions
    const parent = faces.map((_, i) => i);
    const find = (x) => (parent[x] === x ? x : (parent[x] = find(parent[x])));
    const owner = new Map();
    faces.forEach((f, i) => f.p.forEach((p) => {
      const k = key(p);
      if (owner.has(k)) parent[find(i)] = find(owner.get(k)); else owner.set(k, i);
    }));
    const comps = new Map();
    faces.forEach((f, i) => { const r = find(i); if (!comps.has(r)) comps.set(r, []); comps.get(r).push(f); });
    const leftover = [], cubesBefore = cubesSpec.length;
    for (const comp of comps.values()) {
      const cube = asCube(comp);
      if (cube) cubesSpec.push({ cube, base: prim.name, group: prim.group }); else leftover.push(...comp);
    }
    // a primitive whose leftover is a chamfered box (and that has no cube parts of its own) becomes one cube
    const chamfer = simplify && leftover.length && cubesSpec.length === cubesBefore ? chamferedBox(leftover) : null;
    if (chamfer) prim.simplified = true;
    if (leftover.length) meshSpec.push({ name: prim.multi >= 0 ? `${prim.name}_${prim.multi}` : prim.name, faces: leftover, group: prim.group, cube: chamfer });
  }
  let n = 0;
  const colour = () => (n++) % 8;
  const grow = (p) => {
    for (let a = 0; a < 3; a++) { stats.bboxMin[a] = Math.min(stats.bboxMin[a], p[a]); stats.bboxMax[a] = Math.max(stats.bboxMax[a], p[a]); }
  };
  cubesSpec.forEach(({ cube, base }, i) => {
    const uuid = stableUuid(`${name}/cube/${i}`);
    const faces = {};
    for (const s of SIDES) {
      const f = cube.faces[s];
      faces[s] = f
        ? (f.rotation ? { uv: f.uv.map(r4), texture: 0, rotation: f.rotation } : { uv: f.uv.map(r4), texture: 0 })
        : { uv: [0, 0, 0, 0], texture: null };
    }
    const from = cube.from.map((c) => r4(c * UNITS_PER_METRE)), to = cube.to.map((c) => r4(c * UNITS_PER_METRE));
    elements.push({
      name: `${base}_cube${i}`, type: 'cube', uuid, box_uv: false, rescale: false, locked: false,
      from, to, autouv: 0, color: colour(), origin: [0, 0, 0], rotation: [0, 0, 0], faces,
    });
    (cubesSpec[i].group ? (members[cubesSpec[i].group] ??= []) : outliner).push(uuid);
    stats.cubes++;
    grow(cube.from); grow(cube.to);
  });
  meshSpec.forEach((m, mi) => {
    const uuid = stableUuid(`${name}/mesh/${mi}`);
    if (m.cube) {
      const c = m.cube, cf = {};
      for (const s of SIDES) {
        const f = c.faces[s];
        cf[s] = f ? (f.rotation ? { uv: f.uv.map(r4), texture: 0, rotation: f.rotation } : { uv: f.uv.map(r4), texture: 0 }) : { uv: [0, 0, 0, 0], texture: null };
      }
      elements.push({
        name: m.name, type: 'cube', uuid, box_uv: false, rescale: false, locked: false, export: true, visibility: true,
        from: c.from.map((x) => r4(x * UNITS_PER_METRE)), to: c.to.map((x) => r4(x * UNITS_PER_METRE)), autouv: 0,
        color: colour(), origin: [0, 0, 0], rotation: [0, 0, 0], render_order: 'default', allow_mirror_modeling: true, faces: cf,
      });
      (m.group ? (members[m.group] ??= []) : outliner).push(uuid);
      src.simplifiedUuids.add(uuid);
      stats.simplified++; grow(c.from); grow(c.to);
      return;
    }
    const vmap = new Map(), vertices = {}, faces = {};
    const vid = (p) => {
      const k = key(p);
      if (!vmap.has(k)) { const id = `v${vmap.size}`; vmap.set(k, id); vertices[id] = p.map((c) => r4(c * UNITS_PER_METRE)); }
      return vmap.get(k);
    };
    m.faces.forEach((f, fi) => {
      const ids = f.p.map(vid), uv = {};
      ids.forEach((id, i) => { uv[id] = f.uv[i].map(r4); });
      faces[`f${fi}`] = { uv, vertices: ids, texture: 0 };
      f.p.forEach(grow);
      stats.meshFaces++; stats.meshTriangles += f.p.length - 2;
    });
    elements.push({
      name: m.name, color: colour(), origin: [0, 0, 0], rotation: [0, 0, 0], export: true, visibility: true, locked: false,
      render_order: 'default', allow_mirror_modeling: true, vertices, faces, type: 'mesh', uuid,
    });
    (m.group ? (members[m.group] ??= []) : outliner).push(uuid);
    stats.meshElements++;
  });
  // outliner groups: the pivot is the hinge line (a door's hinge, the hiding spot's door), everything stays in model space
  for (const [gname, pivot] of src.groups) {
    outliner.push({
      name: gname, origin: pivot.map((c) => r4(c * UNITS_PER_METRE)), rotation: [0, 0, 0], color: 0, uuid: stableUuid(`${name}/group/${gname}`),
      export: true, isOpen: true, locked: false, visibility: true, autouv: 0, children: members[gname] || [],
    });
  }
  stats.cubeFaces = elements.filter((e) => e.type === 'cube').reduce((a, e) => a + Object.values(e.faces).filter((f) => f.texture === 0).length, 0);
  stats.totalFaces = stats.cubeFaces + stats.meshFaces;
  stats.totalTriangles = stats.cubeFaces * 2 + stats.meshTriangles;
  stats.size = [0, 1, 2].map((a) => r4(stats.bboxMax[a] - stats.bboxMin[a]));
  const bb = {
    meta: { format_version: '4.10', model_format: 'free', box_uv: false },
    name, model_identifier: '', visible_box: [1, 1, 0], variable_placeholders: '', variable_placeholder_buttons: [], timeline_setups: [], unhandled_root_fields: {},
    resolution: { width: src.width, height: src.height },
    elements, outliner,
    textures: [{
      path: '', name: atlasName, folder: '', namespace: '', id: '0', group: '', width: src.width, height: src.height, uv_width: src.width, uv_height: src.height,
      particle: false, use_as_default: false, layers_enabled: false, sync_to_project: '', render_mode: 'default', render_sides: 'auto', frame_time: 1,
      frame_order_type: 'loop', frame_order: '', frame_interpolate: false, visible: true, internal: true, saved: true,
      uuid: stableUuid(`${name}/texture/${atlasName}`), relative_path: atlasName, source: 'data:image/png;base64,' + src.png.toString('base64'),
    }],
  };
  return { bbmodel: bb, stats, source: src };
}

// ---------- loader replica (mirrors src/bin/last_route/render/bbmodel.rs) for round-trip checks ----------
/** Faces the Rust loader would build: [{corners:[[x,y,z]..] (bbmodel units), uv:[[px,px]..]}] */
export function loadLikeRust(bb) {
  const out = [];
  for (const e of bb.elements) {
    if (e.visibility === false || e.export === false) continue;
    if ((e.type || 'cube') === 'cube') {
      for (const s of SIDES) {
        const fj = e.faces && e.faces[s];
        if (!fj || fj.texture === null || fj.texture === undefined) continue;
        const [u1, v1, u2, v2] = fj.uv;
        if (Math.abs(u1 - u2) < 1e-6 && Math.abs(v1 - v2) < 1e-6) continue;
        out.push({ corners: sideCorners(s, e.from, e.to), uv: cubeUvCorners(fj.uv, fj.rotation) });
      }
    } else if (e.type === 'mesh') {
      for (const fj of Object.values(e.faces)) {
        if (fj.texture === null || fj.texture === undefined) continue;
        out.push({ corners: fj.vertices.map((k) => e.vertices[k]), uv: fj.vertices.map((k) => fj.uv[k]) });
      }
    }
  }
  return out;
}

function bary(P, A, B, C, ua, ub, uc, U) {
  const v0 = sub(B, A), v1 = sub(C, A), v2 = sub(P, A);
  const d00 = dot(v0, v0), d01 = dot(v0, v1), d11 = dot(v1, v1), d20 = dot(v2, v0), d21 = dot(v2, v1), den = d00 * d11 - d01 * d01;
  if (Math.abs(den) < 1e-12) return false;
  const b = (d11 * d20 - d01 * d21) / den, c = (d00 * d21 - d01 * d20) / den, a = 1 - b - c, e = 0.02; // slack for sliver faces: 1e-4 rounding of vertices
  if (a < -e || b < -e || c < -e) return false;
  const q = [A[0] * a + B[0] * b + C[0] * c, A[1] * a + B[1] * b + C[1] * c, A[2] * a + B[2] * b + C[2] * c];
  const d = sub(P, q);
  if (Math.hypot(d[0], d[1], d[2]) > 0.02) return false;
  return Math.abs(ua[0] * a + ub[0] * b + uc[0] * c - U[0]) < 0.05 && Math.abs(ua[1] * a + ub[1] * b + uc[1] * c - U[1]) < 0.05;
}
const triArea = (a, b, c) => { const x = cross(sub(b, a), sub(c, a)); return 0.5 * Math.hypot(x[0], x[1], x[2]); };

/**
 * Round trip: every source triangle (x16) must be covered, position and uv, by a face the loader would build,
 * and the loader's total surface area must equal the source's (no extra or missing surface).
 */
export function roundTrip(bb, source) {
  // chamfered boxes replaced by a cube are checked by chamferedBox() itself (their bevels are the intended loss)
  const skip = source.simplifiedUuids || new Set();
  const got = loadLikeRust({ ...bb, elements: bb.elements.filter((e) => !skip.has(e.uuid)) });
  let missing = 0, srcArea = 0, outArea = 0, srcTris = 0;
  for (const prim of source.prims) for (const t of prim.simplified ? [] : prim.tris) {
    const p = t.map((v) => prim.pos[v].map((c) => c * UNITS_PER_METRE)), u = t.map((v) => prim.uv[v]);
    if (new Set(p.map(key)).size < 3 || triArea(p[0], p[1], p[2]) < 1e-6) continue; // zero-area slivers draw nothing
    srcTris++;
    srcArea += triArea(p[0], p[1], p[2]);
    const inFace = (pt, k) => got.some((f) => {
      for (let i = 1; i + 1 < f.corners.length; i++) {
        if (bary(pt, f.corners[0], f.corners[i], f.corners[i + 1], f.uv[0], f.uv[i], f.uv[i + 1], u[k])) return true;
      }
      return false;
    });
    const hit = p.every(inFace);
    if (!hit) missing++;
  }
  for (const f of got) for (let i = 1; i + 1 < f.corners.length; i++) outArea += triArea(f.corners[0], f.corners[i], f.corners[i + 1]);
  const areaOk = Math.abs(srcArea - outArea) <= 1e-3 * Math.max(1, srcArea);
  return { ok: missing === 0 && areaOk, missing, srcTris, srcArea: r4(srcArea), outArea: r4(outArea) };
}

// ---------- CLI ----------
function walk(dir, recursive, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (recursive) walk(p, true, out); } else if (e.name.toLowerCase().endsWith('.glb')) out.push(p);
  }
  return out;
}
// Default paths: the GLB sources live in tools/props/source_glb/<path under assets/>.glb and every one converts to
// src/bin/last_route/assets/<same path>.bbmodel (the game loads only the bbmodels).
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
export const SOURCE_GLB = path.join(ROOT, 'tools', 'props', 'source_glb');
export const ASSETS = path.join(ROOT, 'src', 'bin', 'last_route', 'assets');
const toPosix = (p) => p.split(path.sep).join('/');
/** The atlas name a model's texture is stored under: the blocky props share one atlas, every other model has its own. */
export function atlasNameFor(rel) {
  return rel.startsWith('props/blocky/') ? 'blocky_atlas.png' : undefined;
}

function main(argv) {
  const opt = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--quiet' || a === '--recursive' || a === '--all' || a === '--check' || a === '--no-simplify') opt[a.slice(2)] = true;
    else if (a.startsWith('--')) opt[a.slice(2)] = argv[++i];
    else opt._.push(a);
  }
  const jobs = [];
  if (opt.all || opt.check) {
    // every GLB under tools/props/source_glb -> the bbmodel beside its place under assets/. --check writes nothing and
    // fails when a bbmodel on disk differs from what the converter makes now.
    let bad = 0, n = 0;
    for (const f of walk(SOURCE_GLB, true)) {
      const rel = toPosix(path.relative(SOURCE_GLB, f)).replace(/\.glb$/i, '');
      const outp = path.join(ASSETS, rel + '.bbmodel');
      const name = path.basename(outp, '.bbmodel');
      try {
        const r = convert(fs.readFileSync(f), name, { atlasName: atlasNameFor(rel + '.glb'), simplify: !opt['no-simplify'] });
        const text = JSON.stringify(r.bbmodel);
        const rt = roundTrip(r.bbmodel, r.source);
        if (!rt.ok) { console.error(`ROUND TRIP ${rel}: missing ${rt.missing}, area ${rt.srcArea} vs ${rt.outArea}`); bad++; }
        if (opt.check) {
          if (!fs.existsSync(outp) || fs.readFileSync(outp, 'utf8') !== text) { console.error(`DIFFERS ${rel}`); bad++; }
        } else {
          fs.mkdirSync(path.dirname(outp), { recursive: true });
          fs.writeFileSync(outp, text);
        }
        n++;
      } catch (e) { console.error(`FAILED ${rel}: ${e.message}`); bad++; }
    }
    console.log(`${n} models ${opt.check ? 'checked' : 'written'}, ${bad} problems`);
    if (bad) process.exitCode = 1;
    return;
  }
  if (opt.dir) {
    if (!opt.out) { console.error('--dir needs --out'); process.exit(2); }
    for (const f of walk(opt.dir, !!opt.recursive)) jobs.push([f, path.join(opt.out, path.basename(f, '.glb') + '.bbmodel')]);
  } else if (opt._.length === 2) jobs.push([opt._[0], opt._[1]]);
  else {
    console.error('usage: glb_to_bbmodel.mjs <in.glb> <out.bbmodel> [--name N] [--atlas-name F]\n       glb_to_bbmodel.mjs --dir <dir> --out <dir> [--recursive]\n       glb_to_bbmodel.mjs --all | --check   (tools/props/source_glb -> src/bin/last_route/assets)');
    process.exit(2);
  }
  let failed = 0;
  for (const [inp, outp] of jobs) {
    try {
      const name = opt.name || path.basename(outp, '.bbmodel');
      const r = convert(fs.readFileSync(inp), name, { atlasName: opt['atlas-name'], simplify: !opt['no-simplify'] });
      fs.mkdirSync(path.dirname(outp), { recursive: true });
      fs.writeFileSync(outp, JSON.stringify(r.bbmodel));
      const rt = roundTrip(r.bbmodel, r.source);
      if (!opt.quiet) {
        console.log(`${name}: ${r.stats.cubes} cubes, ${r.stats.meshElements} mesh elements (${r.stats.meshFaces} faces), size ${r.stats.size.join(' x ')} m, roundtrip ${rt.ok ? 'OK' : 'MISMATCH (missing ' + rt.missing + ', area ' + rt.srcArea + ' vs ' + rt.outArea + ')'}`);
      }
      if (!rt.ok) failed++;
    } catch (e) { failed++; console.error(`FAILED ${inp}: ${e.message}`); }
  }
  if (failed) process.exitCode = 1;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) main(process.argv.slice(2));
