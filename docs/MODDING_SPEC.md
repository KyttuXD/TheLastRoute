# Last Route modding spec (Rust port)

This spec is written from the parsers, not from the Godot notes. The sources are:

- `src/packs/regions.rs` for `lr.region/1` (region files; `content/regions.rs` re-exports it)
- `src/bin/last_route/content/packs.rs` for the pack folder layout and the local loader
- `src/bin/lr_server/packs/mod_packs.rs` for the server-side pack index and its limits
- `src/bin/lr_server/packs/pack_kinds.rs` for the list of allowed pack folders

Where a rule is only in the code and not in the module docs, the section says so. Section 6 lists places where the two sides disagree.

## 1. Status and versioning

- The region schema is `lr.region/1`. The `geometry` block is optional and was added to `/1` before the first full release, so `/1` stays the schema name until then. The next shape change is `lr.region/2`, added in both engines.
- The pack manifest schema is `lr.pack/1`.
- Built-in region files live in `src/bin/last_route/data/regions/` (apartment, beach, carnival, gothic, greenhouse, hospital, hotel, lab, mall, mansion, maze, prison, sewer) and are embedded at build time. The other data files are listed in [DATA_FILES.md](DATA_FILES.md).

## 2. Pack layout

A pack is a folder that contains a `pack.json`. The local loader reads these folders and nothing else.

### 2.1 Where packs load from

Packs load from these roots, in this order. A root that resolves to the same folder as an earlier root is skipped.

1. `mods/` beside the executable
2. `mods/` in the working folder
3. `mods/` under the per-user folder, which is `%APPDATA%\KYTTU\TheLastRoute\mods` on Windows (`shared::paths::user_dir`)

Within each root, pack folders load in name order. A folder without `pack.json` is ignored.

### 2.2 `pack.json`

| Key | Type | Required | Notes |
|---|---|---|---|
| `schema` | string | Required by the index | Must be `"lr.pack/1"`. The local loader accepts a missing schema, but the index refuses it (section 6). |
| `id` | string | Yes | Must be non-empty and unique across loaded packs. A duplicate id is rejected with a warning. Index rule: section 4. |
| `name` | string | No | Defaults to the id. |
| `version` | string | No | Free text in the local loader. Index rule: section 4. |
| `author` | string | No | |
| `description` | string | No | |
| `overrides` | array of strings | No | Lists the base content this pack replaces. Section 2.4. |

Example:

```json
{ "schema": "lr.pack/1", "id": "neon_halls", "name": "Neon Halls", "version": "1.0.0", "author": "KT", "overrides": ["regions/mall"] }
```

### 2.3 Content folders

Only the top-level `*.json` files in each folder are read. Subfolders are not scanned.

| Folder | Read by | Contents |
|---|---|---|
| `regions/` | `src/packs/regions.rs` | `lr.region/1` files. Section 3. |
| `hunters/` | `content/packs.rs` | `lr.hunter/1` files. A hunter may bring a `.bbmodel` with its atlas and a portrait PNG. A model in another format falls back to the base body. |
| `props/` | `content/packs.rs` | A `props` array. Model paths are pack-relative and are stored as `pack:<pack id>:<path>`. A model is a Blockbench `.bbmodel` (the format to use) or a legacy `.glb`; see section 2.6. |
| `loot/` | `content/packs.rs` | `lr.loot/1` files. A file with `"region": "<id>"` is that region's own loot table and needs no override (section 2.9). Any other file replaces the base table of the same `id` (`route`, `sewer`) only when `overrides` lists it. |

Other folders are not part of the local loader. The index accepts more folder kinds (section 4.4).


### 2.4 Adding and replacing content

- A pack adds new ids freely.
- A pack may replace an existing id only when that id is listed in its `overrides`. Without the listing, the second definition is dropped with a warning.
- Both forms of an `overrides` entry work, for every kind: `"<kind>/<id>"` (the form the index requires, for example `"hunters/mimic"`, `"props/officePrinter"`, `"loot/route"`, `"regions/mall"`) and the bare id (`"mimic"`). The kind in a `<kind>/<id>` entry must match the folder the file is in: `"props/aki"` does not replace the hunter `aki`.
- Replacing a region is covered in section 3.4.

### 2.5 Path safety

Pack file reads are resolved inside the pack folder. A path that canonicalises outside the pack folder is refused.

### 2.6 Prop models

A pack prop model is a Blockbench `.bbmodel`. It goes through the same loader as the built-in props (`src/bin/last_route/assets/props/*.bbmodel`, see [PROPS_BBMODEL.md](PROPS_BBMODEL.md)): cubes and mesh elements, the outliner groups the door and locker props use, and one texture per model. The texture is either embedded in the model as a `data:image/png;base64,` URI (what Blockbench writes), or a pack `.png` the model names in `relative_path` / `name`, looked up beside the model first. Put both the model and the png under `assets/`, and list both in `files`.

A `.glb` model still loads (static triangle meshes, base colour textures, vertex colours; no skins or animation), but it is the legacy format: convert it with `node tools/props/glb_to_bbmodel.mjs <in.glb> <out.bbmodel>`. A model in any other format is ignored. A `.bbmodel` is JSON and counts against the same 16 MiB per-file limit as every other file.

`--check-pack <folder>` accepts both extensions.

### 2.7 Hunter files: `lr.hunter/1`

One hunter per file in `hunters/`. A key that is left out takes the default below, or, for a replacement of a base hunter, the base hunter's value. The id must be unique unless `overrides` lists it (section 2.4). Source: `content/hunters.rs` `parse`.

| Key | Type | Default | Rule |
|---|---|---|---|
| `schema` | string | none | If present, must be `"lr.hunter/1"`. |
| `id` | string | none (required) | Lower case letters, digits and `_` only. |
| `name` | string | the id | Display name. |
| `tier` | string | `"easy"` for a new hunter | `"intermediate"`, `"difficult"`; any other text is the first tier. |
| `speed` | number | 0.8 | Above 0 and below 50. |
| `chase` | number | 3.55 | Above 0 and below 50. |
| `sight` | number | 8.0 | Above 0 and below 50. |
| `prog` | number | 0.18 | Progress rate. |
| `hearing` | number | 1.0 | Hearing range factor. |
| `spawn` | number | 12.0 | Spawn distance. |
| `chase_style` | string | `"pump"` | Chase movement style. |
| `taser_immune` | boolean | true | |
| `melee_immune` | boolean | false | |
| `taser_slow_factor` | number | none | Must be a number when present. |
| `taser_slow_seconds` | number | none | Must be a number when present. |
| `category` | string | `"az"` | Roster group. |
| `model` | string | none | Pack-relative `.bbmodel`. Another format falls back to the base body. |
| `atlas` | string | none | Pack-relative texture PNG. |
| `portrait` | string | none | Pack-relative portrait PNG. |

### 2.8 Prop files

`props/*.json` hold `{ "props": [ ... ] }`. A prop whose `id` is already in the catalogue needs `overrides`. Source: `content/props.rs` `parse_file`.

| Key | Type | Default | Rule |
|---|---|---|---|
| `id` | string | none (required) | An item without an id is skipped. |
| `models` | array of strings | none | Pack-relative `.bbmodel` (or legacy `.glb`) paths; stored as `pack:<pack id>:<path>`. |
| `light` | object | no light | Keys below. |
| `light.color` | string | `"ffffff"` | Hex, with or without `#`. |
| `light.energy` | number | 0.4 | |
| `light.range` | number | 3.5 | Metres. |
| `light.offset` | array of 3 numbers | `[0, 0.5, 0]` | Light position from the prop. |
| `light.flicker_hz` | number | 0 | Never below 0. |
| `light.flicker_depth` | number | 0 | Clamped to 0 to 1. |

### 2.9 Loot files: `lr.loot/1`

`loot/*.json`. Source: `content/loot.rs` `Loot::parse`. Fields: `id` (default `route`), `base` (array of item names), `rolls` (array of `{kind, chance}`), `tools` (array of `{item, weight}`), `backpacks` (array of item names), `aid` (array of `{item, chance}`).

- Without `region`, the file replaces the base table with the same `id` (`route` or `sewer`) and only when `overrides` lists it.
- With `"region": "<region id>"` (letters, digits and `_`, at most 64), the file is that region's own table. It needs no override. A region without a pack table rolls the game's table (`sewer` for the sewer region, `route` for the others). When two packs name the same region, the later pack wins. The region need not be a built-in one, so a pack region (section 3.4) can carry its own table. The game reads the table through `content::packs::region_loot(region)`.

### 2.10 Size limits, the download check and the validator

- Limits are data: `src/bin/last_route/data/packs/limits.json` (`lr.pack_limits/1`: `max_files`, `max_file_bytes`, `max_pack_bytes`, `max_total_bytes`, `max_listed`). The shipped numbers are the index's `MOD_LIMITS` (section 4). A pack folder over a limit is not loaded, and the warning names the limit.
- A file downloaded from the pack index is checked against its listed `size` and `sha256` before it is written or loaded (`content::packs::verify_download`). A wrong size or hash rejects the file and the pack. The hash is the lower case hex SHA-256.
- `last_route --validate-pack <folder>` loads the folder the way the game does, with no window. It prints `OK:` and what the pack adds, or one `ERROR:` line per problem (bad JSON, unknown hunter keys, an id that already exists without `overrides`, a bad region file, a limit passed, a pack that adds nothing), and exits 0 or 1. A new region id is reported as added, not as an error, although no level places it. The server has its own check, `lr_server --check-pack <folder>`.

## 3. Region files: `lr.region/1`

A region file holds only facts the data owns. The rules that place a region (`route::regions::region_for`, the sewer and Inferno levels, the layout classes) stay in code. A file cannot name a code path, because the schema has no key for one.

A region file is a JSON object. Any top-level key not in the table below is an error, and so is any missing required key. Every error names the file and the key.

### 3.1 Top-level keys

| Key | Type | Required | Rule | Example |
|---|---|---|---|---|
| `schema` | string | Yes | Must equal `"lr.region/1"` | `"schema": "lr.region/1"` |
| `id` | string | Yes | Non-empty, only ASCII letters, digits and `_` | `"id": "mall"` |
| `hall_floor` | array of strings | Yes | Floor texture names for hall tiles | `["terrazzo0", "terrazzo1"]` |
| `hall_runner` | boolean | Yes | Whether halls get a runner | `false` |
| `room_default_floor` | array of strings | Yes | Floor texture names for rooms | `["terrazzo0"]` |
| `room_roles` | array of strings | Yes | Room roles this region uses | `["donut_shop", "cinema"]` |
| `prop_set` | array of objects | Yes | Prop choices (section 3.2). May be empty. | `[{"type": "mallBench", "size": [1.8, 0.5, 0.6]}]` |
| `shop_names` | object of strings | No | Maps a room role to its sign text. Default: no signs. | `{"cinema": "CINEMA"}` |
| `room_door` | string | No | Door type for ordinary rooms | `"normalRoomDoor"` |
| `double_room_door` | string | No | Door type for double-width rooms | `"normalDoubleRoomDoor"` |
| `geometry` | object | No | Wall height and room sizes (section 3.3). Default: game-wide sizes. | `{"wall_height": 6.5}` |
| `safe_light` | object | No | Safe-zone checkpoint light (section 3.3). Default: colour `[0.95, 0.78, 0.55]`, energy 2.0. | `{"color": [0.2, 0.5, 1], "energy": 4.5}` |
| `accent` | object | No | Trim of hall pilasters and arches (section 3.3). Default: the maze's accent. | `{"texture": "floor", "kind": "metal0", "tint": "#c9a25a", "metal": true}` |

The `id` is not checked against the file name. Only the `id` inside the file matters.

### 3.2 `prop_set` items

Each item is an object. Any key other than the three below is an error.

| Key | Type | Required | Rule | Example |
|---|---|---|---|---|
| `type` | string | Yes | Non-empty. The parser does not check it against the prop catalogue. | `"hospitalBed"` |
| `size` | array of exactly 3 numbers | Yes | The blocker box in metres, `[x, y, z]` | `[0.95, 1.05, 2.1]` |
| `solid` | boolean | No | Default `true`. `false` means the prop does not block movement. | `"solid": false` |

### 3.3 `geometry` (optional)

The block is optional. A region without it uses the game defaults. Any key other than the three below is an error.

| Key | Type | Range | Default | Example |
|---|---|---|---|---|
| `wall_height` | number | 2.5 to 8.0 inclusive, in metres | 5.0 | `6.5` |
| `room_min` | whole number | 4 to 26 inclusive, in grid cells | 10 | `8` |
| `room_max` | whole number | 4 to 26 inclusive, in grid cells | 26 | `14` |

Rules for the sizes:

- A room's side is drawn from `room_min..=room_max`.
- If only `room_max` is given, `room_min` becomes the smaller of its default (10) and `room_max`. For example, `{"room_max": 6}` gives 6 to 6.
- If `room_min` is greater than `room_max`, the file is rejected.

Example:

```json
"geometry": { "wall_height": 6.5, "room_min": 8, "room_max": 14 }
```

`safe_light` (optional) sets the light at the region's safe-zone checkpoints. Any other key is an error. A key left out keeps its default, and a region without the block has the game's light.

| Key | Type | Range | Default |
|---|---|---|---|
| `color` | array of 3 numbers | each 0 to 1 (linear RGB) | `[0.95, 0.78, 0.55]` |
| `energy` | number | 0.2 to 6.0 | 2.0 |

`accent` (optional) sets the trim of the region's hall pilasters and arches: the pilaster caps and the keystones (the shafts and arch blocks take the hall's own wall). When the block is present, all four keys are required and any other key is an error. A region without the block uses the maze's accent.

| Key | Type | Rule |
|---|---|---|
| `texture` | string | Painted texture group: `floor`, `wall`, `ceiling` or `duct` |
| `kind` | string | A kind of that group, for example `metal0` |
| `tint` | string | Colour over the texture, written `#rrggbb` |
| `metal` | boolean | Whether the trim is metal (read, not yet used by the renderer) |

### 3.4 Pack region rules

A pack adds regions with a `regions/` folder. Each `regions/*.json` file is one region. The file name is free. Files are read in name order.

- A new id loads, but no level places it. The loader reports this as a warning. Only built-in regions are placed.
- A pack may replace a built-in region only when its `overrides` lists `"regions/<id>"`. Replacing a region the game does not have is an error.
- A region added by one pack cannot be redefined by another pack. The second pack is rejected.
- A replacement without the override listing is rejected.
- Any error in any region file of a pack rejects every region file of that pack. The pack then adds or replaces no regions. Each error is reported with the file name and key.
- A file may not define the same id twice in one pack.

### 3.5 Complete example

```json
{
  "schema": "lr.region/1",
  "id": "dungeon",
  "hall_floor": ["stone0"],
  "hall_runner": false,
  "room_default_floor": ["stone0"],
  "room_roles": ["cell", "armoury"],
  "shop_names": { "armoury": "ARMOURY" },
  "prop_set": [
    { "type": "crate", "size": [1.0, 1.0, 1.0] },
    { "type": "menuBoard", "size": [1.5, 0.9, 0.08], "solid": false }
  ],
  "geometry": { "wall_height": 4.0, "room_min": 6, "room_max": 12 }
}
```

This file is valid as a pack region, but `dungeon` would load without being placed by any level (section 3.4). Its `wall_height` of 4.0 is in range.

## 4. Pack index limits (`lr_server`)

The server-side index (`mod_packs.rs`, `validate_pack_manifest`) checks a submitted manifest before it is stored. A manifest it refuses is never listed. The values below are `MOD_LIMITS`.

### 4.1 Manifest keys

Allowed keys are `schema`, `id`, `name`, `version`, `author`, `description`, `game_version`, `overrides` and `files`. Any other key is refused (`UNKNOWN_PACK_FIELD`). `gameVersion` is accepted as an alias of `game_version`.

| Key | Rule |
|---|---|
| `schema` | Must be `"lr.pack/1"` |
| `id` | `^[a-z0-9][a-z0-9_]{2,31}$`. Lower case, digits and underscore, 3 to 32 characters, no hyphen. |
| `version` | `^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$`, and it may not contain `..` |
| `name` | Required, non-empty. Control characters become spaces, then the value is trimmed and cut to 120 characters. |
| `author` | Required, non-empty. Same cleaning, cut to 120 characters. |
| `description` | Same cleaning, cut to 1000 characters. Optional. |
| `game_version` | Same cleaning, cut to 32 characters. Optional. |
| `overrides` | An array of `<kind>/<id>` strings (section 4.3) |
| `files` | Required, a non-empty array (section 4.2) |

### 4.2 `files`

Each entry has:

| Field | Rule |
|---|---|
| `path` | Must match the path rule below. No `.` or `..` parts. Paths are compared case-insensitively, and a repeat is refused. |
| `sha256` | 64 lower-case hex characters (upper case is accepted and lower-cased on input) |
| `size` | A safe integer, zero or more, at most 16 MiB (16,777,216 bytes) |

Path rule. A path must be one of:

- `pack.json`
- `<kind>/<segment>.json`, where `<kind>` is one of the pack kinds in section 4.4
- `assets/<up to 6 folders>/<segment>.bbmodel`, `.glb`, `.png` or `.ogg` (`.bbmodel` is the model format; `.glb` is still accepted so older packs keep working)

Where `<segment>` is `[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}`.

Limits for the whole manifest:

- at most 512 files
- total size at most 128 MiB (134,217,728 bytes)
- `pack.json` must be one of the files

### 4.3 `overrides`

Each entry is either `<kind>/<id>` where the id is `[A-Za-z0-9_]{1,64}`, or `ambience/<id>`, where the id may also contain `:` (for example `ambience/room:chapel`). The colon may not be the first or last character.

### 4.4 Pack kinds

The allowed folder names come from the game's `PACK_KINDS` list. The server reads `pack_kinds.json` beside the backend, or the repo when it runs from a checkout, and falls back to this list when neither exists:

`props`, `regions`, `hunters`, `grabs`, `sounds`, `ambience`, `floors`, `walls`, `ceilings`, `rooms`, `deaths`, `emotes`

A kind name matches `^[a-z][a-z0-9_]{0,31}$`.

### 4.5 Index capacity and versions

- At most 2000 packs in the index. A new pack past that is refused with `INDEX_FULL` (HTTP 507).
- Each pack keeps at most 5 versions. Older versions drop out of the index. Their files stay on the download host until cleaned up.
- Registering a version that already exists is refused with `VERSION_EXISTS` (HTTP 409) unless the request sets `replace`.

### 4.6 Identity

A pack's `sha256` is the SHA-256 of the file lines `"<file sha256> <path>"`, sorted by path and joined with newlines. It covers every file, including `pack.json`. The client uses it as a cache key and in the room handshake.

### 4.7 Download links and the mods folder

- `LASTROUTE_MODS_BASE` is the public URL of the mod folder on VPS 2 (for example `https://mods.kyttuslab.org`). When it is set, each pack's `downloadBase` in the catalogue is `<base>/<pack_id>/<version>/`, and each file is `downloadBase + path`, checked against its listed `sha256` and `size`. A trailing slash is dropped. The older name `LASTROUTE_MOD_DOWNLOAD_BASE` is still read when `LASTROUTE_MODS_BASE` is unset. When neither is set, the catalogue has no download links and `downloadConfigured` is `false`.
- The folder layout is `<pack_id>/<version>/pack.json` plus the pack's files. `tools/mods/publish-index.mjs <modsRoot>` writes `index.json` at the root, listing every pack, version, file path, `sha256` and `size`. It writes only where it is told, and it never uploads. Run it with `--stdout` to review first. Test with `node --test tools/mods/publish-index.test.mjs`.

## 5. Keys checked against the source

Every key in this spec was taken from `src/packs/regions.rs` (`KEYS`, `REQUIRED`, `PROP_KEYS`, `GEOMETRY_KEYS`), `src/bin/last_route/content/packs.rs` (`load_source`), `content/hunters.rs` (`parse`), `content/props.rs` (`parse_file`) or `src/bin/lr_server/packs/mod_packs.rs` (`MANIFEST_KEYS`, `validate_pack_manifest`). Nothing here comes from the Godot files. The `geometry` ranges and defaults come from `WALL_HEIGHT_RANGE`, the `4..=MAX_AUTHORED_ROOM_SIZE` check, `config::WALL_HEIGHT` and `world::facility::MIN_AUTHORED_ROOM_SIZE` and `MAX_AUTHORED_ROOM_SIZE`.

## 6. Known mismatches

These are places where the local loader and the server index do not agree. A pack author should follow the stricter rule until they are fixed.

1. Override form. Fixed: the local loader now accepts both `<kind>/<id>` and the bare id for hunters, props and loot (section 2.4), as the index does. A bare id is still accepted locally; the index requires the `<kind>/<id>` form.
2. Schema. The local loader accepts a `pack.json` with no `schema` key. The index refuses a manifest without `"lr.pack/1"`.
3. Prop type names. The region parser checks that `prop_set[].type` is a non-empty string. It does not check the name against the prop catalogue, so a misspelt type loads without an error.

## 7. Importing a pack in the browser

The browser build has no mods folder. Its MODS tab has IMPORT FOLDER (choose the pack folder) and IMPORT .ZIP (choose one or more zip files). `content/pack_import.rs` checks each pack, keeps it in the page's IndexedDB (database `last_route_mods`) and loads it with the other packs when the page starts, so an import or a REMOVE shows up after a page reload.

- Manifest: the rules in section 4.1 (`schema` must be `lr.pack/1`; unknown keys, including `scripts`, are refused). `overrides` may use the index form (`hunters/<id>`) or the bare id the local loader compares.
- Limits: 512 files, 16 MiB a file, 128 MiB a pack (`MOD_LIMITS`), and 256 MiB of imported packs in all; the numbers are data (`src/bin/last_route/data/packs/limits.json`, section 2.10). A test in `pack_import.rs` compares these numbers and the pack kinds with `mod_packs.rs`.
- Files: `pack.json`, `<kind>/<name>.json`, and `.bbmodel`, `.glb`, `.png`, `.ogg` files up to 8 folders deep. Unlike the index, models and pictures need not sit under `assets/` (the sample pack keeps `hunters/mimic.bbmodel` beside its JSON). Other files (README.md, tools/) are ignored and reported.
- `regions/` is refused: the region loader reads folders only. Use the mods folder of the desktop game for region packs. (Open: the browser import does not read region files yet.)
- A pack is also loaded once against the base content and the packs already imported; a hunter that does not parse, an id that already exists without `overrides`, or a pack that adds nothing is refused with the loader's own words.
- A zip may hold stored or deflate entries (no zip64, no password); sizes are checked before anything is unpacked.
