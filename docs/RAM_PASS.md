# RAM pass

Goal: less memory before and during play. Started 2026-11-12 on KT-FW13 (Linux x86_64, Mesa, dev build).

## How to measure

- `sh tools/perf/mem.sh 12 target/debug/last_route --offline` prints resident memory each second (menu).
- `sh tools/perf/mem.sh 45 target/debug/last_route --offline --stress 40 --stress-scenario mixed` does it for a run.
- On Linux every `startup timing:` log line now ends with `rss N MB` (`app::perf::rss_mb`).

Set `LR_PROFILE_DIR` to a scratch folder so the real profile is not touched.

## Baseline

| Point | Resident memory |
|---|---|
| Window open, nothing loaded | 89 MB (about 60 MB of it is the Mesa driver: libLLVM 46 MB, libgallium 14 MB) |
| Boot card up, before `Content::load` | 133 to 181 MB (varies between runs) |
| After `Assets::load` | +83 MB |
| Main menu, settled | 210 to 216 MB (heap 78 MB, other anonymous 31 MB, the executable 32 MB) |
| 40 s `mixed` stress run, peak | 371 MB |

## Findings

1. **Prop models are the asset cost, not pictures.** All embedded PNGs decode to 3.1 MB. `load_models`
   (`shared/assets.rs`) parses all 978 `props/*.bbmodel` files (30 MB of JSON) at startup, whether or not the
   route ever shows them. That is most of the +83 MB.
2. **The boot card costs 45 to 90 MB before any game data loads** (`BootLoader::new` and its first frames: two
   system fonts and the baked card). Not yet broken down.
3. **Freed load buffers are a small part.** `app::perf::trim_heap` (glibc `malloc_trim`) after the load gives back
   about 6 MB. It is in, because it is free, but the memory above is live data.
4. Play adds about 160 MB over the menu. Not yet broken down (chunk meshes, hunters, the streaming window).

## Done

| Change | Menu | 40 s `mixed` stress peak | `Assets::load` |
|---|---|---|---|
| Baseline | 210 MB | 371 MB | 250 ms |
| Prop models built on first use (`ModelStore`, `shared/assets.rs`) | 190 MB | 292 MB | 77 ms |
| Fonts: each file loaded once, unused glyphs emptied, caption face on first use | 165 MB | 248 MB | |
| Identical embedded model pictures share one texture; mesh tiles and model parts trimmed to size | 165 MB | 240 MB | |
| Loaded sounds held as 16-bit samples instead of f32 (quad-snd patch) | 160 MB | 240 MB | |
| Glyph atlas row-wrap fix (vendored macroquad) | 144 MB | 217 MB | |

Average frame time in the stress run did not change (1.24 ms before, 1.17 ms after). A model is built the first time
`Assets::models.get` or `contains_key` asks for it; picking a prop's variant builds that prop type's variants only.

Numbers move by about 6 MB between runs; each row is the middle of two or three.

About 75 MB of every figure is not the game's to save on this PC: the Mesa driver's libraries (libLLVM 46 MB,
libgallium 14 MB) and the pages of the executable in use.

### Fonts (the largest single find)

fontdue builds every glyph's outline when a font loads, about 4 KB a glyph: a 2400-glyph face (Liberation, Courier
New) is about 10 MB. The game loaded five (two for the boot card, three in `Assets`), four of them two files twice.
Now `shared/assets.rs` loads each file once (`system_font`, used by the boot card too), `shared/font_subset.rs`
empties the outlines outside Latin, punctuation, arrows, maths and box drawing before the font is parsed, and the
italic caption face loads when a caption is first drawn. Metrics, kerning and glyph numbers are unchanged.
`LR_FULL_FONTS=1` loads whole fonts (Greek, Cyrillic), at about 8 MB a face.

### Textures

Shrinking the pictures would save almost nothing: every standalone PNG decodes to 3.1 MB in total, and the pictures
embedded in the models are 128x64 or smaller (two 512x512 spider skins and three 256x256 environment sheets aside).
The waste was duplication: 1047 models embed a picture and only 207 differ (26.5 MB decoded against 5.3 MB), so
equal pictures now share one texture (`PropModel::parse`, by content hash), which also joins their draw batches.

### What the heap holds during play (allocation profile, 60 s into a `mixed` stress run, before the last row)

| Owner | Live |
|---|---|
| World mesh tiles (`render/mesh_batch.rs::split_into_tiles`) | 51 MB |
| Fonts (after the change above) | 22 MB |
| Prop model geometry (`PropModel::parse`) | 18 MB |
| One font's glyph atlas, 2048x2048 RGBA (macroquad keeps the CPU copy) | 16 MB |
| The draw batch buffer (`geometry` in `submit`) | 10 MB |
| Decoded sounds (quad-snd keeps f32 samples) | 9.5 MB |
| Rig and model parse caches, chunk grids | about 10 MB |

`tools/perf/alloc_prof.rs` is the profiler that made this table (how to use it is in the file).

### Tried, no gain (2026-11-12)

- **Welding duplicate mesh vertices.** Counted over 3200 world meshes in a stress run: 2,063,220 vertices, 2,020,189
  distinct (2% duplicates), 1.5 indices a vertex. The meshes are plain quads with nothing to weld.
- **glibc allocator settings** (`MALLOC_ARENA_MAX`, mmap and trim thresholds): within run-to-run noise.
- **Fewer preloaded levels** (`data/stairs_links.json`). One level each way saved about 9 MB, but
  `the_levels_near_the_runner_are_all_kept_within_the_frame_budget` requires two levels below to stay built, and
  with that kept (one up, two down) the peak did not move. Left as it was.

## Next, in order of expected gain

- [ ] World mesh tiles (51 MB): the vertex is 40 bytes (position, UV, colour, a 16-byte normal+mode). Fewer chunks kept around the player, or a smaller vertex, are the two levers; both change rendering code and view distance.
- [ ] The glyph atlas still stores one-channel glyphs as RGBA and keeps a CPU copy. Small now that it no longer over-grows (see `vendor/macroquad/LAST_ROUTE_PATCHES.md`); a one-channel atlas would need shader changes.
- [ ] Check that the 110 MB dev executable (32 MB resident) shrinks in the release build, and measure release numbers.
- [ ] Repeat on the Intel Macs (no `/proc`; use `footprint` or Activity Monitor).
