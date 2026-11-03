#!/usr/bin/env node
// fidelity_report: prints the numbers behind docs/PROPS_BBMODEL.md. Node built-ins only.
//   node tools/props/fidelity_report.mjs [godot_assets_dir]
// Sections: (1) the five converted Godot models vs their GLBs, (2) scale check against existing props,
// (3) variants of one prop, (4) every GLB the Rust port still ships through the converter.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { convert, roundTrip, loadLikeRust } from './glb_to_bbmodel.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..');
const godot = process.argv[2] || path.resolve(root, '..', 'LastRoute Godot', 'KyttusLabApp', 'assets');
const r3 = (x) => Math.round(x * 1000) / 1000;

function bboxOfBb(bb) {
  const mn = [1e9, 1e9, 1e9], mx = [-1e9, -1e9, -1e9];
  for (const f of loadLikeRust(bb)) for (const c of f.corners) for (let a = 0; a < 3; a++) { mn[a] = Math.min(mn[a], c[a]); mx[a] = Math.max(mx[a], c[a]); }
  return { mn, mx, size: [0, 1, 2].map((a) => r3((mx[a] - mn[a]) / 16)) };
}
function trisOfBb(bb) { return loadLikeRust(bb).reduce((a, f) => a + f.corners.length - 2, 0); }
const rel = (p) => path.relative(root, p).replace(/\\/g, '/');

// (1) the five models
console.log('== 1. Godot-only models: GLB vs converted bbmodel ==');
const five = [
  ['door_frame', 'environment/door_frame.glb', 'assets/environment/door_frame.bbmodel'],
  ['floor_tile', 'environment/floor_tile.glb', 'assets/environment/floor_tile.bbmodel'],
  ['wall_block', 'environment/wall_block.glb', 'assets/environment/wall_block.bbmodel'],
  ['kathy_scythe', 'props/hunter_items/kathy_scythe.glb', 'assets/props/hunter_items/kathy_scythe.bbmodel'],
  ['sewer_outflow_pipe_broken', 'props/sewer/sewer_outflow_pipe_broken.glb', 'assets/props/sewer/sewer_outflow_pipe_broken.bbmodel'],
];
for (const [name, glb, out] of five) {
  const r = convert(fs.readFileSync(path.join(godot, glb)), name);
  const onDisk = JSON.parse(fs.readFileSync(path.join(root, out), 'utf8'));
  const bb = bboxOfBb(onDisk), rt = roundTrip(onDisk, r.source);
  const glbMn = [1e9, 1e9, 1e9], glbMx = [-1e9, -1e9, -1e9];
  for (const p of r.source.prims) for (const v of p.pos) for (let a = 0; a < 3; a++) { glbMn[a] = Math.min(glbMn[a], v[a]); glbMx[a] = Math.max(glbMx[a], v[a]); }
  const glbSize = [0, 1, 2].map((a) => r3(glbMx[a] - glbMn[a]));
  const tex = onDisk.textures[0], texOk = tex.source.startsWith('data:image/png;base64,') && Buffer.from(tex.source.split(',')[1], 'base64').equals(r.source.png);
  console.log(`${name}: GLB bbox ${glbSize.join('x')} m | bbmodel bbox ${bb.size.join('x')} m | GLB tris ${r.stats.srcTriangles} | bbmodel tris ${trisOfBb(onDisk)} (cubes ${r.stats.cubes}, mesh elements ${r.stats.meshElements}, mesh faces ${r.stats.meshFaces}) | texture ${tex.width}x${tex.height} embedded PNG identical=${texOk} | roundtrip ${rt.ok ? 'OK' : 'FAIL'} area ${rt.srcArea} vs ${rt.outArea}`);
}

// (2) scale check
console.log('\n== 2. Scale check: converter output vs existing assets/props/*.bbmodel (same prop, variant _0) ==');
for (const name of ['arcadeCabinet', 'diningChair', 'bedroomRug', 'labBench', 'gothicRoomDoor', 'ivStand']) {
  const g = ['_0.glb', '.glb'].map((s) => path.join(godot, 'props', 'blocky', name + s)).find((p) => fs.existsSync(p));
  const existing = path.join(root, 'assets', 'props', name + '.bbmodel');
  if (!g || !fs.existsSync(existing)) { console.log(`${name}: skipped (no ${!g ? 'Godot GLB' : 'Rust bbmodel'})`); continue; }
  const r = convert(fs.readFileSync(g), name), a = bboxOfBb(r.bbmodel), b = bboxOfBb(JSON.parse(fs.readFileSync(existing, 'utf8')));
  const same = [0, 1, 2].every((i) => Math.abs(a.mn[i] - b.mn[i]) < 0.01 && Math.abs(a.mx[i] - b.mx[i]) < 0.01);
  console.log(`${name}: converter min/max ${a.mn.map(r3)} / ${a.mx.map(r3)} units | existing ${b.mn.map(r3)} / ${b.mx.map(r3)} units | identical bounds ${same}`);
}

// (3) variants
console.log('\n== 3. Variants: arcadeCabinet_0/1/2 -> tools/props/out/variants ==');
const vdir = path.join(here, 'out', 'variants');
fs.mkdirSync(vdir, { recursive: true });
const ex = bboxOfBb(JSON.parse(fs.readFileSync(path.join(root, 'assets', 'props', 'arcadeCabinet.bbmodel'), 'utf8')));
console.log(`existing arcadeCabinet.bbmodel size ${ex.size.join('x')} m`);
for (const i of [0, 1, 2]) {
  const name = `arcadeCabinet_${i}`;
  const r = convert(fs.readFileSync(path.join(godot, 'props', 'blocky', name + '.glb')), name, { atlasName: 'blocky_atlas.png' });
  fs.writeFileSync(path.join(vdir, name + '.bbmodel'), JSON.stringify(r.bbmodel));
  const b = bboxOfBb(r.bbmodel), rt = roundTrip(r.bbmodel, r.source);
  const shared = Buffer.from(JSON.parse(fs.readFileSync(path.join(root, 'assets', 'props', 'arcadeCabinet.bbmodel'), 'utf8')).textures[0].source.split(',')[1], 'base64').equals(r.source.png);
  console.log(`${name}: size ${b.size.join('x')} m | cubes ${r.stats.cubes} mesh elements ${r.stats.meshElements} faces ${r.stats.totalFaces} | atlas same PNG as existing: ${shared} | roundtrip ${rt.ok ? 'OK' : 'FAIL'} | same bounds as existing: ${[0, 1, 2].every((k) => Math.abs(b.size[k] - ex.size[k]) < 0.01)}`);
}

// (4) the 430 GLBs the Rust port ships
console.log('\n== 4. All GLBs shipped in assets/ ==');
const all = [];
(function walk(d) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (e.name.endsWith('.glb')) all.push(p); } })(path.join(root, 'assets'));
const per = {}, rows = [];
let fail = 0;
for (const f of all) {
  const folder = path.relative(path.join(root, 'assets'), path.dirname(f)).replace(/\\/g, '/');
  const t = (per[folder] ||= { n: 0, pure: 0, mixed: 0, meshOnly: 0, fail: 0, rtFail: 0, cubes: 0, meshFaces: 0, faces: 0 });
  t.n++;
  try {
    const r = convert(fs.readFileSync(f), path.basename(f, '.glb')), rt = roundTrip(r.bbmodel, r.source);
    const s = r.stats;
    if (!rt.ok) t.rtFail++;
    if (s.meshElements === 0) t.pure++; else if (s.cubes === 0) t.meshOnly++; else t.mixed++;
    t.cubes += s.cubes; t.meshFaces += s.meshFaces; t.faces += s.totalFaces;
    rows.push({ folder, name: path.basename(f), cubes: s.cubes, meshEls: s.meshElements, meshFaces: s.meshFaces, srcTris: s.srcTriangles, rt: rt.ok });
  } catch (e) { t.fail++; fail++; console.log(`FAILED ${rel(f)}: ${e.message}`); }
}
console.log('folder | glbs | pure cubes | mixed | mesh only | failed | roundtrip fail | cubes | mesh faces');
const tot = { n: 0, pure: 0, mixed: 0, meshOnly: 0, fail: 0, rtFail: 0, cubes: 0, meshFaces: 0 };
for (const [k, t] of Object.entries(per)) {
  console.log(`${k} | ${t.n} | ${t.pure} | ${t.mixed} | ${t.meshOnly} | ${t.fail} | ${t.rtFail} | ${t.cubes} | ${t.meshFaces}`);
  for (const key of Object.keys(tot)) tot[key] += t[key];
}
console.log(`TOTAL | ${tot.n} | ${tot.pure} | ${tot.mixed} | ${tot.meshOnly} | ${tot.fail} | ${tot.rtFail} | ${tot.cubes} | ${tot.meshFaces}`);
console.log(`cube share of all faces: ${r3(100 * (1 - tot.meshFaces / Math.max(1, per && Object.values(per).reduce((a, t) => a + t.faces, 0))))}%`);
// 14 spread samples
console.log('\nsample (every ' + Math.floor(rows.length / 14) + 'th):');
const step = Math.floor(rows.length / 14);
for (let i = 0; i < rows.length && i < step * 14; i += step) { const r = rows[i]; console.log(`${r.folder}/${r.name}: ${r.cubes} cubes, ${r.meshEls} mesh elements (${r.meshFaces} faces), src tris ${r.srcTris}, roundtrip ${r.rt ? 'OK' : 'FAIL'}`); }
