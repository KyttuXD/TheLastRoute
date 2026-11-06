#!/usr/bin/env node
// QA and format checker. Dependency-free. Run from the repo root: node tools/qa/check.mjs [files...]
// Exit code 1 when any problem is found. See README.md.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const JSON_DIRS = ['src/bin/last_route/data', 'src/bin/last_route/assets/hunters', 'src/bin/last_route/data/regions'];
const SKIP_DIRS = new Set(['target', 'node_modules', '.git', 'quad-snd']);
const problems = [];
const bad = (kind, file, msg) => problems.push(`${kind}: ${path.relative(root, file).replace(/\\/g, '/')}${msg ? ' - ' + msg : ''}`);

export function walk(dir, ext, out = []) {
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of entries) {
    if (SKIP_DIRS.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, ext, out);
    else if (e.name.endsWith(ext)) out.push(p);
  }
  return out;
}

export function mixedLineEndings(text) {
  const crlf = (text.match(/\r\n/g) || []).length;
  const lf = (text.match(/\n/g) || []).length - crlf;
  return crlf > 0 && lf > 0;
}

// True when a tab sits in indentation or between tokens outside string literals.
export function jsonHasTabs(text) {
  let inStr = false, esc = false;
  for (const ch of text) {
    if (inStr) {
      if (esc) esc = false; else if (ch === '\\') esc = true; else if (ch === '"') inStr = false;
    } else if (ch === '"') inStr = true;
    else if (ch === '\t') return true;
  }
  return false;
}

// Strip comments and string literals so the debug-macro scan ignores them.
export function stripRustNoise(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '))
    .replace(/\/\/[^\n]*/g, '')
    .replace(/r#*"[\s\S]*?"#*|"(?:\\.|[^"\\])*"/g, m => m.replace(/[^\n]/g, ' '));
}

export function debugMacros(src) {
  const hits = [];
  stripRustNoise(src).split('\n').forEach((line, i) => {
    if (/\b(println|dbg)!\s*[(\[{]/.test(line)) hits.push(i + 1);
  });
  return hits;
}

export function includeStrPaths(src) {
  return [...src.matchAll(/include_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*\)/g)].map(m => m[1]);
}

export const stripLeadingTabs = t => t.replace(/^\t+/gm, '');

function changedFiles() {
  try {
    const run = a => execFileSync('git', a, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).split('\n').filter(Boolean);
    return [...new Set([...run(['diff', '--name-only', 'HEAD']), ...run(['ls-files', '--others', '--exclude-standard'])])]
      .map(f => path.join(root, f)).filter(f => fs.existsSync(f));
  } catch { return []; }
}

function main() {
  const args = process.argv.slice(2).map(f => path.resolve(root, f));
  const jsonFiles = JSON_DIRS.flatMap(d => walk(path.join(root, d), '.json'));
  const rsFiles = walk(path.join(root, 'src'), '.rs');
  const listed = args.length ? args : changedFiles();

  for (const f of jsonFiles) {
    let t;
    try { t = fs.readFileSync(f, 'utf8'); } catch (e) { bad('json-read', f, e.message); continue; }
    try { JSON.parse(t.replace(/^\uFEFF/, '')); } catch (e) { bad('json-parse', f, e.message); }
    // Many data files are tab-indented; only changed or listed files must be tab-free.
    if (jsonHasTabs(t) && (listed.includes(f) || /\t/.test(stripLeadingTabs(t)))) bad('json-tabs', f, 'tab character outside strings');
  }

  const lineCheck = new Set([...listed, ...jsonFiles]);
  const textExt = /\.(rs|json|toml|mjs|md|yml|yaml)$/;
  for (const f of lineCheck) {
    if (!textExt.test(f) || !fs.existsSync(f)) continue;
    if (mixedLineEndings(fs.readFileSync(f, 'utf8'))) bad('mixed-eol', f, 'both CRLF and LF');
  }

  for (const f of rsFiles) {
    const src = fs.readFileSync(f, 'utf8');
    const rel = path.relative(root, f).replace(/\\/g, '/');
    const isBin = /(^|\/)(bin\/night_watch|bin\/lr_server)\//.test(rel);
    const isTestFile = /(^|\/)tests?(\/|\.rs$)|_tests?\.rs$/.test(rel) || /\bbuild\.rs$/.test(rel);
    // Only files inside the changed list are held to the debug-macro rule when a list exists.
    const inScope = args.length || listed.length ? listed.includes(f) : true;
    if (inScope && !isBin && !isTestFile) {
      for (const ln of debugMacros(src)) bad('debug-macro', f, `line ${ln} has println!/dbg!`);
    }
    for (const p of includeStrPaths(src)) {
      if (!fs.existsSync(path.resolve(path.dirname(f), p))) bad('include-missing', f, `include path not found: ${p}`);
    }
  }

  console.log(`checked ${jsonFiles.length} json, ${rsFiles.length} rust, ${lineCheck.size} line-ending targets`);
  if (problems.length) {
    console.error(problems.join('\n'));
    console.error(`${problems.length} problem(s)`);
    process.exit(1);
  }
  console.log('ok');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
