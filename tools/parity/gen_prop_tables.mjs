// Generates src/world/route/prop_tables.rs from the Godot sources (read-only).
//
// Usage (from the Rust repo root or anywhere):
//   node tools/parity/gen_prop_tables.mjs          write src/world/route/prop_tables.rs
//   node tools/parity/gen_prop_tables.mjs --check  write nothing; exit 1 when the file on disk is not what this would write
//
// The Godot app root is $KT_GODOT_APP, else ../LastRoute Godot/KyttusLabApp beside this Rust repo.
//
// The Godot files are GDScript const dictionaries and arrays. The parser reads only top-level `const NAME := value`
// declarations it is asked for, and understands exactly the value forms those tables use: numbers (also `.5` and
// `-0.09`), strings, true/false, [ ] arrays (trailing commas allowed), { } string-keyed dictionaries, VectorN(...)
// constructors, `ContentRegistry.DATA` (the marker for a table that data/ supplies), a reference to another const in
// the same file, and `+` between two arrays. Any other form is an error with file and line; nothing is guessed.
//
// Exit codes: 0 ok, 1 parse error or stale output, 2 Godot sources not found.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const RUST_ROOT = path.resolve(HERE, '..', '..');
const OUT_PATH = path.join(RUST_ROOT, 'src', 'world', 'route', 'prop_tables.rs');
const GODOT_APP = process.env.KT_GODOT_APP
	? path.resolve(process.env.KT_GODOT_APP)
	: path.resolve(RUST_ROOT, '..', 'LastRoute Godot', 'KyttusLabApp');

const SRG = 'game/world/generation/streamed_route_generator.gd';
const SIZES_FILE = 'game/config/prop_collider_sizes.gd';
const PROFILES_FILE = 'game/config/room_size_profiles.gd';
const CATALOG_FILE = 'game/config/prop_model_catalog.gd';

class GenError extends Error {}

// ---------------------------------------------------------------------------------------------------------------------
// Tokenizer

const PUNCT2 = [':='];
const PUNCT1 = new Set([':', ',', '(', ')', '[', ']', '{', '}', '+', '-', '.', '=']);

function tokenize(src, file, startLine = 1) {
	const toks = [];
	const n = src.length;
	let i = 0;
	let line = startLine;
	let lineStart = 0;
	const fail = (msg) => {
		throw new GenError(`${file}:${line}: ${msg}`);
	};
	while (i < n) {
		const c = src[i];
		if (c === '\n') {
			line++;
			i++;
			lineStart = i;
			continue;
		}
		if (c === ' ' || c === '\t' || c === '\r') {
			i++;
			continue;
		}
		if (c === '#') {
			while (i < n && src[i] !== '\n') i++;
			continue;
		}
		const col = i - lineStart;
		if (c === '"') {
			let j = i + 1;
			let out = '';
			for (;;) {
				if (j >= n || src[j] === '\n') fail('unterminated string literal');
				const d = src[j];
				if (d === '"') break;
				if (d === '\\') {
					const e = src[j + 1];
					const map = { '"': '"', '\\': '\\', n: '\n', t: '\t' };
					if (!Object.prototype.hasOwnProperty.call(map, e)) fail(`unsupported string escape \\${e}`);
					out += map[e];
					j += 2;
					continue;
				}
				out += d;
				j++;
			}
			toks.push({ t: 'str', v: out, line, col });
			i = j + 1;
			continue;
		}
		if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] ?? ''))) {
			const m = /^(?:\d+\.\d*|\.\d+|\d+)/.exec(src.slice(i, i + 64));
			const text = m[0];
			const after = src[i + text.length] ?? '';
			if (/[A-Za-z_]/.test(after)) fail(`malformed number '${text}${after}'`);
			toks.push({ t: 'num', v: Number(text.startsWith('.') ? '0' + text : text), line, col });
			i += text.length;
			continue;
		}
		const idm = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i, i + 128));
		if (idm) {
			toks.push({ t: 'id', v: idm[0], line, col });
			i += idm[0].length;
			continue;
		}
		const two = src.slice(i, i + 2);
		if (PUNCT2.includes(two)) {
			toks.push({ t: 'p', v: two, line, col });
			i += 2;
			continue;
		}
		if (PUNCT1.has(c)) {
			toks.push({ t: 'p', v: c, line, col });
			i++;
			continue;
		}
		fail(`unexpected character '${c}'`);
	}
	return toks;
}

// ---------------------------------------------------------------------------------------------------------------------
// Declarations: top-level chunks split at column-0 keywords, so a function body or a multi-line table never ends early.

// Top-level declarations start at a line with a column-0 keyword. Only the `const` ones are tokenized, so the
// function bodies in the same files (which use syntax this parser does not need) are never read.
const TOP_LEVEL_LINE = /^(?:const|var|static|func|class_name|extends|enum|signal|class)\b/;

function splitRaw(src) {
	const lines = src.split('\n');
	const starts = [];
	lines.forEach((l, idx) => {
		if (TOP_LEVEL_LINE.test(l)) starts.push(idx);
	});
	return starts.map((from, k) => {
		const to = k + 1 < starts.length ? starts[k + 1] : lines.length;
		return { text: lines.slice(from, to).join('\n'), line: from + 1 };
	});
}


class FileConsts {
	constructor(file, src) {
		this.file = file;
		this.chunks = new Map();
		this.memo = new Map();
		this.busy = new Set();
		for (const raw of splitRaw(src)) {
			if (!/^const\b/.test(raw.text)) continue;
			const head = /^const\s+([A-Za-z_][A-Za-z0-9_]*)\s*(:=|=)/.exec(raw.text);
			if (!head) throw new GenError(`${file}:${raw.line}: malformed const declaration`);
			const name = head[1];
			if (this.chunks.has(name)) throw new GenError(`${file}:${raw.line}: duplicate const ${name}`);
			const toks = tokenize(raw.text, file, raw.line);
			this.chunks.set(name, { toks, line: raw.line });
		}
	}

	has(name) {
		return this.chunks.has(name);
	}

	line(name) {
		return this.chunks.get(name).line;
	}

	// The evaluated value node of a const. Parsed on first use, memoised.
	value(name) {
		if (this.memo.has(name)) return this.memo.get(name);
		const chunk = this.chunks.get(name);
		if (!chunk) throw new GenError(`${this.file}: const ${name} not found`);
		if (this.busy.has(name)) throw new GenError(`${this.file}:${chunk.line}: cyclic const ${name}`);
		this.busy.add(name);
		const toks = chunk.toks;
		if (toks[2]?.t !== 'p' || (toks[2].v !== ':=' && toks[2].v !== '=')) {
			throw new GenError(`${this.file}:${chunk.line}: const ${name} must be untyped (':=')`);
		}
		const body = toks.slice(3);
		const p = new Parser(body, this.file, this);
		const node = p.parseExpr();
		if (p.i !== body.length) p.err('unexpected token after the value', body[p.i]);
		this.busy.delete(name);
		this.memo.set(name, node);
		return node;
	}
}

// ---------------------------------------------------------------------------------------------------------------------
// Value parser

class Parser {
	constructor(toks, file, consts) {
		this.toks = toks;
		this.i = 0;
		this.file = file;
		this.consts = consts;
	}

	err(msg, tok) {
		const t = tok ?? this.toks[this.i] ?? this.toks[this.i - 1];
		throw new GenError(`${this.file}:${t ? t.line : '?'}: ${msg}`);
	}

	peek() {
		return this.toks[this.i];
	}

	next() {
		return this.toks[this.i++];
	}

	isP(v) {
		const t = this.peek();
		return t !== undefined && t.t === 'p' && t.v === v;
	}

	expectP(v) {
		const t = this.next();
		if (!t || t.t !== 'p' || t.v !== v) this.err(`expected '${v}'`, t);
		return t;
	}

	parseExpr() {
		let left = this.parsePrimary();
		while (this.isP('+')) {
			const plus = this.next();
			const right = this.parsePrimary();
			if (left.k !== 'arr' || right.k !== 'arr') this.err("'+' is supported only between arrays", plus);
			left = { k: 'arr', items: [...left.items, ...right.items], line: left.line };
		}
		return left;
	}

	parsePrimary() {
		const t = this.next();
		if (!t) this.err('unexpected end of declaration');
		if (t.t === 'p' && t.v === '-') {
			const n = this.next();
			if (!n || n.t !== 'num') this.err("'-' must be followed by a number", t);
			return { k: 'num', v: -n.v, line: t.line };
		}
		if (t.t === 'num') return { k: 'num', v: t.v, line: t.line };
		if (t.t === 'str') return { k: 'str', v: t.v, line: t.line };
		if (t.t === 'p' && t.v === '[') {
			const items = [];
			while (!this.isP(']')) {
				items.push(this.parseExpr());
				if (this.isP(',')) this.next();
				else break;
			}
			this.expectP(']');
			return { k: 'arr', items, line: t.line };
		}
		if (t.t === 'p' && t.v === '{') {
			const entries = [];
			while (!this.isP('}')) {
				const key = this.parsePrimary();
				if (key.k !== 'str') this.err('dictionary keys must be string literals', t);
				this.expectP(':');
				entries.push({ key: key.v, value: this.parseExpr(), line: key.line });
				if (this.isP(',')) this.next();
				else break;
			}
			this.expectP('}');
			return { k: 'dict', entries, line: t.line };
		}
		if (t.t === 'id') {
			if (t.v === 'true' || t.v === 'false') return { k: 'bool', v: t.v === 'true', line: t.line };
			if (/^Vector[234]i?$/.test(t.v) && this.isP('(')) {
				this.next();
				const args = [];
				while (!this.isP(')')) {
					const a = this.parseExpr();
					if (a.k !== 'num') this.err(`${t.v} takes numbers only`, t);
					args.push(a.v);
					if (this.isP(',')) this.next();
					else break;
				}
				this.expectP(')');
				const arity = { Vector2: 2, Vector2i: 2, Vector3: 3, Vector4: 4 }[t.v];
				if (args.length !== arity) this.err(`${t.v} takes ${arity} numbers, got ${args.length}`, t);
				if (t.v === 'Vector2i' && !args.every(Number.isInteger)) this.err('Vector2i takes integers', t);
				return { k: 'vec', type: t.v, v: args, line: t.line };
			}
			if (t.v === 'ContentRegistry') {
				this.expectP('.');
				const d = this.next();
				if (!d || d.t !== 'id' || d.v !== 'DATA') this.err('only ContentRegistry.DATA is understood', t);
				return { k: 'data', line: t.line };
			}
			if (this.isP('(') || this.isP('.')) this.err(`unsupported expression starting '${t.v}'`, t);
			if (!this.consts.has(t.v)) this.err(`unknown name '${t.v}' (only same-file consts are referenced)`, t);
			return this.consts.value(t.v);
		}
		this.err(`unexpected token '${t.v}'`, t);
	}
}

// ---------------------------------------------------------------------------------------------------------------------
// Typed readers over the parsed value nodes. Each one fails with file and line when the shape is not the expected one.

function where(node) {
	return node?.line ?? '?';
}

function asNum(node, what, file) {
	if (!node || node.k !== 'num') throw new GenError(`${file}:${where(node)}: ${what} must be a number`);
	return node.v;
}

function asInt(node, what, file) {
	const v = asNum(node, what, file);
	if (!Number.isInteger(v)) throw new GenError(`${file}:${where(node)}: ${what} must be an integer`);
	return v;
}

function asStr(node, what, file) {
	if (!node || node.k !== 'str') throw new GenError(`${file}:${where(node)}: ${what} must be a string`);
	return node.v;
}

function asBool(node, what, file) {
	if (!node || node.k !== 'bool') throw new GenError(`${file}:${where(node)}: ${what} must be true or false`);
	return node.v;
}

function asArr(node, what, file) {
	if (!node || node.k !== 'arr') throw new GenError(`${file}:${where(node)}: ${what} must be an array`);
	return node.items;
}

function asVec(node, type, what, file) {
	if (!node || node.k !== 'vec' || node.type !== type) {
		throw new GenError(`${file}:${where(node)}: ${what} must be a ${type}(...)`);
	}
	return node.v;
}

// A dictionary as ordered [key, valueNode] pairs; duplicate keys are an error (GDScript rejects them too).
function asDict(node, what, file) {
	if (!node || node.k !== 'dict') throw new GenError(`${file}:${where(node)}: ${what} must be a dictionary`);
	const seen = new Set();
	for (const e of node.entries) {
		if (seen.has(e.key)) throw new GenError(`${file}:${e.line}: duplicate key "${e.key}" in ${what}`);
		seen.add(e.key);
	}
	return node.entries.map((e) => [e.key, e.value]);
}

// Reads a dictionary entry as an object of named fields: unknown keys and missing required keys are errors.
function fields(node, spec, what, file) {
	const dict = asDict(node, what, file);
	const out = {};
	for (const [key, value] of dict) {
		if (!Object.prototype.hasOwnProperty.call(spec, key)) {
			throw new GenError(`${file}:${where(value)}: unexpected key "${key}" in ${what}`);
		}
		out[key] = value;
	}
	for (const [key, required] of Object.entries(spec)) {
		if (required && out[key] === undefined) throw new GenError(`${file}:${where(node)}: ${what} lacks "${key}"`);
	}
	return out;
}

// ---------------------------------------------------------------------------------------------------------------------
// Rust text helpers

// A Godot float (f32 in the game) as a Rust f32 literal. JS String() gives the shortest round-trip decimal.
function rf(n) {
	if (!Number.isFinite(n)) throw new GenError(`non-finite number ${n}`);
	const s = String(n);
	return /^-?\d+$/.test(s) ? `${s}.0` : s;
}

function ri(n) {
	return String(n);
}

function rs(s) {
	return JSON.stringify(s);
}

function rarr(items) {
	return `[${items.join(', ')}]`;
}

function rsliceStr(list) {
	return `&[${list.map(rs).join(', ')}]`;
}

function rcf(vals) {
	return rarr(vals.map(rf));
}

// ---------------------------------------------------------------------------------------------------------------------
// Table emitters. Each returns the Rust text of one const and the count the summary reports.

const sections = [];
const counts = [];

function emit(lines, name, count) {
	sections.push(lines.join('\n'));
	counts.push(`${name}: ${count}`);
}

function docLine(file, godotName, line) {
	return `/// Godot \`${godotName}\`: \`${file}\` line ${line}.`;
}

function buildRegionPropSets(srg) {
	const name = '_CODE_REGION_PROP_SETS';
	const node = srg.value(name);
	const dict = asDict(node, name, SRG);
	const regions = [];
	const dataRegions = [];
	let total = 0;
	for (const [region, value] of dict) {
		if (value.k === 'data') {
			dataRegions.push(region);
			continue;
		}
		const items = asArr(value, `${name}["${region}"]`, SRG);
		const entries = items.map((item) => {
			const f = fields(item, { type: true, size: true, solid: false }, `${name}["${region}"] entry`, SRG);
			const propType = asStr(f.type, 'type', SRG);
			const size = asVec(f.size, 'Vector3', 'size', SRG);
			const solid = f.solid === undefined ? true : asBool(f.solid, 'solid', SRG);
			return `        PropSetEntry { prop_type: ${rs(propType)}, size: ${rcf(size)}, solid: ${solid} },`;
		});
		total += items.length;
		regions.push(`    (${rs(region)}, &[\n${entries.join('\n')}\n    ]),`);
	}
	const lines = [
		docLine(SRG, name, srg.line(name)) + ' Mall and hospital are data-backed (see DATA_BACKED_REGION_PROP_SETS).',
		'pub const REGION_PROP_SETS: &[(&str, &[PropSetEntry])] = &[',
		...regions,
		'];',
		'/// Regions whose Godot `REGION_PROP_SETS` value comes from `data/regions/*.json` (`ContentRegistry.DATA`), not from code.',
		`pub const DATA_BACKED_REGION_PROP_SETS: &[&str] = &${rarr(dataRegions.map(rs))};`,
	];
	emit(lines, 'REGION_PROP_SETS', `${regions.length} code regions, ${total} entries; data-backed ${dataRegions.join(',')}`);
}

function buildVec3Dict(file, name, rustName, rustDoc, node, count) {
	const dict = asDict(node, name, file);
	const rows = dict.map(([k, v]) => `    (${rs(k)}, ${rcf(asVec(v, 'Vector3', `${name}["${k}"]`, file))}),`);
	emit(
		[docLine(file, name, count.line), `${rustDoc}`, `pub const ${rustName}: &[(&str, [f32; 3])] = &[`, ...rows, '];'],
		rustName,
		dict.length,
	);
}

function buildStrArray(file, name, rustName, rustDoc, node, line) {
	const items = asArr(node, name, file).map((x) => asStr(x, `${name} item`, file));
	emit([docLine(file, name, line), rustDoc, `pub const ${rustName}: &[&str] = ${rsliceStr(items)};`], rustName, items.length);
}

function buildStrDict(file, name, rustName, rustDoc, node, line) {
	const dict = asDict(node, name, file);
	const rows = dict.map(([k, v]) => `    (${rs(k)}, ${rs(asStr(v, `${name}["${k}"]`, file))}),`);
	emit([docLine(file, name, line), rustDoc, `pub const ${rustName}: &[(&str, &str)] = &[`, ...rows, '];'], rustName, dict.length);
}

function buildTemplateSizes(srg) {
	buildVec3Dict(SRG, 'TEMPLATE_PROP_SIZES', 'TEMPLATE_PROP_SIZES',
		'/// Collider envelopes for furnished-room recipes (metres, x / height / z).',
		srg.value('TEMPLATE_PROP_SIZES'), { line: srg.line('TEMPLATE_PROP_SIZES') });
}

function buildHunters(srg) {
	buildStrArray(SRG, 'STREAMED_HUNTER_IDS', 'STREAMED_HUNTER_IDS',
		'/// Hunter ids that enter the streamed route.', srg.value('STREAMED_HUNTER_IDS'), srg.line('STREAMED_HUNTER_IDS'));
}

function buildRoomSizes(srg) {
	const name = 'ROOM_SIZES';
	const items = asArr(srg.value(name), name, SRG).map((x) => {
		const [w, h] = asVec(x, 'Vector2i', name, SRG);
		return `[${ri(w)}, ${ri(h)}]`;
	});
	emit([docLine(SRG, name, srg.line(name)), 'pub const ROOM_SIZES: &[[i32; 2]] = &' + `[${items.join(', ')}];`], 'ROOM_SIZES', items.length);
}

function buildLargeRooms(srg) {
	const chance = asDict(srg.value('LARGE_ROOM_CHANCE'), 'LARGE_ROOM_CHANCE', SRG);
	emit([
		docLine(SRG, 'LARGE_ROOM_CHANCE', srg.line('LARGE_ROOM_CHANCE')),
		'pub const LARGE_ROOM_CHANCE: &[(&str, f32)] = &[',
		...chance.map(([k, v]) => `    (${rs(k)}, ${rf(asNum(v, `LARGE_ROOM_CHANCE["${k}"]`, SRG))}),`),
		'];',
	], 'LARGE_ROOM_CHANCE', chance.length);

	const types = asDict(srg.value('LARGE_ROOM_TYPES'), 'LARGE_ROOM_TYPES', SRG);
	emit([
		docLine(SRG, 'LARGE_ROOM_TYPES', srg.line('LARGE_ROOM_TYPES')),
		'pub const LARGE_ROOM_TYPES: &[(&str, &[&str])] = &[',
		...types.map(([k, v]) => `    (${rs(k)}, ${rsliceStr(asArr(v, `LARGE_ROOM_TYPES["${k}"]`, SRG).map((x) => asStr(x, 'room type', SRG)))}),`),
		'];',
	], 'LARGE_ROOM_TYPES', types.length);

	const size = asInt(srg.value('LARGE_ROOM_SIZE'), 'LARGE_ROOM_SIZE', SRG);
	emit([docLine(SRG, 'LARGE_ROOM_SIZE', srg.line('LARGE_ROOM_SIZE')), `pub const LARGE_ROOM_SIZE: i32 = ${ri(size)};`], 'LARGE_ROOM_SIZE', 1);
}

function buildOrientPairs(srg) {
	const name = 'ORIENT_PAIRS';
	const items = asArr(srg.value(name), name, SRG).map((item) => {
		const f = fields(item, { child: true, anchor: true, reach: true }, `${name} entry`, SRG);
		const child = asArr(f.child, 'child', SRG).map((x) => asStr(x, 'child kind', SRG));
		const anchor = asArr(f.anchor, 'anchor', SRG).map((x) => asStr(x, 'anchor kind', SRG));
		const reach = asNum(f.reach, 'reach', SRG);
		return `    OrientPair { child: ${rsliceStr(child)}, anchor: ${rsliceStr(anchor)}, reach: ${rf(reach)} },`;
	});
	emit([
		docLine(SRG, name, srg.line(name)),
		'pub const ORIENT_PAIRS: &[OrientPair] = &[',
		...items,
		'];',
	], 'ORIENT_PAIRS', items.length);
	buildStrArray(SRG, 'ORIENT_EXCLUDE', 'ORIENT_EXCLUDE', '/// Child kinds that `ORIENT_PAIRS` never turns.', srg.value('ORIENT_EXCLUDE'), srg.line('ORIENT_EXCLUDE'));
	emit([
		docLine(SRG, 'SCREEN_SEAT_REACH', srg.line('SCREEN_SEAT_REACH')),
		`pub const SCREEN_SEAT_REACH: f32 = ${rf(asNum(srg.value('SCREEN_SEAT_REACH'), 'SCREEN_SEAT_REACH', SRG))};`,
	], 'SCREEN_SEAT_REACH', 1);
}

function buildMisc(srg) {
	buildStrArray(SRG, 'BED_TEMPLATES', 'BED_TEMPLATES', '/// Room templates that guarantee a bed.', srg.value('BED_TEMPLATES'), srg.line('BED_TEMPLATES'));
	buildStrArray(SRG, 'FALLBACK_BEDS', 'FALLBACK_BEDS', '/// Bed kinds the bed guarantee falls back to.', srg.value('FALLBACK_BEDS'), srg.line('FALLBACK_BEDS'));
	buildStrDict(SRG, 'CEILING_FIXTURE_FAMILY', 'CEILING_FIXTURE_FAMILY', '/// Ceiling fixture kind to fixture family.',
		srg.value('CEILING_FIXTURE_FAMILY'), srg.line('CEILING_FIXTURE_FAMILY'));
	buildStrArray(SRG, 'CEILING_FAMILY_ORDER', 'CEILING_FAMILY_ORDER', '/// Ceiling fixture families in placement order.',
		srg.value('CEILING_FAMILY_ORDER'), srg.line('CEILING_FAMILY_ORDER'));
	buildStrDict(SRG, 'HIDING_PROP_STYLES', 'HIDING_PROP_STYLES', '/// Prop kind to hiding style (bed or wardrobe).',
		srg.value('HIDING_PROP_STYLES'), srg.line('HIDING_PROP_STYLES'));
}

function buildColliders(sizes) {
	const offsets = asDict(sizes.value('OFFSETS'), 'OFFSETS', SIZES_FILE);
	emit([
		docLine(SIZES_FILE, 'OFFSETS', sizes.line('OFFSETS')),
		'pub const PROP_COLLIDER_OFFSETS: &[(&str, [f32; 2])] = &[',
		...offsets.map(([k, v]) => `    (${rs(k)}, ${rcf(asVec(v, 'Vector2', `OFFSETS["${k}"]`, SIZES_FILE))}),`),
		'];',
	], 'PROP_COLLIDER_OFFSETS', offsets.length);

	const boxes4 = (name) => asDict(sizes.value(name), name, SIZES_FILE).map(([k, v]) => {
		const list = asArr(v, `${name}["${k}"]`, SIZES_FILE).map((b) => rcf(asVec(b, 'Vector4', `${name}["${k}"] box`, SIZES_FILE)));
		return `    (${rs(k)}, &[${list.join(', ')}]),`;
	});
	const extra = asDict(sizes.value('EXTRA_BOXES'), 'EXTRA_BOXES', SIZES_FILE);
	emit([
		docLine(SIZES_FILE, 'EXTRA_BOXES', sizes.line('EXTRA_BOXES')),
		'pub const PROP_COLLIDER_EXTRA_BOXES: &[(&str, &[[f32; 4]])] = &[',
		...boxes4('EXTRA_BOXES'),
		'];',
	], 'PROP_COLLIDER_EXTRA_BOXES', extra.length);

	const variant = asDict(sizes.value('VARIANT_BOXES'), 'VARIANT_BOXES', SIZES_FILE);
	emit([
		docLine(SIZES_FILE, 'VARIANT_BOXES', sizes.line('VARIANT_BOXES')),
		'pub const PROP_COLLIDER_VARIANT_BOXES: &[(&str, &[[f32; 4]])] = &[',
		...boxes4('VARIANT_BOXES'),
		'];',
	], 'PROP_COLLIDER_VARIANT_BOXES', variant.length);

	buildVec3Dict(SIZES_FILE, 'SIZES', 'PROP_COLLIDER_SIZES', '/// Derived collision box sizes (x, height, z) per solid prop.',
		sizes.value('SIZES'), { line: sizes.line('SIZES') });
}

function buildProfiles(profiles) {
	const name = 'PROFILES';
	const dict = asDict(profiles.value(name), name, PROFILES_FILE);
	const rows = dict.map(([room, value]) => {
		const f = fields(value, {
			min: true, preferred: true, max: true, tables: true, signature: true, generated: true, seated: false,
		}, `${name}["${room}"]`, PROFILES_FILE);
		const vec2 = (key) => {
			const [a, b] = asVec(f[key], 'Vector2i', `${room}.${key}`, PROFILES_FILE);
			return `[${ri(a)}, ${ri(b)}]`;
		};
		const signature = asArr(f.signature, `${room}.signature`, PROFILES_FILE).map((x) => asStr(x, 'signature', PROFILES_FILE));
		const generated = asBool(f.generated, `${room}.generated`, PROFILES_FILE);
		const seated = f.seated === undefined ? 'None' : `Some(${asBool(f.seated, `${room}.seated`, PROFILES_FILE)})`;
		return `    RoomSizeProfile { room_type: ${rs(room)}, min: ${vec2('min')}, preferred: ${vec2('preferred')}, ` +
			`max: ${vec2('max')}, tables: ${vec2('tables')}, signature: ${rsliceStr(signature)}, generated: ${generated}, ` +
			`seated: ${seated} },`;
	});
	emit([
		docLine(PROFILES_FILE, name, profiles.line(name)),
		'pub const ROOM_SIZE_PROFILES: &[RoomSizeProfile] = &[',
		...rows,
		'];',
	], 'ROOM_SIZE_PROFILES', dict.length);
}

function buildWallMounted(catalog) {
	const frames = asArr(catalog.value('EMPTY_FRAME_TYPES'), 'EMPTY_FRAME_TYPES', CATALOG_FILE).map((x) => asStr(x, 'frame type', CATALOG_FILE));
	emit([
		docLine(CATALOG_FILE, 'EMPTY_FRAME_TYPES', catalog.line('EMPTY_FRAME_TYPES')),
		`pub const EMPTY_FRAME_TYPES: &[&str] = ${rsliceStr(frames)};`,
	], 'EMPTY_FRAME_TYPES', frames.length);
	const mounted = asArr(catalog.value('WALL_MOUNTED_TYPES'), 'WALL_MOUNTED_TYPES', CATALOG_FILE).map((x) => asStr(x, 'mounted type', CATALOG_FILE));
	emit([
		docLine(CATALOG_FILE, 'WALL_MOUNTED_TYPES', catalog.line('WALL_MOUNTED_TYPES')) + ' (EMPTY_FRAME_TYPES followed by the rest, in order.)',
		`pub const WALL_MOUNTED_TYPES: &[&str] = ${rsliceStr(mounted)};`,
	], 'WALL_MOUNTED_TYPES', mounted.length);
}

// ---------------------------------------------------------------------------------------------------------------------
// Output

const HEADER = `// GENERATED by tools/parity/gen_prop_tables.mjs from the Godot sources. Do not edit by hand.
// Regenerate with: node tools/parity/gen_prop_tables.mjs
// (cargo test checks this file is current: generated_file_matches_generator.)
//
// Source files, relative to the Godot app root (LastRoute Godot/KyttusLabApp):
//   ${SRG}
//   ${SIZES_FILE}
//   ${PROFILES_FILE}
//   ${CATALOG_FILE}
//
// Values are copied exactly. Godot's Vector2/Vector3/Vector4 and float literals are f32 in the game; the
// Vector2i sizes are cells. Each table keeps the Godot name, with a prefix where two Godot classes share one
// (PropColliderSizes.SIZES is PROP_COLLIDER_SIZES). Dictionaries keep their Godot source order.
// The mall and hospital region prop sets are not inlined: they come from data/regions (see DATA_BACKED_REGION_PROP_SETS).
`;

const TYPES = `
/// One entry of a region's prop set (SRG \`REGION_PROP_SETS\`). \`solid\` is Godot's default true unless the entry says false.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropSetEntry {
    pub prop_type: &'static str,
    /// Authored (x, height, z) in metres: the fallback blocker size.
    pub size: [f32; 3],
    pub solid: bool,
}

/// One row of \`ROOM_SIZE_PROFILES\` (Godot \`RoomSizeProfiles.PROFILES\`). Sizes are cells (x, z) in either orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomSizeProfile {
    pub room_type: &'static str,
    pub min: [i32; 2],
    pub preferred: [i32; 2],
    pub max: [i32; 2],
    pub tables: [i32; 2],
    pub signature: &'static [&'static str],
    pub generated: bool,
    /// Godot's optional \`"seated"\` key. \`None\` when the key is absent.
    pub seated: Option<bool>,
}

/// One entry of \`ORIENT_PAIRS\` (SRG): a child kind turns to face an anchor kind within \`reach\` metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientPair {
    pub child: &'static [&'static str],
    pub anchor: &'static [&'static str],
    pub reach: f32,
}

/// The value stored under \`key\` in one of the generated tables (a linear scan: the tables are small).
pub fn lookup<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}
`;

const TESTS = `
#[cfg(test)]
mod tests {
    use super::*;

    fn region(name: &str) -> &'static [PropSetEntry] {
        lookup(REGION_PROP_SETS, name).expect("the region has a code prop set")
    }

    fn profile(name: &str) -> &'static RoomSizeProfile {
        ROOM_SIZE_PROFILES
            .iter()
            .find(|p| p.room_type == name)
            .expect("the room type has a profile")
    }

    // Line numbers are streamed_route_generator.gd unless a comment names another file.

    #[test]
    fn region_prop_sets_match_godot() {
        // _CODE_REGION_PROP_SETS, lines 134-248: eleven code regions, 88 entries. mall and hospital (lines 229-230)
        // are ContentRegistry.DATA.
        assert_eq!(REGION_PROP_SETS.len(), 11);
        assert_eq!(REGION_PROP_SETS.iter().map(|(_, s)| s.len()).sum::<usize>(), 88);
        assert_eq!(DATA_BACKED_REGION_PROP_SETS, &["mall", "hospital"]);
        assert!(REGION_PROP_SETS.iter().all(|(k, _)| *k != "mall" && *k != "hospital"));
        assert_eq!(region("maze").len(), 8);
        // line 136
        assert_eq!(
            region("maze")[0],
            PropSetEntry { prop_type: "arcadeCabinet", size: [0.75, 1.85, 0.7], solid: true }
        );
        // line 143: a picture frame on the wall, not solid
        assert_eq!(
            region("maze")[7],
            PropSetEntry { prop_type: "normalPictureFrameEmpty", size: [0.7, 0.9, 0.12], solid: false }
        );
        // line 151, index 5 of mansion (lines 146-155)
        assert_eq!(
            region("mansion")[5],
            PropSetEntry { prop_type: "bunkBed", size: [1.06, 1.88, 2.1], solid: true }
        );
        // lines 186-187: an entry split over two lines; the solid flag is on the second
        assert_eq!(
            region("beach")[4],
            PropSetEntry { prop_type: "frozenTidelinePool", size: [6.2, 0.2, 5.2], solid: false }
        );
        assert_eq!(region("carnival").len(), 14);
        assert_eq!(region("gothic").len(), 8);
    }

    #[test]
    fn template_prop_sizes_match_godot() {
        // TEMPLATE_PROP_SIZES, lines 299-494: 234 entries.
        assert_eq!(TEMPLATE_PROP_SIZES.len(), 234);
        // line 300
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "mansionMarbleColumn"), Some([0.5, 2.25, 0.5]));
        // line 336: the tightened corner-rack envelope, not the authored size on line 210
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "prisonKitchenRack"), Some([0.95, 1.82, 0.62]));
        // line 338
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "bedroomWardrobe"), Some([1.25, 2.15, 0.65]));
        // line 386
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "frozenTidelinePool"), Some([6.2, 0.2, 5.2]));
        // line 407
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "hotelCarpet"), Some([2.2, 0.03, 1.4]));
        // line 434: two entries on one line
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "hospitalCurtain"), Some([1.9, 2.4, 0.16]));
        assert_eq!(lookup(TEMPLATE_PROP_SIZES, "noSuchProp"), None);
    }

    #[test]
    fn route_constants_match_godot() {
        // STREAMED_HUNTER_IDS, lines 499-503: fourteen ids.
        assert_eq!(STREAMED_HUNTER_IDS.len(), 14);
        assert_eq!(STREAMED_HUNTER_IDS[0], "dovelle");
        assert_eq!(STREAMED_HUNTER_IDS[13], "verrine");
        // ROOM_SIZES, lines 126-127: eight sizes, from 9x9 to 15x15.
        assert_eq!(ROOM_SIZES.len(), 8);
        assert_eq!(ROOM_SIZES[0], [9, 9]);
        assert_eq!(ROOM_SIZES[7], [15, 15]);
        // LARGE_ROOM_CHANCE line 109, LARGE_ROOM_TYPES line 111, LARGE_ROOM_SIZE line 116.
        assert_eq!(lookup(LARGE_ROOM_CHANCE, "mansion"), Some(0.30));
        assert_eq!(lookup(LARGE_ROOM_TYPES, "maze"), Some(&["restaurant_dining"][..]));
        assert_eq!(LARGE_ROOM_SIZE, 21);
    }

    #[test]
    fn collider_tables_match_godot() {
        // prop_collider_sizes.gd: OFFSETS lines 8-82 (73), EXTRA_BOXES 84-86, VARIANT_BOXES 89-93,
        // SIZES 95-286 (190).
        assert_eq!(PROP_COLLIDER_SIZES.len(), 190);
        assert_eq!(PROP_COLLIDER_OFFSETS.len(), 73);
        // SIZES line 134
        assert_eq!(lookup(PROP_COLLIDER_SIZES, "diningChair"), Some([0.79, 1.41, 0.70]));
        // SIZES line 281
        assert_eq!(lookup(PROP_COLLIDER_SIZES, "woodenChair"), Some([0.79, 1.41, 0.79]));
        // OFFSETS line 16
        assert_eq!(lookup(PROP_COLLIDER_OFFSETS, "bunkBed"), Some([-0.09, 0.0]));
        // EXTRA_BOXES line 85: an L-shaped workstation with two boxes
        assert_eq!(PROP_COLLIDER_EXTRA_BOXES.len(), 1);
        assert_eq!(
            lookup(PROP_COLLIDER_EXTRA_BOXES, "officeCornerWorkstation"),
            Some(&[[0.09, 1.05, 2.86, 1.12], [0.78, -0.03, 1.48, 1.12]][..])
        );
        // VARIANT_BOXES lines 90 (diningChair, thirteen variants: the box changes at index 3), 92 (officeChair, five)
        assert_eq!(PROP_COLLIDER_VARIANT_BOXES.len(), 3);
        let dining = lookup(PROP_COLLIDER_VARIANT_BOXES, "diningChair").expect("diningChair variants");
        assert_eq!(dining.len(), 13);
        assert_eq!(dining[2], [0.0, 0.05, 0.79, 0.70]);
        assert_eq!(dining[3], [0.0, 0.0, 0.79, 0.60]);
        assert_eq!(lookup(PROP_COLLIDER_VARIANT_BOXES, "officeChair").map(|b| b.len()), Some(5));
    }

    #[test]
    fn room_size_profiles_match_godot() {
        // room_size_profiles.gd, PROFILES lines 12-119: 89 entries. The task brief said 12; the file has 89.
        assert_eq!(ROOM_SIZE_PROFILES.len(), 89);
        // line 14
        assert_eq!(profile("music_stage").preferred, [15, 13]);
        // line 27
        let dining = profile("restaurant_dining");
        assert_eq!(dining.min, [11, 8]);
        assert_eq!(dining.tables, [2, 12]);
        assert_eq!(dining.signature, &["restaurantPartyTable"]);
        assert_eq!(dining.seated, None);
        // line 52
        assert_eq!(profile("bedroom").signature, &["doubleBed", "singleBed", "bunkBed", "gothicDoubleBed"]);
        // line 67: seated false
        assert_eq!(profile("ward").seated, Some(false));
        // line 88: a server hall seats nobody but gets 200 table cells of room
        let hall = profile("server_hall");
        assert_eq!(hall.tables, [0, 200]);
        assert_eq!(hall.seated, Some(false));
        assert_eq!(hall.signature, &["serverRack"]);
    }

    #[test]
    fn orient_hide_and_mount_tables_match_godot() {
        // ORIENT_PAIRS lines 3331-3342: five pairs. The sofa rule (line 3336) reaches 4.5 m.
        assert_eq!(ORIENT_PAIRS.len(), 5);
        assert_eq!(ORIENT_PAIRS[1].child, &["Couch", "Sofa", "Chaise"]);
        assert_eq!(ORIENT_PAIRS[1].anchor, &["TvConsole", "CoffeeTable"]);
        assert_eq!(ORIENT_PAIRS[1].reach, 4.5);
        // line 3341
        assert_eq!(ORIENT_PAIRS[4].reach, 2.0);
        // line 3343
        assert_eq!(ORIENT_EXCLUDE, &["TvConsole"]);
        // line 3319
        assert_eq!(SCREEN_SEAT_REACH, 2.6);
        // HIDING_PROP_STYLES lines 5301-5304
        assert_eq!(HIDING_PROP_STYLES.len(), 5);
        assert_eq!(lookup(HIDING_PROP_STYLES, "bedroomWardrobe"), Some("wardrobe"));
        // lines 3250-3251
        assert_eq!(BED_TEMPLATES, &["bedroom", "gothic_bedroom"]);
        assert_eq!(FALLBACK_BEDS, &["singleBed", "bunkBed"]);
        // lines 3277-3278
        assert_eq!(lookup(CEILING_FIXTURE_FAMILY, "ceilingFan"), Some("fan"));
        assert_eq!(CEILING_FAMILY_ORDER, &["chandelier", "fan", "light"]);
        // prop_model_catalog.gd: EMPTY_FRAME_TYPES lines 522-526 (six); WALL_MOUNTED_TYPES lines 531-538 (the six
        // frames, then 24 more)
        assert_eq!(EMPTY_FRAME_TYPES.len(), 6);
        assert_eq!(WALL_MOUNTED_TYPES.len(), 30);
        assert_eq!(&WALL_MOUNTED_TYPES[..6], EMPTY_FRAME_TYPES);
        assert!(WALL_MOUNTED_TYPES.contains(&"onAirSign"));
    }

    #[test]
    fn generated_file_matches_generator() {
        // The generator must produce exactly the bytes in this file. Skipped (with a note) when node or the Godot
        // sources are not available on this machine.
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/parity/gen_prop_tables.mjs");
        let out = match std::process::Command::new("node").arg(&script).arg("--check").output() {
            Ok(out) => out,
            Err(e) => {
                eprintln!("skipped: node is not runnable ({e})");
                return;
            }
        };
        match out.status.code() {
            Some(0) => {}
            Some(2) => eprintln!(
                "skipped: Godot sources not found: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            _ => panic!(
                "prop_tables.rs is stale or the generator failed; run node tools/parity/gen_prop_tables.mjs\\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        }
    }
}
`;

function main() {
	const check = process.argv.includes('--check');
	const godotFile = (rel) => path.join(GODOT_APP, rel);
	const readSource = (rel) => {
		const p = godotFile(rel);
		if (!fs.existsSync(p)) {
			process.stderr.write(`Godot source not found: ${p} (set KT_GODOT_APP)\n`);
			process.exit(2);
		}
		return fs.readFileSync(p, 'utf8').replace(/\r\n/g, '\n');
	};

	const srg = new FileConsts(SRG, readSource(SRG));
	const sizes = new FileConsts(SIZES_FILE, readSource(SIZES_FILE));
	const profiles = new FileConsts(PROFILES_FILE, readSource(PROFILES_FILE));
	const catalog = new FileConsts(CATALOG_FILE, readSource(CATALOG_FILE));

	buildRegionPropSets(srg);
	buildTemplateSizes(srg);
	buildHunters(srg);
	buildRoomSizes(srg);
	buildLargeRooms(srg);
	buildOrientPairs(srg);
	buildMisc(srg);
	buildColliders(sizes);
	buildProfiles(profiles);
	buildWallMounted(catalog);

	const text = [HEADER, TYPES, ...sections, TESTS].join('\n').replace(/\n{3,}/g, '\n\n').replace(/\n*$/, '\n');

	if (check) {
		const current = fs.existsSync(OUT_PATH) ? fs.readFileSync(OUT_PATH, 'utf8') : null;
		if (current !== text) {
			process.stderr.write(`stale: ${OUT_PATH} differs from the generator output; run node tools/parity/gen_prop_tables.mjs\n`);
			process.exit(1);
		}
		process.stdout.write('prop_tables.rs is up to date\n');
		return;
	}
	fs.writeFileSync(OUT_PATH, text, 'utf8');
	process.stdout.write(`wrote ${OUT_PATH}\n`);
	for (const c of counts) process.stdout.write(`  ${c}\n`);
}

try {
	main();
} catch (e) {
	if (e instanceof GenError) {
		process.stderr.write(`gen_prop_tables: ${e.message}\n`);
		process.exit(1);
	}
	throw e;
}
