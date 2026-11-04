# Night Watch data layer

Every Night Watch tunable lives in JSON under `src/bin/night_watch/data/`. Code holds logic only. The files are embedded with `include_str!`, so the executable needs no data folder.

## Layout

```
src/bin/night_watch/data/
  rules/<mode>.json       all 9 modes: timings, AI tables, scoring, presets, Custom Night sliders
  cast/<mode>.json        classic, bedtime, dead_air, masked_hours, facility, open_air, mixed_shift
  buildings/<mode>.json   classic, bedtime, dead_air, masked_hours, facility, mixed_shift
  maps/<mode>.json        all 9 modes (floor plans, see Building maps)
  sounds.json             the nw_ cue table and the per-mode event-to-cue map (see Sounds)
```

Mode ids: `classic`, `bedtime`, `salvage_run`, `descent`, `masked_hours`, `dead_air`, `facility`, `open_air`, `mixed_shift`.

The older `assets/` folder (Godot rules, cast and building JSON for classic) and `calls/` (Zellia's scripts, loaded by `CallsData`) stay as they are. A mode agent migrating classic moves its `assets/*_classic.json` content into `data/` and deletes the copy.

## Schema rules

- Every file is a JSON object with `"mode": "<id>"` (must equal the file name) and `"version": 1`.
- Keys are `snake_case`. Times are milliseconds and keys end in `_ms`. Per-level tables are arrays indexed by level.
- Every tunable (timings, odds, counts, thresholds, labels, point values) is in JSON. If a number would appear in a mode file, it belongs here instead.
- A missing required key is a content bug, so the accessors panic naming the mode and the key.
- A driver's screen timings and on-screen text live in a `ui` block of its rules file (`ui.note_ms`, `ui.labels.<key>` with `{placeholder}` fills, `ui.stalkers`). Classic's driver reads its block this way; the other drivers move over as they are touched.

## Accessors (`data.rs`)

- `rules(mode_id)`, `cast(mode_id)`, `building(mode_id)` return `&'static Value`, parsed once (`OnceLock`).
- `num(v, "a.b")` gives `f64`, `int` gives `i64`, `text` gives `&str`, `list` gives `&[Value]`. Paths are dotted and may index arrays: `"doors.grace_ms.2"`.

```rust
let r = data::rules("classic");
let night_ms = data::num(r, "night_ms");
let grace = data::num(r, &format!("door_grace_ms.{level}"));
```

## Adding a mode

1. Add `data/rules/<mode>.json` (plus `cast`, `buildings` if needed) with `mode` and `version`.
2. Add one line per file to the matching `*_FILES` array in `data.rs` and bump its length. The registry test fails if a file does not parse or names the wrong mode.
3. Fill the JSON, then read it through the accessors from `modes/<mode>.rs`.

The registry already lists every current file, so converting a mode only edits its own JSON and its own `modes/` file.

## Building maps (`building_map.rs`)

`data/maps/<mode>.json` (schema `nw.map/1`, all 9 modes) holds each mode's whole building as a floor plan. It sits apart from `data/buildings/`, which is the sim graph the modes read. `building_map::load("classic")` gives a `BuildingMap`:

- `floors`, `rooms` (id, label, `rect` or `poly` in metres with z running south, `kind` room/hall/door/vent/office/outdoor, optional `alias` for the Godot name), `links` (a, b, `kind` door/corridor/vent/stairs/path/window, `walk` false for a window, `via` drawing waypoints), `cameras` (id, `covers` rooms), `offices` (one, or three for Open Air) and `spawns` (default hunter markers).
- Room ids are the sim ids of `data/buildings/<mode>.json` where it exists (Mixed Shift keeps Classic's ids, with `alias` giving the transit-hub name), so a mode can feed its hunter positions straight in. A room is `live` when a camera covers it, it is the office, or the map has no cameras; other rooms are drawn dimmed.
- Queries: `neighbours`, `rooms_adjacent`, `link_between`, `room_at(floor, point)`, `path` and `path_where`, `camera_for_room`, `office`, `is_live`, `door_between`.
- Drawing: `scene(area, &MapView)` makes primitives, `draw` paints them with macroquad and `to_svg` writes the same picture. `MapView` takes the active camera, the `(room, hunter id)` markers, `floor` (None draws every floor side by side, Some(i) shows one floor as a tab) and the legend flag. `room_at_screen` and `camera_at_screen` turn clicks into rooms and cameras.
- Screenshots: `NW_MAP_SVG_DIR=<folder> cargo test --bin night_watch svg_screenshots` writes `<mode>.svg` (and `<mode>_<floor>.svg` for multi-floor maps) to open in a browser.
- The tests check adjacency, camera coverage, link endpoints, the office, room overlap, that every room reaches the office, and that the maps still hold the edges and camera numbers of the mode data.

## Sounds (`data/sounds.json`, `nw_sound.rs`)

Schema `nw.sounds/1`. Night Watch cannot import the game binary's cue table, so the `nw_` cues are data here and `nw_sound.rs` renders them to a mono 16-bit WAV (same WAV writer as `blip.rs`) and plays them with macroquad audio.

- `sample_rate`, `master` and `tone_volume`: render rate and gain.
- `cues.<name>`: a list of layers. A layer is `{type: tone|noise, hz, ms, gain, wave (sine, square, triangle, saw), delay_ms, k}`; `k` marks a layer the caller's closeness value scales.
- `events.<mode>.<event>`: the cue a mode event plays. Classic has its block; each other mode adds one as its driver starts calling `nw_sound::event(mode, event, pan)`. A missing event or cue is silent with one log line, and `resolve` returns an error naming the missing key for the tests.
- API: `cue(name, pan, k)`, `event(mode, event, pan)`, `room_tone(name, on)`, and the pure `render(name, pan, k)` the tests use.
- To add a cue or a mapping, edit the JSON only; no code changes.
