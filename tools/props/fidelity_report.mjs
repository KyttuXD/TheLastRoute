#!/usr/bin/env node
// fidelity_report: prints the numbers behind docs/PROPS_BBMODEL.md. Node built-ins only.
//   node tools/props/fidelity_report.mjs [godot_assets_dir]
// Sections: (1) the five converted Godot models vs their GLBs, (2) scale check against existing props,
// (3) variants of one prop, (4) every prop model: GLB bbox vs bbmodel bbox, (5) the one-per-prop bbmodels vs the GLB variants, (6) coverage.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { convert, roundTrip, loadLikeRust, readGlb } from './glb_to_bbmodel.mjs';

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
  ['door_frame', 'environment/door_frame.glb', 'src/bin/last_route/assets/environment/door_frame.bbmodel'],
  ['floor_tile', 'environment/floor_tile.glb', 'src/bin/last_route/assets/environment/floor_tile.bbmodel'],
  ['wall_block', 'environment/wall_block.glb', 'src/bin/last_route/assets/environment/wall_block.bbmodel'],
  ['kathy_scythe', 'props/hunter_items/kathy_scythe.glb', 'src/bin/last_route/assets/props/hunter_items/kathy_scythe.bbmodel'],
  ['sewer_outflow_pipe_broken', 'props/sewer/sewer_outflow_pipe_broken.glb', 'src/bin/last_route/assets/props/sewer/sewer_outflow_pipe_broken.bbmodel'],
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
  const existing = path.join(root, 'src', 'bin', 'last_route', 'assets', 'props', name + '.bbmodel');
  if (!g || !fs.existsSync(existing)) { console.log(`${name}: skipped (no ${!g ? 'Godot GLB' : 'Rust bbmodel'})`); continue; }
  const r = convert(fs.readFileSync(g), name), a = bboxOfBb(r.bbmodel), b = bboxOfBb(JSON.parse(fs.readFileSync(existing, 'utf8')));
  const same = [0, 1, 2].every((i) => Math.abs(a.mn[i] - b.mn[i]) < 0.01 && Math.abs(a.mx[i] - b.mx[i]) < 0.01);
  console.log(`${name}: converter min/max ${a.mn.map(r3)} / ${a.mx.map(r3)} units | existing ${b.mn.map(r3)} / ${b.mx.map(r3)} units | identical bounds ${same}`);
}

// (3) variants
console.log('\n== 3. Variants: arcadeCabinet_0/1/2 -> tools/props/out/variants ==');
const vdir = path.join(here, 'out', 'variants');
fs.mkdirSync(vdir, { recursive: true });
const ex = bboxOfBb(JSON.parse(fs.readFileSync(path.join(root, 'src', 'bin', 'last_route', 'assets', 'props', 'arcadeCabinet.bbmodel'), 'utf8')));
console.log(`existing arcadeCabinet.bbmodel size ${ex.size.join('x')} m`);
for (const i of [0, 1, 2]) {
  const name = `arcadeCabinet_${i}`;
  const r = convert(fs.readFileSync(path.join(godot, 'props', 'blocky', name + '.glb')), name, { atlasName: 'blocky_atlas.png' });
  fs.writeFileSync(path.join(vdir, name + '.bbmodel'), JSON.stringify(r.bbmodel));
  const b = bboxOfBb(r.bbmodel), rt = roundTrip(r.bbmodel, r.source);
  const shared = Buffer.from(JSON.parse(fs.readFileSync(path.join(root, 'src', 'bin', 'last_route', 'assets', 'props', 'arcadeCabinet.bbmodel'), 'utf8')).textures[0].source.split(',')[1], 'base64').equals(r.source.png);
  console.log(`${name}: size ${b.size.join('x')} m | cubes ${r.stats.cubes} mesh elements ${r.stats.meshElements} faces ${r.stats.totalFaces} | atlas same PNG as existing: ${shared} | roundtrip ${rt.ok ? 'OK' : 'FAIL'} | same bounds as existing: ${[0, 1, 2].every((k) => Math.abs(b.size[k] - ex.size[k]) < 0.01)}`);
}

// ---- shared metrics for sections 4 to 6 ----
const assetsDir = path.join(root, 'src', 'bin', 'last_route', 'assets');
// the GLB sources: the game loads only bbmodels, the GLBs stay here so the converter can reproduce them
const sourceGlb = path.join(root, 'tools', 'props', 'source_glb');
const dataDir = path.join(root, 'src', 'bin', 'last_route', 'data');
const MATCH_TOL = { len: 0.003, area: 0.002 };
function walkFiles(d, ext, out = []) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walkFiles(p, ext, out); else if (e.name.endsWith(ext)) out.push(p); }
  return out;
}
const fmt = (a) => a.map((x) => r3(x)).join('x');
const triArea = (a, b, c) => { const x = [(b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]), (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]), (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])]; return 0.5 * Math.hypot(...x); };
/** bbox, triangle count, surface area and area-weighted centroid (metres) of a list of polygons ([[x,y,z]..], metres). */
function metricsOf(polys) {
  const mn = [1e9, 1e9, 1e9], mx = [-1e9, -1e9, -1e9], cen = [0, 0, 0];
  let tris = 0, area = 0;
  for (const poly of polys) {
    for (const p of poly) for (let a = 0; a < 3; a++) { mn[a] = Math.min(mn[a], p[a]); mx[a] = Math.max(mx[a], p[a]); }
    for (let i = 1; i + 1 < poly.length; i++) {
      const ar = triArea(poly[0], poly[i], poly[i + 1]);
      if (ar < 1e-9) continue;
      tris++; area += ar;
      for (let a = 0; a < 3; a++) cen[a] += ar * (poly[0][a] + poly[i][a] + poly[i + 1][a]) / 3;
    }
  }
  return { mn, mx, size: [0, 1, 2].map((a) => mx[a] - mn[a]), tris, area, cen: cen.map((c) => c / Math.max(area, 1e-12)) };
}
const glbMetrics = (buf) => metricsOf(readGlb(buf).prims.flatMap((p) => p.tris.map((t) => t.map((v) => p.pos[v]))));
const bbMetrics = (bb) => metricsOf(loadLikeRust(bb).map((f) => f.corners.map((c) => c.map((x) => x / 16))));
/** Same geometry? Returns null when it matches, else the reason. */
function compare(a, b) {
  const dBox = Math.max(...[0, 1, 2].flatMap((i) => [Math.abs(a.mn[i] - b.mn[i]), Math.abs(a.mx[i] - b.mx[i])]));
  if (dBox > MATCH_TOL.len) return `bbox differs by ${r3(dBox)} m`;
  if (Math.abs(a.area - b.area) > MATCH_TOL.area * Math.max(a.area, 1e-6)) return `surface area ${r3(a.area)} vs ${r3(b.area)} m2`;
  const dCen = Math.max(...[0, 1, 2].map((i) => Math.abs(a.cen[i] - b.cen[i])));
  if (dCen > MATCH_TOL.len) return `centroid moved ${r3(dCen)} m (turned or mirrored)`;
  return null;
}
const readGlbFile = (rel) => {
  for (const base of [sourceGlb, godot]) { const p = path.join(base, rel); if (fs.existsSync(p)) return fs.readFileSync(p); }
  return null;
};

// (4) every model the prop data names, plus every GLB in tools/props/source_glb: the GLB against its bbmodel in assets/
console.log('\n== 4. Every prop model: GLB bbox vs bbmodel bbox ==');
const modelSet = new Map(); // asset path (.glb) -> the prop ids that use it
for (const f of walkFiles(path.join(dataDir, 'props'), '.json')) {
  for (const p of JSON.parse(fs.readFileSync(f, 'utf8')).props || []) {
    // the prop data names bbmodels; the GLB each came from has the same path with .glb
    for (const m of p.models || []) { const rel = m.replace(/^res:\/\/assets\//, '').replace(/\.bbmodel$/, '.glb'); if (!modelSet.has(rel)) modelSet.set(rel, new Set()); modelSet.get(rel).add(p.id); }
  }
}
for (const f of walkFiles(sourceGlb, '.glb')) {
  const rel = path.relative(sourceGlb, f).split(path.sep).join('/');
  if (!modelSet.has(rel)) modelSet.set(rel, new Set());
}
console.log('prop | model | GLB bbox (m) | bbmodel bbox (m) | GLB tris | bbmodel tris | match');
const t4 = { rows: 0, ok: 0, noBbmodel: 0, noGlb: 0, bad: [] };
for (const [rel, ids] of [...modelSet].sort()) {
  const label = [...ids].join(',') || '(not in prop data)';
  if (!rel.endsWith('.glb')) continue; // the void hunters are .glb in the hunter data only
  t4.rows++;
  const bbPath = path.join(assetsDir, rel.replace(/\.glb$/, '.bbmodel'));
  const glbBuf = readGlbFile(rel);
  if (!fs.existsSync(bbPath)) { t4.noBbmodel++; t4.bad.push([rel, 'no bbmodel']); console.log(`${label} | ${rel} | ${glbBuf ? fmt(glbMetrics(glbBuf).size) : 'no GLB'} | MISSING | - | - | NO`); continue; }
  const bm = bbMetrics(JSON.parse(fs.readFileSync(bbPath, 'utf8')));
  if (!glbBuf) { t4.noGlb++; console.log(`${label} | ${rel} | no GLB | ${fmt(bm.size)} | - | ${bm.tris} | n/a`); continue; }
  const gm = glbMetrics(glbBuf), why = compare(gm, bm);
  if (!why) t4.ok++; else t4.bad.push([rel, why]);
  console.log(`${label} | ${rel} | ${fmt(gm.size)} | ${fmt(bm.size)} | ${gm.tris} | ${bm.tris} | ${why ? 'NO (' + why + ')' : 'yes'}`);
}
console.log(`section 4: ${t4.rows} models, ${t4.ok} match, ${t4.noBbmodel} without a bbmodel, ${t4.noGlb} with a bbmodel only, ${t4.bad.length} unresolved`);
for (const [rel, why] of t4.bad) console.log(`  UNRESOLVED ${rel}: ${why}`);

// (5) the one-per-prop bbmodels the game draws today (assets/props/<id>.bbmodel, the blocky rebuilds) against the Godot
// blocky GLBs of the same prop (props/blocky/<id>.glb or <id>_N.glb). A prop whose data names authored models is drawn from
// those once the loader reads bbmodels; this section shows where the old one-per-prop file is not the Godot blocky model.
console.log('\n== 5. assets/props/<id>.bbmodel (blocky rebuild drawn today) vs the Godot blocky GLB variants ==');
console.log('prop | blocky variants | bbmodel bbox (m) | variant 0 GLB bbox (m) | match variant 0 | match any variant');
const defs = new Map();
for (const f of walkFiles(path.join(dataDir, 'props'), '.json')) for (const p of JSON.parse(fs.readFileSync(f, 'utf8')).props || []) defs.set(p.id, (p.models || []).map((m) => m.replace(/^res:\/\/assets\//, '')));
const t5 = { n: 0, v0: 0, any: 0, noBlocky: 0, noBb: 0, bad: [] };
const blockyDir = path.join(godot, 'props', 'blocky');
const blockyNames = fs.existsSync(blockyDir) ? fs.readdirSync(blockyDir) : [];
for (const [id] of [...defs].sort()) {
  const bbPath = path.join(assetsDir, 'props', id + '.bbmodel');
  if (!fs.existsSync(bbPath)) { t5.noBb++; console.log(`${id} | - | none (drawn from its own GLB models, section 4) | - | n/a | n/a`); continue; }
  const bm = bbMetrics(JSON.parse(fs.readFileSync(bbPath, 'utf8')));
  const files = blockyNames.filter((n) => n === id + '.glb' || (n.startsWith(id + '_') && /^[0-9]+[.]glb$/.test(n.slice(id.length + 1)))).sort();
  if (!files.length) { t5.noBlocky++; console.log(`${id} | 0 | ${fmt(bm.size)} | no Godot blocky GLB | n/a | n/a`); continue; }
  const gms = files.map((n) => glbMetrics(fs.readFileSync(path.join(blockyDir, n))));
  t5.n++;
  const why0 = compare(gms[0], bm), anyOk = gms.some((g) => !compare(g, bm));
  if (!why0) t5.v0++;
  if (anyOk) t5.any++; else t5.bad.push([id, why0]);
  console.log(`${id} | ${gms.length} | ${fmt(bm.size)} | ${fmt(gms[0].size)} | ${why0 ? 'NO (' + why0 + ')' : 'yes'} | ${anyOk ? 'yes' : 'NO'}`);
}
console.log(`section 5: ${t5.n} one-per-prop bbmodels with Godot blocky GLBs, ${t5.v0} match variant 0, ${t5.any} match some variant, ${t5.noBb} prop ids without a one-per-prop bbmodel, ${t5.noBlocky} one-per-prop bbmodels with no Godot blocky GLB`);
for (const [id, why] of t5.bad) console.log(`  DIFFERS ${id}: ${why}`);

// (6) coverage by folder
console.log('\n== 6. Coverage by folder ==');
const per = {};
for (const [rel] of modelSet) {
  if (!rel.endsWith('.glb')) continue;
  const folder = rel.split('/').slice(0, -1).join('/');
  const t = (per[folder] ||= { glb: 0, bb: 0 });
  t.glb++;
  if (fs.existsSync(path.join(assetsDir, rel.replace(/\.glb$/, '.bbmodel')))) t.bb++;
}
console.log('folder | GLB models | with a bbmodel');
let tg = 0, tb = 0;
for (const [k, t] of Object.entries(per).sort()) { console.log(`${k} | ${t.glb} | ${t.bb}`); tg += t.glb; tb += t.bb; }
console.log(`TOTAL | ${tg} | ${tb}`);
process.exitCode = t4.bad.length ? 1 : 0;
