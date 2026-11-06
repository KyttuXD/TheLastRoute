# Last Route modding spec (Rust port)

This spec is written from the parsers, not from the Godot notes. The sources are:

- `src/bin/last_route/content/regions.rs` for `lr.region/1` (region files)
- `src/bin/last_route/content/packs.rs` for the pack folder layout and the local loader
- `src/bin/lr_server/mod_packs.rs` for the server-side pack index and its limits
- `src/bin/lr_server/pack_kinds.rs` for the list of allowed pack folders

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
| `regions/` | `content/regions.rs` | `lr.region/1` files. Section 3. |
| `hunters/` | `content/packs.rs` | `lr.hunter/1` files. A hunter may bring a `.bbmodel` with its atlas and a portrait PNG. A model in another format falls back to the base body. |
| `props/` | `content/packs.rs` | A `props` array. Model paths are pack-relative and are stored as `pack:<pack id>:<path>`. A model is a Blockbench `.bbmodel` (the format to use) or a legacy `.glb`; see section 2.6. |
| `loot/` | `content/packs.rs` | Only the file with `"id": "route"` is read, and only when `overrides` contains `"route"`. |

Other folders are not part of the local loader. The index accepts more folder kinds (section 4.4).


### 2.4 Adding and replacing content

- A pack adds new ids freely.
- A pack may replace an existing id only when that id is listed in its `overrides`. Without the listing, the second definition is dropped with a warning.
- Hunters and props are matched by their bare id, for example `"officePrinter"`. Regions are matched as `"regions/<id>"`. Loot is matched as `"route"` (section 6 covers the form the index uses).
- Replacing a region is covered in section 3.4.

### 2.5 Path safety

Pack file reads are resolved inside the pack folder. A path that canonicalises outside the pack folder is refused.

### 2.6 Prop models

A pack prop model is a Blockbench `.bbmodel`. It goes through the same loader as the built-in props (`src/bin/last_route/assets/props/*.bbmodel`, see [PROPS_BBMODEL.md](PROPS_BBMODEL.md)): cubes and mesh elements, the outliner groups the door and locker props use, and one texture per model. The texture is either embedded in the model as a `data:image/png;base64,` URI (what Blockbench writes), or a pack `.png` the model names in `relative_path` / `name`, looked up beside the model first. Put both the model and the png under `assets/`, and list both in `files`.

A `.glb` model still loads (static triangle meshes, base colour textures, vertex colours; no skins or animation), but it is the legacy format: convert it with `node tools/props/glb_to_bbmodel.mjs <in.glb> <out.bbmodel>`. A model in any other format is ignored. A `.bbmodel` is JSON and counts against the same 16 MiB per-file limit as every other file.

`--check-pack <folder>` accepts both extensions.

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

Every key in this spec was taken from `src/bin/last_route/content/regions.rs` (`KEYS`, `REQUIRED`, `PROP_KEYS`, `GEOMETRY_KEYS`), `src/bin/last_route/content/packs.rs` (`load_pack`) or `src/bin/lr_server/mod_packs.rs` (`MANIFEST_KEYS`, `validate_pack_manifest`). Nothing here comes from the Godot files. The `geometry` ranges and defaults come from `WALL_HEIGHT_RANGE`, the `4..=MAX_AUTHORED_ROOM_SIZE` check, `config::WALL_HEIGHT` and `world::facility::MIN_AUTHORED_ROOM_SIZE` and `MAX_AUTHORED_ROOM_SIZE`.

## 6. Known mismatches

These are places where the local loader and the server index do not agree. A pack author should follow the stricter rule until they are fixed.

1. Override form. The index requires `props/<id>` and `hunters/<id>`, and the test suite uses `"props/officePrinter"`. The local loader compares props and hunters by bare id (`officePrinter`). A pack that lists the `kind/id` form passes the index, but the local loader then reports the replacement as "already exists". Regions use `regions/<id>` in both places.
2. Schema. The local loader accepts a `pack.json` with no `schema` key. The index refuses a manifest without `"lr.pack/1"`.
3. Prop type names. The region parser checks that `prop_set[].type` is a non-empty string. It does not check the name against the prop catalogue, so a misspelt type loads without an error.
